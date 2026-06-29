use mysql_async::prelude::*;
use mysql_async::{Opts, Pool};
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::config::Config;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct DbProxy;

struct ConnectionState {
    statements: HashMap<u32, String>,
    next_stmt_id: u32,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            statements: HashMap::new(),
            next_stmt_id: 1,
        }
    }
}

impl DbProxy {
    pub async fn start(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        if !config.db_proxy.enabled {
            return Ok(());
        }

        let db_config = &config.db_proxy;
        let addr = format!("{}:{}", db_config.listen_address, db_config.listen_port);

        if db_config.listen_address != "127.0.0.1" && db_config.listen_address != "localhost" {
            return Err("SECURITY ERROR: DB Proxy MUST bind to 127.0.0.1!".into());
        }

        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            db_config.target.username,
            db_config.target.password,
            db_config.target.host,
            db_config.target.port,
            db_config.target.database
        );
        
        let opts = Opts::from_url(&url)?;
        let pool = Pool::new(opts);
        
        let mut server_version = db_config.spoof_version.clone();
        
        if let Ok(mut conn) = pool.get_conn().await {
            if let Ok(Some(real_version)) = conn.query_first::<String, _>("SELECT VERSION()").await {
                server_version = format!("{}-bakpiarun-proxy", real_version);
                println!("Dynamic Version Detection: Backend is {}", real_version);
            } else {
                println!("Version query returned empty. Using YAML fallback: {}", server_version);
            }
        } else {
            println!("Could not connect to backend for version check. Using YAML fallback: {}", server_version);
        }

        println!("Secure DB Proxy initialized on {} (Spoofing as: {})", addr, server_version);

        let listener = TcpListener::bind(&addr).await?;

        loop {
            let (mut socket, client_addr) = listener.accept().await?;

            if !client_addr.ip().is_loopback() {
                println!("[SECURITY] Blocked remote DB connection from {}", client_addr);
                continue;
            }

            let pool = pool.clone();
            let version = server_version.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_client(&mut socket, pool, version).await {
                    eprintln!("Proxy Error: {}", e);
                }
            });
        }
    }

    async fn handle_client(
        socket: &mut tokio::net::TcpStream, 
        pool: Pool,
        server_version: String
    ) -> Result<(), Box<dyn std::error::Error>> {
        
        let conn_state = Arc::new(Mutex::new(ConnectionState::new()));
        
        let scramble = Self::generate_scramble();
        let handshake = Self::build_handshake_v10(&scramble, &server_version);
        
        socket.write_all(&handshake).await?;
        socket.flush().await?;

        let mut header = [0u8; 4];
        socket.read_exact(&mut header).await?;
        let payload_len = (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
        let mut auth_payload = vec![0u8; payload_len];
        socket.read_exact(&mut auth_payload).await?;

        let ok_packet = Self::build_ok_packet_after_handshake(2);
        socket.write_all(&ok_packet).await?;
        socket.flush().await?;
        
        println!("PHP Client connected & authenticated (Credential Masked)");

        loop {
            let mut header = [0u8; 4];
            match socket.read_exact(&mut header).await {
                Ok(_) => {},
                Err(_) => break,
            }

            let payload_len = (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
            let seq_id = header[3];

            if payload_len == 0 { continue; }

            let mut payload = vec![0u8; payload_len];
            socket.read_exact(&mut payload).await?;

            let command = payload[0];

            match command {
                0x03 => {
                    Self::handle_com_query(socket, seq_id, &payload, &pool).await?;
                }
                0x09 => {
                    Self::handle_com_stmt_prepare(socket, seq_id, &payload, &conn_state).await?;
                }
                0x17 => {
                    Self::handle_com_stmt_execute(socket, seq_id, &payload, &pool, &conn_state).await?;
                }
                0x19 => {
                    Self::handle_com_stmt_close(&payload, &conn_state).await?;
                }
                0x16 => {
                    println!("Client sent COM_STMT_RESET");
                    socket.write_all(&Self::build_ok_packet(seq_id + 1, 0, 0)).await?;
                    socket.flush().await?;
                }
                0x02 => {
                    let db_name = String::from_utf8_lossy(&payload[1..]).to_string();
                    println!("Client sent COM_INIT_DB: {}", db_name);
                    socket.write_all(&Self::build_ok_packet(seq_id + 1, 0, 0)).await?;
                    socket.flush().await?;
                }
                0x01 => {
                    println!("Client sent COM_QUIT");
                    break;
                }
                0x0E => {
                    socket.write_all(&Self::build_ok_packet(seq_id + 1, 0, 0)).await?;
                    socket.flush().await?;
                }
                _ => {
                    println!("Unknown command: 0x{:02X}", command);
                }
            }
        }
        Ok(())
    }

    async fn handle_com_query(
        socket: &mut tokio::net::TcpStream,
        seq_id: u8,
        payload: &[u8],
        pool: &Pool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let query = String::from_utf8_lossy(&payload[1..]).to_string();
        println!("[COM_QUERY]: {}", query);

        match pool.get_conn().await {
            Ok(mut conn) => {
                match conn.query_iter(query.as_str()).await {
                    Ok(mut result) => {
                        let mut current_seq = seq_id + 1;
                        let columns = result.columns().expect("Columns missing");
                        let col_count = columns.len() as u64;

                        if col_count > 0 {
                            socket.write_all(&Self::build_lenenc_int_packet(current_seq, col_count)).await?;
                            current_seq += 1;

                            for col in columns.iter() {
                                let name = col.name_str().to_string();
                                let col_type = col.column_type();
                                socket.write_all(&Self::build_column_def_packet(current_seq, &name, col_type)).await?;
                                current_seq += 1;
                            }

                            socket.write_all(&Self::build_eof_packet(current_seq)).await?;
                            current_seq += 1;

                            while let Some(row) = result.next().await? {
                                let values: Vec<mysql_async::Value> = row.unwrap();
                                socket.write_all(&Self::build_row_packet(current_seq, &values)).await?;
                                current_seq += 1;
                            }

                            socket.write_all(&Self::build_eof_packet(current_seq)).await?;
                        } else {
                            let affected_rows = result.affected_rows();
                            let last_insert_id = result.last_insert_id().unwrap_or(0);
                            socket.write_all(&Self::build_ok_packet(current_seq, affected_rows, last_insert_id)).await?;
                        }
                    }
                    Err(e) => {
                        socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &format!("{}", e))).await?;
                    }
                }
            }
            Err(e) => {
                socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &format!("{}", e))).await?;
            }
        }
        socket.flush().await?;
        Ok(())
    }

    async fn handle_com_stmt_prepare(
        socket: &mut tokio::net::TcpStream,
        seq_id: u8,
        payload: &[u8],
        conn_state: &Arc<Mutex<ConnectionState>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let query = String::from_utf8_lossy(&payload[1..]).to_string();
        println!("[COM_STMT_PREPARE]: {}", query);

        let mut state = conn_state.lock().await;
        let stmt_id = state.next_stmt_id;
        state.next_stmt_id += 1;
        state.statements.insert(stmt_id, query.clone());

        let num_params = query.matches('?').count() as u16;
        let num_columns: u16 = 0;

        let mut current_seq = seq_id + 1;
        
        // Build STMT_PREPARE_OK packet (12 bytes payload)
        let mut prepare_ok = Vec::new();
        prepare_ok.push(0x00); // Status OK
        prepare_ok.extend_from_slice(&stmt_id.to_le_bytes()); // 4 bytes
        prepare_ok.extend_from_slice(&num_columns.to_le_bytes()); // 2 bytes
        prepare_ok.extend_from_slice(&num_params.to_le_bytes()); // 2 bytes
        prepare_ok.push(0x00); // Reserved
        prepare_ok.extend_from_slice(&[0x00, 0x00]); // Warning count
        
        let packet = Self::build_packet(current_seq, &prepare_ok);
        println!("COM_STMT_PREPARE response: {} bytes (payload: {} bytes)", packet.len(), prepare_ok.len());
        
        socket.write_all(&packet).await?;
        current_seq += 1;

        // Send parameter definitions (if any)
        if num_params > 0 {
            for i in 0..num_params {
                socket.write_all(&Self::build_stmt_param_def_packet(current_seq, i as u32)).await?;
                current_seq += 1;
            }
            socket.write_all(&Self::build_eof_packet(current_seq)).await?;
            current_seq += 1;
        }

        // Send EOF for columns (num_columns = 0)
        socket.write_all(&Self::build_eof_packet(current_seq)).await?;

        println!("Prepared statement {} created (params: {})", stmt_id, num_params);
        
        socket.flush().await?;
        Ok(())
    }

    async fn handle_com_stmt_execute(
        socket: &mut tokio::net::TcpStream,
        seq_id: u8,
        payload: &[u8],
        pool: &Pool,
        conn_state: &Arc<Mutex<ConnectionState>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if payload.len() < 5 {
            socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, "Invalid COM_STMT_EXECUTE packet")).await?;
            socket.flush().await?;
            return Ok(());
        }

        let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
        println!("[COM_STMT_EXECUTE]: statement_id={}", stmt_id);

        let state = conn_state.lock().await;
        let query = match state.statements.get(&stmt_id) {
            Some(q) => q.clone(),
            None => {
                socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, "Statement not found")).await?;
                socket.flush().await?;
                return Ok(());
            }
        };
        let num_params = query.matches('?').count();
        drop(state);

        // Parse binary parameters
        let params = Self::parse_binary_parameters_safe(&payload[5..], num_params);
        
        // Substitute parameters into query
        let final_query = Self::substitute_parameters(&query, &params);
        println!("[EXECUTED AS]: {}", final_query);

        // Execute as COM_QUERY
        match pool.get_conn().await {
            Ok(mut conn) => {
                match conn.query_iter(final_query.as_str()).await {
                    Ok(mut result) => {
                        let mut current_seq = seq_id + 1;
                        let columns = result.columns().expect("Columns missing");
                        let col_count = columns.len() as u64;

                        if col_count > 0 {
                            socket.write_all(&Self::build_lenenc_int_packet(current_seq, col_count)).await?;
                            current_seq += 1;

                            for col in columns.iter() {
                                let name = col.name_str().to_string();
                                let col_type = col.column_type();
                                socket.write_all(&Self::build_column_def_packet(current_seq, &name, col_type)).await?;
                                current_seq += 1;
                            }

                            socket.write_all(&Self::build_eof_packet(current_seq)).await?;
                            current_seq += 1;

                            while let Some(row) = result.next().await? {
                                let values: Vec<mysql_async::Value> = row.unwrap();
                                socket.write_all(&Self::build_row_packet_binary(current_seq, &values)).await?;
                                current_seq += 1;
                            }

                            socket.write_all(&Self::build_eof_packet(current_seq)).await?;
                        } else {
                            let affected_rows = result.affected_rows();
                            let last_insert_id = result.last_insert_id().unwrap_or(0);
                            socket.write_all(&Self::build_ok_packet(current_seq, affected_rows, last_insert_id)).await?;
                        }
                    }
                    Err(e) => {
                        socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &format!("{}", e))).await?;
                    }
                }
            }
            Err(e) => {
                socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &format!("{}", e))).await?;
            }
        }
        socket.flush().await?;
        Ok(())
    }

    async fn handle_com_stmt_close(
        payload: &[u8],
        conn_state: &Arc<Mutex<ConnectionState>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if payload.len() >= 5 {
            let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            println!("[COM_STMT_CLOSE]: statement_id={}", stmt_id);
            
            let mut state = conn_state.lock().await;
            state.statements.remove(&stmt_id);
        }
        Ok(())
    }

    fn parse_binary_parameters_safe(payload: &[u8], num_params: usize) -> Vec<String> {
        if num_params == 0 || payload.len() < 6 {
            return vec![];
        }

        let mut params = Vec::new();
        let mut offset = 5;

        let bitmap_len = (num_params + 7) / 8;
        if payload.len() < offset + bitmap_len {
            return vec!["NULL".to_string(); num_params];
        }
        let null_bitmap = &payload[offset..offset + bitmap_len];
        offset += bitmap_len;

        if payload.len() <= offset {
            return vec!["NULL".to_string(); num_params];
        }
        let new_params_bound = payload[offset] == 1;
        offset += 1;

        let mut types = vec![0xfd; num_params];
        if new_params_bound && payload.len() >= offset + num_params * 2 {
            for i in 0..num_params {
                types[i] = payload[offset + i * 2];
            }
            offset += num_params * 2;
        }

        for i in 0..num_params {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if null_bitmap[byte_idx] & (1 << bit_idx) != 0 {
                params.push("NULL".to_string());
                continue;
            }

            if offset >= payload.len() {
                params.push("NULL".to_string());
                continue;
            }

            let param_type = types[i];
            match param_type {
                0x01 => {
                    if offset < payload.len() {
                        let val = payload[offset] as i8;
                        params.push(val.to_string());
                        offset += 1;
                    } else {
                        params.push("NULL".to_string());
                    }
                }
                0x02 | 0x03 => {
                    if offset + 4 <= payload.len() {
                        let val = i32::from_le_bytes([
                            payload[offset],
                            payload[offset + 1],
                            payload[offset + 2],
                            payload[offset + 3],
                        ]);
                        params.push(val.to_string());
                        offset += 4;
                    } else {
                        params.push("NULL".to_string());
                    }
                }
                0x08 => {
                    if offset + 8 <= payload.len() {
                        let val = i64::from_le_bytes([
                            payload[offset],
                            payload[offset + 1],
                            payload[offset + 2],
                            payload[offset + 3],
                            payload[offset + 4],
                            payload[offset + 5],
                            payload[offset + 6],
                            payload[offset + 7],
                        ]);
                        params.push(val.to_string());
                        offset += 8;
                    } else {
                        params.push("NULL".to_string());
                    }
                }
                _ => {
                    let (val, new_offset) = Self::read_lenenc_string_safe(&payload[offset..]);
                    params.push(format!("'{}'", val.replace("'", "''")));
                    offset = new_offset;
                }
            }
        }

        params
    }

    fn read_lenenc_string_safe(data: &[u8]) -> (String, usize) {
        if data.is_empty() {
            return ("".to_string(), 0);
        }

        let first_byte = data[0];
        let (len, offset) = if first_byte < 251 {
            (first_byte as usize, 1)
        } else if first_byte == 0xfc && data.len() >= 3 {
            ((data[1] as usize) | ((data[2] as usize) << 8), 3)
        } else if first_byte == 0xfd && data.len() >= 4 {
            ((data[1] as usize) | ((data[2] as usize) << 8) | ((data[3] as usize) << 16), 4)
        } else {
            return ("".to_string(), 0);
        };

        if data.len() < offset + len {
            return ("".to_string(), data.len());
        }

        let val = String::from_utf8_lossy(&data[offset..offset + len]).to_string();
        (val, offset + len)
    }

    fn substitute_parameters(query: &str, params: &[String]) -> String {
        let mut result = query.to_string();
        for param in params {
            result = result.replacen('?', param, 1);
        }
        result
    }

    fn generate_scramble() -> Vec<u8> {
        use std::time::SystemTime;
        let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos() as u64;
        let mut scramble = Vec::with_capacity(20);
        for i in 0..20 {
            scramble.push(((seed >> (i % 8)) & 0xFF) as u8);
        }
        scramble
    }

    fn build_packet(seq_id: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut packet = Vec::with_capacity(4 + len);
        packet.push((len & 0xFF) as u8);
        packet.push(((len >> 8) & 0xFF) as u8);
        packet.push(((len >> 16) & 0xFF) as u8);
        packet.push(seq_id);
        packet.extend_from_slice(payload);
        packet
    }

    fn build_handshake_v10(scramble: &[u8], version: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(10);
        let version_str = format!("{}\0", version);
        payload.extend_from_slice(version_str.as_bytes());
        payload.extend_from_slice(&[1, 0, 0, 0]);
        payload.extend_from_slice(&scramble[..8]);
        payload.push(0);
        payload.extend_from_slice(&[0xFF, 0xC1]);
        payload.push(0x21);
        payload.extend_from_slice(&[0x02, 0x00]);
        payload.extend_from_slice(&[0x00, 0x80]);
        payload.push(21);
        payload.extend_from_slice(&[0; 10]);
        payload.extend_from_slice(&scramble[8..20]);
        payload.push(0);
        payload.extend_from_slice(b"mysql_native_password\0");
        Self::build_packet(0, &payload)
    }

    fn build_ok_packet(seq_id: u8, affected_rows: u64, last_insert_id: u64) -> Vec<u8> {
        let mut payload = vec![0x00];
        payload.extend_from_slice(&Self::lenenc_int(affected_rows));
        payload.extend_from_slice(&Self::lenenc_int(last_insert_id));
        payload.extend_from_slice(&[0x02, 0x00]);
        payload.extend_from_slice(&[0x00, 0x00]);
        Self::build_packet(seq_id, &payload)
    }

    fn build_ok_packet_after_handshake(seq_id: u8) -> Vec<u8> {
        let mut payload = vec![0x00];
        payload.extend_from_slice(&Self::lenenc_int(0));
        payload.extend_from_slice(&Self::lenenc_int(0));
        payload.extend_from_slice(&[0x02, 0x00]);
        payload.extend_from_slice(&[0x00, 0x00]);
        payload.extend_from_slice(&Self::lenenc_str(b""));
        Self::build_packet(seq_id, &payload)
    }

    fn build_error_packet(seq_id: u8, code: u16, msg: &str) -> Vec<u8> {
        let mut payload = vec![0xff];
        payload.push((code & 0xFF) as u8);
        payload.push(((code >> 8) & 0xFF) as u8);
        payload.extend_from_slice(b"#HY000");
        payload.extend_from_slice(msg.as_bytes());
        Self::build_packet(seq_id, &payload)
    }

    fn build_eof_packet(seq_id: u8) -> Vec<u8> {
        let payload = vec![0xfe, 0x00, 0x00, 0x02, 0x00];
        Self::build_packet(seq_id, &payload)
    }

    fn build_lenenc_int_packet(seq_id: u8, val: u64) -> Vec<u8> {
        let payload = Self::lenenc_int(val);
        Self::build_packet(seq_id, &payload)
    }

    fn build_column_def_packet(seq_id: u8, name: &str, col_type: mysql_async::consts::ColumnType) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&Self::lenenc_str(b"def"));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(name.as_bytes()));
        payload.extend_from_slice(&Self::lenenc_str(name.as_bytes()));
        payload.push(0x0c);
        payload.extend_from_slice(&[0x21, 0x00]);
        payload.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        payload.push(Self::map_column_type(col_type));
        payload.extend_from_slice(&[0x00, 0x00]);
        payload.push(0x00);
        payload.extend_from_slice(&[0x00, 0x00]);
        Self::build_packet(seq_id, &payload)
    }

    fn build_row_packet(seq_id: u8, values: &[mysql_async::Value]) -> Vec<u8> {
        let mut payload = Vec::new();
        for val in values {
            match val {
                mysql_async::Value::NULL => payload.push(0xfb),
                mysql_async::Value::Bytes(b) => payload.extend_from_slice(&Self::lenenc_str(b)),
                mysql_async::Value::Int(i) => payload.extend_from_slice(&Self::lenenc_str(i.to_string().as_bytes())),
                mysql_async::Value::UInt(u) => payload.extend_from_slice(&Self::lenenc_str(u.to_string().as_bytes())),
                mysql_async::Value::Float(f) => payload.extend_from_slice(&Self::lenenc_str(f.to_string().as_bytes())),
                mysql_async::Value::Double(d) => payload.extend_from_slice(&Self::lenenc_str(d.to_string().as_bytes())),
                _ => payload.extend_from_slice(&Self::lenenc_str(b"?")),
            }
        }
        Self::build_packet(seq_id, &payload)
    }

    fn build_row_packet_binary(seq_id: u8, values: &[mysql_async::Value]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0x00);
        let null_bitmap_len = (values.len() + 7) / 8;
        payload.extend_from_slice(&vec![0u8; null_bitmap_len]);
        
        for val in values {
            match val {
                mysql_async::Value::NULL => {},
                mysql_async::Value::Bytes(b) => payload.extend_from_slice(&Self::lenenc_str(b)),
                mysql_async::Value::Int(i) => payload.extend_from_slice(&i.to_le_bytes()),
                mysql_async::Value::UInt(u) => payload.extend_from_slice(&u.to_le_bytes()),
                mysql_async::Value::Float(f) => payload.extend_from_slice(&f.to_le_bytes()),
                mysql_async::Value::Double(d) => payload.extend_from_slice(&d.to_le_bytes()),
                _ => payload.extend_from_slice(&Self::lenenc_str(b"?")),
            }
        }
        Self::build_packet(seq_id, &payload)
    }

    fn build_stmt_param_def_packet(seq_id: u8, param_num: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&Self::lenenc_str(b"def"));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(b""));
        payload.extend_from_slice(&Self::lenenc_str(format!("?{}", param_num).as_bytes()));
        payload.extend_from_slice(&Self::lenenc_str(format!("?{}", param_num).as_bytes()));
        payload.push(0x0c);
        payload.extend_from_slice(&[0x21, 0x00]);
        payload.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        payload.push(0xfd);
        payload.extend_from_slice(&[0x00, 0x00]);
        payload.push(0x00);
        payload.extend_from_slice(&[0x00, 0x00]);
        Self::build_packet(seq_id, &payload)
    }

    fn lenenc_int(val: u64) -> Vec<u8> {
        if val < 251 {
            vec![val as u8]
        } else if val < 65536 {
            vec![0xfc, val as u8, (val >> 8) as u8]
        } else if val < 16777216 {
            vec![0xfd, val as u8, (val >> 8) as u8, (val >> 16) as u8]
        } else {
            vec![0xfe, val as u8, (val >> 8) as u8, (val >> 16) as u8, (val >> 24) as u8, (val >> 32) as u8, (val >> 40) as u8, (val >> 48) as u8, (val >> 56) as u8]
        }
    }

    fn lenenc_str(data: &[u8]) -> Vec<u8> {
        let mut res = Self::lenenc_int(data.len() as u64);
        res.extend_from_slice(data);
        res
    }

    fn map_column_type(t: mysql_async::consts::ColumnType) -> u8 {
        use mysql_async::consts::ColumnType;
        match t {
            ColumnType::MYSQL_TYPE_TINY => 1,
            ColumnType::MYSQL_TYPE_SHORT => 2,
            ColumnType::MYSQL_TYPE_LONG => 3,
            ColumnType::MYSQL_TYPE_FLOAT => 4,
            ColumnType::MYSQL_TYPE_DOUBLE => 5,
            ColumnType::MYSQL_TYPE_NULL => 6,
            ColumnType::MYSQL_TYPE_LONGLONG => 8,
            ColumnType::MYSQL_TYPE_INT24 => 9,
            ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => 10,
            ColumnType::MYSQL_TYPE_TIME => 11,
            ColumnType::MYSQL_TYPE_DATETIME | ColumnType::MYSQL_TYPE_TIMESTAMP => 12,
            ColumnType::MYSQL_TYPE_YEAR => 13,
            ColumnType::MYSQL_TYPE_VARCHAR | ColumnType::MYSQL_TYPE_VAR_STRING => 15,
            ColumnType::MYSQL_TYPE_BIT => 16,
            ColumnType::MYSQL_TYPE_JSON => 245,
            ColumnType::MYSQL_TYPE_NEWDECIMAL => 246,
            ColumnType::MYSQL_TYPE_ENUM => 247,
            ColumnType::MYSQL_TYPE_SET => 248,
            ColumnType::MYSQL_TYPE_BLOB | ColumnType::MYSQL_TYPE_TINY_BLOB | 
            ColumnType::MYSQL_TYPE_MEDIUM_BLOB | ColumnType::MYSQL_TYPE_LONG_BLOB => 252,
            ColumnType::MYSQL_TYPE_STRING => 253,
            ColumnType::MYSQL_TYPE_GEOMETRY => 255,
            _ => 253,
        }
    }
}
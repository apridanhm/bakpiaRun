use mysql_async::prelude::*;
use mysql_async::{Opts, Pool};
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::config::Config;

pub struct DbProxy;

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

        // 1. Setup Connection Pool to Real Backend
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
        
        // 2. DYNAMIC BACKEND FETCHING
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
        
        // ==========================================
        // MILESTONE 1: PERFECT HANDSHAKE
        // ==========================================
        
        let scramble = Self::generate_scramble();
        
        let handshake = Self::build_handshake_v10(&scramble, &server_version);
        
        println!("Sending handshake packet: {} bytes", handshake.len());
        
        socket.write_all(&handshake).await?;
        socket.flush().await?;

        // Read Client Auth Response
        let mut header = [0u8; 4];
        socket.read_exact(&mut header).await?;
        let payload_len = (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
        let mut auth_payload = vec![0u8; payload_len];
        socket.read_exact(&mut auth_payload).await?;

        println!("Received auth response: {} bytes", payload_len);
        
        // Parse capability flags
        if auth_payload.len() >= 4 {
            let client_caps = u32::from_le_bytes([
                auth_payload[0], auth_payload[1], auth_payload[2], auth_payload[3]
            ]);
            println!("Client capability flags: 0x{:08X}", client_caps);
        }

        // Send OK Packet (SEQUENCE ID = 2)
        let ok_packet = Self::build_ok_packet_after_handshake(2);
        
        socket.write_all(&ok_packet).await?;
        socket.flush().await?;
        
        println!("PHP Client connected & authenticated (Credential Masked)");

        // ==========================================
        // MILESTONE 2: QUERY LOOP
        // ==========================================
        let mut packet_count = 0;
        loop {
            let mut header = [0u8; 4];
            match socket.read_exact(&mut header).await {
                Ok(_) => {},
                Err(e) => {
                    println!("Client disconnected after {} packets: {}", packet_count, e);
                    break;
                }
            }

            let payload_len = (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
            let seq_id = header[3];

            packet_count += 1;

            if payload_len == 0 { 
                continue; 
            }

            let mut payload = vec![0u8; payload_len];
            socket.read_exact(&mut payload).await?;

            if payload[0] == 0x03 { // COM_QUERY
                let query = String::from_utf8_lossy(&payload[1..]).to_string();
                println!("[Query]: {}", query);

                match pool.get_conn().await {
                    Ok(mut conn) => {
                        match conn.query_iter(query.as_str()).await {
                            Ok(mut result) => {
                                let mut current_seq = seq_id + 1;
                                let columns = result.columns().expect("Columns missing");
                                let col_count = columns.len() as u64;

                                if col_count > 0 {
                                    // ==========================================
                                    // SELECT-LIKE QUERY: Return Result Set
                                    // ==========================================
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
                                    println!("[Success] SELECT processed: {} columns", col_count);
                                } else {
                                    // ==========================================
                                    // DML/ADMIN QUERY: Return OK Packet
                                    // ==========================================
                                    let affected_rows = result.affected_rows();
                                    let last_insert_id = result.last_insert_id().unwrap_or(0);
                                    
                                    socket.write_all(&Self::build_ok_packet(current_seq, affected_rows, last_insert_id)).await?;
                                    println!("[Success] DML executed. Affected: {}, Last ID: {}", affected_rows, last_insert_id);
                                }
                            }
                            Err(e) => {
                                let err_msg = format!("Query Error: {}", e);
                                socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &err_msg)).await?;
                                eprintln!("{}", err_msg);
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("Pool Error: {}", e);
                        socket.write_all(&Self::build_error_packet(seq_id + 1, 1105, &err_msg)).await?;
                    }
                }
                socket.flush().await?;
            } 
            else if payload[0] == 0x01 { // COM_QUIT
                println!("Client sent COM_QUIT");
                break;
            }
            else if payload[0] == 0x0E { // COM_PING
                println!("Client sent COM_PING");
                socket.write_all(&Self::build_ok_packet(seq_id + 1, 0, 0)).await?;
                socket.flush().await?;
            }
            else {
                println!("Unknown command: 0x{:02X}", payload[0]);
            }
        }
        Ok(())
    }

    // ==========================================
    // RAW BYTE BUILDERS
    // ==========================================

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
        
        payload.push(10); // Protocol version
        
        let version_str = format!("{}\0", version);
        payload.extend_from_slice(version_str.as_bytes());
        
        payload.extend_from_slice(&[1, 0, 0, 0]); // Connection ID
        payload.extend_from_slice(&scramble[..8]); // Auth Plugin Data Part 1
        payload.push(0); // Filler
        payload.extend_from_slice(&[0xFF, 0xC1]); // Capability Flags Lower
        payload.push(0x21); // Character Set (utf8_general_ci)
        payload.extend_from_slice(&[0x02, 0x00]); // Status Flags
        payload.extend_from_slice(&[0x00, 0x80]); // Capability Flags Upper
        payload.push(21); // Length of Auth Plugin Data
        payload.extend_from_slice(&[0; 10]); // Reserved
        payload.extend_from_slice(&scramble[8..20]); // Auth Plugin Data Part 2
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
        let mut payload = vec![0x00]; // OK header
        payload.extend_from_slice(&Self::lenenc_int(0)); // Affected rows
        payload.extend_from_slice(&Self::lenenc_int(0)); // Last insert ID
        payload.extend_from_slice(&[0x02, 0x00]); // Status flags (autocommit)
        payload.extend_from_slice(&[0x00, 0x00]); // Warnings
        payload.extend_from_slice(&Self::lenenc_str(b"")); // Info field (empty string)
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
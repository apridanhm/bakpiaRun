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

        // 1. SECURITY CHECK: Hanya izinkan localhost
        if db_config.listen_address != "127.0.0.1" && db_config.listen_address != "localhost" {
            return Err("SECURITY ERROR: DB Proxy MUST bind to 127.0.0.1!".into());
        }

        // 2. SETUP CONNECTION POOL (Ke Database Asli)
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
        
        println!(" Secure DB Proxy initialized on {}", addr);
        println!("   Target: {}:{}", db_config.target.host, db_config.target.port);
        println!("   Pool: {} - {} connections", db_config.pool.min_connections, db_config.pool.max_connections);

        // 3. START TCP LISTENER
        let listener = TcpListener::bind(&addr).await?;

        loop {
            let (mut socket, client_addr) = listener.accept().await?;

            // Security: Block non-localhost
            if !client_addr.ip().is_loopback() {
                println!("[SECURITY] Blocked remote DB connection from {}", client_addr);
                continue;
            }

            let pool = pool.clone();
            
            // Handle setiap koneksi PHP di thread terpisah
            tokio::spawn(async move {
                if let Err(e) = Self::handle_client(&mut socket, pool).await {
                    eprintln!(" Proxy Error: {}", e);
                }
            });
        }
    }

    async fn handle_client(
        socket: &mut tokio::net::TcpStream, 
        pool: Pool
    ) -> Result<(), Box<dyn std::error::Error>> {
        
        // --- TAHAP 1: HANDSHAKE (CREDENTIAL MASKING) ---
        // Kirim Greeting ke PHP (Pura-pura jadi MySQL Server)
        let greeting = Self::build_handshake_packet();
        socket.write_all(&greeting).await?;
        socket.flush().await?;

        // Baca response dari PHP (Laravel/WordPress)
        // PHP akan kirim username "dummy" dan password "kosong". KITA ABAIKAN.
        let mut auth_response = vec![0u8; 256];
        let n = socket.read(&mut auth_response).await?;
        auth_response.truncate(n);
        
        // Kirim "OK Packet" ke PHP. PHP akan mengira login berhasil!
        let ok_packet = Self::build_ok_packet(1);
        socket.write_all(&ok_packet).await?;
        socket.flush().await?;

        println!("PHP Client connected & authenticated (Credential Masked)");

        // --- TAHAP 2: QUERY LOOP ---
        loop {
            // Baca header packet (4 bytes: 3 byte length, 1 byte sequence)
            let mut header = [0u8; 4];
            if socket.read_exact(&mut header).await.is_err() {
                break; // Client disconnect
            }

            let length = (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
            let seq_id = header[3];

            if length == 0 { continue; }

            // Baca isi packet (Query)
            let mut payload = vec![0u8; length];
            socket.read_exact(&mut payload).await?;

            // Packet pertama biasanya byte command (0x03 = COM_QUERY)
            if payload[0] == 0x03 {
                let query = String::from_utf8_lossy(&payload[1..]);
                println!("[Query Received]: {}", query);

                // Eksekusi di Pool (Backend)
                // Note: Ini PoC. Nanti kita upgrade buat forward raw bytes & return result set.
                let mut conn = pool.get_conn().await?;
                let _: Vec<mysql_async::Row> = conn.query(query.as_ref()).await.unwrap_or_default();
                
                println!("[Query Executed] via Pool");

                // Kirim "OK Packet" balik ke PHP
                let ok_response = Self::build_ok_packet(seq_id + 1);
                socket.write_all(&ok_response).await?;
                socket.flush().await?;
            } else if payload[0] == 0x01 { // COM_QUIT
                break;
            }
        }

        Ok(())
    }

    // Helper: Buat MySQL Handshake Packet (Simplified for PoC)
    fn build_handshake_packet() -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(&[0, 0, 0]); // Placeholder length
        packet.push(0); // Sequence ID
        
        packet.push(10); // Protocol version 10
        packet.extend_from_slice(b"5.7.0-bakpiarun-proxy\0"); // Server version
        packet.extend_from_slice(&[1, 0, 0, 0]); // Connection ID
        packet.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // Auth plugin data part 1
        packet.push(0); // Filler
        packet.extend_from_slice(&[0xff, 0xf7]); // Capability flags (lower)
        packet.push(0x21); // Charset utf8_general_ci
        packet.extend_from_slice(&[0x02, 0x00]); // Status flags
        packet.extend_from_slice(&[0x00, 0x80]); // Capability flags (upper)
        packet.push(21); // Length of auth plugin data
        packet.extend_from_slice(&[0; 10]); // Reserved
        packet.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // Auth plugin data part 2
        packet.extend_from_slice(b"mysql_native_password\0");
        
        // Update length
        let len = packet.len() - 4;
        packet[0] = (len & 0xFF) as u8;
        packet[1] = ((len >> 8) & 0xFF) as u8;
        packet[2] = ((len >> 16) & 0xFF) as u8;
        
        packet
    }

    // Helper: Buat OK Packet
    fn build_ok_packet(seq_id: u8) -> Vec<u8> {
        vec![
            7, 0, 0, seq_id, // Length: 7, Seq ID
            0x00,       // OK header
            0x00, 0x00, // Affected rows
            0x00, 0x00, // Last insert ID
            0x02, 0x00, // Status flags
            0x00, 0x00  // Warnings
        ]
    }
}

pub mod mysql_protocol;

use crate::config::Config;
use mysql_async::prelude::*;
use mysql_async::{Conn, Opts, OptsBuilder, Params, Pool, PoolConstraints, PoolOpts};
use mysql_protocol::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

static NEXT_CLIENT_CONN_ID: AtomicU32 = AtomicU32::new(1);

pub struct DbProxy;

struct StoredStatement {
    query: String,
    num_params: usize,
    /// Cached `(field_type, is_unsigned)` descriptors from the initial `COM_STMT_EXECUTE`
    /// so subsequent executions with `new_params_bound == 0` decode accurately.
    param_types: Vec<(u8, bool)>,
    /// Streamed BLOB chunks sent via `COM_STMT_SEND_LONG_DATA` (`0x18`)
    long_data: HashMap<u16, Vec<u8>>,
}

const MAX_PREPARED_STMTS_PER_SESSION: usize = 4096;

struct SessionState {
    default_db: String,
    statements: HashMap<u32, StoredStatement>,
    next_stmt_id: u32,
    /// Backend connection pinned to this client while a transaction or stateful session setting is active
    pinned_conn: Option<Conn>,
    in_explicit_tx: bool,
    autocommit_disabled: bool,
    session_pinned: bool,
    db_switched: bool,
}

impl SessionState {
    fn new(default_db: String) -> Self {
        Self {
            default_db,
            statements: HashMap::new(),
            next_stmt_id: 1,
            pinned_conn: None,
            in_explicit_tx: false,
            autocommit_disabled: false,
            session_pinned: false,
            db_switched: false,
        }
    }

    fn status_flags(&self) -> u16 {
        let mut flags = 0u16;
        if !self.autocommit_disabled {
            flags |= 0x0002; // SERVER_STATUS_AUTOCOMMIT
        }
        if self.in_explicit_tx || self.autocommit_disabled {
            flags |= 0x0001; // SERVER_STATUS_IN_TRANS
        }
        flags
    }

    fn should_keep_pinned(&self) -> bool {
        self.in_explicit_tx || self.autocommit_disabled || self.session_pinned || self.db_switched
    }

    async fn reset_session(&mut self) {
        if let Some(mut conn) = self.pinned_conn.take() {
            if self.in_explicit_tx || self.autocommit_disabled {
                let _ = conn.query_drop("ROLLBACK").await;
            }
            if self.autocommit_disabled {
                let _ = conn.query_drop("SET autocommit = 1").await;
            }
            if self.session_pinned {
                let _ = conn.query_drop("UNLOCK TABLES").await;
            }
            if self.db_switched && !self.default_db.is_empty() {
                let safe_default = self.default_db.replace('`', "``");
                let _ = conn.query_drop(format!("USE `{}`", safe_default)).await;
            }
        }
        self.statements.clear();
        self.in_explicit_tx = false;
        self.autocommit_disabled = false;
        self.session_pinned = false;
        self.db_switched = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxIntent {
    Begin,
    CommitOrRollback,
    SetAutocommitOff,
    SetAutocommitOn,
    PinSession,
    Normal,
}

fn classify_sql_intent(sql: &str) -> TxIntent {
    let trimmed = sql.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
    let upper = trimmed.to_ascii_uppercase();

    if upper == "BEGIN"
        || upper.starts_with("BEGIN ")
        || upper.starts_with("BEGIN;")
        || upper.starts_with("START TRANSACTION")
    {
        return TxIntent::Begin;
    }

    if upper == "COMMIT"
        || upper.starts_with("COMMIT ")
        || upper.starts_with("COMMIT;")
        || upper == "ROLLBACK"
        || upper.starts_with("ROLLBACK ")
        || upper.starts_with("ROLLBACK;")
    {
        // Note: `ROLLBACK TO SAVEPOINT` stays inside the current transaction
        if upper.contains(" TO ") {
            return TxIntent::Normal;
        }
        return TxIntent::CommitOrRollback;
    }

    if upper.starts_with("SET ") {
        let compact: String = upper.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("AUTOCOMMIT=0") || compact.contains("AUTOCOMMIT=OFF") {
            return TxIntent::SetAutocommitOff;
        }
        if compact.contains("AUTOCOMMIT=1") || compact.contains("AUTOCOMMIT=ON") {
            return TxIntent::SetAutocommitOn;
        }
        if compact.contains("SET@") && !compact.contains("SET@@") {
            return TxIntent::PinSession;
        }
    }

    if upper.starts_with("LOCK TABLES")
        || upper.starts_with("CREATE TEMPORARY TABLE")
        || upper.starts_with("USE ")
    {
        return TxIntent::PinSession;
    }

    TxIntent::Normal
}

fn count_wire_packets(buf: &[u8]) -> u8 {
    let mut offset = 0usize;
    let mut count = 0u8;
    while offset + 4 <= buf.len() {
        let pkt_len =
            (buf[offset] as usize) | ((buf[offset + 1] as usize) << 8) | ((buf[offset + 2] as usize) << 16);
        count = count.wrapping_add(1);
        offset += 4 + pkt_len;
    }
    count.max(1)
}

fn extract_mysql_error(err: &mysql_async::Error) -> (u16, [u8; 5], String) {
    match err {
        mysql_async::Error::Server(srv) => {
            let mut state_bytes = *b"HY000";
            let raw = srv.state.as_bytes();
            if raw.len() == 5 {
                state_bytes.copy_from_slice(raw);
            }
            (srv.code, state_bytes, srv.message.clone())
        }
        other => (1105, *b"HY000", format!("{}", other)),
    }
}

impl DbProxy {
    pub async fn start(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        if !config.db_proxy.enabled {
            return Ok(());
        }

        let db_config = &config.db_proxy;
        let is_debug = config.logging.is_debug();
        let addr = format!("{}:{}", db_config.listen_address, db_config.listen_port);

        if db_config.listen_address != "127.0.0.1" && db_config.listen_address != "localhost" {
            return Err("SECURITY ERROR: DB Proxy MUST bind to 127.0.0.1!".into());
        }

        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            urlencoding::encode(&db_config.target.username),
            urlencoding::encode(&db_config.target.password),
            db_config.target.host,
            db_config.target.port,
            urlencoding::encode(&db_config.target.database)
        );

        let opts = Opts::from_url(&url)?;
        let constraints = PoolConstraints::new(
            db_config.pool.min_connections,
            db_config.pool.max_connections,
        )
        .unwrap_or_default();
        let pool_opts = PoolOpts::default().with_constraints(constraints);
        let builder = OptsBuilder::from_opts(opts).pool_opts(pool_opts);
        let pool = Pool::new(builder);

        let mut server_version = db_config.spoof_version.clone();

        if let Ok(mut conn) = pool.get_conn().await {
            if let Ok(Some(real_version)) = conn.query_first::<String, _>("SELECT VERSION()").await
            {
                server_version = format!("{}-bakpiarun-proxy", real_version);
                println!("[DB Proxy] Dynamic Version Detection: Backend is {}", real_version);
            } else {
                println!(
                    "[DB Proxy] Version query returned empty. Using YAML fallback: {}",
                    server_version
                );
            }
        } else {
            println!(
                "[DB Proxy] Could not connect to backend at startup check. Using YAML fallback: {}",
                server_version
            );
        }

        println!(
            "Secure DB Proxy initialized on {} (Spoofing as: {})",
            addr, server_version
        );

        let listener = TcpListener::bind(&addr).await?;

        loop {
            let (mut socket, client_addr) = listener.accept().await?;

            if !client_addr.ip().is_loopback() {
                println!(
                    "[SECURITY] Blocked non-loopback DB connection attempt from {}",
                    client_addr
                );
                continue;
            }

            let _ = socket.set_nodelay(true);
            let pool = pool.clone();
            let version = server_version.clone();
            let default_db = db_config.target.database.clone();
            let conn_id = NEXT_CLIENT_CONN_ID.fetch_add(1, Ordering::Relaxed);

            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_client(&mut socket, pool, version, default_db, conn_id, is_debug)
                        .await
                {
                    if is_debug {
                        eprintln!("[DB Proxy] Client #{} error: {}", conn_id, e);
                    }
                }
            });
        }
    }

    async fn handle_client(
        socket: &mut TcpStream,
        pool: Pool,
        server_version: String,
        default_db: String,
        conn_id: u32,
        is_debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut session = SessionState::new(default_db);

        // 1. Send Server HandshakeV10
        let scramble = generate_scramble(conn_id);
        let handshake = build_handshake_v10(conn_id, &scramble, &server_version);
        socket.write_all(&handshake).await?;
        socket.flush().await?;

        // 2. Read Client HandshakeResponse41
        let mut header = [0u8; 4];
        if socket.read_exact(&mut header).await.is_err() {
            return Ok(());
        }
        let payload_len =
            (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
        let auth_seq = header[3];
        let mut auth_payload = vec![0u8; payload_len];
        socket.read_exact(&mut auth_payload).await?;

        // 3. Reply OK_Packet (Credential Masking: local worker authenticated via proxy)
        let ok_packet = build_ok_packet(auth_seq.wrapping_add(1), 0, 0, session.status_flags());
        socket.write_all(&ok_packet).await?;
        socket.flush().await?;

        if is_debug {
            println!(
                "[DB Proxy] PHP Client #{} connected & authenticated (Credential Masked)",
                conn_id
            );
        }

        // 4. Command Loop
        loop {
            let mut header = [0u8; 4];
            if socket.read_exact(&mut header).await.is_err() {
                break;
            }

            let mut payload_len =
                (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
            let mut seq_id = header[3];

            if payload_len == 0 {
                continue;
            }

            let mut payload = vec![0u8; payload_len];
            socket.read_exact(&mut payload).await?;

            // Support MySQL multi-packet command payloads (> 16 MB split across 0xFF_FF_FF chunks)
            while payload_len == 0x00FF_FFFF {
                let mut next_hdr = [0u8; 4];
                socket.read_exact(&mut next_hdr).await?;
                payload_len = (next_hdr[0] as usize)
                    | ((next_hdr[1] as usize) << 8)
                    | ((next_hdr[2] as usize) << 16);
                seq_id = next_hdr[3];
                if payload_len > 0 {
                    let prev = payload.len();
                    payload.resize(prev + payload_len, 0);
                    socket.read_exact(&mut payload[prev..]).await?;
                }
            }

            let command = payload[0];

            match command {
                COM_QUIT => {
                    if is_debug {
                        println!("[DB Proxy] Client #{} sent COM_QUIT", conn_id);
                    }
                    break;
                }
                COM_PING => {
                    let pkt = build_ok_packet(seq_id.wrapping_add(1), 0, 0, session.status_flags());
                    socket.write_all(&pkt).await?;
                    socket.flush().await?;
                }
                COM_STATISTICS => {
                    let stats_msg = format!("BakpiaRun Secure DB Proxy | ConnID: {}", conn_id);
                    let pkt = build_packet(seq_id.wrapping_add(1), stats_msg.as_bytes());
                    socket.write_all(&pkt).await?;
                    socket.flush().await?;
                }
                COM_INIT_DB => {
                    let db_name = String::from_utf8_lossy(&payload[1..])
                        .trim_end_matches('\0')
                        .trim()
                        .to_string();
                    if is_debug {
                        println!("[DB Proxy] COM_INIT_DB: {}", db_name);
                    }
                    let valid_ident = !db_name.is_empty()
                        && db_name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                    if valid_ident {
                        // Execute real `USE \`db\`` on a pinned backend connection so schema switch actually takes effect!
                        let mut conn: Conn = match session.pinned_conn.take() {
                            Some(c) => c,
                            None => match pool.get_conn().await {
                                Ok(c) => c,
                                Err(e) => {
                                    let (code, state, msg) = extract_mysql_error(&e);
                                    socket
                                        .write_all(&build_error_packet(
                                            seq_id.wrapping_add(1),
                                            code,
                                            &state,
                                            &msg,
                                        ))
                                        .await?;
                                    socket.flush().await?;
                                    continue;
                                }
                            },
                        };
                        match conn.query_drop(format!("USE `{}`", db_name)).await {
                            Ok(_) => {
                                session.db_switched = db_name != session.default_db;
                                if session.should_keep_pinned() {
                                    session.pinned_conn = Some(conn);
                                }
                                let pkt = build_ok_packet(
                                    seq_id.wrapping_add(1),
                                    0,
                                    0,
                                    session.status_flags(),
                                );
                                socket.write_all(&pkt).await?;
                            }
                            Err(e) => {
                                if !matches!(e, mysql_async::Error::Io(_))
                                    && session.should_keep_pinned()
                                {
                                    session.pinned_conn = Some(conn);
                                }
                                let (code, state, msg) = extract_mysql_error(&e);
                                socket
                                    .write_all(&build_error_packet(
                                        seq_id.wrapping_add(1),
                                        code,
                                        &state,
                                        &msg,
                                    ))
                                    .await?;
                            }
                        }
                    } else {
                        let pkt = build_error_packet(
                            seq_id.wrapping_add(1),
                            1049,
                            b"42000",
                            "Unknown or invalid database name",
                        );
                        socket.write_all(&pkt).await?;
                    }
                    socket.flush().await?;
                }
                COM_CHANGE_USER | COM_RESET_CONNECTION => {
                    // Used by PHP persistent connections (`PDO::ATTR_PERSISTENT => true` / `mysqli` `p:`)
                    session.reset_session().await;
                    let pkt = build_ok_packet(seq_id.wrapping_add(1), 0, 0, session.status_flags());
                    socket.write_all(&pkt).await?;
                    socket.flush().await?;
                }
                COM_QUERY => {
                    Self::handle_com_query(socket, seq_id, &payload, &pool, &mut session, is_debug)
                        .await?;
                }
                COM_STMT_PREPARE => {
                    Self::handle_com_stmt_prepare(
                        socket,
                        seq_id,
                        &payload,
                        &pool,
                        &mut session,
                        is_debug,
                    )
                    .await?;
                }
                COM_STMT_EXECUTE => {
                    Self::handle_com_stmt_execute(
                        socket,
                        seq_id,
                        &payload,
                        &pool,
                        &mut session,
                        is_debug,
                    )
                    .await?;
                }
                COM_STMT_SEND_LONG_DATA => {
                    Self::handle_com_stmt_send_long_data(&payload, &mut session);
                }
                COM_STMT_CLOSE => {
                    Self::handle_com_stmt_close(&payload, &mut session, is_debug);
                }
                COM_STMT_RESET => {
                    Self::handle_com_stmt_reset(socket, seq_id, &payload, &mut session).await?;
                }
                unknown => {
                    if is_debug {
                        eprintln!(
                            "[DB Proxy] Unsupported MySQL command 0x{:02X} on client #{}",
                            unknown, conn_id
                        );
                    }
                    // Always reply ERR_Packet so PHP client never hangs indefinitely
                    let err_pkt = build_error_packet(
                        seq_id.wrapping_add(1),
                        1047,
                        b"08S01",
                        &format!("Unsupported command 0x{:02X}", unknown),
                    );
                    socket.write_all(&err_pkt).await?;
                    socket.flush().await?;
                }
            }
        }

        // Cleanup: reset any open uncommitted transaction, table lock, or DB switch before returning connection to pool
        session.reset_session().await;

        Ok(())
    }

    async fn handle_com_query(
        socket: &mut TcpStream,
        seq_id: u8,
        payload: &[u8],
        pool: &Pool,
        session: &mut SessionState,
        is_debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let query = String::from_utf8_lossy(&payload[1..]).to_string();
        if is_debug {
            println!("[DB Proxy] [COM_QUERY]: {}", query);
        }

        let intent = classify_sql_intent(&query);

        // Update pre-execution pinning intent
        match intent {
            TxIntent::Begin => session.in_explicit_tx = true,
            TxIntent::SetAutocommitOff => session.autocommit_disabled = true,
            TxIntent::PinSession => session.session_pinned = true,
            _ => {}
        }

        // Acquire either the session's pinned connection or a pooled connection
        let mut conn: Conn = match session.pinned_conn.take() {
            Some(c) => c,
            None => match pool.get_conn().await {
                Ok(c) => c,
                Err(e) => {
                    let (code, state, msg) = extract_mysql_error(&e);
                    socket
                        .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                        .await?;
                    socket.flush().await?;
                    return Ok(());
                }
            },
        };

        let mut io_broken = false;
        {
            let query_res = conn.query_iter(query.as_str()).await;

            // Update post-execution transaction state
            if query_res.is_ok() {
                match intent {
                    TxIntent::CommitOrRollback => {
                        session.in_explicit_tx = false;
                    }
                    TxIntent::SetAutocommitOn => {
                        session.autocommit_disabled = false;
                        session.in_explicit_tx = false;
                    }
                    _ => {}
                }
            }

            let status_flags = session.status_flags();

            match query_res {
                Ok(mut result) => {
                    let mut current_seq = seq_id.wrapping_add(1);
                    let columns: Vec<mysql_async::Column> = result
                        .columns()
                        .map(|c| c.as_ref().to_vec())
                        .unwrap_or_default();

                    if !columns.is_empty() {
                        let mut write_buf = Vec::with_capacity(8192);
                        write_buf.extend_from_slice(&build_lenenc_int_packet(
                            current_seq,
                            columns.len() as u64,
                        ));
                        current_seq = current_seq.wrapping_add(1);

                        for col in &columns {
                            write_buf
                                .extend_from_slice(&build_column_def_from_mysql(current_seq, col));
                            current_seq = current_seq.wrapping_add(1);
                        }

                        write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
                        current_seq = current_seq.wrapping_add(1);

                        while let Some(row) = result.next().await? {
                            let values = row.unwrap();
                            let row_pkt = build_text_row_packet(current_seq, &values);
                            current_seq = current_seq.wrapping_add(count_wire_packets(&row_pkt));
                            write_buf.extend_from_slice(&row_pkt);

                            if write_buf.len() >= 65536 {
                                socket.write_all(&write_buf).await?;
                                write_buf.clear();
                            }
                        }

                        write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
                        socket.write_all(&write_buf).await?;
                    } else {
                        let affected_rows = result.affected_rows();
                        let last_insert_id = result.last_insert_id().unwrap_or(0);
                        while (result.next().await?).is_some() {}
                        socket
                            .write_all(&build_ok_packet(
                                current_seq,
                                affected_rows,
                                last_insert_id,
                                status_flags,
                            ))
                            .await?;
                    }
                }
                Err(e) => {
                    if matches!(e, mysql_async::Error::Io(_)) {
                        io_broken = true;
                    }
                    let (code, state, msg) = extract_mysql_error(&e);
                    socket
                        .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                        .await?;
                }
            }
        }

        // Keep connection pinned if inside active transaction, otherwise drop back to pool
        if !io_broken && session.should_keep_pinned() {
            session.pinned_conn = Some(conn);
        }

        socket.flush().await?;
        Ok(())
    }

    async fn handle_com_stmt_prepare(
        socket: &mut TcpStream,
        seq_id: u8,
        payload: &[u8],
        pool: &Pool,
        session: &mut SessionState,
        is_debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let query = String::from_utf8_lossy(&payload[1..]).to_string();
        if is_debug {
            println!("[DB Proxy] [COM_STMT_PREPARE (0x16)]: {}", query);
        }

        // Prepare directly on backend via mysql_async to validate SQL syntax and obtain
        // exact parameter count & result column metadata (immune to '?' inside string literals!)
        let mut conn: Conn = match session.pinned_conn.take() {
            Some(c) => c,
            None => match pool.get_conn().await {
                Ok(c) => c,
                Err(e) => {
                    let (code, state, msg) = extract_mysql_error(&e);
                    socket
                        .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                        .await?;
                    socket.flush().await?;
                    return Ok(());
                }
            },
        };

        let backend_stmt = match conn.prep(query.as_str()).await {
            Ok(stmt) => stmt,
            Err(e) => {
                if !matches!(e, mysql_async::Error::Io(_)) && session.should_keep_pinned() {
                    session.pinned_conn = Some(conn);
                }
                let (code, state, msg) = extract_mysql_error(&e);
                socket
                    .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                    .await?;
                socket.flush().await?;
                return Ok(());
            }
        };

        if session.should_keep_pinned() {
            session.pinned_conn = Some(conn);
        }

        let num_params = backend_stmt.params().len() as u16;
        let columns: Vec<mysql_async::Column> = backend_stmt.columns().to_vec();
        let num_columns = columns.len() as u16;

        if session.statements.len() >= MAX_PREPARED_STMTS_PER_SESSION {
            socket
                .write_all(&build_error_packet(
                    seq_id.wrapping_add(1),
                    1461,
                    b"42000",
                    "Can't create more than max_prepared_stmt_count statements",
                ))
                .await?;
            socket.flush().await?;
            return Ok(());
        }

        let stmt_id = session.next_stmt_id;
        session.next_stmt_id = session.next_stmt_id.wrapping_add(1).max(1);
        session.statements.insert(
            stmt_id,
            StoredStatement {
                query,
                num_params: num_params as usize,
                param_types: Vec::new(),
                long_data: HashMap::new(),
            },
        );

        let status_flags = session.status_flags();
        let mut current_seq = seq_id.wrapping_add(1);
        let mut write_buf = Vec::with_capacity(512);

        // 1. COM_STMT_PREPARE_OK (12 bytes payload)
        let mut prepare_ok = Vec::with_capacity(12);
        prepare_ok.push(0x00); // status OK
        prepare_ok.extend_from_slice(&stmt_id.to_le_bytes()); // statement_id (4)
        prepare_ok.extend_from_slice(&num_columns.to_le_bytes()); // num_columns (2)
        prepare_ok.extend_from_slice(&num_params.to_le_bytes()); // num_params (2)
        prepare_ok.push(0x00); // reserved_1 (1)
        prepare_ok.extend_from_slice(&[0x00, 0x00]); // warning_count (2)

        write_buf.extend_from_slice(&build_packet(current_seq, &prepare_ok));
        current_seq = current_seq.wrapping_add(1);

        // 2. Parameter Definition Block + EOF (ONLY when num_params > 0)
        if num_params > 0 {
            for _ in 0..num_params {
                write_buf.extend_from_slice(&build_stmt_param_def_packet(current_seq));
                current_seq = current_seq.wrapping_add(1);
            }
            write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
            current_seq = current_seq.wrapping_add(1);
        }

        // 3. Column Definition Block + EOF (ONLY when num_columns > 0 — MySQL Spec!)
        if num_columns > 0 {
            for col in &columns {
                write_buf.extend_from_slice(&build_column_def_from_mysql(current_seq, col));
                current_seq = current_seq.wrapping_add(1);
            }
            write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
        }

        socket.write_all(&write_buf).await?;
        socket.flush().await?;
        Ok(())
    }

    async fn handle_com_stmt_execute(
        socket: &mut TcpStream,
        seq_id: u8,
        payload: &[u8],
        pool: &Pool,
        session: &mut SessionState,
        is_debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if payload.len() < 10 {
            socket
                .write_all(&build_error_packet(
                    seq_id.wrapping_add(1),
                    1210,
                    b"HY000",
                    "Malformed COM_STMT_EXECUTE packet",
                ))
                .await?;
            socket.flush().await?;
            return Ok(());
        }

        let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);

        let (query, param_values) = {
            let stored = match session.statements.get_mut(&stmt_id) {
                Some(s) => s,
                None => {
                    socket
                        .write_all(&build_error_packet(
                            seq_id.wrapping_add(1),
                            1243,
                            b"HY000",
                            "Unknown prepared statement handler given to mysqld_stmt_execute",
                        ))
                        .await?;
                    socket.flush().await?;
                    return Ok(());
                }
            };

            match decode_stmt_execute_params(
                payload,
                stored.num_params,
                &mut stored.param_types,
                &stored.long_data,
            ) {
                Ok(vals) => {
                    stored.long_data.clear();
                    (stored.query.clone(), vals)
                }
                Err(parse_err) => {
                    socket
                        .write_all(&build_error_packet(
                            seq_id.wrapping_add(1),
                            1210,
                            b"HY000",
                            &parse_err,
                        ))
                        .await?;
                    socket.flush().await?;
                    return Ok(());
                }
            }
        };

        if is_debug {
            println!(
                "[DB Proxy] [COM_STMT_EXECUTE (0x17)] stmt_id={} params={}",
                stmt_id,
                param_values.len()
            );
        }

        let intent = classify_sql_intent(&query);
        match intent {
            TxIntent::Begin => session.in_explicit_tx = true,
            TxIntent::SetAutocommitOff => session.autocommit_disabled = true,
            TxIntent::PinSession => session.session_pinned = true,
            _ => {}
        }

        // Acquire session-pinned or pooled connection and execute natively via binary protocol (100% SQLi-safe!)
        let mut conn: Conn = match session.pinned_conn.take() {
            Some(c) => c,
            None => match pool.get_conn().await {
                Ok(c) => c,
                Err(e) => {
                    let (code, state, msg) = extract_mysql_error(&e);
                    socket
                        .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                        .await?;
                    socket.flush().await?;
                    return Ok(());
                }
            },
        };

        let mysql_params = if param_values.is_empty() {
            Params::Empty
        } else {
            Params::Positional(param_values)
        };

        let mut io_broken = false;
        {
            let exec_res = conn.exec_iter(query.as_str(), mysql_params).await;
            if exec_res.is_ok() {
                match intent {
                    TxIntent::CommitOrRollback => {
                        session.in_explicit_tx = false;
                    }
                    TxIntent::SetAutocommitOn => {
                        session.autocommit_disabled = false;
                        session.in_explicit_tx = false;
                    }
                    _ => {}
                }
            }
            let status_flags = session.status_flags();
            match exec_res {
                Ok(mut result) => {
                    let mut current_seq = seq_id.wrapping_add(1);
                    let columns: Vec<mysql_async::Column> = result
                        .columns()
                        .map(|c| c.as_ref().to_vec())
                        .unwrap_or_default();

                    if !columns.is_empty() {
                        let mut write_buf = Vec::with_capacity(8192);
                        write_buf.extend_from_slice(&build_lenenc_int_packet(
                            current_seq,
                            columns.len() as u64,
                        ));
                        current_seq = current_seq.wrapping_add(1);

                        for col in &columns {
                            write_buf
                                .extend_from_slice(&build_column_def_from_mysql(current_seq, col));
                            current_seq = current_seq.wrapping_add(1);
                        }

                        write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
                        current_seq = current_seq.wrapping_add(1);

                        while let Some(row) = result.next().await? {
                            let values = row.unwrap();
                            let row_pkt = build_binary_row_packet(
                                current_seq,
                                &columns,
                                &values,
                            );
                            current_seq = current_seq.wrapping_add(count_wire_packets(&row_pkt));
                            write_buf.extend_from_slice(&row_pkt);

                            if write_buf.len() >= 65536 {
                                socket.write_all(&write_buf).await?;
                                write_buf.clear();
                            }
                        }

                        write_buf.extend_from_slice(&build_eof_packet(current_seq, status_flags));
                        socket.write_all(&write_buf).await?;
                    } else {
                        let affected_rows = result.affected_rows();
                        let last_insert_id = result.last_insert_id().unwrap_or(0);
                        while (result.next().await?).is_some() {}
                        socket
                            .write_all(&build_ok_packet(
                                current_seq,
                                affected_rows,
                                last_insert_id,
                                status_flags,
                            ))
                            .await?;
                    }
                }
                Err(e) => {
                    if matches!(e, mysql_async::Error::Io(_)) {
                        io_broken = true;
                    }
                    let (code, state, msg) = extract_mysql_error(&e);
                    socket
                        .write_all(&build_error_packet(seq_id.wrapping_add(1), code, &state, &msg))
                        .await?;
                }
            }
        }

        if !io_broken && session.should_keep_pinned() {
            session.pinned_conn = Some(conn);
        }

        socket.flush().await?;
        Ok(())
    }

    fn handle_com_stmt_send_long_data(payload: &[u8], session: &mut SessionState) {
        // [0x18 (1)] + [stmt_id (4)] + [param_id (2)] + [data...]
        if payload.len() >= 7 {
            let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let param_id = u16::from_le_bytes([payload[5], payload[6]]);
            if let Some(stored) = session.statements.get_mut(&stmt_id) {
                stored
                    .long_data
                    .entry(param_id)
                    .or_default()
                    .extend_from_slice(&payload[7..]);
            }
        }
    }

    fn handle_com_stmt_close(payload: &[u8], session: &mut SessionState, is_debug: bool) {
        if payload.len() >= 5 {
            let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            if is_debug {
                println!("[DB Proxy] [COM_STMT_CLOSE (0x19)]: stmt_id={}", stmt_id);
            }
            session.statements.remove(&stmt_id);
        }
    }

    async fn handle_com_stmt_reset(
        socket: &mut TcpStream,
        seq_id: u8,
        payload: &[u8],
        session: &mut SessionState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if payload.len() >= 5 {
            let stmt_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            if let Some(stored) = session.statements.get_mut(&stmt_id) {
                stored.long_data.clear();
            }
        }
        socket
            .write_all(&build_ok_packet(
                seq_id.wrapping_add(1),
                0,
                0,
                session.status_flags(),
            ))
            .await?;
        socket.flush().await?;
        Ok(())
    }
}

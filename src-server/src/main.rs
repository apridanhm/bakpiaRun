mod config;
mod static_file;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use clap::Parser;
use config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use multer::Multipart;
use tempfile::NamedTempFile;
use std::io::Write;

#[derive(Parser, Debug)]
#[command(name = "bakpiarun", about = "PHP Runtime Server")]
struct Cli {
    #[arg(short, long, default_value = "../config/bakpiarun.yaml")]
    config: String,
}

// --- DATA STRUCTURES ---

#[derive(Debug, Serialize)]
struct PhpRequest {
    method: String,
    uri: String,
    file_path: String,
    query_string: String,
    query_params: HashMap<String, String>,
    post_params: HashMap<String, String>,
    cookies: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
    content_type: String,
    content_length: String,
    //files: HashMap<String, FileInfo>,
    files: HashMap<String, Vec<FileInfo>>,

}

#[derive(Debug, Serialize)]
struct FileInfo {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    size: usize,
    // content: String, // Base64 encoded
    tmp_path: String,
}

#[derive(Debug, Deserialize)]
struct PhpResponse {
    status: u16,
    body: String,
    memory: u64,
    peak: u64,
}

// --- WORKER STATE ---

struct Worker {
    index: usize,
    process: Option<Child>,
    socket_path: String,
    requests_handled: u64,
    last_memory: u64,
    last_peak: u64,
}

impl Worker {
    fn new(index: usize, socket_path: String) -> Self {
        Self {
            index,
            process: None,
            socket_path,
            requests_handled: 0,
            last_memory: 0,
            last_peak: 0,
        }
    }

    async fn start(&mut self, config: &Config) -> Result<(), String> {
        println!("[Supervisor] Starting PHP worker #{}...", self.index);

        let _ = std::fs::remove_file(&self.socket_path);

        let child = Command::new("php")
            .arg(&config.php.worker_path)
            .current_dir(&config.php.docroot)
            .env("BAKPIARUN_SOCKET_PATH", &self.socket_path)
            .spawn()
            .map_err(|e| format!("Failed to spawn worker #{}: {}", self.index, e))?;

        self.process = Some(child);
        self.requests_handled = 0;

        println!(
            "[Supervisor] PHP worker #{} started (PID: {:?}, Socket: {})",
            self.index,
            self.process.as_ref().unwrap().id(),
            self.socket_path
        );

        sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            println!("[Supervisor] Stopping PHP worker #{}...", self.index);

            if let Err(e) = child.kill().await {
                eprintln!("[Supervisor] Error killing worker #{}: {}", self.index, e);
            }

            let _ = child.wait().await;
            println!("[Supervisor] PHP worker #{} stopped", self.index);
        }
    }

    async fn check_health(&mut self) -> bool {
        if let Some(ref mut child) = self.process {
            match child.try_wait() {
                Ok(Some(status)) => {
                    println!("[Supervisor] Worker #{} exited: {}", self.index, status);
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    eprintln!("[Supervisor] Error checking worker #{}: {}", self.index, e);
                    false
                }
            }
        } else {
            false
        }
    }

    async fn ensure_running(&mut self, config: &Config) -> Result<(), String> {
        if !self.check_health().await {
            println!("[Supervisor] Worker #{} is dead, restarting...", self.index);
            self.stop().await;
            self.start(config).await?;
        }
        Ok(())
    }

    fn should_restart(&self, config: &Config) -> Option<String> {
        let memory_bytes = config.php.memory_limit_mb * 1024 * 1024;

        if self.last_memory > memory_bytes {
            return Some(format!(
                "Worker #{}: Memory limit exceeded ({} MB > {} MB)",
                self.index,
                self.last_memory / 1024 / 1024,
                config.php.memory_limit_mb
            ));
        }

        if self.requests_handled >= config.php.max_requests {
            return Some(format!(
                "Worker #{}: Max requests reached ({} >= {})",
                self.index, self.requests_handled, config.php.max_requests
            ));
        }

        None
    }

    fn update_stats(&mut self, memory: u64, peak: u64) {
        self.requests_handled += 1;
        self.last_memory = memory;
        self.last_peak = peak;
    }
}

struct WorkerPool {
    workers: Vec<Worker>,
    current_index: AtomicUsize,
}

impl WorkerPool {
    fn new(worker_count: usize, config: &Config) -> Self {
        let mut workers = Vec::with_capacity(worker_count);

        for i in 0..worker_count {
            let socket_path = config.get_worker_socket_path(i);
            workers.push(Worker::new(i, socket_path));
        }

        Self {
            workers,
            current_index: AtomicUsize::new(0),
        }
    }

    async fn start_all(&mut self, config: &Config) -> Result<(), String> {
        for worker in &mut self.workers {
            worker.start(config).await?;
        }
        Ok(())
    }

    async fn stop_all(&mut self) {
        for worker in &mut self.workers {
            worker.stop().await;
        }
    }

    fn get_next_worker(&self) -> usize {
        let index = self.current_index.fetch_add(1, Ordering::SeqCst);
        index % self.workers.len()
    }

    async fn ensure_all_running(&mut self, config: &Config) {
        for worker in &mut self.workers {
            if let Err(e) = worker.ensure_running(config).await {
                eprintln!("[Supervisor] Failed to restart worker #{}: {}", worker.index, e);
            }
        }
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    pool: Arc<Mutex<WorkerPool>>,
}

// --- IPC CLIENT ---

async fn send_to_php_worker(
    socket_path: &str,
    request: PhpRequest,
) -> Result<PhpResponse, String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("Failed to connect to PHP worker: {}", e))?;

    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    let payload_length = request_json.len() as u32;
    let length_bytes = payload_length.to_be_bytes();

    stream
        .write_all(&length_bytes)
        .await
        .map_err(|e| format!("Failed to write length: {}", e))?;
    stream
        .write_all(request_json.as_bytes())
        .await
        .map_err(|e| format!("Failed to write payload: {}", e))?;

    let mut length_buf = [0u8; 4];
    stream
        .read_exact(&mut length_buf)
        .await
        .map_err(|e| format!("Failed to read response length: {}", e))?;

    let response_length = u32::from_be_bytes(length_buf) as usize;

    let mut response_buf = vec![0u8; response_length];
    stream
        .read_exact(&mut response_buf)
        .await
        .map_err(|e| format!("Failed to read response payload: {}", e))?;

    let response: PhpResponse = serde_json::from_slice(&response_buf)
        .map_err(|e| format!("Failed to deserialize response: {}", e))?;

    Ok(response)
}

// --- HTTP HANDLER ---
async fn php_handler(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
    body: Body,
) -> Response {
    let path = uri.path();

    // 1. Check static files dulu
    if static_file::is_static_file(path) {
        println!("[Static] Serving: {}", path);
        return static_file::serve_static_file(&state.config.php.docroot, path).await;
    }
    
    // 2. Cari PHP file dengan routing
    let file_path = match static_file::find_php_file(&state.config.php.docroot, path).await {
        Some(fp) => {
            println!("[Router] {} → {}", path, fp);
            fp
        }
        None => {
            println!("[404] Not found: {}", path);
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("<h1>404 Not Found</h1><p>The requested URL was not found on this server.</p>"))
                .unwrap();
        }
    };
    
    // 3. Pilih worker
    let worker_index = {
        let pool = state.pool.lock().await;
        pool.get_next_worker()
    };

    // 4. Pastikan worker running
    {
        let mut pool = state.pool.lock().await;
        if let Some(worker) = pool.workers.get_mut(worker_index) {
            if let Err(e) = worker.ensure_running(&state.config).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html(format!("<h1>500 Error</h1><p>Failed to start worker: {}</p>", e)),
                )
                    .into_response();
            }
        }
    }

    // 5. Parse headers
    let mut header_map = HashMap::new();
    for (key, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            header_map.insert(key.as_str().to_string(), v.to_string());
        }
    }

    // 6. Parse cookies
    let mut cookies = HashMap::new();
    if let Some(cookie_header) = headers.get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            for cookie in cookie_str.split(';') {
                let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
                if parts.len() == 2 {
                    cookies.insert(parts[0].to_string(), parts[1].to_string());
                }
            }
        }
    }

    // 7. Read body
    let body_bytes = match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Html(format!("<h1>400 Bad Request</h1><p>Failed to read body: {}</p>", e)),
            )
                .into_response();
        }
    };

    let body_length = body_bytes.len();
    let body_string = String::from_utf8_lossy(&body_bytes).to_string();
    let body_string = String::from_utf8_lossy(&body_bytes).to_string();

    // 8. Parse POST data dan FILES
    //let mut post_params = HashMap::new();
    let mut post_params: HashMap<String, String> = HashMap::new();
    let mut files: HashMap<String, Vec<FileInfo>> = HashMap::new();
    
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

        if content_type.contains("multipart/form-data") {
            let boundary = content_type
                .split("boundary=")
                .nth(1)
                .map(|b| b.trim().trim_matches('"'))
                .unwrap_or("");
            
            if !boundary.is_empty() {
                let body_stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(body_bytes) });
                let mut multipart = Multipart::new(body_stream, boundary);
                
                while let Ok(Some(field)) = multipart.next_field().await {
                    let name = field.name().unwrap_or("").to_string();
                    let is_file = field.file_name().is_some();
                    let filename = field.file_name().map(|f| f.to_string());
                    let field_content_type = field.content_type().map(|ct| ct.to_string());
                    
                    if is_file {
                        if let Some(fname) = filename {
                            // Buat temp file di folder yang sama dengan upload.php
                            let upload_dir = "/tmp/bakpiarun_uploads/";
                            let _ = std::fs::create_dir_all(upload_dir);
                            
                            let temp_path = format!("{}{}", upload_dir, format!("{}", uuid::Uuid::new_v4()));
                            
                            match std::fs::File::create(&temp_path) {
                                Ok(mut file) => {
                                    let mut field_stream = field;
                                    let mut total_size = 0usize;
                                    let mut write_ok = true;
                                    
                                    while write_ok {
                                        match field_stream.chunk().await {
                                            Ok(Some(chunk)) => {
                                                use std::io::Write;
                                                if let Err(e) = file.write_all(&chunk) {
                                                    eprintln!("[Upload] Write error: {}", e);
                                                    write_ok = false;
                                                    break;
                                                }
                                                total_size += chunk.len();
                                            }
                                            Ok(None) => break,
                                            Err(e) => {
                                                eprintln!("[Upload] Read error: {}", e);
                                                write_ok = false;
                                                break;
                                            }
                                        }
                                    }
                                    
                                    if write_ok {
                                        println!("[Upload] File saved: {} ({} bytes)", fname, total_size);

                                        files.entry(name.clone()).or_default().push(FileInfo {
                                            name: fname,
                                            content_type: field_content_type
                                                .unwrap_or_else(|| "application/octet-stream".to_string()),
                                            size: total_size,
                                            tmp_path: temp_path,
                                        });
                                    } else {
                                        // Cleanup failed upload
                                        let _ = std::fs::remove_file(&temp_path);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[Upload] Failed to create file {}: {}", temp_path, e);
                                }
                            }
                        }
                    } else {
                        if let Ok(value) = field.text().await {
                            post_params.insert(name, value);
                        }
                    }
                }
            }
        }
        
    
    else if content_type.contains("application/x-www-form-urlencoded") {
        // Parse URL-encoded form data
        for pair in body_string.split('&') {
            let parts: Vec<&str> = pair.splitn(2, '=').collect();
            if parts.len() == 2 {
                if let (Ok(key), Ok(value)) = (
                    urlencoding::decode(parts[0]),
                    urlencoding::decode(parts[1]),
                ) {
                    post_params.insert(key.to_string(), value.to_string());
                }
            }
        }
    }

    let query_string = uri.query().unwrap_or("").to_string();

    // 9. Buat PhpRequest
    let request = PhpRequest {
        method: method.to_string(),
        uri: uri.to_string(),
        file_path,
        query_string,
        query_params,
        post_params,
        cookies,
        headers: header_map,
        body: body_string,
        content_type,
        //content_length: body_bytes.len().to_string(),
        content_length: body_length.to_string(),
        files,
    };

    // 10. Kirim ke PHP worker
    let socket_path = {
        let pool = state.pool.lock().await;
        pool.workers[worker_index].socket_path.clone()
    };

    match send_to_php_worker(&socket_path, request).await {
        Ok(php_response) => {
            let restart_reason = {
                let mut pool = state.pool.lock().await;
                if let Some(worker) = pool.workers.get_mut(worker_index) {
                    worker.update_stats(php_response.memory, php_response.peak);

                    println!(
                        "[Worker #{}] Request #{} handled. Memory: {} MB, Peak: {} MB",
                        worker_index,
                        worker.requests_handled,
                        php_response.memory / 1024 / 1024,
                        php_response.peak / 1024 / 1024
                    );

                    worker.should_restart(&state.config)
                } else {
                    None
                }
            };

            if let Some(reason) = restart_reason {
                println!("[Anti-OOM] {}", reason);
                println!("[Anti-OOM] Restarting worker #{}...", worker_index);

                let mut pool = state.pool.lock().await;
                if let Some(worker) = pool.workers.get_mut(worker_index) {
                    worker.stop().await;
                }
            }

            (
                StatusCode::from_u16(php_response.status).unwrap_or(StatusCode::OK),
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                Html(php_response.body),
            )
                .into_response()
        }
        Err(e) => {
            eprintln!("[Worker #{}] Error: {}", worker_index, e);

            {
                let mut pool = state.pool.lock().await;
                if let Some(worker) = pool.workers.get_mut(worker_index) {
                    let _ = worker.ensure_running(&state.config).await;
                }
            }

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<h1>500 Error</h1><p>{}</p>", e)),
            )
                .into_response()
        }
    }
}
////////////////////////////////////////////////////////////////////////////////////////////
fn resolve_php_file(docroot: &str, uri: &str) -> String {
    let mut path = uri.to_string();

    if path == "/" || path.is_empty() {
        path = "/index.php".to_string();
    }

    if path.contains("..") {
        return format!("{}/403.php", docroot);
    }

    format!("{}{}", docroot, path)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    println!("Initializing bakpiaRun Server with Worker Pool...");

    let mut config = match Config::load_from_file(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    config.apply_env_overrides();

    if let Err(e) = config.validate() {
        eprintln!("Invalid config: {}", e);
        std::process::exit(1);
    }

    println!("Configuration:");
    println!("   - Server: {}:{}", config.server.host, config.server.port);
    println!("   - Docroot: {}", config.php.docroot);
    println!("   - Worker Path: {}", config.php.worker_path);
    println!("   - Workers: {}", config.php.worker_count);
    println!("   - Memory Limit: {} MB", config.php.memory_limit_mb);
    println!("   - Max Requests: {}", config.php.max_requests);
    println!("   - Socket Dir: {}", config.socket.directory);

    std::fs::create_dir_all(&config.socket.directory)
        .expect("Failed to create socket directory");

    let mut pool = WorkerPool::new(config.php.worker_count, &config);

    if let Err(e) = pool.start_all(&config).await {
        eprintln!("Failed to start workers: {}", e);
        std::process::exit(1);
    }

    let state = AppState {
        config: Arc::new(config.clone()),
        pool: Arc::new(Mutex::new(pool)),
    };

    let app = Router::new()
        .route("/", get(php_handler).post(php_handler))
        .route("/*path", get(php_handler).post(php_handler))
        .with_state(state.clone());

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("Listening on http://{}", addr);
    println!("Anti-OOM system active!");
    println!("Worker pool: {} workers with round-robin load balancing", config.php.worker_count);
    println!("Request handling: GET, POST, Cookies, Headers enabled!");

    let pool_clone = state.pool.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            let mut pool = pool_clone.lock().await;
            pool.ensure_all_running(&config_clone).await;
        }
    });

    let state_for_shutdown = state.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("\n[Server] Shutting down gracefully...");
        let mut pool = state_for_shutdown.pool.lock().await;
        pool.stop_all().await;
        std::process::exit(0);
    });

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
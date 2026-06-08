use axum::{
    body::Body,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use crate::types::{AppState, PhpRequest, FileInfo};
use crate::ipc::send_to_php_worker;
use crate::static_file;
use crate::config::Config;
use crate::security::apply_security_headers;

pub async fn php_handler(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Body,
) -> Response {
    let start_time = Instant::now();
    
    let client_ip = if let Some(forwarded) = headers.get("x-forwarded-for") {
        forwarded.to_str().unwrap_or("unknown").to_string()
    } else if let Some(real_ip) = headers.get("x-real-ip") {
        real_ip.to_str().unwrap_or("unknown").to_string()
    } else {
        addr.ip().to_string()
    };

    // RATE LIMIT CHECK
    let rate_limit_headers = if let Ok(ip_addr) = client_ip.parse::<std::net::IpAddr>() {
        match state.rate_limiter.check_rate_limit(ip_addr).await {
            Ok(headers) => Some(headers),
            Err(e) => {
                let duration_ms = start_time.elapsed().as_millis() as u64;
                state.logger.log_error(
                    "WARN",
                    &format!("Rate limit exceeded for IP: {} (URI: {})", client_ip, uri),
                );
                state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 429, duration_ms);
                
                return axum::http::Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .header("Content-Type", "application/json")
                    .header("X-RateLimit-Limit", e.limit.to_string())
                    .header("X-RateLimit-Remaining", "0")
                    .header("X-RateLimit-Reset", e.reset.to_string())
                    .header("Retry-After", e.reset.to_string())
                    .body(axum::body::Body::from(format!(
                        r#"{{"error":"Rate limit exceeded","limit":{},"reset":{}}}"#,
                        e.limit, e.reset
                    )))
                    .unwrap()
                    .into_response();
            }
        }
    } else {
        None
    };

    
    // LOCK CONFIG
    let config = state.config.lock().await.clone();
    
    let path = uri.path();

    if static_file::is_static_file(path) {
        println!("[Static] Serving: {}", path);
        state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 200, 0);
        return static_file::serve_static_file(&config.php.docroot, path).await;
    }
    
    let file_path = match static_file::find_php_file(&config.php.docroot, path).await {
        Some(fp) => {
            println!("[Router] {} → {}", path, fp);
            fp
        }
        None => {
            println!("[404] Not found: {}", path);
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 404, duration_ms);
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("<h1>404 Not Found</h1><p>The requested URL was not found on this server.</p>"))
                .unwrap();
        }
    };
    
    let worker_index = {
        let pool = state.pool.lock().await;
        pool.get_next_worker()
    };

    {
        let mut pool = state.pool.lock().await;
        if let Some(worker) = pool.workers.get_mut(worker_index) {
            if let Err(e) = worker.ensure_running(&config).await {
                let duration_ms = start_time.elapsed().as_millis() as u64;
                state.logger.log_error("ERROR", &format!("Worker #{} failed to start: {} (URI: {})", worker_index, e, uri));
                state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 500, duration_ms);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html(format!("<h1>500 Error</h1><p>Failed to start worker: {}</p>", e)),
                ).into_response();
            }
        }
    }

    let mut header_map = HashMap::new();
    for (key, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            header_map.insert(key.as_str().to_string(), v.to_string());
        }
    }

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

    let body_bytes = match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error("ERROR", &format!("Failed to read body: {} (URI: {})", e, uri));
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 400, duration_ms);
            return (
                StatusCode::BAD_REQUEST,
                Html(format!("<h1>400 Bad Request</h1><p>Failed to read body: {}</p>", e)),
            ).into_response();
        }
    };

    let body_length = body_bytes.len();
    let body_string = String::from_utf8_lossy(&body_bytes).to_string();

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
            let mut multipart = multer::Multipart::new(body_stream, boundary);
            
            while let Ok(Some(field)) = multipart.next_field().await {
                let name = field.name().unwrap_or("").to_string();
                let is_file = field.file_name().is_some();
                let filename = field.file_name().map(|f| f.to_string());
                let field_content_type = field.content_type().map(|ct| ct.to_string());
                
                if is_file {
                    if let Some(fname) = filename {
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
                                        content_type: field_content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                                        size: total_size,
                                        tmp_path: temp_path,
                                    });
                                } else {
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
    } else if content_type.contains("application/x-www-form-urlencoded") {
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
        content_length: body_length.to_string(),
        files,
    };

    // WRAP DENGAN TIMEOUT
    let timeout_duration = std::time::Duration::from_millis(config.php.timeout_ms);
    
    //let result = tokio::time::timeout(timeout_duration, send_to_php_worker(&socket_path, request)).await;
    // ambil data worker dulu (socket_path, pool, pool_size)
    let (socket_path, conn_pool, pool_size) = {
        let pool = state.pool.lock().await;
        let idx = pool.get_next_worker();
        let w = &pool.workers[idx];
        (w.socket_path.clone(), w.connection_pool.clone(), w.pool_size)
    };

    let result = tokio::time::timeout(
        timeout_duration, 
        send_to_php_worker(&socket_path, &conn_pool, pool_size, request)
    ).await;

    match result {
        Ok(Ok(php_response)) => {
            // request sukses dalam timeout
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_access(
                &client_ip,
                &method.to_string(),
                &uri.to_string(),
                php_response.status,
                duration_ms,
            );

            {
                let mut metrics = state.metrics.lock().await;
                metrics.record_request(worker_index, php_response.memory);
            }

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
                    worker.should_restart(&config)
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

            let mut response = axum::http::Response::builder()
                .status(StatusCode::from_u16(php_response.status).unwrap_or(StatusCode::OK))
                .header("Content-Type", "text/html; charset=utf-8");

                let config = state.config.lock().await;
                response = apply_security_headers(response, &config.security);
                drop(config);
            
            if let Some(headers) = rate_limit_headers {
                response = response
                    .header("X-RateLimit-Limit", headers.limit.to_string())
                    .header("X-RateLimit-Remaining", headers.remaining.to_string())
                    .header("X-RateLimit-Reset", headers.reset.to_string());
            }
            
            response
                .body(axum::body::Body::from(php_response.body))
                .unwrap()
                .into_response()
        }
        Ok(Err(e)) => {
            // worker error (bukan timeout)
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error(
                "ERROR",
                &format!("Worker #{} failed: {} (URI: {})", worker_index, e, uri),
            );
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 500, duration_ms);

            {
                let mut metrics = state.metrics.lock().await;
                metrics.record_error();
            }
            eprintln!("[Worker #{}] Error: {}", worker_index, e);
            {
                let mut pool = state.pool.lock().await;
                if let Some(worker) = pool.workers.get_mut(worker_index) {
                    let _ = worker.ensure_running(&config).await;
                }
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<h1>500 Error</h1><p>{}</p>", e)),
            ).into_response()
        }
        Err(_) => {
            // TIMEOUT EXCEEDED
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error(
                "ERROR",
                &format!("Request timeout after {}ms (URI: {})", config.php.timeout_ms, uri),
            );
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 504, duration_ms);

            {
                let mut metrics = state.metrics.lock().await;
                metrics.record_error();
            }
            
            // restart worker karena mungkin hang
            {
                let mut pool = state.pool.lock().await;
                if let Some(worker) = pool.workers.get_mut(worker_index) {
                    println!("[Timeout] Restarting worker #{}...", worker_index);
                    worker.stop().await;
                }
            }
            
            (
                StatusCode::GATEWAY_TIMEOUT,
                Html(format!(
                    "<h1>504 Gateway Timeout</h1><p>The server did not receive a timely response from the PHP worker.</p><p>Timeout: {}ms</p>",
                    config.php.timeout_ms
                )),
            ).into_response()
        }
    }
}

pub async fn health_handler(State(state): State<AppState>) -> Response {
    let pool = state.pool.lock().await;
    let metrics = state.metrics.lock().await;
    
    let mut workers_healthy = 0;
    let mut workers_info = Vec::new();
    
    for worker in &pool.workers {
        let is_alive = worker.process.is_some();
        if is_alive {
            workers_healthy += 1;
        }
        
        workers_info.push(serde_json::json!({
            "index": worker.index,
            "alive": is_alive,
            "requests": worker.requests_handled,
            "memory_mb": worker.last_memory / 1024 / 1024,
        }));
    }
    
    let total_workers = pool.workers.len();
    let is_healthy = workers_healthy == total_workers;
    
    let response = serde_json::json!({
        "status": if is_healthy { "healthy" } else { "unhealthy" },
        "uptime_seconds": metrics.uptime_seconds(),
        "total_requests": metrics.total_requests,
        "total_errors": metrics.total_errors,
        "workers": {
            "total": total_workers,
            "healthy": workers_healthy,
            "details": workers_info,
        }
    });
    
    let status = if is_healthy { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        Body::from(serde_json::to_string_pretty(&response).unwrap()),
    ).into_response()
}

pub async fn metrics_handler(State(state): State<AppState>) -> Response {
    let pool = state.pool.lock().await;
    let metrics = state.metrics.lock().await;
    
    let mut output = String::new();
    
    output.push_str("# HELP bakpiarun_uptime_seconds Server uptime in seconds\n");
    output.push_str("# TYPE bakpiarun_uptime_seconds gauge\n");
    output.push_str(&format!("bakpiarun_uptime_seconds {}\n\n", metrics.uptime_seconds()));
    
    output.push_str("# HELP bakpiarun_requests_total Total number of requests\n");
    output.push_str("# TYPE bakpiarun_requests_total counter\n");
    output.push_str(&format!("bakpiarun_requests_total {}\n\n", metrics.total_requests));
    
    output.push_str("# HELP bakpiarun_errors_total Total number of errors\n");
    output.push_str("# TYPE bakpiarun_errors_total counter\n");
    output.push_str(&format!("bakpiarun_errors_total {}\n\n", metrics.total_errors));
    
    output.push_str("# HELP bakpiarun_worker_requests_total Requests per worker\n");
    output.push_str("# TYPE bakpiarun_worker_requests_total counter\n");
    for (i, &count) in metrics.requests_per_worker.iter().enumerate() {
        output.push_str(&format!("bakpiarun_worker_requests_total{{worker=\"{}\"}} {}\n", i, count));
    }
    output.push('\n');
    
    output.push_str("# HELP bakpiarun_worker_memory_bytes Memory usage per worker\n");
    output.push_str("# TYPE bakpiarun_worker_memory_bytes gauge\n");
    for (i, &mem) in metrics.memory_per_worker.iter().enumerate() {
        output.push_str(&format!("bakpiarun_worker_memory_bytes{{worker=\"{}\"}} {}\n", i, mem));
    }
    output.push('\n');
    
    output.push_str("# HELP bakpiarun_worker_alive Worker alive status\n");
    output.push_str("# TYPE bakpiarun_worker_alive gauge\n");
    for worker in &pool.workers {
        let alive = if worker.process.is_some() { 1 } else { 0 };
        output.push_str(&format!("bakpiarun_worker_alive{{worker=\"{}\"}} {}\n", worker.index, alive));
    }
    
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from(output),
    ).into_response()
}

// reload handler
pub async fn reload_handler(State(state): State<AppState>) -> Response {
    println!("[Reload] HTTP reload triggered");
    
    let config_path = std::env::current_dir()
    .map(|p| p.join("../config/bakpiarun.yaml").to_string_lossy().to_string())
    .unwrap_or_else(|_| "../config/bakpiarun.yaml".to_string()); // suapaya configurable
    
    let mut new_config = match Config::load_from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            let response = serde_json::json!({
                "status": "error",
                "message": format!("Failed to load config: {}", e)
            });
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                Body::from(serde_json::to_string_pretty(&response).unwrap()),
            ).into_response();
        }
    };
    
    new_config.apply_env_overrides();
    
    if let Err(e) = new_config.validate() {
        let response = serde_json::json!({
            "status": "error",
            "message": format!("Invalid config: {}", e)
        });
        return (
            StatusCode::BAD_REQUEST,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            Body::from(serde_json::to_string_pretty(&response).unwrap()),
        ).into_response();
    }
    
    // update shared config
    {
        let mut config = state.config.lock().await;
        *config = new_config.clone();
    }
    
    // rolling restart workers
    let mut pool = state.pool.lock().await;
    match pool.reload(&new_config).await {
        Ok(_) => {
            let response = serde_json::json!({
                "status": "success",
                "message": "Configuration reloaded successfully",
                "workers": new_config.php.worker_count
            });
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                Body::from(serde_json::to_string_pretty(&response).unwrap()),
            ).into_response()
        }
        Err(e) => {
            let response = serde_json::json!({
                "status": "error",
                "message": format!("Failed to reload workers: {}", e)
            });
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                Body::from(serde_json::to_string_pretty(&response).unwrap()),
            ).into_response()
        }
    }
}
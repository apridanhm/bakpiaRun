use axum::{
    body::Body,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};
use axum::extract::Path;
use axum::Json;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use crate::types::{AppState, PhpRequest, FileInfo};
use crate::ipc::send_to_php_worker;
use crate::static_file;
use crate::config::Config;
use crate::security::apply_security_headers;
use crate::anti_oom::read_cgroup_v2_memory;


#[derive(Deserialize)]
pub struct SubmitJobRequest {
    pub task: String,
    pub payload: serde_json::Value,
}

#[derive(Serialize)]
pub struct JobResponse {
    pub job_id: String,
    pub status: String,
    pub message: String,
}


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

    // STATIC FILE CHECK
    if static_file::is_static_file(path) {
        println!("[Static] Serving: {}", path);
        state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 200, 0);
        return static_file::serve_static_file(&config.php.docroot, path, config.server.static_cache_max_age).await;
    }
    
    let file_path = match static_file::find_php_file(&config.php.docroot, path).await {
        Some(fp) => fp,
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

    // 🎯 WORKLOAD-AWARE ROUTER: Pilih pool berdasarkan atribut HTTP Header & URL pattern
    let pool_name = {
        let pm = state.pool_manager.lock().await;
        match pm.get_pool_name_for_request(path, &headers) {
            Some(name) => name,
            None => {
                let duration_ms = start_time.elapsed().as_millis() as u64;
                state.logger.log_error("ERROR", &format!("No suitable pool for path: {} (URI: {})", path, uri));
                state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 500, duration_ms);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html("<h1>500 Error</h1><p>No suitable worker pool found</p>"),
                ).into_response();
            }
        }
    };
    println!("[Router] {} → Pool: {}", path, pool_name);

    // Ambil worker dari pool yang dipilih beserta pool_config (isolasi lock per-worker)
    let (worker_index, worker_arc, pool_config) = {
        let pm = state.pool_manager.lock().await;
        let pool_arc = pm.get_pool(&pool_name).unwrap().clone();
        drop(pm);
        let pool = pool_arc.lock().await;
        let (idx, w_arc) = pool.get_next_worker();
        (idx, w_arc, pool.config.clone())
    };

    // Ensure worker running (hanya lock worker spesifik ini, tidak mengunci seluruh pool!)
    {
        let mut worker = worker_arc.lock().await;
        if let Err(e) = worker.ensure_running(&pool_config).await {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error("ERROR", &format!("Worker #{} in pool '{}' failed to start: {} (URI: {})", worker_index, pool_name, e, uri));
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 500, duration_ms);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<h1>500 Error</h1><p>Failed to start worker: {}</p>", e)),
            ).into_response();
        }
    }

    // Parse headers, cookies, body (BIARKAN SAMA)
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

    let max_body_bytes = config.server.max_body_size_mb * 1024 * 1024;
    let body_bytes = match axum::body::to_bytes(body, max_body_bytes).await {
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
    let mut uploaded_temp_files: Vec<String> = Vec::new();
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
                        let upload_dir = config.php.upload_tmp_dir.trim_end_matches('/');
                        let _ = std::fs::create_dir_all(upload_dir);
                        let temp_path = format!("{}/{}", upload_dir, uuid::Uuid::new_v4());
                        
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
                                    uploaded_temp_files.push(temp_path.clone());
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

    // AMBIL socket_path, conn_pool, pool_size DARI WORKER YANG DIPILIH & tandai active_requests += 1
    let (socket_path, conn_pool, pool_size) = {
        let mut w = worker_arc.lock().await;
        w.active_requests += 1;
        (w.socket_path.clone(), w.connection_pool.clone(), w.pool_size)
    };

    // WRAP DENGAN TIMEOUT (gunakan timeout dari pool yang dipilih)
    let timeout_duration = std::time::Duration::from_millis(pool_config.php.timeout_ms);
    let result = tokio::time::timeout(
        timeout_duration, 
        send_to_php_worker(&socket_path, &conn_pool, pool_size, request)
    ).await;

    // Cleanup temporary uploaded files after PHP worker finishes processing
    for tmp_file in &uploaded_temp_files {
        let _ = std::fs::remove_file(tmp_file);
    }

    match result {
        Ok(Ok(php_response)) => {
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

            // UPDATE stats worker (/proc/[pid]/status VmRSS + PHP memory) tanpa mengunci seluruh pool
            let restart_reason = {
                let mut worker = worker_arc.lock().await;
                worker.active_requests = worker.active_requests.saturating_sub(1);
                worker.update_stats(php_response.memory, php_response.peak);
                println!(
                    "[Pool:{}] Worker #{} (PID: {:?}) Request #{} handled. PHP Mem: {} MB, Peak: {} MB, /proc VmRSS: {} MB",
                    pool_name,
                    worker_index,
                    worker.pid,
                    worker.requests_handled,
                    php_response.memory / 1024 / 1024,
                    php_response.peak / 1024 / 1024,
                    worker.rss_bytes / 1024 / 1024
                );
                worker.should_restart(&pool_config)
            };

            if let Some(reason) = restart_reason {
                println!("[Anti-OOM] {}", reason);
                let mut worker = worker_arc.lock().await;
                if let Err(e) = worker.graceful_recycle(&pool_config).await {
                    eprintln!("[Anti-OOM] Failed to recycle worker #{} in pool '{}': {}", worker_index, pool_name, e);
                }
            }

            let content_type_hdr = php_response
                .headers
                .get("content-type")
                .or_else(|| php_response.headers.get("Content-Type"))
                .cloned()
                .unwrap_or_else(|| config.php.default_content_type.clone());

            let mut response = axum::http::Response::builder()
                .status(StatusCode::from_u16(php_response.status).unwrap_or(StatusCode::OK))
                .header("Content-Type", &content_type_hdr)
                .header("X-BakpiaRun-Pool", &pool_name);

            for (hdr_k, hdr_v) in &php_response.headers {
                if !hdr_k.eq_ignore_ascii_case("content-type") {
                    response = response.header(hdr_k.as_str(), hdr_v.as_str());
                }
            }

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
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error(
                "ERROR",
                &format!("Worker #{} in pool '{}' failed: {} (URI: {})", worker_index, pool_name, e, uri),
            );
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 500, duration_ms);

            {
                let mut metrics = state.metrics.lock().await;
                metrics.record_error();
            }
            eprintln!("[Pool:{}] Worker #{} Error: {}", pool_name, worker_index, e);
            {
                let mut worker = worker_arc.lock().await;
                worker.active_requests = worker.active_requests.saturating_sub(1);
                let _ = worker.ensure_running(&pool_config).await;
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<h1>500 Error</h1><p>{}</p>", e)),
            ).into_response()
        }
        Err(_) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            state.logger.log_error(
                "ERROR",
                &format!("Request timeout after {}ms (URI: {})", pool_config.php.timeout_ms, uri),
            );
            state.logger.log_access(&client_ip, &method.to_string(), &uri.to_string(), 504, duration_ms);

            {
                let mut metrics = state.metrics.lock().await;
                metrics.record_error();
            }
            
            {
                let mut worker = worker_arc.lock().await;
                worker.active_requests = worker.active_requests.saturating_sub(1);
                println!("[Timeout] Gracefully recycling timed-out worker #{} in pool '{}'...", worker_index, pool_name);
                let _ = worker.graceful_recycle(&pool_config).await;
            }
            
            (
                StatusCode::GATEWAY_TIMEOUT,
                Html(format!(
                    "<h1>504 Gateway Timeout</h1><p>The server did not receive a timely response from the PHP worker.</p><p>Timeout: {}ms</p>",
                    pool_config.php.timeout_ms
                )),
            ).into_response()
        }
    }
}

pub async fn health_handler(State(state): State<AppState>) -> Response {
    let config = state.config.lock().await.clone();
    let pm = state.pool_manager.lock().await;
    let metrics = state.metrics.lock().await;
    
    let cgroup_mem = read_cgroup_v2_memory(
        &config.anti_oom.cgroup_memory_current_path,
        &config.anti_oom.cgroup_memory_max_path,
    );

    let mut total_workers = 0;
    let mut workers_healthy = 0;
    let mut total_recycles = 0u64;
    let mut all_pools_info = Vec::new();
    
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        let mut pool_healthy = 0;
        let mut pool_workers_info = Vec::new();
        
        for worker_arc in &pool.workers {
            let mut worker = worker_arc.lock().await;
            worker.refresh_proc_memory();
            let is_alive = worker.process.is_some();
            if is_alive {
                workers_healthy += 1;
                pool_healthy += 1;
            }
            total_workers += 1;
            total_recycles += worker.recycle_count;
            
            pool_workers_info.push(serde_json::json!({
                "index": worker.index,
                "pid": worker.pid,
                "alive": is_alive,
                "draining": worker.draining,
                "active_requests": worker.active_requests,
                "requests": worker.requests_handled,
                "recycle_count": worker.recycle_count,
                "memory_mb": worker.last_memory / 1024 / 1024,
                "peak_memory_mb": worker.last_peak / 1024 / 1024,
                "proc_rss_mb": worker.rss_bytes / 1024 / 1024,
                "proc_hwm_mb": worker.hwm_bytes / 1024 / 1024,
            }));
        }
        
        all_pools_info.push(serde_json::json!({
            "name": pool_name,
            "total": pool.workers.len(),
            "healthy": pool_healthy,
            "memory_limit_mb": pool.config.php.memory_limit_mb,
            "worker_threshold_percent": pool.config.anti_oom.worker_threshold_percent,
            "workers": pool_workers_info,
        }));
    }
    
    let is_healthy = workers_healthy == total_workers;
    
    let response = serde_json::json!({
        "status": if is_healthy { "healthy" } else { "unhealthy" },
        "uptime_seconds": metrics.uptime_seconds(),
        "total_requests": metrics.total_requests,
        "total_errors": metrics.total_errors,
        "anti_oom": {
            "enabled": config.anti_oom.enabled,
            "worker_threshold_percent": config.anti_oom.worker_threshold_percent,
            "cgroup_threshold_percent": config.anti_oom.cgroup_threshold_percent,
            "total_worker_recycles": total_recycles,
            "cgroup_memory": {
                "source": cgroup_mem.source,
                "current_bytes": cgroup_mem.current_bytes,
                "current_mb": cgroup_mem.current_bytes / 1024 / 1024,
                "max_bytes": cgroup_mem.max_bytes,
                "max_mb": cgroup_mem.max_bytes / 1024 / 1024,
                "usage_percent": (cgroup_mem.usage_percent * 100.0).round() / 100.0,
            }
        },
        "pools": all_pools_info,
        "workers": {
            "total": total_workers,
            "healthy": workers_healthy,
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
    let config = state.config.lock().await.clone();
    let pm = state.pool_manager.lock().await;
    let metrics = state.metrics.lock().await;
    
    let cgroup_mem = read_cgroup_v2_memory(
        &config.anti_oom.cgroup_memory_current_path,
        &config.anti_oom.cgroup_memory_max_path,
    );

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

    output.push_str("# HELP bakpiarun_cgroup_memory_current_bytes Aggregate container memory usage from cgroups v2\n");
    output.push_str("# TYPE bakpiarun_cgroup_memory_current_bytes gauge\n");
    output.push_str(&format!("bakpiarun_cgroup_memory_current_bytes{{source=\"{}\"}} {}\n\n", cgroup_mem.source, cgroup_mem.current_bytes));

    output.push_str("# HELP bakpiarun_cgroup_memory_max_bytes Maximum container memory limit from cgroups v2\n");
    output.push_str("# TYPE bakpiarun_cgroup_memory_max_bytes gauge\n");
    output.push_str(&format!("bakpiarun_cgroup_memory_max_bytes{{source=\"{}\"}} {}\n\n", cgroup_mem.source, cgroup_mem.max_bytes));

    output.push_str("# HELP bakpiarun_cgroup_memory_usage_percent Container cgroups v2 memory usage percentage\n");
    output.push_str("# TYPE bakpiarun_cgroup_memory_usage_percent gauge\n");
    output.push_str(&format!("bakpiarun_cgroup_memory_usage_percent{{source=\"{}\"}} {:.2}\n\n", cgroup_mem.source, cgroup_mem.usage_percent));
    
    output.push_str("# HELP bakpiarun_worker_requests_total Requests per worker\n");
    output.push_str("# TYPE bakpiarun_worker_requests_total counter\n");
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        for worker_arc in &pool.workers {
            let worker = worker_arc.lock().await;
            output.push_str(&format!(
                "bakpiarun_worker_requests_total{{pool=\"{}\",worker=\"{}\"}} {}\n",
                pool_name, worker.index, worker.requests_handled
            ));
        }
    }
    output.push('\n');
    
    output.push_str("# HELP bakpiarun_worker_memory_bytes PHP memory usage per worker in bytes\n");
    output.push_str("# TYPE bakpiarun_worker_memory_bytes gauge\n");
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        for worker_arc in &pool.workers {
            let worker = worker_arc.lock().await;
            output.push_str(&format!(
                "bakpiarun_worker_memory_bytes{{pool=\"{}\",worker=\"{}\"}} {}\n",
                pool_name, worker.index, worker.last_memory
            ));
        }
    }
    output.push('\n');

    output.push_str("# HELP bakpiarun_worker_proc_rss_bytes OS VmRSS from /proc/[pid]/status per worker in bytes\n");
    output.push_str("# TYPE bakpiarun_worker_proc_rss_bytes gauge\n");
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        for worker_arc in &pool.workers {
            let mut worker = worker_arc.lock().await;
            worker.refresh_proc_memory();
            output.push_str(&format!(
                "bakpiarun_worker_proc_rss_bytes{{pool=\"{}\",worker=\"{}\"}} {}\n",
                pool_name, worker.index, worker.rss_bytes
            ));
        }
    }
    output.push('\n');

    output.push_str("# HELP bakpiarun_worker_recycles_total Total proactive graceful worker recycles by Anti-OOM Engine\n");
    output.push_str("# TYPE bakpiarun_worker_recycles_total counter\n");
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        for worker_arc in &pool.workers {
            let worker = worker_arc.lock().await;
            output.push_str(&format!(
                "bakpiarun_worker_recycles_total{{pool=\"{}\",worker=\"{}\"}} {}\n",
                pool_name, worker.index, worker.recycle_count
            ));
        }
    }
    output.push('\n');
    
    output.push_str("# HELP bakpiarun_worker_alive Worker alive status\n");
    output.push_str("# TYPE bakpiarun_worker_alive gauge\n");
    for (pool_name, pool_arc) in &pm.pools {
        let pool = pool_arc.lock().await;
        for worker_arc in &pool.workers {
            let worker = worker_arc.lock().await;
            let alive = if worker.process.is_some() { 1 } else { 0 };
            output.push_str(&format!(
                "bakpiarun_worker_alive{{pool=\"{}\",worker=\"{}\"}} {}\n",
                pool_name, worker.index, alive
            ));
        }
    }
    
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from(output),
    ).into_response()
}

pub async fn reload_handler(State(state): State<AppState>) -> Response {
    println!("[Reload] HTTP reload triggered");
    
    let config_path = {
        let current_config = state.config.lock().await;
        if !current_config.config_path.is_empty() {
            current_config.config_path.clone()
        } else {
            "../config/bakpiarun.yaml".to_string()
        }
    };
    
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
    
    {
        let mut config = state.config.lock().await;
        *config = new_config.clone();
    }
    
    // reload semua pools menggunakan konfigurasi spesifik masing-masing pool
    let mut pm = state.pool_manager.lock().await;
    pm.update_routing_table(&new_config);
    let mut reload_results = Vec::new();
    
    for (name, pool_arc) in &pm.pools {
        println!("[Reload] Reloading pool '{}'...", name);
        let pool_specific_config = if let Some(pc) = new_config.pools.iter().find(|p| &p.name == name) {
            crate::pool_manager::PoolManager::build_pool_config(&new_config, pc)
        } else {
            new_config.clone()
        };
        let mut pool = pool_arc.lock().await;
        match pool.reload(&pool_specific_config).await {
            Ok(_) => reload_results.push(format!("Pool '{}': OK", name)),
            Err(e) => {
                eprintln!("[Reload] Failed to reload pool '{}': {}", name, e);
                reload_results.push(format!("Pool '{}': FAILED - {}", name, e));
            }
        }
    }
    
    let response = serde_json::json!({
        "status": "success",
        "message": "Configuration reloaded successfully",
        "pools": reload_results
    });
    
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        Body::from(serde_json::to_string_pretty(&response).unwrap()),
    ).into_response()
}

// Endpoint untuk submit job baru
pub async fn submit_job(
    State(state): State<AppState>,
    Json(req): Json<SubmitJobRequest>,
) -> Response {
    // Check if queue is enabled
    let queue = match &state.queue {
        Some(q) => q,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "Queue system is disabled"
                })),
            ).into_response();
        }
    };
    
    let job_id = queue.submit(req.task, req.payload).await;
    
    Json(JobResponse {
        job_id,
        status: "pending".to_string(),
        message: "Job successfully queued".to_string(),
    }).into_response()
}

// Endpoint untuk cek status job
pub async fn get_job_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    // Check if queue is enabled
    let queue = match &state.queue {
        Some(q) => q,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "Queue system is disabled"
                })),
            ).into_response();
        }
    };
    
    match queue.get_status(&id).await {
        Some(job) => Json(job).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Job not found"
            })),
        ).into_response(),
    }
}
mod config;
mod static_file;
mod types;
mod ipc;
mod worker;
mod worker_pool;
mod metrics;
mod handlers;
mod logger;
mod rate_limiter;
mod security;
mod pool_manager; 
mod queue;

use queue::JobQueue;
use clap::Parser;
use config::Config;
use types::AppState;
use metrics::Metrics;
use handlers::{php_handler, health_handler, metrics_handler, reload_handler};
use logger::Logger;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use axum::routing::get;
use axum::Router;
use rate_limiter::RateLimiter;
use tower_http::compression::CompressionLayer;
use pool_manager::PoolManager;
use std::time::Duration;
use axum::routing::post;

#[derive(Parser, Debug)]
#[command(name = "bakpiarun", about = "PHP Runtime Server")]
struct Cli {
    #[arg(short, long, default_value = "../config/bakpiarun.yaml")]
    config: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    println!(" Initializing bakpiaRun Server...");

    let mut config = match Config::load_from_file(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(" Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    config.apply_env_overrides();

    if let Err(e) = config.validate() {
        eprintln!(" Invalid config: {}", e);
        std::process::exit(1);
    }

    println!("   Configuration:");
    println!("   - Server: {}:{}", config.server.host, config.server.port);
    println!("   - Docroot: {}", config.php.docroot);
    println!("   - Worker Path: {}", config.php.worker_path);
    println!("   - Workers: {}", config.php.worker_count);
    println!("   - Memory Limit: {} MB", config.php.memory_limit_mb);
    println!("   - Max Requests: {}", config.php.max_requests);
    println!("   - Socket Dir: {}", config.socket.directory);

    std::fs::create_dir_all(&config.socket.directory)
        .expect("Failed to create socket directory");

    /*let mut pool = WorkerPool::new(config.php.worker_count, &config);

    if let Err(e) = pool.start_all(&config).await {
        eprintln!(" Failed to start workers: {}", e);
        std::process::exit(1);
    }*/
    // Initialize Pool Manager
    println!(" Initializing Pool Manager...");
    let pool_manager = PoolManager::new(&config).await;
    let pool_manager = Arc::new(tokio::sync::Mutex::new(pool_manager));

    let metrics = Metrics::new(config.php.worker_count);

    let logger = Logger::new(
        config.logging.access_log_enabled,
        &config.logging.access_log,
        config.logging.error_log_enabled,
        &config.logging.error_log,
    );

    let rate_limiter = RateLimiter::new(
        config.rate_limit.enabled,
        config.rate_limit.requests_per_minute,
        config.rate_limit.burst_size,
    );

    // Initialize Job Queue
    let job_queue = Arc::new(JobQueue::new());
    println!(" Queue System initialized");

    let state = AppState {
        //config: Arc::new(config.clone()),
        config: Arc::new(tokio::sync::Mutex::new(config.clone())),
        //pool: Arc::new(Mutex::new(pool)),
        pool_manager: pool_manager.clone(),
        metrics: Arc::new(Mutex::new(metrics)),
        logger: Arc::new(logger),
        rate_limiter: Arc::new(rate_limiter),
        queue: job_queue.clone(), 
    };

    let app = Router::new()
        .route("/", get(php_handler).post(php_handler))
        .route("/*path", get(php_handler).post(php_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/reload", get(reload_handler))
        .route("/api/queue/submit", post(handlers::submit_job))
        .route("/api/queue/status/:id", get(handlers::get_job_status))
        .with_state(state.clone());

    // compression mmiddleware
    let app = if config.compression.enabled {
        app.layer(CompressionLayer::new())
    } else {
        app
    };

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!(" Listening on http://{}", addr);
    println!(" Anti-OOM system active!");
    println!(" Worker pool: {} workers with round-robin load balancing", config.php.worker_count);
    println!(" Request timeout: {}ms", config.php.timeout_ms);
    println!(" Request handling: GET, POST, Cookies, Headers enabled!");
    println!(" Health check: http://{}/health", addr);
    println!(" Metrics: http://{}/metrics", addr);
    
    // PRINT LOGGING CONFIG
    println!(" Logging Configuration:");
    println!("   - Access Log: {}", if config.logging.access_log_enabled { 
        format!("Enabled ({})", config.logging.access_log) 
    } else { 
        "Disabled".to_string() 
    });
    println!("   - Error Log:  {}", if config.logging.error_log_enabled { 
        format!("Enabled ({})", config.logging.error_log) 
    } else { 
        "Disabled".to_string() 
    });

    // rate limit
    println!(" Rate Limiting: {}", if config.rate_limit.enabled { 
        format!("Enabled ({} req/min, burst: {})", 
            config.rate_limit.requests_per_minute,
            config.rate_limit.burst_size)
    } else { 
        "Disabled".to_string() 
    });

    println!(" Security Headers:");
    println!("   - X-Frame-Options: {}", config.security.x_frame_options.as_deref().unwrap_or("Disabled"));
    println!("   - X-Content-Type-Options: {}", if config.security.x_content_type_options { "Enabled" } else { "Disabled" });
    println!("   - X-XSS-Protection: {}", if config.security.x_xss_protection { "Enabled" } else { "Disabled" });
    println!("   - Content-Security-Policy: {}", if config.security.content_security_policy.is_some() { "Enabled" } else { "Disabled" });
    println!("   - Referrer-Policy: {}", config.security.referrer_policy.as_deref().unwrap_or("Disabled"));

    println!(" Compression: {}", if config.compression.enabled {
        format!("Enabled (level: {}, min: {} bytes)", 
            config.compression.level,
            config.compression.min_size_bytes)
    } else {
        "Disabled".to_string()
    });

    if config.server.tls.enabled {
        println!(" HTTPS: Enabled (port {})", config.server.https_port);
        println!("   - Certificate: {}", config.server.tls.cert_path);
        println!("   - Key: {}", config.server.tls.key_path);
    } else {
        println!(" HTTPS: Disabled");
    }
    

    let pool_manager_for_shutdown = state.pool_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("\n[Server] Shutting down gracefully...");
        let pm = pool_manager_for_shutdown.lock().await;
        pm.stop_all().await;
        std::process::exit(0);
    });

    // graceful reload
    let state_for_reload = state.clone();
    let config_path = cli.config.clone();
    tokio::spawn(async move {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("Failed to install SIGHUP handler");
        
        loop {
            sig.recv().await;
            println!("\n[SIGHUP] Received reload signal...");
            
            // reload config dari file
            let mut new_config = match Config::load_from_file(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[Reload] Failed to load config: {}", e);
                    continue;
                }
            };
            
            new_config.apply_env_overrides();
            
            if let Err(e) = new_config.validate() {
                eprintln!("[Reload] Invalid config: {}", e);
                continue;
            }
            
            println!("[Reload] Config reloaded successfully");
            
            // update shared config
            {
                let mut config = state_for_reload.config.lock().await;
                *config = new_config.clone();
            }
            
            // rolling restart all pools
            let pm = state_for_reload.pool_manager.lock().await;
            for (name, pool_arc) in &pm.pools {
                println!("[Reload] Reloading pool '{}'...", name);
                let mut pool = pool_arc.lock().await;
                if let Err(e) = pool.reload(&new_config).await {
                    eprintln!("[Reload] Failed to reload pool '{}': {}", name, e);
                }
            }
        }
    });

    let pool_manager_for_shutdown = state.pool_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("\n[Server] Shutting down gracefully...");
        let pm = pool_manager_for_shutdown.lock().await;  // ← LOCK DULU
        pm.stop_all().await;
        std::process::exit(0);
    });

    use axum_server::tls_rustls::RustlsConfig;

    if config.server.tls.enabled {
        println!(" Setting up HTTPS server...");
        
        let https_addr: std::net::SocketAddr = format!("{}:{}", config.server.host, config.server.https_port)
            .parse()
            .expect("Invalid HTTPS address");
        
        let rustls_config = RustlsConfig::from_pem_file(
            &config.server.tls.cert_path,
            &config.server.tls.key_path,
        )
        .await
        .expect("Failed to load TLS config");
        
        println!(" HTTPS (HTTP/2) listening on https://{}", https_addr);
        
        let app_clone = app.clone();
        tokio::spawn(async move {
            axum_server::bind_rustls(https_addr, rustls_config)
                .serve(app_clone.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .expect("HTTPS server failed");
        });
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    // Start Background Queue Worker
    let queue_worker = job_queue.clone();
    tokio::spawn(async move {
        println!("[Queue Worker] Background worker started!");
        loop {
            // Cek antrian setiap 500ms
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            if let Some(job_id) = queue_worker.dequeue().await {
                println!("[Queue Worker] Processing job: {}", job_id);
                queue_worker.mark_processing(&job_id).await;
                
                // Simulasi proses berat (nanti bisa diganti eksekusi PHP)
                tokio::time::sleep(Duration::from_secs(3)).await;
                
                // Tandai selesai
                queue_worker.mark_completed(
                    &job_id, 
                    serde_json::json!({"message": "Task processed successfully"})
                ).await;
            }
        }
    });
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>()
    ).await.unwrap();

}
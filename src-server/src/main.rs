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
mod job_handlers;
mod middleware;
mod db_proxy;

use queue::JobQueue;
use job_handlers::HandlerRegistry;
use clap::Parser;
use config::Config;
use types::AppState;
use metrics::Metrics;
use handlers::{php_handler, health_handler, metrics_handler, reload_handler};
use logger::Logger;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use axum::routing::{get, post};
use axum::Router;
use rate_limiter::RateLimiter;
use tower_http::compression::CompressionLayer;
use pool_manager::PoolManager;
use std::time::Duration;
use middleware::admin_auth::admin_auth_middleware;
use axum::middleware as axum_middleware;

#[derive(Parser, Debug)]
#[command(name = "bakpiarun", about = "PHP Runtime Server")]
struct Cli {
    #[arg(short, long, default_value = "../config/bakpiarun.yaml")]
    config: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    println!("Initializing bakpiaRun Server...");

    let mut config = match Config::load_from_file(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    config.apply_env_overrides();

    /*if let Err(e) = config.validate() {
        eprintln!("Invalid config: {}", e);
        std::process::exit(1);
    }*/
        // Set admin token from config to environment variable (if not already set)
    if std::env::var("BAKPIA_ADMIN_TOKEN").is_err() {
        if let Some(ref token) = config.server.admin_token {
            std::env::set_var("BAKPIA_ADMIN_TOKEN", token);
            println!("Admin token loaded from config file");
        }
    } else {
        println!("Admin token loaded from environment variable");
    }

    println!("   Configuration:");
    println!("   - Server: {}:{}", config.server.host, config.server.port);
    println!("   - Docroot: {}", config.php.docroot);
    println!("   - Worker Path: {}", config.php.worker_path);
    println!("   - Workers: {}", config.php.worker_count);
    println!("   - Memory Limit: {} MB", config.php.memory_limit_mb);
    println!("   - Max Requests: {}", config.php.max_requests);
    println!("   - Socket Dir: {}", config.socket.directory);
    println!("   - Queue Enabled: {}", config.queue.enabled);

    std::fs::create_dir_all(&config.socket.directory)
        .expect("Failed to create socket directory");

    // Initialize Pool Manager
    println!("Initializing Pool Manager...");
    let pool_manager = PoolManager::new(&config).await;
    let pool_manager = Arc::new(Mutex::new(pool_manager));

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

    // Initialize Job Queue (only if enabled)
    let job_queue: Option<Arc<JobQueue>> = if config.queue.enabled {
        let queue = Arc::new(JobQueue::new());
        println!("Queue System initialized (max jobs: {})", config.queue.max_jobs);
        Some(queue)
    } else {
        println!("Queue System DISABLED in config");
        None
    };

    // Initialize Job Handlers Registry (only if queue enabled)
    let handler_registry: Option<Arc<HandlerRegistry>> = if config.queue.enabled {
        let registry = Arc::new(HandlerRegistry::new());
        println!("Job Handlers Registry initialized");
        Some(registry)
    } else {
        None
    };


    // Initialize Secure DB Proxy (Fase 2)
    if config.db_proxy.enabled {
        let db_config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = db_proxy::DbProxy::start(&db_config).await {
                eprintln!("❌ DB Proxy failed to start: {}", e);
            }
        });
    }

    let state = AppState {
        config: Arc::new(Mutex::new(config.clone())),
        pool_manager: pool_manager.clone(),
        metrics: Arc::new(Mutex::new(metrics)),
        logger: Arc::new(logger),
        rate_limiter: Arc::new(rate_limiter),
        queue: job_queue.clone(),
    };

    // Build Router
    //let mut app = Router::new()
    //    .route("/", get(php_handler).post(php_handler))
    //    .route("/*path", get(php_handler).post(php_handler))
    //    .route("/health", get(health_handler))
    //    .route("/metrics", get(metrics_handler))
    //    .route("/reload", get(reload_handler));

    // Build Router
    let mut app = Router::new()
        .route("/", get(php_handler).post(php_handler))
        .route("/*path", get(php_handler).post(php_handler));

    // Add admin routes with authentication
    if config.queue.enabled {
        // Queue routes + admin routes with auth
        let admin_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/reload", get(reload_handler))
            .route("/api/queue/submit", post(handlers::submit_job))
            .route("/api/queue/status/:id", get(handlers::get_job_status))
            //.layer(middleware::from_fn(admin_auth_middleware));
            .layer(axum_middleware::from_fn(admin_auth_middleware));
        
        app = app.merge(admin_routes);
        println!("Queue API routes registered WITH authentication");
    } else {
        // Admin routes only (queue disabled)
        let admin_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/reload", get(reload_handler))
            //.layer(middleware::from_fn(admin_auth_middleware));
            .layer(axum_middleware::from_fn(admin_auth_middleware));
        app = app.merge(admin_routes);
        println!("Queue API routes DISABLED");
    }

    let app = app.with_state(state.clone());

    let app = app.with_state(state.clone());

    // compression middleware
    let app = if config.compression.enabled {
        app.layer(CompressionLayer::new())
    } else {
        app
    };

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("Listening on http://{}", addr);
    println!("Anti-OOM system active!");
    println!("Worker pool: {} workers with round-robin load balancing", config.php.worker_count);
    println!("Request timeout: {}ms", config.php.timeout_ms);
    println!("Health check: http://{}/health", addr);
    println!("Metrics: http://{}/metrics", addr);

    // Logging config
    println!("Logging Configuration:");
    println!("  - Access Log: {}", if config.logging.access_log_enabled { 
        format!("Enabled ({})", config.logging.access_log) 
    } else { 
        "Disabled".to_string() 
    });
    println!("  - Error Log:  {}", if config.logging.error_log_enabled { 
        format!("Enabled ({})", config.logging.error_log) 
    } else { 
        "Disabled".to_string() 
    });

    // Rate limit
    println!("Rate Limiting: {}", if config.rate_limit.enabled { 
        format!("Enabled ({} req/min, burst: {})", 
            config.rate_limit.requests_per_minute,
            config.rate_limit.burst_size)
    } else { 
        "Disabled".to_string() 
    });

    // Security Headers
    println!("Security Headers:");
    println!("  - X-Frame-Options: {}", config.security.x_frame_options.as_deref().unwrap_or("Disabled"));
    println!("  - X-Content-Type-Options: {}", if config.security.x_content_type_options { "Enabled" } else { "Disabled" });
    println!("  - X-XSS-Protection: {}", if config.security.x_xss_protection { "Enabled" } else { "Disabled" });
    println!("  - Content-Security-Policy: {}", if config.security.content_security_policy.is_some() { "Enabled" } else { "Disabled" });
    println!("  - Referrer-Policy: {}", config.security.referrer_policy.as_deref().unwrap_or("Disabled"));

    // Compression
    println!("Compression: {}", if config.compression.enabled {
        format!("Enabled (level: {}, min: {} bytes)", 
            config.compression.level,
            config.compression.min_size_bytes)
    } else {
        "Disabled".to_string()
    });

    // HTTPS
    if config.server.tls.enabled {
        println!("HTTPS: Enabled (port {})", config.server.https_port);
        println!("  - Certificate: {}", config.server.tls.cert_path);
        println!("  - Key: {}", config.server.tls.key_path);
    } else {
        println!("HTTPS: Disabled");
    }

    // Start Background Queue Worker (only if enabled)
    if let (Some(queue_worker), Some(handlers)) = (job_queue.clone(), handler_registry.clone()) {
        tokio::spawn(async move {
            println!("[Queue Worker] Background worker started with Handlers!");
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                
                if let Some(job_id) = queue_worker.dequeue().await {
                    println!("[Queue Worker] Processing job: {}", job_id);
                    queue_worker.mark_processing(&job_id).await;
                    
                    if let Some(job) = queue_worker.get_status(&job_id).await {
                        match handlers.execute(&job.task, job.payload.clone()) {
                            Ok(result) => {
                                queue_worker.mark_completed(&job_id, result).await;
                            }
                            Err(error) => {
                                println!("[Queue Worker] Job failed: {}", error);
                                queue_worker.mark_failed(&job_id, error).await;
                            }
                        }
                    }
                }
            }
        });
    } else {
        println!("[Queue Worker] DISABLED (queue is off)");
    }

    // Graceful shutdown
    let pool_manager_for_shutdown = state.pool_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("\n[Server] Shutting down gracefully...");
        let pm = pool_manager_for_shutdown.lock().await;
        pm.stop_all().await;
        std::process::exit(0);
    });

    // Graceful reload (SIGHUP)
    let state_for_reload = state.clone();
    let config_path = cli.config.clone();
    tokio::spawn(async move {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("Failed to install SIGHUP handler");
        
        loop {
            sig.recv().await;
            println!("\n[SIGHUP] Received reload signal...");
            
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
            
            {
                let mut config = state_for_reload.config.lock().await;
                *config = new_config.clone();
            }
            
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

    // HTTPS setup
    use axum_server::tls_rustls::RustlsConfig;

    if config.server.tls.enabled {
        println!("Setting up HTTPS server...");
        
        let https_addr: std::net::SocketAddr = format!("{}:{}", config.server.host, config.server.https_port)
            .parse()
            .expect("Invalid HTTPS address");
        
        let rustls_config = RustlsConfig::from_pem_file(
            &config.server.tls.cert_path,
            &config.server.tls.key_path,
        )
        .await
        .expect("Failed to load TLS config");
        
        println!("HTTPS (HTTP/2) listening on https://{}", https_addr);
        
        let app_clone = app.clone();
        tokio::spawn(async move {
            axum_server::bind_rustls(https_addr, rustls_config)
                .serve(app_clone.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .expect("HTTPS server failed");
        });
    }

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>()
    ).await.unwrap();
}
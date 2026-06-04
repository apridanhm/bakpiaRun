mod config;
mod static_file;
mod types;
mod ipc;
mod worker;
mod worker_pool;
mod metrics;
mod handlers;
mod logger;

use clap::Parser;
use config::Config;
use types::AppState;
use worker_pool::WorkerPool;
use metrics::Metrics;
use handlers::{php_handler, health_handler, metrics_handler};
use logger::Logger;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use axum::routing::get;
use axum::Router;
use tokio::time::{sleep, Duration};

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

    let mut pool = WorkerPool::new(config.php.worker_count, &config);

    if let Err(e) = pool.start_all(&config).await {
        eprintln!(" Failed to start workers: {}", e);
        std::process::exit(1);
    }

    let metrics = Metrics::new(config.php.worker_count);

    let logger = Logger::new(
        config.logging.access_log_enabled,
        &config.logging.access_log,
        config.logging.error_log_enabled,
        &config.logging.error_log,
    );

    let state = AppState {
        config: Arc::new(config.clone()),
        pool: Arc::new(Mutex::new(pool)),
        metrics: Arc::new(Mutex::new(metrics)),
        logger: Arc::new(logger),
    };

    let app = Router::new()
        .route("/", get(php_handler).post(php_handler))
        .route("/*path", get(php_handler).post(php_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .with_state(state.clone());

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!(" Listening on http://{}", addr);
    println!(" Anti-OOM system active!");
    println!(" Worker pool: {} workers with round-robin load balancing", config.php.worker_count);
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>()
    ).await.unwrap();
}
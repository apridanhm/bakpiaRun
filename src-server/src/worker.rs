use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};
use crate::config::Config;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::UnixStream;

pub struct Worker {
    pub index: usize,
    pub process: Option<Child>,
    pub socket_path: String,
    pub requests_handled: u64,
    pub last_memory: u64,
    pub last_peak: u64,
    pub connection_pool: Arc<Mutex<Vec<UnixStream>>>,
    pub pool_size: usize,
}

impl Worker {
    pub fn new(index: usize, socket_path: String, pool_size: usize) -> Self {
        Self {
            index,
            process: None,
            socket_path,
            requests_handled: 0,
            last_memory: 0,
            last_peak: 0,
            connection_pool: Arc::new(Mutex::new(Vec::with_capacity(pool_size))),
            pool_size,
        }
    }

    pub async fn start(&mut self, config: &Config) -> Result<(), String> {
        println!("[Supervisor] Starting PHP worker #{}...", self.index);

        let _ = std::fs::remove_file(&self.socket_path);

        let timeout_sec = (config.php.timeout_ms as f64 / 1000.0).max(1.0);
        let connect_host = if config.server.host == "0.0.0.0" {
            "127.0.0.1"
        } else {
            &config.server.host
        };
        let base_url = format!("http://{}:{}", connect_host, config.server.port);

        let mut cmd = Command::new(&config.php.binary_path);
        cmd.arg(&config.php.worker_path)
            .current_dir(&config.php.docroot)
            .env("BAKPIARUN_SOCKET_PATH", &self.socket_path)
            .env("BAKPIARUN_TIMEOUT_SEC", timeout_sec.to_string())
            .env("BAKPIARUN_MAX_PAYLOAD_MB", config.php.max_payload_mb.to_string())
            .env("BAKPIARUN_SERVER_HOST", &config.server.host)
            .env("BAKPIARUN_SERVER_PORT", config.server.port.to_string())
            .env("BAKPIARUN_BASE_URL", &base_url);

        if let Some(ref token) = config.server.admin_token {
            cmd.env("BAKPIARUN_ADMIN_TOKEN", token);
        }

        let child = cmd
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

        // Wait until socket is ready or up to worker_startup_delay_ms
        let max_wait_ms = config.php.worker_startup_delay_ms;
        let mut waited_ms = 0;
        while waited_ms < max_wait_ms {
            if std::path::Path::new(&self.socket_path).exists() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
            waited_ms += 10;
        }
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            println!("[Supervisor] Stopping PHP worker #{}...", self.index);

            if let Err(e) = child.kill().await {
                eprintln!("[Supervisor] Error killing worker #{}: {}", self.index, e);
            }

            let _ = child.wait().await;
            println!("[Supervisor] PHP worker #{} stopped", self.index);
        }
    }

    pub async fn check_health(&mut self) -> bool {
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

    pub async fn ensure_running(&mut self, config: &Config) -> Result<(), String> {
        if !self.check_health().await {
            println!("[Supervisor] Worker #{} is dead, restarting...", self.index);
            self.stop().await;
            self.start(config).await?;
        }
        Ok(())
    }

    pub fn should_restart(&self, config: &Config) -> Option<String> {
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

    pub fn update_stats(&mut self, memory: u64, peak: u64) {
        self.requests_handled += 1;
        self.last_memory = memory;
        self.last_peak = peak;
    }
}

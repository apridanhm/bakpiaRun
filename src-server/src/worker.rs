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

    pub fn should_restart(&self, memory_limit_mb: u64, max_requests: u64) -> Option<String> {
        let memory_bytes = memory_limit_mb * 1024 * 1024;

        if self.last_memory > memory_bytes {
            return Some(format!(
                "Worker #{}: Memory limit exceeded ({} MB > {} MB)",
                self.index,
                self.last_memory / 1024 / 1024,
                memory_limit_mb
            ));
        }

        if self.requests_handled >= max_requests {
            return Some(format!(
                "Worker #{}: Max requests reached ({} >= {})",
                self.index, self.requests_handled, max_requests
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

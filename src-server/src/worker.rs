use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};
use crate::config::Config;
use crate::anti_oom::read_process_proc_status;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::UnixStream;

pub struct Worker {
    pub index: usize,
    pub process: Option<Child>,
    pub pid: Option<u32>,
    pub socket_path: String,
    pub requests_handled: u64,
    pub last_memory: u64,
    pub last_peak: u64,
    /// Resident Set Size (VmRSS) read from `/proc/[pid]/status` in bytes
    pub rss_bytes: u64,
    /// High Water Mark (VmHWM) read from `/proc/[pid]/status` in bytes
    pub hwm_bytes: u64,
    /// Number of currently active in-flight requests on this worker
    pub active_requests: usize,
    /// True when the worker is being drained for graceful recycling (no new requests assigned)
    pub draining: bool,
    /// Total number of times this worker has been proactively recycled by the Anti-OOM Engine
    pub recycle_count: u64,
    pub connection_pool: Arc<Mutex<Vec<UnixStream>>>,
    pub pool_size: usize,
}

impl Worker {
    pub fn new(index: usize, socket_path: String, pool_size: usize) -> Self {
        Self {
            index,
            process: None,
            pid: None,
            socket_path,
            requests_handled: 0,
            last_memory: 0,
            last_peak: 0,
            rss_bytes: 0,
            hwm_bytes: 0,
            active_requests: 0,
            draining: false,
            recycle_count: 0,
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
        // Anti-PHP2Shell & Hardening INI flags
        cmd.arg("-d").arg("allow_url_include=0");
        if !config.php.disable_functions.trim().is_empty() {
            cmd.arg("-d")
                .arg(format!("disable_functions={}", config.php.disable_functions));
        }
        if config.php.open_basedir_docroot {
            let worker_dir = std::path::Path::new(&config.php.worker_path)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(&config.php.docroot);
            let open_basedir = format!(
                "{}:{}:{}:{}:/tmp",
                config.php.docroot,
                worker_dir,
                config.socket.directory,
                config.php.upload_tmp_dir
            );
            cmd.arg("-d").arg(format!("open_basedir={}", open_basedir));
        }

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

        self.pid = child.id();
        self.process = Some(child);
        self.requests_handled = 0;
        self.last_memory = 0;
        self.last_peak = 0;
        self.draining = false;

        println!(
            "[Supervisor] PHP worker #{} started (PID: {:?}, Socket: {})",
            self.index,
            self.pid,
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

        // Clear stale UDS connections from previous worker process
        {
            let mut pool = self.connection_pool.lock().await;
            pool.clear();
        }

        self.refresh_proc_memory();
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            println!("[Supervisor] Stopping PHP worker #{} (PID: {:?})...", self.index, self.pid);

            if let Err(e) = child.kill().await {
                eprintln!("[Supervisor] Error killing worker #{}: {}", self.index, e);
            }

            let _ = child.wait().await;
            self.pid = None;
            self.rss_bytes = 0;
            self.hwm_bytes = 0;
            let mut pool = self.connection_pool.lock().await;
            pool.clear();
            println!("[Supervisor] PHP worker #{} stopped", self.index);
        }
    }

    /// Reads `/proc/[pid]/status` to update OS-level `VmRSS` and `VmHWM` for this worker.
    pub fn refresh_proc_memory(&mut self) {
        if let Some(pid) = self.pid {
            if let Some(proc_mem) = read_process_proc_status(pid) {
                self.rss_bytes = proc_mem.vm_rss_bytes;
                self.hwm_bytes = proc_mem.vm_hwm_bytes;
            }
        }
    }

    /// Graceful Worker Recycling:
    /// 1. Marks worker as `draining = true` so the router stops assigning new requests to it.
    /// 2. Waits for any active in-flight request to finish.
    /// 3. Terminates the old PHP worker process and immediately spawns a fresh replacement.
    pub async fn graceful_recycle(&mut self, config: &Config) -> Result<(), String> {
        if self.draining {
            return Ok(());
        }
        self.draining = true;
        println!(
            "[Anti-OOM] Gracefully recycling worker #{} (PID: {:?}, active_requests: {})...",
            self.index, self.pid, self.active_requests
        );

        // Wait up to 2 seconds if an active request is finishing
        let mut waited_ms = 0u64;
        while self.active_requests > 0 && waited_ms < 2000 {
            sleep(Duration::from_millis(20)).await;
            waited_ms += 20;
        }

        self.stop().await;
        self.recycle_count += 1;
        let res = self.start(config).await;
        self.draining = false;
        res
    }

    pub async fn check_health(&mut self) -> bool {
        if let Some(ref mut child) = self.process {
            match child.try_wait() {
                Ok(Some(status)) => {
                    println!("[Supervisor] Worker #{} exited: {}", self.index, status);
                    self.pid = None;
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

    /// Evaluates whether the worker should be proactively recycled based on:
    /// 1. `/proc/[pid]/status` `VmRSS` exceeding `worker_threshold_percent` of `memory_limit_mb`
    /// 2. PHP `memory_get_peak_usage(true)` or `memory_get_usage(true)` exceeding threshold
    /// 3. `requests_handled` reaching `max_requests`
    pub fn should_restart(&self, config: &Config) -> Option<String> {
        if !config.anti_oom.enabled {
            return None;
        }

        let limit_bytes = config.php.memory_limit_mb * 1024 * 1024;
        let threshold_pct = config.anti_oom.worker_threshold_percent.clamp(1, 100);
        let threshold_bytes = (limit_bytes * threshold_pct) / 100;

        // 1. Check OS-level VmRSS from /proc/[pid]/status
        if self.rss_bytes > threshold_bytes {
            return Some(format!(
                "Worker #{} (PID {:?}): /proc/[pid]/status VmRSS exceeded threshold ({} MB > {} MB [{}% of {} MB])",
                self.index,
                self.pid,
                self.rss_bytes / 1024 / 1024,
                threshold_bytes / 1024 / 1024,
                threshold_pct,
                config.php.memory_limit_mb
            ));
        }

        // 2. Check PHP peak / current memory reported via UDS IPC
        let observed_php_mem = self.last_memory.max(self.last_peak);
        if observed_php_mem > threshold_bytes {
            return Some(format!(
                "Worker #{} (PID {:?}): PHP memory threshold exceeded ({} MB > {} MB [{}% of {} MB])",
                self.index,
                self.pid,
                observed_php_mem / 1024 / 1024,
                threshold_bytes / 1024 / 1024,
                threshold_pct,
                config.php.memory_limit_mb
            ));
        }

        // 3. Check max_requests lifecycle limit
        if self.requests_handled >= config.php.max_requests {
            return Some(format!(
                "Worker #{} (PID {:?}): Max requests reached ({} >= {})",
                self.index, self.pid, self.requests_handled, config.php.max_requests
            ));
        }

        None
    }

    pub fn update_stats(&mut self, memory: u64, peak: u64) {
        self.requests_handled += 1;
        self.last_memory = memory;
        self.last_peak = peak;
        self.refresh_proc_memory();
    }
}

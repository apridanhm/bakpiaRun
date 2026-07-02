use crate::worker::Worker;
use crate::config::Config;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;

pub struct WorkerPool {
    /// Each worker is individually lockable so the request path can operate on a
    /// single worker (start/restart/stats) without blocking the whole pool.
    pub workers: Vec<Arc<Mutex<Worker>>>,
    current_index: AtomicUsize,
    /// Effective per-pool limits (resolved from pool config or global defaults).
    pub memory_limit_mb: u64,
    pub max_requests: u64,
    pub timeout_ms: u64,
}

impl WorkerPool {
    pub fn new(
        worker_count: usize,
        config: &Config,
        memory_limit_mb: u64,
        max_requests: u64,
        timeout_ms: u64,
    ) -> Self {
        let mut workers = Vec::with_capacity(worker_count);

        for i in 0..worker_count {
            let socket_path = config.get_worker_socket_path(i);
            workers.push(Arc::new(Mutex::new(Worker::new(
                i,
                socket_path,
                config.php.connection_pool_size,
            ))));
        }

        Self {
            workers,
            current_index: AtomicUsize::new(0),
            memory_limit_mb,
            max_requests,
            timeout_ms,
        }
    }

    pub async fn start_all(&self, config: &Config) -> Result<(), String> {
        for worker in &self.workers {
            worker.lock().await.start(config).await?;
        }
        Ok(())
    }

    pub async fn stop_all(&self) {
        for worker in &self.workers {
            worker.lock().await.stop().await;
        }
    }

    /// Round-robin selection. Returns the worker index (for logging/metrics)
    /// and a clone of its handle so the caller can lock just that worker.
    pub fn get_next_worker(&self) -> (usize, Arc<Mutex<Worker>>) {
        let index = self.current_index.fetch_add(1, Ordering::SeqCst) % self.workers.len();
        (index, self.workers[index].clone())
    }

    // rolling restart untuk graceful reload
    pub async fn reload(&mut self, config: &Config) -> Result<(), String> {
        println!("[Reload] Starting rolling restart of workers...");

        // stop semua worker lama
        println!("[Reload] Stopping old workers...");
        self.stop_all().await;

        // update jumlah worker kalau berubah
        let new_count = config.php.worker_count;
        let old_count = self.workers.len();

        if new_count != old_count {
            println!("[Reload] Worker count changed: {} → {}", old_count, new_count);
            self.workers.clear();
            for i in 0..new_count {
                let socket_path = config.get_worker_socket_path(i);
                self.workers.push(Arc::new(Mutex::new(Worker::new(
                    i,
                    socket_path,
                    config.php.connection_pool_size,
                ))));
            }
        }

        // start worker baru satu-satu (rolling)
        println!("[Reload] Starting new workers...");
        for worker in &self.workers {
            worker.lock().await.start(config).await?;
            // Delay kecil biar gak overload
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        println!("[Reload] Rolling restart complete! {} workers active", self.workers.len());
        Ok(())
    }
}

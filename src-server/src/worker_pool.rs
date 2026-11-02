use crate::worker::Worker;
use crate::config::Config;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct WorkerPool {
    /// Each worker has its own fine-grained Mutex so recycling or inspecting one worker
    /// never blocks other workers in the same pool during high-concurrency k6 benchmarks.
    pub workers: Vec<Arc<Mutex<Worker>>>,
    pub config: Config,
    current_index: AtomicUsize,
}

impl WorkerPool {
    pub fn new(worker_count: usize, config: &Config) -> Self {
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
            config: config.clone(),
            current_index: AtomicUsize::new(0),
        }
    }

    pub async fn start_all(&mut self, config: &Config) -> Result<(), String> {
        self.config = config.clone();
        for worker_arc in &self.workers {
            let mut worker = worker_arc.lock().await;
            worker.start(config).await?;
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) {
        for worker_arc in &self.workers {
            let mut worker = worker_arc.lock().await;
            worker.stop().await;
        }
    }

    /// Selects the next available (non-draining) worker using lock-free atomic round-robin
    /// and non-blocking `try_lock()` to skip workers currently undergoing graceful recycling.
    pub fn get_next_worker(&self) -> (usize, Arc<Mutex<Worker>>) {
        let len = self.workers.len();
        for _ in 0..len {
            let index = self.current_index.fetch_add(1, Ordering::Relaxed) % len;
            if let Ok(guard) = self.workers[index].try_lock() {
                if !guard.draining {
                    drop(guard);
                    return (index, self.workers[index].clone());
                }
            }
        }
        let fallback_idx = self.current_index.fetch_add(1, Ordering::Relaxed) % len;
        (fallback_idx, self.workers[fallback_idx].clone())
    }

    // rolling restart untuk graceful reload
    pub async fn reload(&mut self, config: &Config) -> Result<(), String> {
        println!("[Reload] Starting rolling restart of workers...");
        self.config = config.clone();

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
        let delay_ms = config.php.rolling_restart_delay_ms;
        for worker_arc in &self.workers {
            {
                let mut worker = worker_arc.lock().await;
                worker.start(config).await?;
            }
            // Delay kecil biar gak overload
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }

        println!(
            "[Reload] Rolling restart complete! {} workers active",
            self.workers.len()
        );
        Ok(())
    }
}

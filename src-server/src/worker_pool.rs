use crate::worker::Worker;
use crate::config::Config;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct WorkerPool {
    pub workers: Vec<Worker>,
    current_index: AtomicUsize,
}

impl WorkerPool {
    pub fn new(worker_count: usize, config: &Config) -> Self {
        let mut workers = Vec::with_capacity(worker_count);

        for i in 0..worker_count {
            let socket_path = config.get_worker_socket_path(i);
            workers.push(Worker::new(i, socket_path));
        }

        Self {
            workers,
            current_index: AtomicUsize::new(0),
        }
    }

    pub async fn start_all(&mut self, config: &Config) -> Result<(), String> {
        for worker in &mut self.workers {
            worker.start(config).await?;
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) {
        for worker in &mut self.workers {
            worker.stop().await;
        }
    }

    pub fn get_next_worker(&self) -> usize {
        let index = self.current_index.fetch_add(1, Ordering::SeqCst);
        index % self.workers.len()
    }

    pub async fn ensure_all_running(&mut self, config: &Config) {
        for worker in &mut self.workers {
            if let Err(e) = worker.ensure_running(config).await {
                eprintln!("[Supervisor] Failed to restart worker #{}: {}", worker.index, e);
            }
        }
    }
}

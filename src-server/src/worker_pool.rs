use crate::config::Config;
use crate::worker::Worker;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Lock-free atomic state handle for fast Idle-First / Least-Loaded worker selection
/// without acquiring any Mutex.
#[derive(Clone)]
struct WorkerSlot {
    in_flight: Arc<AtomicUsize>,
    draining: Arc<AtomicBool>,
}

/// RAII guard that automatically decrements a worker's `in_flight` atomic counter
/// when the HTTP request finishes, errors, or is cancelled/dropped.
pub struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct WorkerPool {
    /// Each worker has its own fine-grained Mutex so recycling or inspecting one worker
    /// never blocks other workers in the same pool during high-concurrency k6 benchmarks.
    pub workers: Vec<Arc<Mutex<Worker>>>,
    slots: Vec<WorkerSlot>,
    pub config: Config,
    current_index: AtomicUsize,
}

impl WorkerPool {
    pub fn new(worker_count: usize, config: &Config) -> Self {
        let mut workers = Vec::with_capacity(worker_count);
        let mut slots = Vec::with_capacity(worker_count);

        for i in 0..worker_count {
            let socket_path = config.get_worker_socket_path(i);
            let worker = Worker::new(i, socket_path);
            slots.push(WorkerSlot {
                in_flight: worker.in_flight.clone(),
                draining: worker.draining.clone(),
            });
            workers.push(Arc::new(Mutex::new(worker)));
        }

        Self {
            workers,
            slots,
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

    /// Selects the optimal worker using Lock-Free **Idle-First / Least-Loaded** scheduling:
    /// 1. Starts from a rotating atomic offset (`current_index`) so load spreads evenly.
    /// 2. Fast-path: Atomically claims (`compare_exchange(0, 1)`) the first non-draining idle worker (`in_flight == 0`).
    /// 3. Fallback (when all workers are busy): Picks the non-draining worker with the lowest `in_flight` queue depth.
    pub fn get_next_worker(&self) -> (usize, Arc<Mutex<Worker>>, InFlightGuard) {
        let len = self.workers.len();
        let start = self.current_index.fetch_add(1, Ordering::Relaxed) % len;

        let mut best_idx: Option<usize> = None;
        let mut min_load = usize::MAX;

        for offset in 0..len {
            let idx = (start + offset) % len;
            let slot = &self.slots[idx];

            if slot.draining.load(Ordering::Acquire) {
                continue;
            }

            // Fast-path: atomically claim an idle worker with zero collision across CPU cores
            match slot.in_flight.compare_exchange(
                0,
                1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return (
                        idx,
                        self.workers[idx].clone(),
                        InFlightGuard {
                            counter: slot.in_flight.clone(),
                        },
                    );
                }
                Err(current_load) => {
                    if current_load < min_load {
                        min_load = current_load;
                        best_idx = Some(idx);
                    }
                }
            }
        }

        // All non-draining workers currently have >= 1 active request; pick the least-loaded one
        let chosen = best_idx.unwrap_or(start);
        let counter = self.slots[chosen].in_flight.clone();
        counter.fetch_add(1, Ordering::AcqRel);

        (
            chosen,
            self.workers[chosen].clone(),
            InFlightGuard { counter },
        )
    }

    // Rolling restart untuk graceful reload
    pub async fn reload(&mut self, config: &Config) -> Result<(), String> {
        println!("[Reload] Starting rolling restart of workers...");
        self.config = config.clone();

        println!("[Reload] Stopping old workers...");
        self.stop_all().await;

        let new_count = config.php.worker_count;
        let old_count = self.workers.len();

        if new_count != old_count {
            println!(
                "[Reload] Worker count changed: {} → {}",
                old_count, new_count
            );
            self.workers.clear();
            self.slots.clear();
            for i in 0..new_count {
                let socket_path = config.get_worker_socket_path(i);
                let worker = Worker::new(i, socket_path);
                self.slots.push(WorkerSlot {
                    in_flight: worker.in_flight.clone(),
                    draining: worker.draining.clone(),
                });
                self.workers.push(Arc::new(Mutex::new(worker)));
            }
        }

        println!("[Reload] Starting new workers...");
        let delay_ms = config.php.rolling_restart_delay_ms;
        for worker_arc in &self.workers {
            {
                let mut worker = worker_arc.lock().await;
                worker.start(config).await?;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }

        println!(
            "[Reload] Rolling restart complete! {} workers active",
            self.workers.len()
        );
        Ok(())
    }
}

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

    /// True Zero-Downtime Rolling Reload:
    /// Recycles existing workers ONE BY ONE using `graceful_recycle` (drain active requests -> restart)
    /// while the rest of the pool continues serving live traffic without interruption.
    pub async fn reload(&mut self, config: &Config) -> Result<(), String> {
        println!("[Reload] Starting zero-downtime rolling reload of workers...");
        self.config = config.clone();

        let new_count = config.php.worker_count;
        let old_count = self.workers.len();
        let delay_ms = config.php.rolling_restart_delay_ms;

        // 1. Scale UP first if worker_count increased so extra capacity is online immediately
        if new_count > old_count {
            println!("[Reload] Scaling up workers: {} → {}", old_count, new_count);
            for i in old_count..new_count {
                let socket_path = config.get_worker_socket_path(i);
                let mut worker = Worker::new(i, socket_path);
                worker.start(config).await?;
                self.slots.push(WorkerSlot {
                    in_flight: worker.in_flight.clone(),
                    draining: worker.draining.clone(),
                });
                self.workers.push(Arc::new(Mutex::new(worker)));
            }
        }

        // 2. Gracefully recycle existing workers one by one (0..min(old_count, new_count))
        let common_count = old_count.min(new_count);
        for i in 0..common_count {
            let worker_arc = self.workers[i].clone();
            {
                let mut worker = worker_arc.lock().await;
                worker.socket_path = config.get_worker_socket_path(i);
                worker.graceful_recycle(config).await?;
            }
            if delay_ms > 0 && i + 1 < common_count {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
        }

        // 3. Scale DOWN excess workers gracefully after active workers are refreshed
        if new_count < old_count {
            println!("[Reload] Scaling down workers: {} → {}", old_count, new_count);
            for i in new_count..old_count {
                let worker_arc = self.workers[i].clone();
                let mut worker = worker_arc.lock().await;
                worker.draining.store(true, Ordering::Release);
                let mut waited = 0u64;
                while worker.active_requests() > 0 && waited < 2000 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    waited += 10;
                }
                worker.stop().await;
            }
            self.workers.truncate(new_count);
            self.slots.truncate(new_count);
        }

        println!(
            "[Reload] Zero-downtime rolling reload complete! {} workers active",
            self.workers.len()
        );
        Ok(())
    }
}

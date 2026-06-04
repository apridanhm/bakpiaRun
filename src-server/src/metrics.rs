use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub total_requests: u64,
    pub total_errors: u64,
    start_time: Instant,
    pub requests_per_worker: Vec<u64>,
    pub memory_per_worker: Vec<u64>,
}

impl Metrics {
    pub fn new(worker_count: usize) -> Self {
        Self {
            total_requests: 0,
            total_errors: 0,
            start_time: Instant::now(),
            requests_per_worker: vec![0; worker_count],
            memory_per_worker: vec![0; worker_count],
        }
    }

    pub fn record_request(&mut self, worker_index: usize, memory: u64) {
        self.total_requests += 1;
        if worker_index < self.requests_per_worker.len() {
            self.requests_per_worker[worker_index] += 1;
            self.memory_per_worker[worker_index] = memory;
        }
    }

    pub fn record_error(&mut self) {
        self.total_errors += 1;
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

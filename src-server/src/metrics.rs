use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

const RING_CAPACITY: usize = 4096;

/// Standard HTTP latency buckets in milliseconds for Prometheus histograms
pub const LATENCY_BUCKETS_MS: [f64; 11] = [
    1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 5000.0,
];

#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub samples: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone)]
pub struct PoolMetrics {
    pub requests: u64,
    pub errors: u64,
    /// Circular ring buffer of recent request durations in microseconds (O(1) zero-alloc updates)
    ring_us: Vec<u64>,
    write_pos: usize,
    /// Cumulative sum of all request durations in milliseconds (for Prometheus `_sum`)
    pub duration_sum_ms: f64,
    /// Cumulative counts per Prometheus `le` bucket
    pub bucket_counts: [u64; 11],
}

impl PoolMetrics {
    pub fn new() -> Self {
        Self {
            requests: 0,
            errors: 0,
            ring_us: Vec::with_capacity(RING_CAPACITY),
            write_pos: 0,
            duration_sum_ms: 0.0,
            bucket_counts: [0; 11],
        }
    }

    pub fn record_latency(&mut self, duration_us: u64) {
        self.requests += 1;
        let duration_ms = (duration_us as f64) / 1000.0;
        self.duration_sum_ms += duration_ms;

        for (i, &bound_ms) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if duration_ms <= bound_ms {
                self.bucket_counts[i] += 1;
            }
        }

        if self.ring_us.len() < RING_CAPACITY {
            self.ring_us.push(duration_us);
        } else {
            self.ring_us[self.write_pos] = duration_us;
        }
        self.write_pos = (self.write_pos + 1) % RING_CAPACITY;
    }

    pub fn record_error(&mut self, duration_us: u64) {
        self.errors += 1;
        self.record_latency(duration_us);
    }

    pub fn summary(&self) -> LatencySummary {
        if self.ring_us.is_empty() {
            return LatencySummary {
                samples: 0,
                avg_ms: 0.0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
                max_ms: 0.0,
            };
        }

        let mut sorted = self.ring_us.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        let sum_us: u128 = sorted.iter().map(|&v| v as u128).sum();
        let avg_ms = (sum_us as f64 / n as f64) / 1000.0;

        let p50_ms = percentile_us(&sorted, 0.50) / 1000.0;
        let p95_ms = percentile_us(&sorted, 0.95) / 1000.0;
        let p99_ms = percentile_us(&sorted, 0.99) / 1000.0;
        let max_ms = (*sorted.last().unwrap_or(&0) as f64) / 1000.0;

        LatencySummary {
            samples: n,
            avg_ms: round2(avg_ms),
            p50_ms: round2(p50_ms),
            p95_ms: round2(p95_ms),
            p99_ms: round2(p99_ms),
            max_ms: round2(max_ms),
        }
    }
}

fn percentile_us(sorted: &[u64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * quantile).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64
}

fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub total_requests: u64,
    pub total_errors: u64,
    start_time: Instant,
    pub pools: HashMap<String, PoolMetrics>,
}

impl Metrics {
    pub fn new(_worker_count: usize) -> Self {
        Self {
            total_requests: 0,
            total_errors: 0,
            start_time: Instant::now(),
            pools: HashMap::new(),
        }
    }

    pub fn record_pool_request(&mut self, pool_name: &str, duration_us: u64) {
        self.total_requests += 1;
        let entry = self
            .pools
            .entry(pool_name.to_string())
            .or_insert_with(PoolMetrics::new);
        entry.record_latency(duration_us);
    }

    pub fn record_pool_error(&mut self, pool_name: &str, duration_us: u64) {
        self.total_requests += 1;
        self.total_errors += 1;
        let entry = self
            .pools
            .entry(pool_name.to_string())
            .or_insert_with(PoolMetrics::new);
        entry.record_error(duration_us);
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MAX_TRACKED_IPS: usize = 16_384;
const STALE_ENTRY_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    pub tokens: f64,
    pub last_update: Instant,
}

#[derive(Debug, Clone)]
pub struct RateLimitHeaders {
    pub limit: u32,
    pub remaining: u32,
    pub reset: u64,
}

#[derive(Debug, Clone)]
pub struct RateLimitError {
    pub limit: u32,
    pub reset: u64,
}

struct RateLimiterState {
    enabled: bool,
    requests_per_minute: u32,
    burst_size: u32,
    clients: HashMap<IpAddr, RateLimitInfo>,
    last_cleanup: Instant,
}

pub struct RateLimiter {
    inner: Mutex<RateLimiterState>,
}

impl RateLimiter {
    pub fn new(enabled: bool, requests_per_minute: u32, burst_size: u32) -> Self {
        Self {
            inner: Mutex::new(RateLimiterState {
                enabled,
                requests_per_minute: requests_per_minute.max(1),
                burst_size: burst_size.max(1),
                clients: HashMap::new(),
                last_cleanup: Instant::now(),
            }),
        }
    }

    pub async fn update_config(&self, enabled: bool, requests_per_minute: u32, burst_size: u32) {
        let mut state = self.inner.lock().await;
        state.enabled = enabled;
        state.requests_per_minute = requests_per_minute.max(1);
        state.burst_size = burst_size.max(1);
        if !enabled {
            state.clients.clear();
        }
    }

    /// Checks token bucket rate limit for `ip`.
    /// Returns `Ok(None)` when rate limiting is disabled, `Ok(Some(headers))` when allowed,
    /// or `Err(RateLimitError)` when throttled (`HTTP 429`).
    pub async fn check_rate_limit(
        &self,
        ip: IpAddr,
    ) -> Result<Option<RateLimitHeaders>, RateLimitError> {
        let mut state = self.inner.lock().await;
        if !state.enabled {
            return Ok(None);
        }

        let now = Instant::now();

        // Periodically evict stale IP entries to keep memory strictly bounded
        if state.clients.len() >= MAX_TRACKED_IPS
            || now.duration_since(state.last_cleanup) >= STALE_ENTRY_TTL
        {
            state
                .clients
                .retain(|_, v| now.duration_since(v.last_update) < STALE_ENTRY_TTL);
            state.last_cleanup = now;
        }

        let rpm = state.requests_per_minute;
        let burst = state.burst_size as f64;
        let refill_rate = (rpm as f64) / 60.0;

        let info = state.clients.entry(ip).or_insert(RateLimitInfo {
            tokens: burst,
            last_update: now,
        });

        let elapsed = now.duration_since(info.last_update).as_secs_f64();
        info.tokens = (info.tokens + elapsed * refill_rate).min(burst);
        info.last_update = now;

        if info.tokens >= 1.0 {
            info.tokens -= 1.0;
            let remaining = info.tokens as u32;
            let reset_seconds = if remaining == 0 {
                ((1.0 - info.tokens) / refill_rate).ceil().max(1.0) as u64
            } else {
                60
            };
            Ok(Some(RateLimitHeaders {
                limit: rpm,
                remaining,
                reset: reset_seconds,
            }))
        } else {
            let needed = 1.0 - info.tokens;
            let reset_seconds = (needed / refill_rate).ceil().max(1.0) as u64;
            Err(RateLimitError {
                limit: rpm,
                reset: reset_seconds,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_disabled_returns_none() {
        let rl = RateLimiter::new(false, 60, 2);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(rl.check_rate_limit(ip).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_rate_limiter_enforces_burst_limit() {
        let rl = RateLimiter::new(true, 60, 2);
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        assert!(rl.check_rate_limit(ip).await.unwrap().is_some());
        assert!(rl.check_rate_limit(ip).await.unwrap().is_some());
        // 3rd immediate request exceeds burst_size = 2
        assert!(rl.check_rate_limit(ip).await.is_err());
    }
}

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
//use std::time::{Duration, Instant};
use std::time::Instant;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    pub tokens: f64,
    pub last_update: Instant,
}

#[derive(Debug)]
pub struct RateLimitHeaders {
    pub limit: u32,
    pub remaining: u32,
    pub reset: u64,
}

pub struct RateLimiter {
    enabled: bool,
    requests_per_minute: u32,
    burst_size: u32,
    clients: Arc<Mutex<HashMap<IpAddr, RateLimitInfo>>>,
}

impl RateLimiter {
    pub fn new(enabled: bool, requests_per_minute: u32, burst_size: u32) -> Self {
        Self {
            enabled,
            requests_per_minute,
            burst_size,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn check_rate_limit(&self, ip: IpAddr) -> Result<RateLimitHeaders, RateLimitError> {
        if !self.enabled {
            return Ok(RateLimitHeaders {
                limit: 0,
                remaining: 0,
                reset: 0,
            });
        }
    
        let mut clients = self.clients.lock().await;
        let now = Instant::now();
        
        let info = clients.entry(ip).or_insert(RateLimitInfo {
            tokens: self.burst_size as f64,
            last_update: now,
        });
    
        let elapsed = now.duration_since(info.last_update);
        let refill_rate = self.requests_per_minute as f64 / 60.0;
        let tokens_to_add = elapsed.as_secs_f64() * refill_rate;
        
        info.tokens = (info.tokens + tokens_to_add).min(self.burst_size as f64);
        info.last_update = now;
    
        let remaining = info.tokens as u32;
        let reset_seconds = if remaining == 0 {
            ((self.burst_size as f64 - info.tokens) / refill_rate).ceil() as u64
        } else {
            60
        };
    
        if info.tokens >= 1.0 {
            info.tokens -= 1.0;
            Ok(RateLimitHeaders {
                limit: self.requests_per_minute,
                remaining: info.tokens as u32,
                reset: reset_seconds,
            })
        } else {
            Err(RateLimitError {
                limit: self.requests_per_minute,
                reset: reset_seconds,
            })
        }
    }
}

#[derive(Debug)]
pub struct RateLimitError {
    pub limit: u32,
    pub reset: u64,
}
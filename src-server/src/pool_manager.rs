use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use axum::http::HeaderMap;
use crate::config::{Config, PoolConfig};
use crate::worker_pool::WorkerPool;

#[derive(Debug, Clone)]
pub struct RoutingRule {
    pub pool_name: String,
    pub patterns: Vec<String>,
    pub headers: HashMap<String, String>,
}

pub struct PoolManager {
    pub pools: HashMap<String, Arc<Mutex<WorkerPool>>>,
    pub routing_table: Vec<RoutingRule>,
}

impl PoolManager {
    pub fn build_pool_config(config: &Config, pool_config: &PoolConfig) -> Config {
        let pool_socket_dir = format!("{}/{}", config.socket.directory, pool_config.name);
        let mut pool_specific_config = config.clone();
        pool_specific_config.socket.directory = pool_socket_dir;
        pool_specific_config.php.worker_count = pool_config.worker_count;
        if let Some(mem) = pool_config.memory_limit_mb {
            pool_specific_config.php.memory_limit_mb = mem;
        }
        if let Some(max_req) = pool_config.max_requests {
            pool_specific_config.php.max_requests = max_req;
        }
        if let Some(timeout) = pool_config.timeout_ms {
            pool_specific_config.php.timeout_ms = timeout;
        }
        pool_specific_config
    }

    pub async fn new(config: &Config) -> Self {
        let mut pools = HashMap::new();
        let mut routing_table = Vec::new();

        // Pastikan ada minimal 1 pool di config
        if config.pools.is_empty() {
            eprintln!("[PoolManager] WARNING: No pools defined in config! Using default.");
        }

        for pool_config in &config.pools {
            println!("[PoolManager] Initializing pool '{}' with {} workers...", 
                     pool_config.name, pool_config.worker_count);
            
            let pool_specific_config = Self::build_pool_config(config, pool_config);
            
            // Create pool-specific socket directory
            std::fs::create_dir_all(&pool_specific_config.socket.directory)
                .expect(&format!("Failed to create socket directory for pool '{}'", pool_config.name));
            
            let mut pool = WorkerPool::new(pool_config.worker_count, &pool_specific_config);
        
            // Start workers untuk pool ini
            if let Err(e) = pool.start_all(&pool_specific_config).await {
                eprintln!("[PoolManager] Failed to start pool '{}': {}", pool_config.name, e);
                continue;
            }
            
            println!("[PoolManager] Pool '{}' successfully started!", pool_config.name);
            
            pools.insert(
                pool_config.name.clone(),
                Arc::new(Mutex::new(pool))
            );
            
            routing_table.push(RoutingRule {
                pool_name: pool_config.name.clone(),
                patterns: pool_config.patterns.clone(),
                headers: pool_config.headers.clone(),
            });
        }

        Self { pools, routing_table }
    }

    pub fn update_routing_table(&mut self, config: &Config) {
        let mut new_table = Vec::new();
        for pool_config in &config.pools {
            if self.pools.contains_key(&pool_config.name) {
                new_table.push(RoutingRule {
                    pool_name: pool_config.name.clone(),
                    patterns: pool_config.patterns.clone(),
                    headers: pool_config.headers.clone(),
                });
            }
        }
        self.routing_table = new_table;
    }

    fn match_header_value(actual: &str, expected_pattern: &str) -> bool {
        let actual_lower = actual.trim().to_lowercase();
        let expected_lower = expected_pattern.trim().to_lowercase();

        if expected_lower == "*" || actual_lower == expected_lower {
            return true;
        }
        if expected_lower.ends_with('*') {
            let prefix = &expected_lower[..expected_lower.len() - 1];
            return actual_lower.starts_with(prefix);
        }
        if expected_lower.starts_with('*') {
            let suffix = &expected_lower[1..];
            return actual_lower.ends_with(suffix);
        }
        false
    }

    fn match_headers(req_headers: &HeaderMap, rule_headers: &HashMap<String, String>) -> bool {
        if rule_headers.is_empty() {
            return false;
        }
        for (rule_key, rule_val) in rule_headers {
            for (req_key, req_val) in req_headers.iter() {
                if req_key.as_str().eq_ignore_ascii_case(rule_key) {
                    if let Ok(val_str) = req_val.to_str() {
                        if Self::match_header_value(val_str, rule_val) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn match_pattern(path: &str, pattern: &str) -> bool {
        // Exact match
        if path == pattern {
            return true;
        }
        
        // Catch-all
        if pattern == "/*" { 
            return true;
        }
        
        // Prefix with wildcard: "/heavy-*" matches "/heavy-db.php"
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return path.starts_with(prefix);
        }
        
        // Directory pattern: "/api/*" matches "/api/users"
        if pattern.ends_with("/*") {
            let prefix = &pattern[..pattern.len() - 2];
            return path.starts_with(prefix);
        }
        
        // Suffix pattern: "/*.php" matches "/index.php"
        if pattern.starts_with("/*") {
            let suffix = &pattern[2..];
            return path.ends_with(suffix);
        }
        
        false
    }

    pub async fn stop_all(&self) {
        println!("[PoolManager] Stopping all pools...");
        for (name, pool) in &self.pools {
            println!("[PoolManager] Stopping pool '{}'...", name);
            let mut pool = pool.lock().await;
            pool.stop_all().await;
        }
        println!("[PoolManager] All pools stopped.");
    }

    /// Klasifikasi traffic HTTP ke Light/Fast Pool atau Heavy Pool berdasarkan
    /// atribut HTTP Header (prioritas pertama) dan pola URL (prioritas kedua).
    pub fn get_pool_name_for_request(&self, path: &str, req_headers: &HeaderMap) -> Option<String> {
        // 1. Cek atribut HTTP Header terlebih dahulu (misal X-Workload: heavy)
        for rule in &self.routing_table {
            if Self::match_headers(req_headers, &rule.headers) {
                return Some(rule.pool_name.clone());
            }
        }

        // 2. Cek pencocokan pola URL (patterns)
        for rule in &self.routing_table {
            for pattern in &rule.patterns {
                if Self::match_pattern(path, pattern) {
                    return Some(rule.pool_name.clone());
                }
            }
        }
        
        // Fallback ke pool pertama
        self.routing_table.first().map(|rule| rule.pool_name.clone())
    }

    #[allow(dead_code)]
    pub fn get_pool_name_for_path(&self, path: &str) -> Option<String> {
        let empty_headers = HeaderMap::new();
        self.get_pool_name_for_request(path, &empty_headers)
    }

    pub fn get_pool(&self, name: &str) -> Option<&Arc<Mutex<WorkerPool>>> {
        self.pools.get(name)
    }
}
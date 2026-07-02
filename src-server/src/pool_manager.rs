use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::config::{Config, PoolConfig};
use crate::worker_pool::WorkerPool;

pub struct PoolManager {
    pub pools: HashMap<String, Arc<Mutex<WorkerPool>>>,
    pub routing_table: Vec<(Vec<String>, String)>,
}

impl PoolManager {
    pub async fn new(config: &Config) -> Self {
        let mut pools = HashMap::new();
        let mut routing_table = Vec::new();

        // Fall back to a single catch-all pool when none are configured, so the
        // server is usable with a minimal config (and never has zero pools).
        let default_pools;
        let pool_configs: &[PoolConfig] = if config.pools.is_empty() {
            eprintln!("[PoolManager] No pools defined in config; creating a default catch-all pool.");
            default_pools = vec![PoolConfig {
                name: "default".to_string(),
                worker_count: config.php.worker_count,
                memory_limit_mb: None,
                max_requests: None,
                timeout_ms: None,
                patterns: vec!["/*".to_string()],
            }];
            &default_pools
        } else {
            &config.pools
        };

        for pool_config in pool_configs {
            println!("[PoolManager] Initializing pool '{}' with {} workers...",
                     pool_config.name, pool_config.worker_count);

            // Create pool-specific socket directory
            let pool_socket_dir = format!("{}/{}", config.socket.directory, pool_config.name);
            std::fs::create_dir_all(&pool_socket_dir)
                .expect(&format!("Failed to create socket directory for pool '{}'", pool_config.name));

            // Create pool-specific config
            let mut pool_specific_config = config.clone();
            pool_specific_config.socket.directory = pool_socket_dir;

            // Resolve effective limits: per-pool override, else global default.
            let memory_limit_mb = pool_config.memory_limit_mb.unwrap_or(config.php.memory_limit_mb);
            let max_requests = pool_config.max_requests.unwrap_or(config.php.max_requests);
            let timeout_ms = pool_config.timeout_ms.unwrap_or(config.php.timeout_ms);
            println!(
                "[PoolManager] Pool '{}' limits: memory={}MB, max_requests={}, timeout={}ms",
                pool_config.name, memory_limit_mb, max_requests, timeout_ms
            );

            let pool = WorkerPool::new(
                pool_config.worker_count,
                &pool_specific_config,
                memory_limit_mb,
                max_requests,
                timeout_ms,
            );

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
            
            routing_table.push((
                pool_config.patterns.clone(),
                pool_config.name.clone()
            ));
        }

        Self { pools, routing_table }
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
            let pool = pool.lock().await;
            pool.stop_all().await;
        }
        println!("[PoolManager] All pools stopped.");
    }

    pub fn get_pool_name_for_path(&self, path: &str) -> Option<String> {
        for (patterns, pool_name) in &self.routing_table {
            for pattern in patterns {
                if Self::match_pattern(path, pattern) {
                    return Some(pool_name.clone());
                }
            }
        }
        
        // Fallback ke pool pertama
        self.routing_table.first().map(|(_, name)| name.clone())
    }

    pub fn get_pool(&self, name: &str) -> Option<&Arc<Mutex<WorkerPool>>> {
        self.pools.get(name)
    }
}
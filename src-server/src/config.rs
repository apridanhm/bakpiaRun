use serde::Deserialize;
use std::env;
use std::fs;
//use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub php: PhpConfig,
    pub socket: SocketConfig,
    pub logging: LoggingConfig,
    pub rate_limit: RateLimitConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SecurityConfig {
    pub x_frame_options: Option<String>,
    pub x_content_type_options: bool,
    pub x_xss_protection: bool,
    pub content_security_policy: Option<String>,
    pub strict_transport_security: Option<String>,
    pub referrer_policy: Option<String>,
    pub permissions_policy: Option<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            x_frame_options: Some("DENY".to_string()),
            x_content_type_options: true,
            x_xss_protection: true,
            content_security_policy: None,
            strict_transport_security: None,
            referrer_policy: Some("strict-origin-when-cross-origin".to_string()),
            permissions_policy: None,
        }
    }
}


#[derive(Debug, Deserialize, Clone)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_minute: 60,
            burst_size: 10,
        }
    }
}


#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PhpConfig {
    pub docroot: String,
    pub worker_path: String,
    pub worker_count: usize,
    pub memory_limit_mb: u64,
    pub max_requests: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SocketConfig {
    pub directory: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    pub access_log_enabled: bool,
    pub access_log: String,
    pub error_log_enabled: bool,
    pub error_log: String,
}

impl Config {
    // Load config dari file YAML
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        
        let config: Config = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        
        Ok(config)
    }

    // Override config dengan environment variables
    pub fn apply_env_overrides(&mut self) {
        if let Ok(host) = env::var("BAKPIARUN_HOST") {
            self.server.host = host;
        }
        
        if let Ok(port) = env::var("BAKPIARUN_PORT") {
            if let Ok(port_num) = port.parse::<u16>() {
                self.server.port = port_num;
            }
        }
        
        if let Ok(docroot) = env::var("BAKPIARUN_DOCROOT") {
            self.php.docroot = docroot;
        }
        
        if let Ok(worker_count) = env::var("BAKPIARUN_WORKER_COUNT") {
            if let Ok(count) = worker_count.parse::<usize>() {
                self.php.worker_count = count;
            }
        }
        
        if let Ok(memory_limit) = env::var("BAKPIARUN_MEMORY_LIMIT_MB") {
            if let Ok(limit) = memory_limit.parse::<u64>() {
                self.php.memory_limit_mb = limit;
            }
        }
    }

    // Get socket path untuk worker index
    pub fn get_worker_socket_path(&self, worker_index: usize) -> String {
        format!("{}/worker_{}.sock", self.socket.directory, worker_index)
    }

    // Validate config
    pub fn validate(&self) -> Result<(), String> {
        if self.server.port == 0 {
            return Err("Port cannot be 0".to_string());
        }
        
        if self.php.worker_count == 0 {
            return Err("Worker count must be at least 1".to_string());
        }
        
        if self.php.memory_limit_mb == 0 {
            return Err("Memory limit must be at least 1 MB".to_string());
        }
        
        if self.php.max_requests == 0 {
            return Err("Max requests must be at least 1".to_string());
        }

        // validasi timeout
        if self.php.timeout_ms == 0 {
            return Err("Timeout cannot be 0. Set at least 1000ms (1 second)".to_string());
        }
        
        if self.php.timeout_ms < 1000 {
            return Err("Timeout too small. Minimum 1000ms (1 second) recommended".to_string());
        }
        
        Ok(())
    }


}
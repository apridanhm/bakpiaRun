use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
//use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(skip)]
    pub config_path: String,
    pub server: ServerConfig,
    pub php: PhpConfig,
    pub socket: SocketConfig,
    pub logging: LoggingConfig,
    pub rate_limit: RateLimitConfig,
    pub security: SecurityConfig,
    pub compression: CompressionConfig,
    #[serde(default)]
    pub anti_oom: AntiOomConfig,
    pub pools: Vec<PoolConfig>,
    pub queue: QueueConfig,
    pub db_proxy: DbProxyConfig, 
}

#[derive(Debug, Deserialize, Clone)]
pub struct AntiOomConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_anti_oom_check_interval_ms")]
    pub check_interval_ms: u64,
    #[serde(default = "default_worker_threshold_percent")]
    pub worker_threshold_percent: u64,
    #[serde(default = "default_cgroup_threshold_percent")]
    pub cgroup_threshold_percent: u64,
    #[serde(default = "default_cgroup_memory_current_path")]
    pub cgroup_memory_current_path: String,
    #[serde(default = "default_cgroup_memory_max_path")]
    pub cgroup_memory_max_path: String,
}

impl Default for AntiOomConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_ms: default_anti_oom_check_interval_ms(),
            worker_threshold_percent: default_worker_threshold_percent(),
            cgroup_threshold_percent: default_cgroup_threshold_percent(),
            cgroup_memory_current_path: default_cgroup_memory_current_path(),
            cgroup_memory_max_path: default_cgroup_memory_max_path(),
        }
    }
}

fn default_anti_oom_check_interval_ms() -> u64 {
    1000
}

fn default_worker_threshold_percent() -> u64 {
    80
}

fn default_cgroup_threshold_percent() -> u64 {
    85
}

fn default_cgroup_memory_current_path() -> String {
    "/sys/fs/cgroup/memory.current".to_string()
}

fn default_cgroup_memory_max_path() -> String {
    "/sys/fs/cgroup/memory.max".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct PoolConfig {
    pub name: String,
    pub worker_count: usize,
    pub memory_limit_mb: Option<u64>,
    pub max_requests: Option<u64>,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct QueueConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_queue_max_jobs")]
    pub max_jobs: usize,
    #[serde(default = "default_queue_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_true() -> bool {
    true
}

fn default_queue_max_jobs() -> usize {
    10000
}

fn default_queue_poll_interval_ms() -> u64 {
    500
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
pub struct CompressionConfig {
    pub enabled: bool,
    pub min_size_bytes: usize,
    pub level: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_size_bytes: 1024,
            level: 6,
        }
    }
}


#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub https_port: u16,
    pub tls: TlsConfig,
    pub admin_token: Option<String>,
    #[serde(default = "default_max_body_size_mb")]
    pub max_body_size_mb: usize,
    #[serde(default = "default_static_cache_max_age")]
    pub static_cache_max_age: u64,
}

fn default_max_body_size_mb() -> usize {
    50
}

fn default_static_cache_max_age() -> u64 {
    31536000
}

#[derive(Debug, Deserialize, Clone)]
pub struct PhpConfig {
    #[serde(default = "default_php_binary")]
    pub binary_path: String,
    pub docroot: String,
    pub worker_path: String,
    pub worker_count: usize,
    pub memory_limit_mb: u64,
    pub max_requests: u64,
    pub timeout_ms: u64,
    #[allow(dead_code)]
    #[serde(default = "default_connection_pool_size")]
    pub connection_pool_size: usize,
    #[serde(default = "default_upload_tmp_dir")]
    pub upload_tmp_dir: String,
    #[serde(default = "default_max_payload_mb")]
    pub max_payload_mb: usize,
    #[serde(default = "default_worker_startup_delay_ms")]
    pub worker_startup_delay_ms: u64,
    #[serde(default = "default_rolling_restart_delay_ms")]
    pub rolling_restart_delay_ms: u64,
    #[serde(default = "default_content_type")]
    pub default_content_type: String,
    #[serde(default = "default_disable_functions")]
    pub disable_functions: String,
    #[serde(default = "default_true")]
    pub open_basedir_docroot: bool,
}

fn default_php_binary() -> String {
    "php".to_string()
}

fn default_connection_pool_size() -> usize {
    1
}

fn default_upload_tmp_dir() -> String {
    "/tmp/bakpiarun_uploads".to_string()
}

fn default_max_payload_mb() -> usize {
    100
}

fn default_worker_startup_delay_ms() -> u64 {
    500
}

fn default_rolling_restart_delay_ms() -> u64 {
    200
}

fn default_content_type() -> String {
    "text/html; charset=utf-8".to_string()
}

fn default_disable_functions() -> String {
    "exec,passthru,shell_exec,system,proc_open,popen,pcntl_exec".to_string()
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
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl LoggingConfig {
    pub fn is_debug(&self) -> bool {
        self.level.eq_ignore_ascii_case("debug")
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
}

impl Config {
    // Load config dari file YAML
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        
        let mut config: Config = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        config.config_path = path.to_string();
        
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

        if let Ok(worker_path) = env::var("BAKPIARUN_WORKER_PATH") {
            self.php.worker_path = worker_path;
        }

        if let Ok(socket_dir) = env::var("BAKPIARUN_SOCKET_DIR") {
            self.socket.directory = socket_dir;
        }

        if let Ok(binary_path) = env::var("BAKPIARUN_PHP_BINARY") {
            self.php.binary_path = binary_path;
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

        if let Ok(tls_enabled) = env::var("BAKPIARUN_TLS_ENABLED") {
            if let Ok(enabled) = tls_enabled.parse::<bool>() {
                self.server.tls.enabled = enabled;
            }
        }

        if let Ok(queue_enabled) = env::var("BAKPIARUN_QUEUE_ENABLED") {
            if let Ok(enabled) = queue_enabled.parse::<bool>() {
                self.queue.enabled = enabled;
            }
        }

        if let Ok(db_proxy_enabled) = env::var("BAKPIARUN_DB_PROXY_ENABLED") {
            if let Ok(enabled) = db_proxy_enabled.parse::<bool>() {
                self.db_proxy.enabled = enabled;
            }
        }

        if let Ok(db_host) = env::var("BAKPIARUN_DB_HOST") {
            self.db_proxy.target.host = db_host;
        }
        if let Ok(db_port) = env::var("BAKPIARUN_DB_PORT") {
            if let Ok(p) = db_port.parse::<u16>() {
                self.db_proxy.target.port = p;
            }
        }
        if let Ok(db_user) = env::var("BAKPIARUN_DB_USER") {
            self.db_proxy.target.username = db_user;
        }
        if let Ok(db_pass) = env::var("BAKPIARUN_DB_PASSWORD") {
            self.db_proxy.target.password = db_pass;
        }
        if let Ok(db_name) = env::var("BAKPIARUN_DB_NAME") {
            self.db_proxy.target.database = db_name;
        }

        if let Ok(worker_thresh) = env::var("BAKPIARUN_ANTI_OOM_WORKER_THRESHOLD") {
            if let Ok(pct) = worker_thresh.parse::<u64>() {
                self.anti_oom.worker_threshold_percent = pct;
            }
        }
        if let Ok(cgroup_thresh) = env::var("BAKPIARUN_ANTI_OOM_CGROUP_THRESHOLD") {
            if let Ok(pct) = cgroup_thresh.parse::<u64>() {
                self.anti_oom.cgroup_threshold_percent = pct;
            }
        }
        if let Ok(log_level) = env::var("BAKPIARUN_LOG_LEVEL") {
            self.logging.level = log_level;
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

        if self.php.timeout_ms < 1000 {
            return Err("Timeout too small. Minimum 1000ms (1 second) required".to_string());
        }

        if !std::path::Path::new(&self.php.docroot).exists() {
            return Err(format!("PHP docroot does not exist: {}", self.php.docroot));
        }

        if !std::path::Path::new(&self.php.worker_path).exists() {
            return Err(format!("PHP worker_path does not exist: {}", self.php.worker_path));
        }

        if self.pools.is_empty() {
            return Err("At least one worker pool must be defined in 'pools'".to_string());
        }

        for pool in &self.pools {
            if pool.name.trim().is_empty() {
                return Err("Pool name cannot be empty".to_string());
            }
            if pool.worker_count == 0 {
                return Err(format!("Pool '{}': worker_count must be at least 1", pool.name));
            }
            if pool.memory_limit_mb == Some(0) {
                return Err(format!("Pool '{}': memory_limit_mb must be at least 1 MB", pool.name));
            }
            if pool.max_requests == Some(0) {
                return Err(format!("Pool '{}': max_requests must be at least 1", pool.name));
            }
            if let Some(t) = pool.timeout_ms {
                if t < 1000 {
                    return Err(format!("Pool '{}': timeout_ms must be at least 1000ms", pool.name));
                }
            }
        }

        if self.anti_oom.worker_threshold_percent == 0 || self.anti_oom.worker_threshold_percent > 100 {
            return Err("anti_oom.worker_threshold_percent must be between 1 and 100".to_string());
        }

        if self.anti_oom.cgroup_threshold_percent == 0 || self.anti_oom.cgroup_threshold_percent > 100 {
            return Err("anti_oom.cgroup_threshold_percent must be between 1 and 100".to_string());
        }
        
        Ok(())
    }


}


// DB PROXY CONFIGURATION
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct DbProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_listen_address")]
    pub listen_address: String,
    #[serde(default = "default_db_proxy_port")]
    pub listen_port: u16,
    
    // INI JADI FALLBACK KALAU GAK BISA BACA DARI DB ASLI
    #[serde(default = "default_spoof_version")]
    pub spoof_version: String,
    
    pub target: DbTargetConfig,
    pub pool: DbPoolConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DbTargetConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct DbPoolConfig {
    #[serde(default = "default_min_conn")]
    pub min_connections: usize,
    #[serde(default = "default_max_conn")]
    pub max_connections: usize,
}

fn default_listen_address() -> String { "127.0.0.1".to_string() }
fn default_db_proxy_port() -> u16 { 3307 }
fn default_spoof_version() -> String { "8.0.33-bakpiarun-proxy".to_string() }
fn default_min_conn() -> usize { 2 }
fn default_max_conn() -> usize { 20 }
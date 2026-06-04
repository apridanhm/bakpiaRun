use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use chrono::Local;

pub struct Logger {
    access_enabled: bool,
    access_file: Option<Mutex<std::fs::File>>,
    error_enabled: bool,
    error_file: Option<Mutex<std::fs::File>>,
}

impl Logger {
    pub fn new(
        access_enabled: bool, access_path: &str,
        error_enabled: bool, error_path: &str
    ) -> Self {
        
        let access_file = if access_enabled {
            if let Some(parent) = std::path::Path::new(access_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new().create(true).append(true).open(access_path) {
                Ok(f) => Some(Mutex::new(f)),
                Err(e) => {
                    eprintln!("[Logger] Failed to open access log {}: {}", access_path, e);
                    None
                }
            }
        } else {
            None
        };

        let error_file = if error_enabled {
            if let Some(parent) = std::path::Path::new(error_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new().create(true).append(true).open(error_path) {
                Ok(f) => Some(Mutex::new(f)),
                Err(e) => {
                    eprintln!("[Logger] Failed to open error log {}: {}", error_path, e);
                    None
                }
            }
        } else {
            None
        };

        Self {
            access_enabled,
            access_file,
            error_enabled,
            error_file,
        }
    }

    pub fn log_access(&self, ip: &str, method: &str, uri: &str, status: u16, duration_ms: u64) {
        if !self.access_enabled { return; }
        
        if let Some(file_mutex) = &self.access_file {
            let timestamp = Local::now().format("%d/%b/%Y:%H:%M:%S %z");
            let log_line = format!(
                "[{}] {} {} {} {} {}ms\n",
                timestamp, ip, method, uri, status, duration_ms
            );

            if let Ok(mut file) = file_mutex.lock() {
                let _ = file.write_all(log_line.as_bytes());
                let _ = file.flush();
            }
        }
    }

    pub fn log_error(&self, level: &str, message: &str) {
        if !self.error_enabled { return; }
        
        if let Some(file_mutex) = &self.error_file {
            let timestamp = Local::now().format("%d/%b/%Y:%H:%M:%S %z");
            let log_line = format!(
                "[{}] [{}] {}\n",
                timestamp, level.to_uppercase(), message
            );

            if let Ok(mut file) = file_mutex.lock() {
                let _ = file.write_all(log_line.as_bytes());
                let _ = file.flush();
            }
        }
    }
}
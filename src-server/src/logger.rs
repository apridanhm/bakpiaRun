use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use chrono::Local;

enum LogTarget {
    Stdout,
    Stderr,
    File(Mutex<std::fs::File>),
}

pub struct Logger {
    access_enabled: bool,
    access_target: Option<LogTarget>,
    error_enabled: bool,
    error_target: Option<LogTarget>,
}

impl Logger {
    fn open_target(path: &str, label: &str) -> Option<LogTarget> {
        match path.trim() {
            "/dev/stdout" | "stdout" => Some(LogTarget::Stdout),
            "/dev/stderr" | "stderr" => Some(LogTarget::Stderr),
            file_path => {
                if let Some(parent) = std::path::Path::new(file_path).parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                match OpenOptions::new().create(true).append(true).open(file_path) {
                    Ok(f) => Some(LogTarget::File(Mutex::new(f))),
                    Err(e) => {
                        eprintln!("[Logger] Failed to open {} log {}: {}", label, file_path, e);
                        None
                    }
                }
            }
        }
    }

    pub fn new(
        access_enabled: bool,
        access_path: &str,
        error_enabled: bool,
        error_path: &str,
    ) -> Self {
        let access_target = if access_enabled {
            Self::open_target(access_path, "access")
        } else {
            None
        };

        let error_target = if error_enabled {
            Self::open_target(error_path, "error")
        } else {
            None
        };

        Self {
            access_enabled,
            access_target,
            error_enabled,
            error_target,
        }
    }

    fn write_line(target: &LogTarget, line: &str) {
        match target {
            LogTarget::Stdout => print!("{}", line),
            LogTarget::Stderr => eprint!("{}", line),
            LogTarget::File(file_mutex) => {
                if let Ok(mut file) = file_mutex.lock() {
                    let _ = file.write_all(line.as_bytes());
                    let _ = file.flush();
                }
            }
        }
    }

    pub fn log_access(&self, ip: &str, method: &str, uri: &str, status: u16, duration_ms: u64) {
        if !self.access_enabled {
            return;
        }

        if let Some(target) = &self.access_target {
            let timestamp = Local::now().format("%d/%b/%Y:%H:%M:%S %z");
            let log_line = format!(
                "[{}] {} {} {} {} {}ms\n",
                timestamp, ip, method, uri, status, duration_ms
            );
            Self::write_line(target, &log_line);
        }
    }

    pub fn log_error(&self, level: &str, message: &str) {
        if !self.error_enabled {
            return;
        }

        if let Some(target) = &self.error_target {
            let timestamp = Local::now().format("%d/%b/%Y:%H:%M:%S %z");
            let log_line = format!(
                "[{}] [{}] {}\n",
                timestamp,
                level.to_uppercase(),
                message
            );
            Self::write_line(target, &log_line);
        }
    }
}

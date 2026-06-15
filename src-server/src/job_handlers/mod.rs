use serde_json::Value;
use std::collections::HashMap;

pub mod email;
pub mod report;

pub trait JobHandler: Send + Sync {
    fn execute(&self, payload: Value) -> Result<Value, String>;
}

pub struct HandlerRegistry {
    handlers: HashMap<String, Box<dyn JobHandler>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };
        
        // Daftarkan SEMUA task yang diizinkan di sini
        // Hanya task yang didaftarkan yang bisa diproses
        registry.register("send_welcome_email", Box::new(email::SendWelcomeEmailHandler));
        registry.register("generate_report", Box::new(report::GenerateReportHandler));
        
        // Tambahkan task baru di sini sesuai kebutuhan
        // registry.register("nama_task_baru", Box::new(NamaHandlerBaru));
        
        registry
    }
    
    pub fn register(&mut self, task_name: &str, handler: Box<dyn JobHandler>) {
        self.handlers.insert(task_name.to_string(), handler);
    }
    
    pub fn execute(&self, task_name: &str, payload: Value) -> Result<Value, String> {
        // HANYA task yang terdaftar yang bisa diproses
        match self.handlers.get(task_name) {
            Some(handler) => handler.execute(payload),
            None => {
                // Log attempt untuk audit
                println!("[SECURITY] Blocked unknown task attempt: {}", task_name);
                Err(format!("Task '{}' is not registered. Only registered tasks can be executed.", task_name))
            }
        }
    }
    
    // Helper untuk list semua task yang terdaftar
    #[allow(dead_code)]
    pub fn list_tasks(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}
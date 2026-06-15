use serde_json::Value;
use std::collections::HashMap;

pub mod email;
pub mod report;
pub mod generic;

pub trait JobHandler: Send + Sync {
    fn execute(&self, payload: Value) -> Result<Value, String>;
}

pub struct HandlerRegistry {
    handlers: HashMap<String, Box<dyn JobHandler>>,
    default_handler: Box<dyn JobHandler>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
            default_handler: Box::new(generic::GenericHandler),
        };
        
        // Register specific handlers (optional)
        registry.register("send_welcome_email", Box::new(email::SendWelcomeEmailHandler));
        registry.register("generate_report", Box::new(report::GenerateReportHandler));
        
        registry
    }
    
    pub fn register(&mut self, task_name: &str, handler: Box<dyn JobHandler>) {
        self.handlers.insert(task_name.to_string(), handler);
    }
    
    pub fn execute(&self, task_name: &str, payload: Value) -> Result<Value, String> {
        // Try specific handler first
        if let Some(handler) = self.handlers.get(task_name) {
            return handler.execute(payload);
        }
        
        // Fallback to generic handler
        println!("[Registry] No specific handler for '{}', using GenericHandler", task_name);
        self.default_handler.execute(payload)
    }
}
use super::JobHandler;
use serde_json::Value;

pub struct SendWelcomeEmailHandler;

impl JobHandler for SendWelcomeEmailHandler {
    fn execute(&self, payload: Value) -> Result<Value, String> {
        let to = payload.get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown@example.com");
        
        let name = payload.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("User");
        
        println!("[EmailHandler] Sending welcome email to {} ({})", to, name);
        
        // Simulasi proses kirim email (3 detik)
        std::thread::sleep(std::time::Duration::from_secs(3));
        
        Ok(serde_json::json!({
            "success": true,
            "message": format!("Welcome email sent to {}", to),
            "to": to,
            "name": name
        }))
    }
}
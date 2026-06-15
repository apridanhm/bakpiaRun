use super::JobHandler;
use serde_json::Value;

pub struct GenericHandler;

impl JobHandler for GenericHandler {
    fn execute(&self, payload: Value) -> Result<Value, String> {
        // ambil method dari payload (default: "log")
        let method = payload.get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("log");
        
        println!("[GenericHandler] Executing method: {}", method);
        
        match method {
            "shell" => execute_shell(payload),
            "php" => execute_php(payload),
            "http" => execute_http(payload),
            "log" => execute_log(payload),
            "custom" => execute_custom(payload),
            _ => Err(format!("Unknown method: {}", method)),
        }
    }
}

// execute shell command
fn execute_shell(payload: Value) -> Result<Value, String> {
    let command = payload.get("command")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'command' in payload")?;
    
    println!("[ShellHandler] Running: {}", command);
    
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Failed to execute shell: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok(serde_json::json!({
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": output.status.code()
    }))
}

// execute PHP script
fn execute_php(payload: Value) -> Result<Value, String> {
    let script = payload.get("script")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'script' in payload")?;
    
    println!("[PhpHandler] Running: {}", script);
    
    let output = std::process::Command::new("php")
        .arg(script)
        .arg(serde_json::to_string(&payload).unwrap())
        .output()
        .map_err(|e| format!("Failed to execute PHP: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    // try parse as JSON, fallback to raw string
    let result = serde_json::from_str::<Value>(&stdout)
        .unwrap_or(serde_json::json!({"output": stdout}));
    
    Ok(serde_json::json!({
        "success": output.status.success(),
        "result": result,
        "stderr": stderr
    }))
}

// execute HTTP request
fn execute_http(payload: Value) -> Result<Value, String> {
    let url = payload.get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'url' in payload")?;
    
    let method = payload.get("http_method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    
    println!("[HttpHandler] {} {}", method, url);
    
    // simulate HTTP request (karena kita gak mau add dependency reqwest)
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    Ok(serde_json::json!({
        "success": true,
        "url": url,
        "method": method,
        "status_code": 200,
        "message": "HTTP request simulated"
    }))
}

// just log the payload
fn execute_log(payload: Value) -> Result<Value, String> {
    println!("[LogHandler] Payload: {}", serde_json::to_string_pretty(&payload).unwrap());
    
    Ok(serde_json::json!({
        "success": true,
        "message": "Job logged successfully",
        "payload": payload
    }))
}

// custom logic (placeholder for user-defined logic)
fn execute_custom(payload: Value) -> Result<Value, String> {
    let task_name = payload.get("task_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    
    println!("[CustomHandler] Processing custom task: {}", task_name);
    
    // simulate custom processing
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    Ok(serde_json::json!({
        "success": true,
        "message": format!("Custom task '{}' processed", task_name),
        "payload": payload
    }))
}
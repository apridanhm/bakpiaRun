use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

pub async fn admin_auth_middleware(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Get token from environment variable
    let expected_token = std::env::var("BAKPIA_ADMIN_TOKEN")
        .unwrap_or_default();
    
    // If no token configured, skip auth (for development)
    if expected_token.is_empty() {
        return Ok(next.run(req).await);
    }
    
    // Get token from header
    let provided_token = req
        .headers()
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    // Validate
    if provided_token == expected_token {
        Ok(next.run(req).await)
    } else {
        println!("[SECURITY] Unauthorized admin access attempt");
        Err(StatusCode::UNAUTHORIZED)
    }
}
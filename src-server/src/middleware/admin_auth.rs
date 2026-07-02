use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

/// Compare two byte strings in constant time w.r.t. their contents.
///
/// The length is allowed to leak (token length is not meaningfully secret),
/// but the comparison never short-circuits on the first differing byte, so it
/// does not expose how many leading bytes matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn admin_auth_middleware(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Token comes from the environment (loaded from config or set directly).
    let expected_token = std::env::var("BAKPIA_ADMIN_TOKEN").unwrap_or_default();

    // Fail CLOSED: if no admin token is configured, admin routes are refused.
    // Local development can explicitly opt out with BAKPIA_ADMIN_INSECURE=1.
    if expected_token.is_empty() {
        let insecure = std::env::var("BAKPIA_ADMIN_INSECURE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if insecure {
            return Ok(next.run(req).await);
        }
        println!("[SECURITY] Admin route blocked: BAKPIA_ADMIN_TOKEN not configured");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // Token from the request header.
    let provided_token = req
        .headers()
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes()) {
        Ok(next.run(req).await)
    } else {
        println!("[SECURITY] Unauthorized admin access attempt");
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn equal_tokens_match() {
        assert!(constant_time_eq(b"s3cret-token", b"s3cret-token"));
    }

    #[test]
    fn different_tokens_do_not_match() {
        assert!(!constant_time_eq(b"s3cret-token", b"s3cret-toketX"));
        assert!(!constant_time_eq(b"s3cret-token", b"wrong"));
    }

    #[test]
    fn different_lengths_do_not_match() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn empty_equals_empty() {
        assert!(constant_time_eq(b"", b""));
    }
}

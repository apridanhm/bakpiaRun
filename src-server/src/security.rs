use axum::http::HeaderValue;
use crate::config::SecurityConfig;

pub fn apply_security_headers(
    builder: axum::http::response::Builder,
    config: &SecurityConfig,
) -> axum::http::response::Builder {
    let mut builder = builder;

    // X-Frame-Options
    if let Some(ref value) = config.x_frame_options {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            builder = builder.header("X-Frame-Options", header_value);
        }
    }

    // X-Content-Type-Options
    if config.x_content_type_options {
        builder = builder.header("X-Content-Type-Options", "nosniff");
    }

    // X-XSS-Protection
    if config.x_xss_protection {
        builder = builder.header("X-XSS-Protection", "1; mode=block");
    }

    // Content-Security-Policy
    if let Some(ref value) = config.content_security_policy {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            builder = builder.header("Content-Security-Policy", header_value);
        }
    }

    // Strict-Transport-Security
    if let Some(ref value) = config.strict_transport_security {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            builder = builder.header("Strict-Transport-Security", header_value);
        }
    }

    // Referrer-Policy
    if let Some(ref value) = config.referrer_policy {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            builder = builder.header("Referrer-Policy", header_value);
        }
    }

    // Permissions-Policy
    if let Some(ref value) = config.permissions_policy {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            builder = builder.header("Permissions-Policy", header_value);
        }
    }

    builder
}

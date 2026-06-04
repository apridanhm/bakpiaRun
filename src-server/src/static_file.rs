use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use std::path::Path;
use tokio::fs;

// MIME type mapping
pub fn get_mime_type(extension: &str) -> &'static str {
    match extension.to_lowercase().as_str() {
        // Text
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        
        // Fonts
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        
        // Media
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        
        // Documents
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        
        // Other
        "wasm" => "application/wasm",
        "map" => "application/json",
        
        _ => "application/octet-stream",
    }
}

// Check if file is static (not PHP)
pub fn is_static_file(path: &str) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    
    // List of static file extensions
    matches!(
        extension.to_lowercase().as_str(),
        "css" | "js" | "mjs" | "json" | "xml" | "txt" |
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" |
        "woff" | "woff2" | "ttf" | "otf" | "eot" |
        "mp3" | "mp4" | "webm" | "ogg" | "wav" |
        "pdf" | "zip" | "gz" | "wasm" | "map" |
        "html" | "htm"
    )
}

// Serve static file
pub async fn serve_static_file(docroot: &str, uri: &str) -> Response<Body> {
    // Sanitize path (prevent directory traversal)
    let clean_path = uri.trim_start_matches('/');
    if clean_path.contains("..") {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("403 Forbidden"))
            .unwrap();
    }
    
    let file_path = format!("{}/{}", docroot, clean_path);
    let path = Path::new(&file_path);
    
    // Check if file exists
    if !path.exists() || !path.is_file() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap();
    }
    
    // Read file
    let content = match fs::read(path).await {
        Ok(c) => c,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("500 Internal Server Error"))
                .unwrap();
        }
    };
    
    // Get MIME type
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    let mime_type = get_mime_type(extension);
    
    // Calculate ETag (simple hash based on file size + modification time)
    let metadata = fs::metadata(path).await.unwrap();
    let etag = format!(
        "\"{}-{}\"",
        metadata.len(),
        metadata.modified().unwrap().elapsed().unwrap().as_secs()
    );
    
    // Build response with caching headers
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000") // 1 year
        .header(header::ETAG, etag)
        .header(header::LAST_MODIFIED, "Wed, 21 Oct 2015 07:28:00 GMT") // Placeholder
        .body(Body::from(content))
        .unwrap()
}


// Check if PHP file exists
pub async fn find_php_file(docroot: &str, uri: &str) -> Option<String> {
    let path = Path::new(docroot).join(uri.trim_start_matches('/'));
    
    // Case 1: Exact file exists (e.g., /index.php)
    if path.exists() && path.is_file() {
        return Some(path.to_string_lossy().to_string());
    }
    
    // Case 2: Try adding .php extension (e.g., /users → /users.php)
    let php_path = path.with_extension("php");
    if php_path.exists() && php_path.is_file() {
        return Some(php_path.to_string_lossy().to_string());
    }
    
    // Case 3: Try as directory with index.php (e.g., /admin → /admin/index.php)
    let index_path = path.join("index.php");
    if index_path.exists() && index_path.is_file() {
        return Some(index_path.to_string_lossy().to_string());
    }
    
    // Case 4: Try directory.php/index.php (e.g., /admin/dashboard → /admin/dashboard.php)
    if let Some(parent) = path.parent() {
        if let Some(file_name) = path.file_name() {
            let dir_file_path = parent.join(file_name).with_extension("php");
            if dir_file_path.exists() && dir_file_path.is_file() {
                return Some(dir_file_path.to_string_lossy().to_string());
            }
        }
    }
    
    None
}
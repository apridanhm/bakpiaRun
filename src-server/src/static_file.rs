use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use std::path::{Component, Path, PathBuf};
use tokio::fs;

/// Reject request sub-paths that could escape the document root.
///
/// The sub-path must be relative and must not contain any `..` (ParentDir)
/// component, an absolute-path root, or a Windows prefix. Note this operates on
/// the raw (percent-encoded) request path: encoded sequences like `%2e%2e` are
/// treated as ordinary name characters and simply won't resolve on disk.
pub fn is_safe_subpath(subpath: &str) -> bool {
    let p = Path::new(subpath);
    if p.is_absolute() {
        return false;
    }
    for comp in p.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
            Component::Normal(_) | Component::CurDir => {}
        }
    }
    true
}

/// Resolve `candidate` and confirm it is a regular file located inside
/// `docroot` after canonicalization (which also collapses symlinks). Returns
/// the canonical path only when it is safely contained in the document root.
async fn resolve_within(docroot: &str, candidate: &Path) -> Option<PathBuf> {
    if !candidate.is_file() {
        return None;
    }
    let canonical_root = fs::canonicalize(docroot).await.ok()?;
    let canonical = fs::canonicalize(candidate).await.ok()?;
    if canonical.starts_with(&canonical_root) {
        Some(canonical)
    } else {
        None
    }
}

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
    if !is_safe_subpath(clean_path) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("403 Forbidden"))
            .unwrap();
    }

    // Resolve and confirm the file is contained within the document root.
    let candidate = Path::new(docroot).join(clean_path);
    let path = match resolve_within(docroot, &candidate).await {
        Some(p) => p,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("404 Not Found"))
                .unwrap();
        }
    };
    let path = path.as_path();

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


// Check if PHP file exists. The resolved file is always confirmed to live
// inside `docroot` (see `resolve_within`), preventing path-traversal / LFI.
pub async fn find_php_file(docroot: &str, uri: &str) -> Option<String> {
    let subpath = uri.trim_start_matches('/');
    if !is_safe_subpath(subpath) {
        return None;
    }

    let base = Path::new(docroot).join(subpath);

    // Candidates, in priority order:
    //  1. Exact file              (/index.php)
    //  2. With .php extension      (/users → /users.php, /admin/x → /admin/x.php)
    //  3. Directory index         (/admin → /admin/index.php)
    let candidates = [base.clone(), base.with_extension("php"), base.join("index.php")];

    for candidate in candidates {
        if let Some(resolved) = resolve_within(docroot, &candidate).await {
            return Some(resolved.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::is_safe_subpath;

    #[test]
    fn rejects_parent_dir_traversal() {
        assert!(!is_safe_subpath("../etc/passwd"));
        assert!(!is_safe_subpath("a/../../etc/passwd"));
        assert!(!is_safe_subpath("../../../../etc/passwd"));
        assert!(!is_safe_subpath(".."));
    }

    #[test]
    fn rejects_absolute_paths() {
        assert!(!is_safe_subpath("/etc/passwd"));
    }

    #[test]
    fn allows_normal_paths() {
        assert!(is_safe_subpath(""));
        assert!(is_safe_subpath("index.php"));
        assert!(is_safe_subpath("admin/index.php"));
        assert!(is_safe_subpath("css/style.css"));
        // Dots that are part of a name (not a whole component) are fine.
        assert!(is_safe_subpath("my..weird..name.php"));
    }

    #[test]
    fn encoded_traversal_is_not_a_parent_component() {
        // Percent-encoded sequences are ordinary name chars here; they pass the
        // lexical guard but cannot resolve outside docroot on disk.
        assert!(is_safe_subpath("%2e%2e/%2e%2e/etc/passwd"));
    }
}
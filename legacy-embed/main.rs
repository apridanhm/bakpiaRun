mod php_sys {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]
    #![allow(unsafe_op_in_unsafe_fn)]
    #![allow(static_mut_refs)]
    #![allow(unsafe_code)]
    #![allow(unused_imports)]
    
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use php_sys::*;
use std::ffi::CString;
use std::sync::mpsc;
use std::fs;
use tokio::sync::oneshot;
use axum::{
    http::{Method, Uri},
    response::Response,
    routing::get,
    Router,
};
use axum::body::Body;
use axum::http::StatusCode;

pub struct PhpRequest {
    pub uri: String,
    pub method: String,
    pub file_path: String,
}

pub struct PhpResponse {
    pub status: u16,
    pub body: String,
}

fn strip_php_tags(code: &str) -> String {
    let mut result = code.to_string();
    
    if result.starts_with("<?php") {
        result = result[5..].to_string();
    } else if result.starts_with("<?") {
        result = result[2..].to_string();
    }
    
    if result.ends_with("?>") {
        result = result[..result.len()-2].to_string();
    }
    
    result.trim().to_string()
}

fn read_php_file(path: &str) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            println!("Baca file: {}", path);
            Ok(strip_php_tags(&content))
        }
        Err(e) => {
            eprintln!("Gagal baca file {}: {}", path, e);
            Err(format!("File not found: {}", e))
        }
    }
}

fn php_worker_thread(rx: mpsc::Receiver<(PhpRequest, oneshot::Sender<PhpResponse>)>) {
    println!("[PHP Thread] Menginisialisasi Zend Engine...");
    
    let arg = CString::new("bakpiarun-worker").unwrap();
    let argv_raw = arg.into_raw();
    let mut argv: [*mut i8; 1] = [argv_raw];

    unsafe {
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            eprintln!("Gagal inisialisasi PHP di worker thread!");
            return;
        }
        println!("[PHP Thread] Zend Engine siap!");
    }

    while let Ok((req, responder)) = rx.recv() {
        println!("\n[Worker] Request: {} {}", req.method, req.uri);
        
        let php_code = match read_php_file(&req.file_path) {
            Ok(code) => code,
            Err(e) => {
                let _ = responder.send(PhpResponse {
                    status: 404,
                    body: format!("<h1>404 - File Not Found</h1><p>{}</p>", e),
                });
                continue;
            }
        };

        let response = unsafe {
            // Inject $_SERVER variables sebelum eksekusi file PHP
            let server_setup = format!(
                r#"
                $_SERVER['REQUEST_METHOD'] = '{}';
                $_SERVER['REQUEST_URI'] = '{}';
                $_SERVER['SERVER_NAME'] = 'localhost';
                $_SERVER['SERVER_PORT'] = '8080';
                $_SERVER['SERVER_PROTOCOL'] = 'HTTP/1.1';
                $_SERVER['HTTP_HOST'] = 'localhost:8080';
                $_SERVER['SCRIPT_NAME'] = '/index.php';
                $_SERVER['PHP_SELF'] = '/index.php';
                "#,
                req.method, req.uri
            );
            
            // Gabungkan setup code dengan user code
            let full_code = format!("{}\n{}", server_setup, php_code);
            
            let c_code = CString::new(full_code).unwrap();
            let c_name = CString::new(&*req.file_path).unwrap();
            
            let result = zend_eval_string(
                c_code.as_ptr(), 
                std::ptr::null_mut(),
                c_name.as_ptr()
            );
            
            println!("PHP Exec Result = {}", result);

            let html = format!(
                r#"<!DOCTYPE html>
<html>
<head>
    <title>bakpiaRun - File-based PHP</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; background: #f5f5f5; }}
        .container {{ background: white; padding: 30px; border-radius: 8px; max-width: 800px; margin: 0 auto; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        h1 {{ color: #333; border-bottom: 2px solid #667eea; padding-bottom: 10px; }}
        .info {{ margin: 15px 0; padding: 10px; background: #e9ecef; border-radius: 4px; border-left: 4px solid #667eea; }}
        .success {{ color: #27ae60; font-weight: bold; }}
        .error {{ color: #e74c3c; font-weight: bold; }}
        .note {{ margin-top: 20px; padding: 15px; background: #fff3cd; border-left: 4px solid #ffc107; font-style: italic; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>bakpiaRun Web Server</h1>
        <p class="{}">PHP Execution: {}</p>
        <div class="info"><strong>Method:</strong> {}</div>
        <div class="info"><strong>URI:</strong> {}</div>
        <div class="info"><strong>File:</strong> {}</div>
        <div class="note">
            <strong>Note:</strong> Output dari PHP (echo/print) ditampilkan di terminal server. 
            Halaman ini di-generate oleh Rust.
        </div>
    </div>
</body>
</html>"#,
                if result == 0 { "success" } else { "error" },
                if result == 0 { "Success" } else { "Failed" },
                req.method,
                req.uri,
                req.file_path
            );

            PhpResponse {
                status: if result == 0 { 200 } else { 500 },
                body: html,
            }
        };

        let _ = responder.send(response);
    }

    println!("[PHP Thread] Shutting down...");
    unsafe {
        php_embed_shutdown();
        let _ = CString::from_raw(argv[0]);
    }
}

async fn php_handler(
    method: Method, 
    uri: Uri, 
    axum::extract::State(tx): axum::extract::State<mpsc::Sender<(PhpRequest, oneshot::Sender<PhpResponse>)>>
) -> Response<Body> {
    let (response_tx, response_rx) = oneshot::channel();
    
    let file_path = "index.php".to_string();
    
    let req = PhpRequest {
        uri: uri.to_string(),
        method: method.to_string(),
        file_path,
    };

    if tx.send((req, response_tx)).is_err() {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("PHP Worker thread is dead"))
            .unwrap();
    }

    match response_rx.await {
        Ok(php_res) => {
            Response::builder()
                .status(StatusCode::from_u16(php_res.status).unwrap_or(StatusCode::OK))
                .header("Content-Type", "text/html; charset=utf-8")
                .header("X-Powered-By", "bakpiaRun/0.2.0")
                .body(Body::from(php_res.body))
                .unwrap()
        }
        Err(_) => {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Failed to receive response from PHP Worker"))
                .unwrap()
        }
    }
}

#[tokio::main]
async fn main() {
    println!("Initializing bakpiaRun Web Server v0.2.0...");

    let (tx, rx) = mpsc::channel::<(PhpRequest, oneshot::Sender<PhpResponse>)>();

    std::thread::spawn(move || {
        php_worker_thread(rx);
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let tx_clone = tx.clone();

    let app = Router::new()
        .route("/", get(php_handler).post(php_handler))
        .route("/*path", get(php_handler).post(php_handler))
        .with_state(tx_clone);

    let addr = "0.0.0.0:8080";
    println!("bakpiaRun listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
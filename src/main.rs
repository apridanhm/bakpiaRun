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
}

pub struct PhpResponse {
    pub status: u16,
    pub body: String,
}

fn php_worker_thread(rx: mpsc::Receiver<(PhpRequest, oneshot::Sender<PhpResponse>)>) {
    println!("🔧 [PHP Thread] Menginisialisasi Zend Engine...");
    
    let arg = CString::new("bakpiarun-worker").unwrap();
    let argv_raw = arg.into_raw();
    let mut argv: [*mut i8; 1] = [argv_raw];

    unsafe {
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            eprintln!("❌ Gagal inisialisasi PHP di worker thread!");
            return;
        }
        println!("✅ [PHP Thread] Zend Engine siap!");
    }

    while let Ok((req, responder)) = rx.recv() {
        let response = unsafe {
            // PHP hanya echo ke stdout (sudah terbukti bekerja)
            let php_code = format!(
                r#"
                echo "=== bakpiaRun Request Log ===\n";
                echo "Method: {}\n";
                echo "URI: {}\n";
                echo "PHP Version: " . phpversion() . "\n";
                echo "Memory: " . memory_get_usage() . " bytes\n";
                echo "=============================\n";
                "#,
                req.method, req.uri
            );
            
            let c_code = CString::new(php_code).unwrap();
            let c_name = CString::new("bakpiarun_worker").unwrap();
            
            // Eksekusi tanpa return (output ke stdout)
            let result = zend_eval_string(
                c_code.as_ptr(), 
                std::ptr::null_mut(),
                c_name.as_ptr()
            );
            
            println!("🔍 PHP Exec Result = {}", result);
        
            // RUST yang generate HTML response untuk browser
            let html = format!(
                r#"<!DOCTYPE html>
        <html>
        <head>
            <title>bakpiaRun - PHP Embed Web Server</title>
            <style>
                body {{ 
                    font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; 
                    margin: 0; 
                    padding: 0; 
                    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
                    min-height: 100vh;
                }}
                .container {{ 
                    max-width: 800px; 
                    margin: 50px auto; 
                    background: white; 
                    padding: 40px; 
                    border-radius: 12px; 
                    box-shadow: 0 10px 40px rgba(0,0,0,0.2);
                }}
                h1 {{ 
                    color: #667eea; 
                    margin-top: 0;
                    font-size: 2.5em;
                }}
                .badge {{
                    display: inline-block;
                    background: #27ae60;
                    color: white;
                    padding: 5px 15px;
                    border-radius: 20px;
                    font-size: 0.9em;
                    margin-bottom: 20px;
                }}
                .info-grid {{
                    display: grid;
                    grid-template-columns: 1fr 1fr;
                    gap: 20px;
                    margin: 30px 0;
                }}
                .info-card {{
                    background: #f8f9fa;
                    padding: 20px;
                    border-radius: 8px;
                    border-left: 4px solid #667eea;
                }}
                .info-label {{
                    font-weight: bold;
                    color: #495057;
                    font-size: 0.9em;
                    text-transform: uppercase;
                    letter-spacing: 0.5px;
                }}
                .info-value {{
                    color: #212529;
                    font-size: 1.2em;
                    margin-top: 5px;
                }}
                .tech-stack {{
                    background: #e9ecef;
                    padding: 20px;
                    border-radius: 8px;
                    margin-top: 30px;
                }}
                .tech-stack h3 {{
                    margin-top: 0;
                    color: #495057;
                }}
                .tech-list {{
                    display: flex;
                    gap: 10px;
                    flex-wrap: wrap;
                }}
                .tech-tag {{
                    background: #667eea;
                    color: white;
                    padding: 8px 16px;
                    border-radius: 20px;
                    font-size: 0.9em;
                }}
                .footer {{
                    margin-top: 30px;
                    padding-top: 20px;
                    border-top: 2px solid #e9ecef;
                    color: #6c757d;
                    font-size: 0.9em;
                }}
            </style>
        </head>
        <body>
            <div class="container">
                <span class="badge">✅ PHP Executed Successfully</span>
                <h1>🎉 bakpiaRun Web Server</h1>
                <p>Web server PHP modern yang berjalan di atas Rust dengan performa tinggi!</p>
                
                <div class="info-grid">
                    <div class="info-card">
                        <div class="info-label">Request Method</div>
                        <div class="info-value">{}</div>
                    </div>
                    <div class="info-card">
                        <div class="info-label">Request URI</div>
                        <div class="info-value">{}</div>
                    </div>
                    <div class="info-card">
                        <div class="info-label">Server</div>
                        <div class="info-value">bakpiaRun/0.1.0</div>
                    </div>
                    <div class="info-card">
                        <div class="info-label">Status</div>
                        <div class="info-value">200 OK</div>
                    </div>
                </div>
        
                <div class="tech-stack">
                    <h3>🛠️ Technology Stack</h3>
                    <div class="tech-list">
                        <span class="tech-tag">Rust</span>
                        <span class="tech-tag">Tokio (Async)</span>
                        <span class="tech-tag">Axum (HTTP)</span>
                        <span class="tech-tag">PHP 8.3 Embed</span>
                        <span class="tech-tag">Zend Engine</span>
                    </div>
                </div>
        
                <div class="footer">
                    <p><strong>📝 Catatan:</strong> PHP dieksekusi oleh Zend Engine dan output-nya dicatat di terminal server. 
                    Response HTML ini di-generate oleh Rust dan dikirim ke browser Anda.</p>
                    <p><strong>🚀 Arsitektur:</strong> HTTP Request → Axum → PHP Worker Thread → Zend Engine → Response</p>
                </div>
            </div>
        </body>
        </html>"#,
                req.method, req.uri
            );
        
            if result == 0 {
                PhpResponse { status: 200, body: html }
            } else {
                zend_clear_exception();
                PhpResponse { 
                    status: 500, 
                    body: format!("<h1>❌ PHP Error</h1><p>Result: {}</p>", result) 
                }
            }
        };

        let _ = responder.send(response);
    }

    println!("🛑 [PHP Thread] Shutting down Zend Engine...");
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
    
    let req = PhpRequest {
        uri: uri.to_string(),
        method: method.to_string(),
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
                .header("X-Powered-By", "bakpiaRun/0.1.0")
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
    println!("🚀 Initializing bakpiaRun Web Server...");

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
    println!("🌐 bakpiaRun listening on http://{}", addr);
    println!("💡 Buka browser: http://localhost:8080/halo-dunia");
    println!("💡 Output PHP sekarang dikirim ke browser, bukan ke terminal!");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

//// Async channel done//////
/*mod php_sys {
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
use tokio::sync::oneshot;

pub struct PhpRequest {
    pub uri: String,
    pub method: String,
}

pub struct PhpResponse {
    pub status: u16,
    pub body: String,
}

fn php_worker_thread(rx: mpsc::Receiver<(PhpRequest, oneshot::Sender<PhpResponse>)>) {
    println!("[Dedicated Thread] Menginisialisasi PHP Engine...");
    
    let arg = CString::new("bakpiarun-worker").unwrap();
    let argv_raw = arg.into_raw();
    let mut argv: [*mut i8; 1] = [argv_raw];

    unsafe {
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            eprintln!("Gagal inisialisasi PHP di worker thread!");
            return;
        }
        println!("[Dedicated Thread] PHP Engine siap menerima request!");
    }

    while let Ok((req, responder)) = rx.recv() {
        println!("\n[Worker] Menerima request: {} {}", req.method, req.uri);

        let response = unsafe {
            // ULTRA MINIMAL: Tanpa php_request_startup, tanpa retval capture
            let php_code = format!(
                r#"echo "Halo dari bakpiaRun! Request: {} {} berhasil.";"#,
                req.method, req.uri
            );
            
            let c_code = CString::new(php_code).unwrap();
            let c_name = CString::new("bakpiarun_worker").unwrap();
            
            // PENTING: Gunakan std::ptr::null_mut() untuk retval
            let result = zend_eval_string(
                c_code.as_ptr(), 
                std::ptr::null_mut(), // Jangan tangkap return value
                c_name.as_ptr()
            );
            
            println!("Debug: Exec Result = {}", result);

            if result == 0 {
                PhpResponse { 
                    status: 200, 
                    body: "Eksekusi berhasil (output dicetak ke stdout)".to_string() 
                }
            } else {
                PhpResponse { 
                    status: 500, 
                    body: format!("PHP execution failed. Result: {}", result) 
                }
            }
        }; 

        let _ = responder.send(response);
    }

    println!("[Dedicated Thread] Shutting down PHP Engine...");
    unsafe {
        php_embed_shutdown();
        let _ = CString::from_raw(argv[0]);
    }
}

#[tokio::main]
async fn main() {
    println!("Initializing bakpiaRun (Main Thread)...");

    let (tx, rx) = mpsc::channel::<(PhpRequest, oneshot::Sender<PhpResponse>)>();

    std::thread::spawn(move || {
        php_worker_thread(rx);
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("Main Thread siap mengirim request...\n");

    let (response_tx, response_rx) = oneshot::channel();
    
    let mock_request = PhpRequest {
        uri: "/api/users".to_string(),
        method: "GET".to_string(),
    };

    tx.send((mock_request, response_tx)).unwrap();

    if let Ok(response) = response_rx.await {
        println!("[Server] Response diterima dari Worker:");
        println!("   Status: {}", response.status);
        println!("   Body:\n{}", response.body);
    }

    drop(tx);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("bakpiaRun Shutdown cleanly.");
}*/



//// Async channel setangah berhasil //////
/*mod php_sys {
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
use tokio::sync::oneshot;

pub struct PhpRequest {
    pub uri: String,
    pub method: String,
}

pub struct PhpResponse {
    pub status: u16,
    pub body: String,
}

fn php_worker_thread(rx: mpsc::Receiver<(PhpRequest, oneshot::Sender<PhpResponse>)>) {
    println!("[Dedicated Thread] Menginisialisasi PHP Engine...");
    
    unsafe {
        let arg = CString::new("bakpiarun-worker").unwrap();
        let mut argv: [*mut i8; 1] = [arg.into_raw()];
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            eprintln!("Gagal inisialisasi PHP di worker thread!");
            return;
        }
        println!("[Dedicated Thread] PHP Engine siap menerima request!");
    }

    while let Ok((req, responder)) = rx.recv() {
        println!("\n[Worker] Menerima request: {} {}", req.method, req.uri);

        let response = unsafe {
            // 1. Start Request
            if php_request_startup() != 0 {
                eprintln!("php_request_startup gagal!");
                let _ = responder.send(PhpResponse { status: 500, body: "Request startup failed".to_string() });
                continue;
            }

            // 2. Kode PHP SANGAT SEDERHANA (Tanpa ob_start untuk menghindari masalah inisialisasi output global di embed mode)
            let php_code = format!(
                r#"$res = "Halo dari bakpiaRun! Request: {} {} berhasil."; return $res;"#,
                req.method, req.uri
            );
            
            let c_code = CString::new(php_code).unwrap();
            let c_name = CString::new("bakpiarun_worker").unwrap();
            
            let mut retval: zval = std::mem::zeroed();
            let result = zend_eval_string(c_code.as_ptr(), &mut retval, c_name.as_ptr());
            
            println!("Debug: Exec Result = {}", result);

            // 3. Ambil hasil
            let final_response = if result == 0 && retval.u1.v.type_ as u32 == IS_STRING {
                let zstr = retval.value.str_;
                if !zstr.is_null() {
                    let len = (*zstr).len;
                    let val_ptr = (*zstr).val.as_ptr() as *const u8;
                    let slice = std::slice::from_raw_parts(val_ptr, len);
                    let body = String::from_utf8_lossy(slice).into_owned();
                    PhpResponse { status: 200, body }
                } else {
                    PhpResponse { status: 500, body: "Null string pointer".to_string() }
                }
            } else {
                // CRITICAL FIX: Bersihkan exception yang tertinggal agar shutdown tidak crash!
                zend_clear_exception();
                
                PhpResponse { 
                    status: 500, 
                    body: format!("PHP execution failed. Result: {}", result) 
                }
            };

            // 4. HANYA panggil dtor jika berhasil dan tipenya STRING
            if result == 0 && retval.u1.v.type_ as u32 == IS_STRING {
                zval_ptr_dtor(&mut retval);
            }

            // 5. Shutdown Request
            php_request_shutdown(std::ptr::null_mut());

            final_response
        }; 

        let _ = responder.send(response);
    }

    println!("[Dedicated Thread] Shutting down PHP Engine...");
    unsafe {
        php_embed_shutdown();
    }
}

#[tokio::main]
async fn main() {
    println!("Initializing bakpiaRun (Main Thread)...");

    let (tx, rx) = mpsc::channel::<(PhpRequest, oneshot::Sender<PhpResponse>)>();

    std::thread::spawn(move || {
        php_worker_thread(rx);
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("Main Thread siap mengirim request...\n");

    let (response_tx, response_rx) = oneshot::channel();
    
    let mock_request = PhpRequest {
        uri: "/api/users".to_string(),
        method: "GET".to_string(),
    };

    tx.send((mock_request, response_tx)).unwrap();

    if let Ok(response) = response_rx.await {
        println!("[Server] Response diterima dari Worker:");
        println!("   Status: {}", response.status);
        println!("   Body:\n{}", response.body);
    }

    drop(tx);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("bakpiaRun Shutdown cleanly.");
}*/

// try read index php file //
/*mod php_sys {
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

// 2. Import semua fungsi dan tipe dari module tersebut ke scope utama.
// Jadi kita tidak perlu mengubah kode di bawah (tetap pakai php_embed_init, dll)
use php_sys::*;

use std::ffi::CString;
use std::fs;

fn main() {
    println!("Initializing bakpiaRun (PHP Embed)...");

    unsafe {
        let arg = CString::new("bakpiarun").unwrap();
        let mut argv: [*mut i8; 1] = [arg.into_raw()];
        
        // Nyalakan mesin PHP
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            panic!("Failed to initialize PHP embed! Error code: {}", result);
        }
        
        println!("PHP Engine Initialized Successfully!\n");

        // Baca file index.php menggunakan Rust (I/O yang sangat cepat)
        let file_path = "index.php";
        let php_code_string = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Gagal membaca file {}: {}", file_path, e);
                eprintln!("Pastikan kamu menjalankan 'cargo run' di folder yang sama dengan index.php");
                php_embed_shutdown();
                return;
            }
        };

        println!("Mengeksekusi file: {}", file_path);
        println!("--------------------------------------------------");

        // Konversi string Rust ke CString yang aman untuk C/PHP
        let php_code = CString::new(php_code_string).expect("String mengandung null byte");
        let script_name = CString::new(file_path).unwrap();
        
        // Eksekusi kode dari file tersebut
        let eval_result = zend_eval_string(
            php_code.as_ptr(), 
            std::ptr::null_mut(), 
            script_name.as_ptr()
        );
        
        println!("--------------------------------------------------\n");
        
        // Cek hasil
        if eval_result == 0 {
            println!("File PHP berhasil dieksekusi tanpa error!");
        } else {
            println!("Terjadi error saat mengeksekusi file PHP. Cek output di atas.");
        }

        // Matikan mesin PHP
        php_embed_shutdown();
        println!("PHP Engine Shutdown cleanly.");
        
        // Bersihkan memori CString
        let _ = CString::from_raw(argv[0]);
    }
}*/

//// string hard code php test /////
/*#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(static_mut_refs)]

// Memuat semua binding C dari PHP yang di-generate oleh build.rs
// Di sinilah fungsi zend_eval_string yang asli berada!
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::ffi::CString;

fn main() {
    println!("Initializing bakpiaRun (PHP Embed)...");

    unsafe {
        // 1. Siapkan argumen untuk inisialisasi
        let arg = CString::new("bakpiarun").unwrap();
        let mut argv: [*mut i8; 1] = [arg.into_raw()];
        
        // 2. Nyalakan mesin PHP
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            panic!("Failed to initialize PHP embed! Error code: {}", result);
        }
        
        println!("PHP Engine Initialized Successfully!");
        println!("Executing PHP code directly from Rust memory...\n");
        println!("--------------------------------------------------");

        // 3. Siapkan kode PHP yang ingin dijalankan
        let php_code = CString::new("echo 'Hello from bakpiaRun! PHP 8.3 is running natively inside Rust!';").unwrap();
        let script_name = CString::new("bakpiarun_runtime").unwrap();
        
        // 4. Eksekusi kode PHP!
        // Kita pakai std::ptr::null_mut() untuk retval_ptr karena kita hanya ingin mengeksekusi, 
        // bukan menangkap nilai return-nya ke variabel Rust.
        let eval_result = zend_eval_string(
            php_code.as_ptr(), 
            std::ptr::null_mut(), 
            script_name.as_ptr()
        );
        
        println!("--------------------------------------------------\n");
        
        // 5. Cek hasil eksekusi (ZEND_SUCCESS = 0)
        if eval_result == 0 {
            println!("Skrip PHP berhasil dieksekusi tanpa error!");
        } else {
            println!("Terjadi error saat mengeksekusi skrip PHP.");
        }

        // 6. Matikan mesin PHP dengan bersih
        php_embed_shutdown();
        println!("PHP Engine Shutdown cleanly.");
        
        // Bersihkan memori CString
        let _ = CString::from_raw(argv[0]);
    }
}*/


/////////////// trying version //////////
/*#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unsafe_op_in_unsafe_fn)] 
#![allow(static_mut_refs)]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::ffi::CString;

fn main() {
    println!("Initializing bakpiaRun (PHP Embed)...");

    unsafe {
        let arg = CString::new("bakpiarun").unwrap();
        let mut argv: [*mut i8; 1] = [arg.into_raw()];
        
        let result = php_embed_init(1, argv.as_mut_ptr());
        if result != 0 {
            panic!("Failed to initialize PHP embed! Error code: {}", result);
        }
        
        println!("PHP Engine Initialized Successfully! Ready to handle requests.");

        php_embed_shutdown();
        println!("PHP Engine Shutdown cleanly.");
        
        let _ = CString::from_raw(argv[0]);
    }
}*/
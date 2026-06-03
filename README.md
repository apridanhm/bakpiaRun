================================================================================
                            BAKPIARUN
================================================================================

Runtime PHP Modern yang Dibangun dengan Rust

bakpiaRun adalah web server PHP custom yang menggunakan Rust sebagai runtime
dan PHP 8.3 sebagai embedded engine. Terinspirasi dari FrankenPHP dan 
RoadRunner, bakpiaRun bertujuan untuk memberikan performa tinggi dengan 
arsitektur async modern.

================================================================================
FITUR UTAMA
================================================================================

- High Performance: Async HTTP server menggunakan Axum
- Memory Safe: Dibangun dengan Rust, bebas memory leak
- PHP 8.3 Embedded: PHP berjalan native di dalam Rust
- Dedicated Worker Thread: Isolasi penuh antara HTTP server dan PHP engine
- Zero Overhead: Tidak ada fork() atau process spawning

================================================================================
ARSITEKTUR
================================================================================

[HTTP Client / Browser]
         |
         | HTTP Request
         v
[Axum HTTP Server - Rust Async]
  - Handle koneksi
  - Parse request
  - Route ke handler
         |
         | Channel (mpsc)
         v
[PHP Worker Thread - Dedicated]
  - Inisialisasi Zend Engine
  - Eksekusi PHP code
  - Return response

================================================================================
CARA MENJALANKAN
================================================================================

PRASYARAT:
- Rust 1.82+
- PHP 8.3 (sudah di-compile dengan mode Embed + ZTS)
- Ubuntu 22.04 atau compatible

BUILD & RUN:

    git clone https://github.com/username/bakpiarun.git
    cd bakpiarun
    
    cargo build --release
    cargo run

Server akan berjalan di http://localhost:8080

================================================================================
STRUKTUR PROYEK
================================================================================

bakpiarun/
- src/main.rs          : Main application code
- build.rs             : Build script untuk bindgen
- wrapper.h            : C header untuk PHP bindings
- Cargo.toml           : Rust dependencies
- .gitignore           : Git ignore rules
- README.txt           : File ini

================================================================================
STATUS DEVELOPMENT
================================================================================

[x] PHP 8.3 Embed compilation
[x] Rust FFI binding dengan bindgen
[x] Dedicated thread architecture
[x] HTTP server dengan Axum
[x] Basic PHP execution
[ ] Output buffering & capture
[ ] File-based routing
[ ] $_GET/$_POST handling
[ ] Static file serving
[ ] Performance benchmarking

================================================================================
ROADMAP
================================================================================

Phase 1 (Current): Basic HTTP server + PHP execution
Phase 2: Output capture & proper response handling
Phase 3: File routing & framework support
Phase 4: Production features (logging, monitoring, etc.)

================================================================================
TEKNOLOGI YANG DIGUNAKAN
================================================================================

- Rust 1.82+ (Edition 2024)
- PHP 8.3.8 (Embed SAPI + ZTS)
- Axum 0.7 (Web framework)
- Tokio 1.35 (Async runtime)
- Bindgen 0.71 (FFI code generation)

================================================================================
LISENSI
================================================================================

MIT License

================================================================================
KONTAK
================================================================================

GitHub: https://github.com/username/bakpiarun

================================================================================
                    Made with Rust + PHP
================================================================================

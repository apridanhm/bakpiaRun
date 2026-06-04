![BakpiaRun](https://drive.usercontent.google.com/download?id=1i-TBji1mVPeurn3IsLLf_UzRAFvL5sTL&export=download&authuser=0&confirm=t&uuid=9b4602e2-1e21-4148-a131-16178645d61c&at=AAINaIK-KlQh8EpVLdXEhavEX26N:1780559647577)


#  bakpiaRun

**High-Performance PHP Runtime Server** - Built with Rust 

bakpiaRun adalah web server PHP modern yang menggabungkan performa Rust dengan fleksibilitas PHP. Dirancang untuk production dengan fokus pada **stabilitas**, **efisiensi memory**, dan **anti-OOM**.

## Fitur Utama

###  Performance & Stability
- **Worker Pool Architecture** - Multiple PHP workers dengan round-robin load balancing
- **Process Isolation** - PHP crash tidak affect server utama (Rust)
- **Anti-OOM System** - Auto-restart worker saat memory limit tercapai
- **Static File Serving** - CSS/JS/images served langsung oleh Rust (10x lebih cepat)
- **Zero-Copy IPC** - Komunikasi efisien via Unix Domain Sockets

### Request Handling
- `$_GET` parameters
- `$_POST` parameters (form-urlencoded + JSON)
- `$_FILES` upload (single & multiple files)
- `$_COOKIE` handling
- `$_SERVER` + HTTP Headers
- `php://input` (raw body)
- Multipart/form-data parsing

### Web Server Features
- **Clean URLs** - Routing tanpa `.php` extension
- **Directory Routing** - `/admin` otomatis ke `/admin/index.php`
- **MIME Type Detection** - 30+ file types supported
- **Caching Headers** - ETag, Cache-Control, Last-Modified
- **404 Handling** - Proper error pages

### Configuration
- **YAML Config** - Semua setting di 1 file
- **Environment Variables** - Override config via env vars
- **CLI Arguments** - Flexible command-line options
- **No Hardcoded Paths** - Fully configurable

## Arsitektur
BAKPIARUN (Rust + Axum)
│
├── HTTP Server (Axum)
│ │
│ ├── Router
│ │ ├── Static Files → Served by Rust (fast!)
│ │ └── PHP Files → Worker Pool
│ │
│ └── Worker Pool (4 workers, round-robin)
│ ├── Worker 0: PHP Process + Memory Monitor
│ ├── Worker 1: PHP Process + Memory Monitor
│ ├── Worker 2: PHP Process + Memory Monitor
│ └── Worker 3: PHP Process + Memory Monitor
│
└── PHP Worker (Long-running daemon)
└── Communication: Unix Domain Socket

### Requirements
- **Rust 1.70+** (untuk compile server)
- **PHP 8.0+** (CLI mode)
- **Linux/macOS** (Unix Domain Socket support)

### Quick Start

Clone Repository
cd src-server
cargo build --release


### Configure
Edit file config/bakpiarun.yaml sesuai kebutuhan:

server:
  host: "0.0.0.0"
  port: 8080

php:
  docroot: "/path/to/your/public"
  worker_path: "/path/to/bakpiarun/src-worker/worker.php"
  worker_count: 4
  memory_limit_mb: 128
  max_requests: 1000

socket:
  directory: "/tmp/bakpiarun"

logging:
  level: "info"
  file: "/var/log/bakpiarun.log"


### ️ Configuration Reference
server:
  host: "0.0.0.0"      # Bind address
  port: 8080           # HTTP port

php:
  docroot: "/var/www/html"              # Document root
  worker_path: "/opt/bakpiarun/worker.php"  # Worker script path
  worker_count: 4                       # Jumlah worker aktif
  memory_limit_mb: 128                  # Batas memori per worker (MB)
  max_requests: 1000                    # Auto-restart setelah N request

php:
  docroot: "/var/www/html"              # Document root
  worker_path: "/opt/bakpiarun/worker.php"  # Worker script path
  worker_count: 4                       # Jumlah worker aktif
  memory_limit_mb: 128                  # Batas memori per worker (MB)
  max_requests: 1000                    # Auto-restart setelah N request


### Run
cd src-server
cargo run -- --config ../config/bakpiarun.yaml

### Test
http://ip:8080/

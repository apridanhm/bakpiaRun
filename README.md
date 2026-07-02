<p align="center">
  <img src="https://drive.usercontent.google.com/download?id=1i-TBji1mVPeurn3IsLLf_UzRAFvL5sTL&export=download&authuser=0&confirm=t&uuid=9b4602e2-1e21-4148-a131-16178645d61c&at=AAINaIK-KlQh8EpVLdXEhavEX26N:1780559647577" alt="BakpiaRun">
</p>

<h1 align="center">BakpiaRun</h1>
<p align="center">High Performance PHP Runtime Server Built with Rust</p>

bakpiaRun is a modern PHP web server that combines Rust's performance with PHP's flexibility. Designed for production with focus on **stability**, **memory efficiency**, and **anti OOM protection**.

## Features

### Performance and Stability
- **Worker Pool Architecture** - Multiple PHP workers with round-robin load balancing
- **Process Isolation** - PHP crashes do not affect the main Rust server
- **Anti-OOM System** - Auto-restart workers when memory limit is reached
- **Static File Serving** - CSS/JS/images served directly by Rust (10x faster)
- **Lightweight IPC** - Length-prefixed JSON over Unix Domain Sockets

### Request Handling
- `$_GET` parameters
- `$_POST` parameters (form-urlencoded + JSON)
- `$_FILES` upload (single and multiple files)
- `$_COOKIE` handling
- `$_SERVER` and HTTP Headers
- `php://input` (raw body)
- Multipart/form-data parsing

### Web Server Features
- **Clean URLs** - Routing without `.php` extension
- **Directory Routing** - `/admin` automatically routes to `/admin/index.php`
- **MIME Type Detection** - 30+ file types supported
- **Caching Headers** - ETag, Cache-Control, Last-Modified
- **404 Handling** - Proper error pages

### Configuration
- **YAML Config** - All settings in one file
- **Environment Variables** - Override config via env vars
- **CLI Arguments** - Flexible command-line options
- **No Hardcoded Paths** - Fully configurable

## Architecture
```code
bakpiaRun (Rust + Axum)
│
├─ HTTP Server (Axum)
│ │
│ ├─ Router
│ │ ├─ Static Files → Served by Rust
│ │ └─ PHP Files → Worker Pool
│ │
│ └─ Worker Pool (4 workers, round-robin)
│ ├─ Worker 0: PHP + Memory Monitor
│ ├─ Worker 1: PHP + Memory Monitor
│ ├─ Worker 2: PHP + Memory Monitor
│ └─ Worker 3: PHP + Memory Monitor
│
└─ PHP Worker (Long-running daemon)
└─ Communication: Unix Domain Socket

```

## Dedicated Worker Pools

bakpiaRun supports **intelligent request routing** with multiple worker pools for optimal resource utilization.

### Pool Configuration

**Fast Pool (Normal Traffic)**
- Workers: 32
- Memory Limit: 128 MB per worker
- Timeout: 30 seconds
- Use Case: Normal page requests, API calls, static files

**Heavy Pool (Background Jobs)**
- Workers: 8
- Memory Limit: 512 MB per worker
- Timeout: 5 minutes (300,000 ms)
- Use Case: Database exports, batch processing, heavy queries

### URL Pattern Routing

Requests are automatically routed to the appropriate pool based on URL patterns:

```yaml
pools:
  - name: "heavy"
    worker_count: 8
    memory_limit_mb: 512
    max_requests: 100
    timeout_ms: 300000
    patterns:
      - "/heavy-*"
      - "/api/export/*"
      - "/api/report/*"
      - "/admin/bulk-*"
  
  - name: "fast"
    worker_count: 32
    memory_limit_mb: 128
    max_requests: 5000
    timeout_ms: 30000
    patterns:
      - "/*"    
```

### Benefits

1. Pool Isolation - Heavy queries never impact normal traffic
2. Resource Optimization - Each pool configured for its workload
3. Better Stability - Memory-intensive tasks isolated
4. Production-Ready - Proven under extreme load (650+ concurrent heavy requests)


### Real-World Example
E-commerce Website:

- Users browsing products → Fast Pool (1,800 req/sec)
- Admin exporting reports → Heavy Pool (isolated)
- Payment batch processing → Heavy Pool (isolated)
- Result: Zero impact on user experience

### Requirements
- Rust 1.70+ (to compile server)
- PHP 8.0+ (CLI mode)
- Linux/macOS (Unix Domain Socket support)

### Project Structure
```code
bakpiarun/
├── src-server/          # Rust backend server
│   ├── src/
│   │   ├── main.rs      # Entry point
│   │   ├── config.rs    # Configuration parsing
│   │   ├── pool_manager.rs  # Multi-pool architecture
│   │   ├── worker_pool.rs   # Worker management
│   │   ├── handlers.rs      # HTTP request handlers
│   │   └── ...
├── src-worker/          # PHP worker bootstrap
│   └── worker.php       # Main worker script
├── public/              # Document root (PHP files)
├── config/              # Configuration files
├── logs/                # Log files
├── certs/               # SSL certificates
└── README.md
```
### Quick Start

#### 1. Clone Repository

```code
git clone https://github.com/apridana/bakpiarun.git
cd bakpiarun
```

#### 2. Build Server

```code
cd src-server
cargo build --release
```

#### 3. Configure

```yaml
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
```

##### Configuration Reference
```yaml
server:
  host: "0.0.0.0"      # Bind address
  port: 8080           # HTTP port

php:
  docroot: "/var/www/html"              # Document root
  worker_path: "/opt/bakpiarun/worker.php"  # Worker script path
  worker_count: 4                       # Number of active workers
  memory_limit_mb: 128                  # Memory limit per worker (MB)
  max_requests: 1000                    # Auto-restart after N requests
```

#### 4. Run Server
```code
cd src-server
cargo run -- --config ../config/bakpiarun.yaml
```

#### 5. Test
```code
curl http://localhost:8080/
```

## Performance Benchmark

> **Read this before quoting numbers.** The figures below come from ApacheBench
> (`ab -n 10000 -c 100`) on a standard VPS using a **trivial PHP script** (not a
> full framework). Treat them as an indicative microbenchmark, not a guarantee:
> re-run on your own hardware and workload, and compare like-for-like (same PHP
> version, OpCache/JIT settings, and a realistic application).

### bakpiaRun vs Competitors (trivial-script microbenchmark)

| Runtime | Req/sec | Memory/worker¹ | Median Latency |
|---------|---------|----------------|----------------|
| bakpiaRun (32w) | ~1,871 | ~2 MB¹ | ~37 ms |
| FrankenPHP | 800–1,200 | 20–50 MB | 50–100 ms |
| RoadRunner | 1,000–2,000 | 30–60 MB | 40–80 ms |

¹ The "~2 MB" figure is PHP's internal arena as reported by
`memory_get_usage(true)` for a trivial script — **not the process RSS**. Real
resident memory per PHP CLI worker (with PDO / a framework loaded) is typically
~15–40 MB. Measure RSS (e.g. `ps`/`smem`) for capacity planning.

### Notes & caveats
- **IPC** is length-prefixed JSON over a Unix Domain Socket (not "zero-copy");
  request/response bodies are serialized on both sides.
- **Fair comparison:** FrankenPHP runs PHP in-process with OpCache/JIT and a
  worker mode, so a meaningful head-to-head needs a realistic framework workload.
  See `REVIEW.md` for current framework-compatibility status and roadmap.
- **Built-in:** HTTP/2, Gzip, rate limiting, and security headers.
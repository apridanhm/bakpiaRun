<p align="center">
  <img src="public/bakpiarun-logo.png" alt="bakpiaRun Logo" width="380">
</p>

<h1 align="center">bakpiaRun</h1>
<p align="center">
  <strong>Cloud-Native PHP 8.3 Application Runtime Built in Rust</strong><br>
  Workload-Aware Multi-Pool Routing &bull; Proactive Anti-OOM Engine &bull; Persistent Keep-Alive UDS &bull; Secure MySQL Wire Proxy
</p>

---

**bakpiaRun** adalah *application server runtime* modern berbasis **Rust (`Axum` + `Tokio` + `Hyper`)** yang mengeksekusi aplikasi PHP di atas **Persistent PHP 8.3 CLI Workers** melalui komunikasi **Unix Domain Socket (UDS)** berkecepatan tinggi. Dirancang khusus untuk lingkungan kontainer (*Docker / Kubernetes*) guna mengatasi masalah *Head-of-Line (HoL) Blocking*, kebocoran memori worker jangka panjang, dan insiden `OOMKilled (Exit Code 137)`.

---

## Fitur Utama

### 1. Workload-Aware Multi-Pool Router
- Memisahkan trafik ringan (**`fast` pool**) dan komputasi/query berat (**`heavy` pool**) ke kolam pekerja terisolasi.
- Klasifikasi dua lapis yang diatur penuh dari YAML:
  1. **Atribut HTTP Header** (mis. `X-Workload: heavy`, `X-Pool: fast`).
  2. **Pencocokan Pola URL** (mis. `/heavy-*`, `/api/report/*`, `/api/export/*`).
- Penjadwalan **Lock-Free Idle-First / Least-Loaded** berbasis operasi atomik CPU (`AtomicUsize::compare_exchange`) tanpa *pool-wide lock contention*.

### 2. Proactive Anti-OOM Engine (`/proc/[pid]/status` + `cgroups v2`)
- **Telemetri Memori Proses Kernel**: Membaca `VmRSS` & `VmHWM` langsung dari `/proc/[pid]/status` di samping telemetri internal ZendMM (`memory_get_peak_usage`).
- **Telemetri Kontainer Agregat**: Memantau tekanan memori kontainer secara *real-time* melalui **`cgroups v2`** (`/sys/fs/cgroup/memory.current` & `memory.max`, dengan fallback otomatis ke `cgroups v1` / `/proc/meminfo`).
- **Graceful Worker Recycling (0ms P99 Impact)**: Saat worker melewati ambang batas proaktif (misal `80%`), worker ditandai `draining`, menyelesaikan request aktif, lalu didaur ulang secara asinkron di background tanpa menjatuhkan satu pun koneksi klien.

### 3. Persistent Keep-Alive UDS & Kompatibilitas Framework Modern
- Pipa **Keep-Alive Unix Domain Socket** dengan framing biner *4-byte Big-Endian length-prefixed JSON* (*cancel-safe* & *auto-reconnect*).
- **Isolated Static Closure Sandbox + `gc_mem_caches()`**: Variabel lokal skrip tidak bocor antar-request dan memori besar (>8 MB) langsung dikembalikan ke kernel Linux setelah request selesai.
- Mendukung **PHP Native**, **REST Microservices** (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, `HEAD`, multi `Set-Cookie`, `front_controller`), serta **Persistent Laravel Worker Bridge** ([`lib/BakpiaLaravelBridge.php`](lib/BakpiaLaravelBridge.php)) yang mem-*boot* aplikasi Laravel cukup 1 kali di RAM.

### 4. Secure Database Proxy & Async Job Queue
- **Secure MySQL/MariaDB Wire-Protocol Proxy (`127.0.0.1:3307`)**: Mendukung protokol teks (`COM_QUERY`) maupun *native binary prepared statements* (`COM_STMT_PREPARE` `0x16` / `COM_STMT_EXECUTE` `0x17`), *Credential Masking*, *Dynamic Version Spoofing*, dan *Transaction-Aware Connection Pinning* (`BEGIN` / `COMMIT` / `ROLLBACK`).
- **In-Memory Async Job Queue**: Offloading tugas latar belakang (`/api/queue/submit`) dengan proteksi kapasitas `max_jobs` dan *Whitelist Handler Registry*.

### 5. Hardening Keamanan (Anti-PHP2Shell) & Observabilitas
- Menyuntikkan proteksi tingkat engine `-d allow_url_include=0`, `-d disable_functions=...`, dan *filesystem sandbox* `-d open_basedir=...` otomatis pada setiap worker PHP.
- Dilengkapi TLS/HTTPS (`HTTP/2` via `rustls`), *Token Bucket Rate Limiter*, *Security Headers*, *Zero-Downtime Rolling Reload* (`SIGHUP` / `/reload`), serta telemetri **JSON `/health`** dan **Prometheus `/metrics`** (termasuk persentil latensi `P50`/`P95`/`P99` per pool).

---

## Arsitektur Sistem

```text
[ HTTP/1.1 & HTTPS (HTTP/2) Clients ]
                 │
                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                    bakpiaRun Engine (Rust: Axum + Tokio)                │
│                                                                         │
│  ├─ Rate Limiter & Security Headers Middleware                          │
│  ├─ Direct Static Asset Server (ETag, Cache-Control, Last-Modified)     │
│  ├─ Workload-Aware Router (HTTP Header Rules + URL Pattern Rules)       │
│  │     ├──► [ Fast Pool  ] (Idle-First Atomic Dispatch, Keep-Alive UDS) │
│  │     └──► [ Heavy Pool ] (Isolated Workers, Independent Timeouts/RAM) │
│  ├─ Proactive Anti-OOM Engine (/proc/[pid]/status VmRSS + cgroups v2)   │
│  ├─ Async Job Queue (/api/queue/submit + Whitelisted Handlers)          │
│  └─ Secure DB Proxy (127.0.0.1:3307 ──► Pooled MySQL/MariaDB Backend)   │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Struktur Proyek

```text
bakpiarun/
├── src-server/                  # Rust Control & Ingress Plane
│   └── src/
│       ├── main.rs              # HTTP/HTTPS server, SIGTERM/SIGHUP lifecycle
│       ├── config.rs            # Full YAML + Environment configuration
│       ├── pool_manager.rs      # Workload-Aware Router (Header + URL)
│       ├── worker_pool.rs       # Lock-free Idle-First scheduling & rolling reload
│       ├── worker.rs            # PHP 8.3 supervisor & Anti-PHP2Shell hardening
│       ├── anti_oom.rs          # /proc/[pid]/status & cgroups v2 monitor
│       ├── ipc.rs               # Persistent Keep-Alive UDS framing
│       ├── db_proxy/            # MySQL 4.1 Wire-Protocol Connection Pool Proxy
│       ├── queue.rs             # Bounded async job queue
│       ├── metrics.rs           # Per-pool P50/P95/P99 & Prometheus histograms
│       └── handlers.rs          # HTTP, Health, Metrics, Reload & Queue endpoints
├── src-worker/
│   └── worker.php               # Persistent Keep-Alive PHP 8.3 Worker
├── lib/
│   ├── BakpiaLaravelBridge.php  # Persistent Laravel 8-12 Worker Bridge
│   ├── BakpiaPhpInfoFormatter.php # Styled HTML phpinfo() formatter for CLI
│   └── BakpiaQueue.php          # PHP client helper for Async Queue
├── config/
│   ├── bakpiarun.yaml           # Main runtime configuration
│   ├── bakpiarun.docker.yaml    # Container-optimized configuration
│   └── bakpiarun.example.yaml   # Documented reference configuration
├── benchmarks/                  # Apple-to-apple k6 lab (vs PHP-FPM & FrankenPHP)
├── public/                      # Default document root & verification endpoints
└── Dockerfile                   # Multi-stage Alpine 3.21 + PHP 8.3 + OPcache image
```

---

## Cara Menjalankan

### 1. Build & Jalankan secara Native (Linux / macOS)

```bash
cd src-server

# Jalankan unit test otomatis
cargo test

# Compile binary mode release yang teroptimasi penuh
cargo build --release

# Jalankan server menggunakan file konfigurasi YAML
./target/release/bakpiarun-server --config ../config/bakpiarun.yaml
```

### 2. Build & Jalankan menggunakan Docker (`cgroups v2` aktif)

```bash
docker build -t bakpiarun:latest .

docker run -d \
  --name bakpiarun \
  --memory="512m" \
  -p 8080:8080 \
  bakpiarun:latest
```

### 3. Observabilitas & Zero-Downtime Reload

```bash
# Cek status kesehatan per-pool, VmRSS, cgroups v2, dan persentil latensi
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/health

# Tarik metrik format Prometheus (untuk Grafana / evaluasi)
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/metrics

# Zero-downtime rolling reload seluruh worker PHP
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/reload
```

---

## Hasil Pengujian Performa (Release Build)

| Skenario Uji | Metrik | Hasil |
| :--- | :--- | :--- |
| **Pure Light Dynamic PHP** (`6.400` req, Keep-Alive UDS) | Throughput & Latensi | **`3.755,9 req/detik`** &bull; `P50: 1,86 ms` &bull; `P95: 3,79 ms` &bull; `P99: 5,28 ms` &bull; `0% Error` |
| **Mixed Workload + Memory Stress** (`Light` + `Heavy` + `Leak` simultan) | Isolasi Latensi & Anti-OOM | Pool `fast` terjaga di **`P50: 1,18 ms` (`P95: 5,66 ms`)** saat Pool `heavy` jenuh (`234 ms`) + **`10x Graceful Recycles` (`0% Dropped Requests`)** |

Untuk menjalankan suite benchmark komparatif **Grafana k6** melawan **Nginx + PHP-FPM 8.3** dan **FrankenPHP 8.3** pada batas kontainer identik (`2 vCPU`, `384 MB RAM`), lihat panduan lengkap di [`benchmarks/README.md`](benchmarks/README.md).

---

## Lisensi

Proyek ini dilisensikan di bawah ketentuan yang tertera pada berkas [LICENSE](LICENSE).

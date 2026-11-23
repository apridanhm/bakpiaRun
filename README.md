<p align="center">
  <img src="public/bakpiarun-logo.png" alt="bakpiaRun Logo" width="360">
</p>

<h1 align="center">bakpiaRun</h1>

<p align="center">
  <strong>A fast, friendly PHP 8.3 runtime powered by Rust.</strong><br>
  Built to keep your containers alive, your fast endpoints snappy, and your setup simple.
</p>

<p align="center">
  <a href="#english"><strong>English</strong></a> &nbsp;|&nbsp; <a href="#bahasa-indonesia"><strong>Bahasa Indonesia</strong></a>
</p>

---

<a id="english"></a>

## English

### Why bakpiaRun?

Running PHP in production (especially inside Docker or Kubernetes) usually comes with a few classic headaches:
1. **One heavy request slows everyone down.** Somebody triggers a massive report export, eats up your worker pool, and suddenly normal users just opening the homepage are stuck waiting.
2. **Containers randomly die with `OOMKilled (Exit 137)`.** Long-running PHP processes hold onto memory over time until Linux loses patience and kills the whole container, dropping active user requests along with it.
3. **Too many moving parts.** Stitching together Nginx, PHP-FPM, supervisord, a connection pooler, and a queue worker just to ship one app gets tiring fast.

**bakpiaRun** puts a lightweight **Rust (`Axum` + `Tokio`)** engine in front of **persistent PHP 8.3 CLI workers** talking over Keep-Alive **Unix Domain Sockets**. PHP stays warm in memory (so it is super fast), while Rust handles HTTP/2, static files, smart routing, memory protection, and database pooling in one neat package.

---

### What Makes It Cool

- **Fast Lane & Heavy Lane (`Workload-Aware Router`)**  
  Split your workers into separate pools (like `fast` and `heavy`). Route requests automatically using **URL patterns** (`/api/export/*`, `/heavy-*`) or **HTTP headers** (`X-Workload: heavy`). Heavy jobs stay in their own lane and never block regular users.

- **Proactive Anti-OOM (No More Surprise `Exit 137`)**  
  Instead of waiting for a crash, bakpiaRun checks each worker's real OS memory (`/proc/[pid]/status` `VmRSS`) and overall container memory (`cgroups v2`). When a worker gets close to its limit (say 80%), bakpiaRun stops sending it new traffic, lets it finish its current request, and swaps in a fresh worker in the background. Zero dropped requests, zero latency spikes.

- **Keep-Alive Unix Sockets + Automatic RAM Cleanup**  
  Rust and PHP talk over persistent Unix sockets with lock-free *Idle-First* scheduling. Each PHP request runs inside an isolated scope that cleans up big temporary memory chunks (`gc_mem_caches`) right after the request finishes.

- **Built-in MySQL/MariaDB Connection Proxy (`127.0.0.1:3307`)**  
  Your PHP scripts connect locally without needing the real DB password (*credential masking*). Under the hood, Rust pools backend connections, handles real binary prepared statements (`PDO::ATTR_EMULATE_PREPARES => false`), and pins connections automatically during `BEGIN` / `COMMIT` / `ROLLBACK` transactions.

- **Works with Native PHP, REST APIs & Laravel**  
  Full support for `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, clean front-controller URLs, multiple `Set-Cookie` headers, and a built-in **Laravel Persistent Bridge** that boots Laravel once in memory and reuses it across thousands of requests.

- **Hardened Out of the Box (Anti-PHP2Shell)**  
  Automatically locks down dangerous shell functions (`exec`, `shell_exec`, `system`, `proc_open`, etc.) and sandboxes file access via `open_basedir`, all configurable from a single YAML file.

---

### How It Works Under the Hood

```text
Browser / API Client
         |
         v  HTTP/1.1 or HTTPS (HTTP/2)
+----------------------------------------------------------------------+
|                     bakpiaRun Server (Rust + Axum)                   |
|                                                                      |
|  - Static files (.css, .js, images) --> Served directly by Rust      |
|  - Workload-Aware Router (checks HTTP Headers, then URL Patterns)    |
|       +--> Normal traffic --> [ Fast Pool  ] (e.g. 32 warm workers)  |
|       +--> Heavy reports  --> [ Heavy Pool ] (e.g.  8 warm workers)  |
|  - Proactive Anti-OOM (/proc/[pid]/status VmRSS + cgroups v2)        |
|  - Built-in Async Job Queue & Local MySQL Proxy (127.0.0.1:3307)     |
+----------------------------------------------------------------------+
```

---

### Quick Start

#### Option 1: Run Directly on Linux / macOS
Make sure you have **Rust** and **PHP 8.3 CLI** (`pcntl`, `pdo_mysql`, `curl`, `mbstring`, `opcache`) installed.

```bash
cd src-server

# Run unit tests (optional, super quick)
cargo test

# Build the optimized release binary
cargo build --release

# Start the server
./target/release/bakpiarun-server --config ../config/bakpiarun.yaml
```

#### Option 2: Run with Docker
```bash
docker build -t bakpiarun:latest .

# Run with a 512MB limit so cgroups v2 memory tracking kicks in
docker run -d \
  --name bakpiarun \
  --memory="512m" \
  -p 8080:8080 \
  bakpiarun:latest
```

---

### Handy Admin Endpoints

Protected by the `X-Admin-Token` (or `Authorization: Bearer`) header set in `config/bakpiarun.yaml`:

```bash
# Detailed JSON health, per-pool P50/P95/P99 latency, VmRSS & cgroups v2 stats
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/health

# Prometheus metrics (ready for Grafana)
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/metrics

# True zero-downtime rolling reload of all PHP workers
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/reload
```

---

### Speed & Stability Numbers

Tested on the release build over persistent Keep-Alive UDS:
- **Light Dynamic PHP**: **`3,755.9 req/sec`** (`P50: 1.86 ms`, `P95: 3.79 ms`, `P99: 5.28 ms`, `0% errors`).
- **Mixed Traffic + Memory Stress**: Even while the `heavy` pool is hammered at `~235 ms` and recycling workers in the background, the `fast` pool stays smooth at **`P50: 1.18 ms` (`P95: 5.66 ms`)** with **zero dropped requests**.

Want to compare **bakpiaRun vs Nginx+PHP-FPM vs FrankenPHP** yourself using **Grafana k6**? Check out the ready-to-run lab in [`benchmarks/README.md`](benchmarks/README.md).

---
---

<a id="bahasa-indonesia"></a>

## Bahasa Indonesia

<p align="right"><a href="#english">Back to English</a></p>

### Kenapa Ada bakpiaRun?

Kalau kita jalanin aplikasi PHP di server produksi (apalagi di dalam kontainer Docker atau Kubernetes), biasanya ada tiga penyakit klasik yang sering bikin pusing:
1. **Satu request berat bikin pengguna lain ikut lemot.** Misalnya ada admin lagi tarik laporan transaksi puluhan ribu baris, worker langsung penuh, dan user biasa yang cuma mau buka halaman depan malah ikut bengong nungguin antrean.
2. **Kontainer tiba-tiba mati kena `OOMKilled (Exit 137)`.** Worker PHP yang nyala terus lama-lama suka menahan pemakaian RAM. Begitu lewat batas kontainer, Linux langsung mematikan paksa kontainernya dan request user yang lagi jalan ikut putus.
3. **Setup yang terlalu ribet.** Mau deploy satu aplikasi saja harus ngurusin Nginx, PHP-FPM, supervisord, connection pooler database, sampai worker antrean secara terpisah.

**bakpiaRun** menyatukan semuanya dengan menaruh mesin **Rust (`Axum` + `Tokio`)** yang ringan di depan **worker PHP 8.3 CLI persisten** yang berkomunikasi lewat jalur lokal **Unix Domain Socket**. Proses PHP-nya tetap hangat di memori (jadi kencang karena tidak perlu *booting* ulang tiap request), sementara Rust mengurus HTTP/2, file statis, pembagian jalur beban kerja, penjaga memori, sampai proxy database dalam satu binary.

---

### Fitur Unggulan

- **Pisah Jalur Cepat & Jalur Berat (`Workload-Aware Router`)**  
  Kamu bisa membagi worker ke beberapa kolam terpisah (misalnya `fast` dan `heavy`) langsung dari file `bakpiarun.yaml`. Pengarahannya otomatis lewat **pola URL** (`/heavy-*`, `/api/report/*`) atau **HTTP Header** (`X-Workload: heavy`). Hasilnya, proses laporan seberat apa pun tidak akan mengganggu user lain di jalur cepat.

- **Penjaga Memori Proaktif (`Anti-OOM Engine`)**  
  Daripada menunggu kontainer meledak, bakpiaRun rutin memantau pemakaian RAM asli tiap proses PHP lewat `/proc/[pid]/status` (`VmRSS`) dan total RAM kontainer lewat `cgroups v2`. Kalau ada worker yang mulai mendekati batas (misal 80%), bakpiaRun menyetop kiriman tugas baru ke worker itu, membiarkan tugas yang sedang jalan sampai tuntas, lalu menggantinya dengan proses baru yang segar secara diam-diam di background. **Nol error, nol request putus.**

- **Pipa UDS Keep-Alive & Bersih-Bersih Memori Otomatis**  
  Komunikasi Rust ke PHP memakai pipa socket yang terus terbuka (*keep-alive*) dan selalu memilih worker yang sedang menganggur duluan (*Idle-First*). Setiap skrip PHP dijalankan di ruang lingkup terisolasi sehingga variabel besar langsung dibersihkan dan dikembalikan ke OS begitu request beres.

- **Proxy Database MySQL/MariaDB Bawaan (`127.0.0.1:3307`)**  
  Kode PHP kamu cukup konek ke port lokal `3307` tanpa perlu tahu password database asli (*credential masking*). Di belakang layar, Rust yang mengatur *connection pool* agar database tidak jebol kebanjiran koneksi, mendukung *native prepared statements* (`PDO::ATTR_EMULATE_PREPARES => false`), dan otomatis mengunci satu koneksi utuh saat aplikasi menjalankan transaksi `BEGIN` / `COMMIT` / `ROLLBACK`.

- **Siap untuk PHP Native, REST API, sampai Laravel**  
  Mendukung penuh method `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, *clean URL* (`front_controller`), multi `Set-Cookie`, tampilan `phpinfo()` kustom berlogo bakpiaRun, hingga **Laravel Persistent Bridge** yang mem-*boot* aplikasi Laravel cukup sekali di RAM (serupa Laravel Octane).

- **Aman dari Bawaan (Anti-PHP2Shell)**  
  Saat menyalakan worker PHP, Rust otomatis mematikan fungsi eksekusi shell berbahaya (`exec`, `shell_exec`, `system`, `proc_open`, dll.) dan mengurung akses folder lewat `open_basedir`. Semua tetap bisa kamu atur lewat YAML tanpa ada nilai yang *hardcode*.

---

### Cara Menjalankan

#### Opsi 1: Langsung di Mesin Linux / macOS
Pastikan sudah terpasang **Rust** dan **PHP 8.3 CLI** (beserta ekstensi `pcntl`, `pdo_mysql`, `curl`, `mbstring`, `opcache`).

```bash
cd src-server

# Cek semua unit test otomatis
cargo test

# Build binary versi release biar kencang maksimal
cargo build --release

# Jalankan server
./target/release/bakpiarun-server --config ../config/bakpiarun.yaml
```

#### Opsi 2: Pakai Docker
```bash
docker build -t bakpiarun:latest .

# Jalankan dengan batas RAM 512MB supaya sensor cgroups v2 aktif bekerja
docker run -d \
  --name bakpiarun \
  --memory="512m" \
  -p 8080:8080 \
  bakpiarun:latest
```

---

### Endpoint Monitoring & Reload Tanpa Downtime

Semua endpoint admin dilindungi token yang kamu atur di `server.admin_token` pada `config/bakpiarun.yaml`:

```bash
# Cek kondisi worker, RAM VmRSS, cgroups v2, dan latensi P50/P95/P99 tiap pool
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/health

# Ambil metrik format Prometheus (tinggal sambung ke Grafana)
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/metrics

# Reload ulang seluruh worker PHP secara bergiliran tanpa memutus trafik
curl -H "X-Admin-Token: dev-token-for-local-testing" http://127.0.0.1:8080/reload
```

---

### Seberapa Kencang?

Dari pengujian pada binary `--release`:
- **Trafik PHP Dinamis Ringan**: Tembus **`3.755,9 request/detik`** dengan waktu respons tengah **`P50: 1,86 ms`** (`P95: 3,79 ms`, `P99: 5,28 ms`, `0% error`).
- **Trafik Campuran + Tekanan Memori**: Saat kolam `heavy` sengaja dihajar beban berat (`~235 ms`) sekaligus simulasi bocor memori, kolam `fast` tetap melaju mulus di **`P50: 1,18 ms` (`P95: 5,66 ms`)** dan mesin Anti-OOM mendaur ulang worker di background tanpa menjatuhkan satu pun request pengguna.

Kalau kamu mau mencoba adu performa langsung (**bakpiaRun vs Nginx+PHP-FPM vs FrankenPHP**) memakai **Grafana k6**, skrip dan kontainer ujinya sudah siap pakai di folder [`benchmarks/README.md`](benchmarks/README.md).

---

## License / Lisensi

Dirilis di bawah lisensi yang tertera pada berkas [LICENSE](LICENSE).

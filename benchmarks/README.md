# Lab Evaluasi Komparatif Tesis S2 — BakpiaRun

Folder ini berisi lingkungan eksperimen terstandarisasi (*apple-to-apple*) untuk **Bab 4 (Hasil dan Pembahasan)** guna membandingkan **BakpiaRun** melawan **Nginx + PHP-FPM 8.3** dan **FrankenPHP 8.3** menggunakan **Grafana k6**.

## 1. Kesetaraan Parameter Lingkungan Uji (Apple-to-Apple)
Seluruh arsitektur dijalankan di atas kontainer Linux terisolasi dengan spesifikasi identik pada [`docker-compose.benchmark.yaml`](./docker-compose.benchmark.yaml):
- **Batas CPU**: `2.0 vCPU`
- **Batas Memori (`cgroups v2`)**: `384 MB` (`mem_limit: 384m`, `memswap_limit: 384m` — tanpa *swap* agar penegakan batas memori kernel akurat)
- **Kapasitas Pekerja**: `20` pekerja/thread (`BakpiaRun`: 16 `fast` + 4 `heavy`; `PHP-FPM`: `pm.max_children = 20`; `FrankenPHP`: `num_threads 20`)
- **Akselerasi Bytecode**: Zend OPcache aktif (`128 MB`, `validate_timestamps=0`) pada ketiga runtime
- **Kode Aplikasi Uji**: Ketiga kontainer melayani skrip PHP yang 100% identik di `public/bench/` (`light.php`, `heavy.php`, `leak.php`)

## 2. Tiga Skenario Pengujian (`benchmarks/k6/`)
1. **`scenario1_baseline_light.js` (Baseline Pure Light Workload)**:
   Mengukur *throughput* maksimum (RPS), latensi `P50`/`P95`/`P99`, dan *error rate* pada beban ringan murni (`20 -> 50 -> 100 VUs`).
2. **`scenario2_mixed_workload.js` (Mixed Workload / Head-of-Line Blocking)**:
   Menjalankan `40 VUs` trafik ringan (`/bench/light.php`) dan `12 VUs` trafik berat (`/bench/heavy.php`) secara bersamaan. Membuktikan bagaimana **Workload-Aware Router** menjaga latensi `P95`/`P99` pengguna ringan tetap rendah tanpa terkena antrean *noisy neighbor*.
3. **`scenario3_anti_oom_stress.js` (Memory Pressure & Anti-OOM Survival)**:
   Menjalankan beban kebocoran memori bertahap & lonjakan RSS simultan (`/bench/leak.php`) di dalam batas kontainer `384 MB` sembari memantau ketersediaan layanan (`availability_probe`) dan insiden **`OOMKilled` (`Exit Code 137`)**.

## 3. Cara Menjalankan Eksperimen

```bash
# 1. Build ketiga image kontainer benchmark
docker compose -f benchmarks/docker-compose.benchmark.yaml build

# 2. Jalankan kontainer
docker compose -f benchmarks/docker-compose.benchmark.yaml up -d

# 3. Jalankan seluruh skenario k6 secara otomatis + generate tabel Markdown Bab 4
chmod +x benchmarks/run_benchmarks.sh
./benchmarks/run_benchmarks.sh
```

Atau jalankan satu skenario k6 secara manual ke salah satu target (`:8081` BakpiaRun, `:8082` PHP-FPM, `:8083` FrankenPHP):

```bash
k6 run -e BASE_URL=http://127.0.0.1:8081 benchmarks/k6/scenario2_mixed_workload.js
```

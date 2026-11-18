import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate, Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL || 'http://127.0.0.1:8081';

// Metrik terpisah agar Bab 4 Tesis dapat menampilkan perbandingan latensi P95/P99
// trafik ringan saat terjadi gempuran trafik berat (Head-of-Line Blocking Test)
const lightLatency = new Trend('light_workload_latency_ms', true);
const heavyLatency = new Trend('heavy_workload_latency_ms', true);
const lightErrorRate = new Rate('light_workload_error_rate');
const heavyErrorRate = new Rate('heavy_workload_error_rate');

export const options = {
  summaryTrendStats: ['avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
  scenarios: {
    // 80-90% pengguna reguler mengakses endpoint ringan secara terus-menerus
    light_traffic: {
      executor: 'constant-vus',
      vus: 40,
      duration: '45s',
      exec: 'runLight',
    },
    // Pengguna laporan berat mengakses endpoint komputasi + memori tinggi secara bersamaan
    heavy_traffic: {
      executor: 'constant-vus',
      vus: 12,
      duration: '45s',
      exec: 'runHeavy',
    },
  },
  thresholds: {
    light_workload_error_rate: ['rate<0.01'],
    light_workload_latency_ms: ['p(95)<150'],
  },
};

export function runLight() {
  const res = http.get(`${BASE_URL}/bench/light.php`, {
    headers: { 'X-Workload': 'light' },
    tags: { workload: 'light' },
  });

  lightLatency.add(res.timings.duration);
  const ok = check(res, {
    'light status 200': (r) => r.status === 200,
  });
  lightErrorRate.add(!ok);
}

export function runHeavy() {
  const res = http.get(`${BASE_URL}/bench/heavy.php?rows=25000&delay_ms=60`, {
    headers: { 'X-Workload': 'heavy' },
    tags: { workload: 'heavy' },
    timeout: '30s',
  });

  heavyLatency.add(res.timings.duration);
  const ok = check(res, {
    'heavy status 200': (r) => r.status === 200,
  });
  heavyErrorRate.add(!ok);
  sleep(0.05);
}

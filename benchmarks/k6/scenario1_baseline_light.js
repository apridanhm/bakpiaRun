import http from 'k6/http';
import { check } from 'k6';
import { Rate, Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL || 'http://127.0.0.1:8081';

const errorRate = new Rate('bench_error_rate');
const lightLatency = new Trend('bench_light_latency_ms', true);

export const options = {
  summaryTrendStats: ['avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
  stages: [
    { duration: '10s', target: 20 },  // Warm-up
    { duration: '25s', target: 50 },  // Beban menengah
    { duration: '25s', target: 100 }, // Beban puncak konkurensi tinggi
    { duration: '5s', target: 0 },    // Cool-down
  ],
  thresholds: {
    http_req_failed: ['rate<0.01'],
    'http_req_duration': ['p(95)<100', 'p(99)<250'],
  },
};

export default function () {
  const res = http.get(`${BASE_URL}/bench/light.php`, {
    tags: { workload: 'light' },
  });

  lightLatency.add(res.timings.duration);
  const ok = check(res, {
    'status is 200': (r) => r.status === 200,
  });
  errorRate.add(!ok);
}

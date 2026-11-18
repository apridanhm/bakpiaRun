import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate, Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL || 'http://127.0.0.1:8081';

const availabilityRate = new Rate('availability_success_rate');
const oomErrorRate = new Rate('oom_induced_error_rate');
const lightDuringStressLatency = new Trend('light_during_mem_stress_ms', true);

export const options = {
  summaryTrendStats: ['avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
  scenarios: {
    // Probe ketersediaan layanan bagi pengguna reguler selama tekanan memori berlangsung
    availability_probe: {
      executor: 'constant-vus',
      vus: 20,
      duration: '45s',
      exec: 'probeLight',
    },
    // Pemicu akumulasi memori & lonjakan RSS simultan di kontainer 384 MB (No Swap)
    memory_pressure: {
      executor: 'constant-vus',
      vus: 10,
      duration: '45s',
      exec: 'triggerMemoryPressure',
    },
  },
};

export function probeLight() {
  const res = http.get(`${BASE_URL}/bench/light.php`, {
    tags: { role: 'probe' },
    timeout: '5s',
  });

  lightDuringStressLatency.add(res.timings.duration);
  const ok = check(res, {
    'probe 200 OK': (r) => r.status === 200,
  });
  availabilityRate.add(ok);
  oomErrorRate.add(!ok);
  sleep(0.02);
}

export function triggerMemoryPressure() {
  const res = http.get(`${BASE_URL}/bench/leak.php?leak_mb=8&spike_mb=24&hold_ms=40`, {
    headers: { 'X-Workload': 'heavy' },
    tags: { role: 'memory_bomber' },
    timeout: '15s',
  });

  const ok = check(res, {
    'memory stress handled (200 OK)': (r) => r.status === 200,
  });
  oomErrorRate.add(!ok);
  sleep(0.05);
}

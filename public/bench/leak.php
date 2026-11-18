<?php
// Benchmark Endpoint 3: Memory Stress & Stateful Leak Simulation
// Menguji ketahanan Proactive Anti-OOM Engine (/proc/[pid]/status + cgroups v2)
// dalam mencegah insiden OOMKilled (Exit Code 137) pada kontainer berbatas memori ketat.

if (function_exists('bakpiarun_header')) {
    bakpiarun_header('Content-Type: application/json; charset=utf-8');
} else {
    header('Content-Type: application/json; charset=utf-8');
}

$leakMb = isset($_GET['leak_mb']) ? max(1, min(32, (int)$_GET['leak_mb'])) : 6;
$spikeMb = isset($_GET['spike_mb']) ? max(0, min(64, (int)$_GET['spike_mb'])) : 24;
$holdMs = isset($_GET['hold_ms']) ? max(0, min(1000, (int)$_GET['hold_ms'])) : 40;

// 1. Akumulasi cache statis/global antar-request pada long-running worker
if (!isset($GLOBALS['__BAKPIA_LEAK_STORE'])) {
    $GLOBALS['__BAKPIA_LEAK_STORE'] = [];
}
$GLOBALS['__BAKPIA_LEAK_STORE'][] = random_bytes($leakMb * 1024 * 1024);

// 2. Lonjakan memori sementara selama request aktif (memicu concurrent RSS spike pada PHP-FPM)
$transientSpike = $spikeMb > 0 ? random_bytes($spikeMb * 1024 * 1024) : '';
if ($holdMs > 0) {
    usleep($holdMs * 1000);
}

$retainedChunks = count($GLOBALS['__BAKPIA_LEAK_STORE']);
$currentMb = round(memory_get_usage(true) / 1024 / 1024, 2);
$peakMb = round(memory_get_peak_usage(true) / 1024 / 1024, 2);
unset($transientSpike);

echo json_encode([
    'status' => 'ok',
    'workload' => 'memory_stress',
    'pid' => getmypid(),
    'retained_chunks' => $retainedChunks,
    'current_memory_mb' => $currentMb,
    'peak_memory_mb' => $peakMb,
]);

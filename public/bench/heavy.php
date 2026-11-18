<?php
// Benchmark Endpoint 2: Heavy Workload (Agregasi Laporan / Export Data Besar)
// Digunakan untuk menguji isolasi Workload-Aware Router terhadap Head-of-Line Blocking.

if (function_exists('bakpiarun_header')) {
    bakpiarun_header('Content-Type: application/json; charset=utf-8');
} else {
    header('Content-Type: application/json; charset=utf-8');
}

$rows = isset($_GET['rows']) ? max(1000, min(80000, (int)$_GET['rows'])) : 25000;
$delayMs = isset($_GET['delay_ms']) ? max(0, min(2000, (int)$_GET['delay_ms'])) : 60;

$start = microtime(true);

// Simulasi alokasi memori dataset besar (~30-45 MB) + komputasi agregasi
$dataset = [];
$totalAmount = 0;
for ($i = 0; $i < $rows; $i++) {
    $val = ($i % 97) * 1000;
    $totalAmount += $val;
    $dataset[] = [
        'row_id' => $i,
        'reference' => str_repeat('R', 256) . $i,
        'amount' => $val,
    ];
}

// Simulasi latensi I/O database/disk pada query laporan berat
if ($delayMs > 0) {
    usleep($delayMs * 1000);
}

$peakMb = round(memory_get_peak_usage(true) / 1024 / 1024, 2);
$elapsedMs = round((microtime(true) - $start) * 1000, 2);

echo json_encode([
    'status' => 'ok',
    'workload' => 'heavy',
    'pid' => getmypid(),
    'rows_processed' => count($dataset),
    'total_amount' => $totalAmount,
    'peak_memory_mb' => $peakMb,
    'duration_ms' => $elapsedMs,
]);

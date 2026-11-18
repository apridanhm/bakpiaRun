<?php
// Benchmark Endpoint 1: Light Workload (API / Transaksi Cepat)
// Digunakan secara identik pada BakpiaRun, Nginx+PHP-FPM, dan FrankenPHP.

if (function_exists('bakpiarun_header')) {
    bakpiarun_header('Content-Type: application/json; charset=utf-8');
} else {
    header('Content-Type: application/json; charset=utf-8');
}

$items = [];
for ($i = 1; $i <= 20; $i++) {
    $items[] = [
        'id' => $i,
        'sku' => 'BKP-' . str_pad((string)$i, 4, '0', STR_PAD_LEFT),
        'checksum' => hash('xxh3', "bakpia-item-{$i}"),
        'price' => $i * 15000,
    ];
}

echo json_encode([
    'status' => 'ok',
    'workload' => 'light',
    'pid' => getmypid(),
    'count' => count($items),
    'items' => $items,
]);

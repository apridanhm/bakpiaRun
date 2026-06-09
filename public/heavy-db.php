<?php
//header('Content-Type: text/plain; charset=utf-8');

echo "=== SIMULASI QUERY BERAT (FETCH 50,000 ROWS) ===\n\n";

// Cek memory sebelum eksekusi
$memoryBefore = memory_get_usage(true) / 1024 / 1024;
$startTime = microtime(true);

echo "1. Memulai eksekusi...\n";
echo "   Memory awal (Base): " . round($memoryBefore, 2) . " MB\n\n";

// Simulasi Query Berat: Membuat array besar (seperti hasil PDO fetchAll)
$massiveData = [];
for ($i = 0; $i < 50000; $i++) {
    $massiveData[] = [
        'id' => $i,
        'name' => 'User_' . $i . '_' . str_repeat('x', 100), // String 100 byte
        'email' => 'user' . $i . '@example.com',
        'bio' => str_repeat('Lorem ipsum dolor sit amet. ', 20) // String ~600 byte
    ];
}

// Cek memory sesudah eksekusi
$memoryAfter = memory_get_usage(true) / 1024 / 1024;
$peakMemory = memory_get_peak_usage(true) / 1024 / 1024;
$endTime = microtime(true);

echo "2. Eksekusi Selesai!\n";
echo "   Waktu eksekusi: " . round(($endTime - $startTime) * 1000, 2) . " ms\n";
echo "   Memory akhir: " . round($memoryAfter, 2) . " MB\n";
echo "   PEAK MEMORY: " . round($peakMemory, 2) . " MB  <-- INI YANG DIPAKAI SAAT PROSES\n";
echo "   Jumlah data: " . number_format(count($massiveData)) . " rows\n\n";

echo "=== KESIMPULAN ===\n";
echo "Memory naik dari " . round($memoryBefore, 2) . " MB menjadi " . round($peakMemory, 2) . " MB.\n";
echo "Setelah request ini selesai, worker akan reset memory-nya untuk request berikutnya.\n";

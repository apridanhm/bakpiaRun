<?php
try {
    $pdo = new PDO(
        'mysql:host=127.0.0.1;port=3307',
        'dummy',
        '',
        [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]
    );
    echo "DB Proxy CONNECTED!\n";
    echo "Server: " . $pdo->getAttribute(PDO::ATTR_SERVER_VERSION) . "\n";
} catch (PDOException $e) {
    echo "FAILED: " . $e->getMessage() . "\n";
}

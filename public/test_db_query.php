<?php
try {
    $pdo = new PDO(
        'mysql:host=127.0.0.1;port=3307;dbname=bakpiarun_db',
        'dummy',
        '',
        [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]
    );
    
    echo "Connected!\n";
    echo "Server: " . $pdo->getAttribute(PDO::ATTR_SERVER_VERSION) . "\n\n";
    
    // Test 1: Simple SELECT
    $stmt = $pdo->query("SELECT 1 as test, 'hello' as greeting");
    $result = $stmt->fetch(PDO::FETCH_ASSOC);
    echo "Test 1 - Simple SELECT:\n";
    echo json_encode($result) . "\n\n";
    
    // Test 2: SHOW TABLES
    $stmt = $pdo->query("SHOW TABLES");
    $tables = $stmt->fetchAll(PDO::FETCH_COLUMN);
    echo "Test 2 - Tables in database:\n";
    echo count($tables) . " tables found\n";
    if (count($tables) > 0) {
        echo "First 5: " . implode(", ", array_slice($tables, 0, 5)) . "\n";
    }
    
} catch (PDOException $e) {
    echo "Error: " . $e->getMessage() . "\n";
}

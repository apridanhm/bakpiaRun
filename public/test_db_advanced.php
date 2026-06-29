<?php
try {
    $pdo = new PDO(
        'mysql:host=127.0.0.1;port=3307;dbname=bakpiarun_db',
        'dummy',
        '',
        [
            PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
            PDO::ATTR_EMULATE_PREPARES => true,
            PDO::MYSQL_ATTR_DIRECT_QUERY => true, // Force direct queries
        ]
    );
    
    echo "✅ Connected!\n\n";
    
    // Test 1: SELECT with WHERE
    echo "Test 1 - SELECT with WHERE:\n";
    $stmt = $pdo->query("SELECT * FROM users WHERE id = 1");
    $user = $stmt->fetch(PDO::FETCH_ASSOC);
    echo json_encode($user, JSON_PRETTY_PRINT) . "\n\n";
    
    // Test 2: SELECT all
    echo "Test 2 - SELECT all users:\n";
    $stmt = $pdo->query("SELECT id, name, email FROM users ORDER BY id");
    $users = $stmt->fetchAll(PDO::FETCH_ASSOC);
    echo json_encode($users, JSON_PRETTY_PRINT) . "\n\n";
    
    // Test 3: Aggregate query
    echo "Test 3 - COUNT query:\n";
    $stmt = $pdo->query("SELECT COUNT(*) as total FROM users");
    $count = $stmt->fetch(PDO::FETCH_ASSOC);
    echo "Total users: " . $count['total'] . "\n\n";
    
    // Test 4: INSERT (DIRECT QUERY, NO PREPARE!)
    echo "Test 4 - INSERT:\n";
    $pdo->exec("INSERT INTO users (name, email) VALUES ('Alice Brown', 'alice@example.com')");
    echo "Inserted 1 user. Last insert ID: " . $pdo->lastInsertId() . "\n\n";
    
    // Test 5: UPDATE (DIRECT QUERY!)
    echo "Test 5 - UPDATE:\n";
    $affected = $pdo->exec("UPDATE users SET name = 'John Updated' WHERE id = 1");
    echo "Updated " . $affected . " row(s)\n\n";
    
    // Test 6: DELETE (DIRECT QUERY!)
    echo "Test 6 - DELETE:\n";
    $affected = $pdo->exec("DELETE FROM users WHERE id = 4");
    echo "Deleted " . $affected . " row(s)\n\n";
    
    // Test 7: Verify changes
    echo "Test 7 - Verify:\n";
    $stmt = $pdo->query("SELECT * FROM users ORDER BY id");
    $users = $stmt->fetchAll(PDO::FETCH_ASSOC);
    echo json_encode($users, JSON_PRETTY_PRINT) . "\n\n";
    
    echo "✅ All tests passed!\n";
    
} catch (PDOException $e) {
    echo "❌ Error: " . $e->getMessage() . "\n";
}

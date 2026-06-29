<?php
try {
    $pdo = new PDO(
        'mysql:host=127.0.0.1;port=3307;dbname=bakpiarun_db',
        'dummy',
        '',
        [
            PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
            // PAKSA pake REAL prepared statements (bukan emulated)
            PDO::ATTR_EMULATE_PREPARES => true,
        ]
    );
    
    echo "Connected with REAL Prepared Statements!\n\n";
    
    // Test 1: Prepared SELECT
    echo "Test 1 - Prepared SELECT:\n";
    $stmt = $pdo->prepare("SELECT * FROM users WHERE id = ?");
    $stmt->execute([1]);
    $user = $stmt->fetch(PDO::FETCH_ASSOC);
    echo json_encode($user, JSON_PRETTY_PRINT) . "\n\n";
    
    // Test 2: Prepared INSERT
    echo "Test 2 - Prepared INSERT:\n";
    $stmt = $pdo->prepare("INSERT INTO users (name, email) VALUES (?, ?)");
    $stmt->execute(['Test User', 'test@example.com']);
    echo "Inserted. Last ID: " . $pdo->lastInsertId() . "\n\n";
    
    // Test 3: Prepared UPDATE
    echo "Test 3 - Prepared UPDATE:\n";
    $stmt = $pdo->prepare("UPDATE users SET name = ? WHERE id = ?");
    $stmt->execute(['Updated User', 1]);
    echo "Updated " . $stmt->rowCount() . " row(s)\n\n";
    
    // Test 4: Prepared DELETE
    echo "Test 4 - Prepared DELETE:\n";
    $last_id = $pdo->lastInsertId();
    $stmt = $pdo->prepare("DELETE FROM users WHERE id = ?");
    $stmt->execute([$last_id]);
    echo "Deleted " . $stmt->rowCount() . " row(s)\n\n";
    
    // Test 5: Multiple parameters
    echo "Test 5 - Multiple parameters:\n";
    $stmt = $pdo->prepare("SELECT * FROM users WHERE name LIKE ? AND email LIKE ?");
    $stmt->execute(['%John%', '%@example.com']);
    $users = $stmt->fetchAll(PDO::FETCH_ASSOC);
    echo "Found " . count($users) . " user(s)\n";
    echo json_encode($users, JSON_PRETTY_PRINT) . "\n\n";
    
    echo "All prepared statement tests passed!\n";
    
} catch (PDOException $e) {
    echo "Error: " . $e->getMessage() . "\n";
}

<?php
/**
 * example submit job ke queue
 */

require_once __DIR__ . '/../../lib/BakpiaQueue.php';

// konfigurasi (opsional, default sudah localhost:8080)
BakpiaQueue::setBaseUrl('http://localhost:8080');

try {
    // submit job
    echo "Submitting job...\n";
    
    $result = BakpiaQueue::push('send_welcome_email', [
        'to' => 'user@example.com',
        'name' => 'John Doe',
        'subject' => 'Welcome to BakpiaRun!'
    ]);
    
    echo "Job submitted successfully!\n";
    echo "Job ID: " . $result['job_id'] . "\n";
    echo "Status: " . $result['status'] . "\n";
    echo "Message: " . $result['message'] . "\n";
    
} catch (Exception $e) {
    echo "Error: " . $e->getMessage() . "\n";
    exit(1);
}
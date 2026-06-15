<?php
/**
 * example: cek status job
 */

require_once __DIR__ . '/../../lib/BakpiaQueue.php';

// ambil job ID dari command line argument
if ($argc < 2) {
    echo "Usage: php check_status.php <job_id>\n";
    exit(1);
}

$jobId = $argv[1];

try {
    echo "Checking status for job: $jobId\n";
    
    $status = BakpiaQueue::status($jobId);
    
    echo "\nJob Status:\n";
    echo "ID: " . $status['id'] . "\n";
    echo "Task: " . $status['task'] . "\n";
    echo "Status: " . $status['status'] . "\n";
    echo "Created: " . $status['created_at'] . "\n";
    
    if (isset($status['completed_at'])) {
        echo "Completed: " . $status['completed_at'] . "\n";
    }
    
    if (isset($status['result'])) {
        echo "Result: " . json_encode($status['result'], JSON_PRETTY_PRINT) . "\n";
    }
    
} catch (Exception $e) {
    echo "Error: " . $e->getMessage() . "\n";
    exit(1);
}
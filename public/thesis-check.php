<?php
// Test endpoint untuk memverifikasi Klaim Keamanan Anti-PHP2Shell & Custom Headers/Status
http_response_code(201);
if (function_exists('bakpiarun_header')) {
    bakpiarun_header('Content-Type: application/json; charset=utf-8');
    bakpiarun_header('X-Custom-Thesis: BakpiaRun-Verified');
} else {
    header('Content-Type: application/json; charset=utf-8');
    header('X-Custom-Thesis: BakpiaRun-Verified');
}

$disabled = array_map('trim', explode(',', ini_get('disable_functions')));
$shell_blocked = in_array('shell_exec', $disabled) || !function_exists('shell_exec');
$exec_blocked  = in_array('exec', $disabled) || !function_exists('exec');
$system_blocked = in_array('system', $disabled) || !function_exists('system');

// Coba baca /etc/passwd untuk menguji open_basedir
$passwd_readable = @is_readable('/etc/passwd');

echo json_encode([
    'claim' => 'Security Hardening & Custom Headers/Status',
    'php_version' => PHP_VERSION,
    'pid' => getmypid(),
    'status_code_set' => 201,
    'security' => [
        'allow_url_include' => ini_get('allow_url_include'),
        'open_basedir' => ini_get('open_basedir'),
        'disable_functions' => ini_get('disable_functions'),
        'shell_exec_blocked' => $shell_blocked,
        'exec_blocked' => $exec_blocked,
        'system_blocked' => $system_blocked,
        'etc_passwd_blocked_by_open_basedir' => !$passwd_readable,
    ]
], JSON_PRETTY_PRINT);

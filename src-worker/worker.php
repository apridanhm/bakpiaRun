<?php
// src-worker/worker.php

$running = true;
pcntl_async_signals(true);
pcntl_signal(SIGTERM, function() use (&$running) { $running = false; });
pcntl_signal(SIGINT, function() use (&$running) { $running = false; });

$socketPath = getenv('BAKPIARUN_SOCKET_PATH') ?: '/tmp/bakpiarun.sock';

$socketDir = dirname($socketPath);
if (!is_dir($socketDir)) {
    mkdir($socketDir, 0755, true);
}

if (file_exists($socketPath)) {
    unlink($socketPath);
}

$server = stream_socket_server("unix://{$socketPath}", $errno, $errstr);
if (!$server) {
    die("Gagal membuat socket: {$errstr} ({$errno})\n");
}

echo "[PHP Worker] Listening on unix://{$socketPath}\n";
echo "[PHP Worker] Ready to accept requests...\n";

while ($running) {
    $client = @stream_socket_accept($server, 1.0);
    if (!$client) {
        continue; 
    }

    stream_set_timeout($client, 5.0);

    // Baca request dari Rust
    $lengthHeader = fread($client, 4);
    if ($lengthHeader === false || strlen($lengthHeader) < 4) {
        fclose($client);
        continue;
    }

    $payloadLength = unpack('N', $lengthHeader)[1];
    $payload = fread($client, $payloadLength);
    
    $request = json_decode($payload, true);

    if (!$request) {
        fclose($client);
        continue;
    }

    // --- INJECT SUPERGLOBALS ---
    
    // $_SERVER
    $_SERVER = [];
    $_SERVER['REQUEST_METHOD'] = $request['method'] ?? 'GET';
    $_SERVER['REQUEST_URI']    = $request['uri'] ?? '/';
    $_SERVER['SERVER_NAME']    = 'localhost';
    $_SERVER['SERVER_PORT']    = '8080';
    $_SERVER['SERVER_PROTOCOL'] = 'HTTP/1.1';
    $_SERVER['HTTP_HOST']      = 'localhost:8080';
    $_SERVER['SCRIPT_NAME']    = '/index.php';
    $_SERVER['PHP_SELF']       = '/index.php';
    $_SERVER['QUERY_STRING']   = $request['query_string'] ?? '';
    $_SERVER['CONTENT_TYPE']   = $request['content_type'] ?? '';
    $_SERVER['CONTENT_LENGTH'] = $request['content_length'] ?? '0';
    
    // Parse URI untuk PATH_INFO
    $uri_parts = parse_url($request['uri'] ?? '/');
    $_SERVER['PATH_INFO'] = $uri_parts['path'] ?? '/';
    
    // HTTP Headers
    if (isset($request['headers']) && is_array($request['headers'])) {
        foreach ($request['headers'] as $key => $value) {
            $header_name = 'HTTP_' . strtoupper(str_replace('-', '_', $key));
            $_SERVER[$header_name] = $value;
        }
    }
    
    // $_GET
    $_GET = [];
    if (isset($request['query_params']) && is_array($request['query_params'])) {
        $_GET = $request['query_params'];
    }
    
    // $_POST
    $_POST = [];
    if (isset($request['post_params']) && is_array($request['post_params'])) {
        $_POST = $request['post_params'];
    }
    
    // $_COOKIE
    $_COOKIE = [];
    if (isset($request['cookies']) && is_array($request['cookies'])) {
        $_COOKIE = $request['cookies'];
    }
    
    // $_REQUEST (gabungan GET + POST + COOKIE)
    $_REQUEST = array_merge($_GET, $_POST, $_COOKIE);
    
    // $_FILES
    $_FILES = [];
    if (isset($request['files']) && is_array($request['files'])) {
        foreach ($request['files'] as $field_name => $file_info) {
            // Simpan file temporary
            $tmp_file = tempnam(sys_get_temp_dir(), 'bakpiarun_upload_');
            file_put_contents($tmp_file, base64_decode($file_info['content']));
            
            $_FILES[$field_name] = [
                'name' => $file_info['name'],
                'type' => $file_info['type'],
                'tmp_name' => $tmp_file,
                'error' => UPLOAD_ERR_OK,
                'size' => $file_info['size'],
            ];
        }
    }
    
    // Raw body untuk php://input
    $GLOBALS['HTTP_RAW_POST_DATA'] = $request['body'] ?? '';

    // --- EKSEKUSI PHP ---
    ob_start();
    
    $filePath = $request['file_path'] ?? '';
    $status = 200;

    if ($filePath && file_exists($filePath)) {
        include $filePath;
    } else {
        $status = 404;
        echo "<h1>404 Not Found</h1><p>File tidak ditemukan: " . htmlspecialchars($filePath) . "</p>";
    }

    $body = ob_get_clean();

    // Cleanup uploaded files
    foreach ($_FILES as $file) {
        if (isset($file['tmp_name']) && file_exists($file['tmp_name'])) {
            unlink($file['tmp_name']);
        }
    }

    $currentMemory = memory_get_usage(true);
    $peakMemory = memory_get_peak_usage(true);

    // --- KIRIM RESPONSE ---
    $response = [
        'status'  => $status,
        'body'    => $body,
        'memory'  => $currentMemory,
        'peak'    => $peakMemory,
    ];

    $responseJson = json_encode($response);
    $responsePayload = pack('N', strlen($responseJson)) . $responseJson;
    fwrite($client, $responsePayload);

    fclose($client);
}

if (file_exists($socketPath)) {
    unlink($socketPath);
}
echo "[PHP Worker] Shutting down gracefully.\n";
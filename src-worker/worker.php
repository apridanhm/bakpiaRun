<?php
// src-worker/worker.php - Robust Version

error_reporting(E_ALL);
ini_set('display_errors', '1');

function bakpia_log(string $msg): void {
    fwrite(STDERR, $msg . "\n");
}

// Helper bagi aplikasi PHP di mode CLI jika ingin menambahkan custom response header
$GLOBALS['__BAKPIARUN_HEADERS'] = [];
function bakpiarun_header(string $header_line, bool $replace = true, int $response_code = 0): void {
    if ($response_code > 0) {
        http_response_code($response_code);
    }
    $parts = explode(':', $header_line, 2);
    if (count($parts) === 2) {
        $GLOBALS['__BAKPIARUN_HEADERS'][trim($parts[0])] = trim($parts[1]);
    }
}

$running = true;
pcntl_async_signals(true);
pcntl_signal(SIGTERM, function() use (&$running) { 
    bakpia_log("[PHP Worker] Received SIGTERM, shutting down...");
    $running = false; 
});
pcntl_signal(SIGINT, function() use (&$running) { 
    bakpia_log("[PHP Worker] Received SIGINT, shutting down...");
    $running = false; 
});
pcntl_signal(SIGPIPE, SIG_IGN); // Ignore SIGPIPE

$socketPath = getenv('BAKPIARUN_SOCKET_PATH') ?: '/tmp/bakpiarun.sock';
$timeoutSec = (float)(getenv('BAKPIARUN_TIMEOUT_SEC') ?: 30.0);
$maxPayloadMb = (int)(getenv('BAKPIARUN_MAX_PAYLOAD_MB') ?: 100);
$maxPayloadBytes = $maxPayloadMb * 1024 * 1024;
$defaultServerHost = getenv('BAKPIARUN_SERVER_HOST') ?: 'localhost';
$defaultServerPort = getenv('BAKPIARUN_SERVER_PORT') ?: '8080';

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

bakpia_log("[PHP Worker] Listening on unix://{$socketPath}");
bakpia_log("[PHP Worker] Ready to accept requests...");
bakpia_log("[PHP Worker] PID: " . getmypid());

while ($running) {
    try {
        $client = @stream_socket_accept($server, 1.0);
        if (!$client) {
            continue;
        }

        stream_set_timeout($client, (int)$timeoutSec);
        stream_set_blocking($client, true);

        // Baca length header (4 bytes)
        $lengthHeader = @fread($client, 4);
        if ($lengthHeader === false || strlen($lengthHeader) < 4) {
            bakpia_log("[PHP Worker] Failed to read length header");
            fclose($client);
            continue;
        }

        $payloadLength = unpack('N', $lengthHeader)[1];
        
        // Sanity check: jangan terima payload melebihi batas config
        if ($payloadLength > $maxPayloadBytes) {
            bakpia_log("[PHP Worker] Payload too large: {$payloadLength} bytes (max: {$maxPayloadBytes})");
            fclose($client);
            continue;
        }

        $payload = '';
        $bytesRead = 0;
        $readFailed = false;
        
        while ($bytesRead < $payloadLength) {
            $chunk = @fread($client, min(8192, $payloadLength - $bytesRead));
            
            if ($chunk === false) {
                bakpia_log("[PHP Worker] fread() returned false");
                $readFailed = true;
                break;
            }
            
            if (strlen($chunk) === 0) {
                bakpia_log("[PHP Worker] Connection closed (EOF)");
                $readFailed = true;
                break;
            }
            
            $payload .= $chunk;
            $bytesRead += strlen($chunk);
        }
        
        if ($readFailed || $bytesRead !== $payloadLength) {
            bakpia_log("[PHP Worker] Failed to read full payload (got {$bytesRead} of {$payloadLength} bytes)");
            fclose($client);
            continue;
        }
        
        $request = json_decode($payload, true);
        if (!$request) {
            bakpia_log("[PHP Worker] Invalid JSON: " . json_last_error_msg());
            fclose($client);
            continue;
        }

        bakpia_log("[PHP Worker] Handling request: " . ($request['method'] ?? '?') . " " . ($request['uri'] ?? '?'));

        // --- INJECT SUPERGLOBALS ---
        $docroot = getcwd();
        $reqUri = $request['uri'] ?? '/';
        $uri_parts = parse_url($reqUri);
        $uriPath = $uri_parts['path'] ?? '/';
        $filePath = $request['file_path'] ?? '';
        $scriptName = $uriPath;
        if ($filePath && str_starts_with($filePath, $docroot)) {
            $scriptName = substr($filePath, strlen($docroot));
        }

        $hostHeader = $request['headers']['host'] ?? "{$defaultServerHost}:{$defaultServerPort}";
        $hostParts = explode(':', $hostHeader);
        $serverName = $hostParts[0] ?: $defaultServerHost;
        $serverPort = $hostParts[1] ?? $defaultServerPort;

        $_SERVER = [];
        $_SERVER['REQUEST_METHOD']  = $request['method'] ?? 'GET';
        $_SERVER['REQUEST_URI']     = $reqUri;
        $_SERVER['SERVER_NAME']     = $serverName;
        $_SERVER['SERVER_PORT']     = $serverPort;
        $_SERVER['SERVER_PROTOCOL'] = 'HTTP/1.1';
        $_SERVER['HTTP_HOST']       = $hostHeader;
        $_SERVER['SCRIPT_NAME']     = $scriptName;
        $_SERVER['PHP_SELF']        = $scriptName;
        $_SERVER['SCRIPT_FILENAME'] = $filePath;
        $_SERVER['QUERY_STRING']    = $request['query_string'] ?? '';
        $_SERVER['CONTENT_TYPE']    = $request['content_type'] ?? '';
        $_SERVER['CONTENT_LENGTH']  = $request['content_length'] ?? '0';
        $_SERVER['DOCUMENT_ROOT']   = $docroot;
        $_SERVER['PATH_INFO']       = $uriPath;
        
        if (isset($request['headers']) && is_array($request['headers'])) {
            foreach ($request['headers'] as $key => $value) {
                $header_name = 'HTTP_' . strtoupper(str_replace('-', '_', $key));
                $_SERVER[$header_name] = $value;
            }
        }
        
        $_GET = $request['query_params'] ?? [];
        $_POST = $request['post_params'] ?? [];
        $_COOKIE = $request['cookies'] ?? [];
        $_REQUEST = array_merge($_GET, $_POST, $_COOKIE);
        
        // Handle $_FILES dengan struktur array (support multiple files)
        $_FILES = [];
        if (isset($request['files']) && is_array($request['files'])) {
            foreach ($request['files'] as $field_name => $file_list) {
                $is_array_field = str_ends_with($field_name, '[]');
                $clean_name = rtrim($field_name, '[]');
                
                if ($is_array_field) {
                    $_FILES[$clean_name] = [
                        'name' => [],
                        'type' => [],
                        'tmp_name' => [],
                        'error' => [],
                        'size' => []
                    ];
                    
                    foreach ($file_list as $file_info) {
                        $tmp_path = $file_info['tmp_path'] ?? '';
                        if (!empty($tmp_path) && file_exists($tmp_path)) {
                            $_FILES[$clean_name]['name'][] = $file_info['name'];
                            $_FILES[$clean_name]['type'][] = $file_info['type'];
                            $_FILES[$clean_name]['tmp_name'][] = realpath($tmp_path);
                            $_FILES[$clean_name]['error'][] = UPLOAD_ERR_OK;
                            $_FILES[$clean_name]['size'][] = $file_info['size'];
                        }
                    }
                } else {
                    if (isset($file_list[0])) {
                        $file_info = $file_list[0];
                        $tmp_path = $file_info['tmp_path'] ?? '';
                        
                        if (!empty($tmp_path) && file_exists($tmp_path)) {
                            $_FILES[$clean_name] = [
                                'name' => $file_info['name'],
                                'type' => $file_info['type'],
                                'tmp_name' => realpath($tmp_path),
                                'error' => UPLOAD_ERR_OK,
                                'size' => $file_info['size'],
                            ];
                            if (PHP_VERSION_ID >= 80100) {
                                $_FILES[$clean_name]['full_path'] = $file_info['name'];
                            }
                        }
                    }
                }
            }
        }
        
        // Set up php://input
        $body_content = $request['body'] ?? '';
        if (!empty($body_content)) {
            file_put_contents('php://temp', $body_content);
        }

        // --- EKSEKUSI PHP ---
        $GLOBALS['__BAKPIARUN_HEADERS'] = [];
        http_response_code(200);
        if (function_exists('header_remove')) {
            @header_remove();
        }
        ob_start();
        
        $filePath = $request['file_path'] ?? '';
        $status = 200;
        $exec_error = null;

        try {
            if ($filePath && file_exists($filePath)) {
                include $filePath;
                $code = http_response_code();
                if (is_int($code) && $code >= 100 && $code <= 599) {
                    $status = $code;
                }
            } else {
                $status = 404;
                echo "<h1>404 Not Found</h1><p>File tidak ditemukan: " . htmlspecialchars($filePath) . "</p>";
            }
        } catch (Throwable $e) {
            $exec_error = $e->getMessage();
            $status = 500;
            echo "<h1>500 Error</h1><p>" . htmlspecialchars($exec_error) . "</p>";
        }

        $body = ob_get_clean();
        $respHeaders = $GLOBALS['__BAKPIARUN_HEADERS'];
        if (function_exists('headers_list')) {
            foreach (headers_list() as $rawHeader) {
                $parts = explode(':', $rawHeader, 2);
                if (count($parts) === 2) {
                    $respHeaders[trim($parts[0])] = trim($parts[1]);
                }
            }
        }

        $currentMemory = memory_get_usage(true);
        $peakMemory = memory_get_peak_usage(true);

        // --- KIRIM RESPONSE ---
        $response = [
            'status'  => $status,
            'headers' => (object)$respHeaders,
            'body'    => $body,
            'memory'  => $currentMemory,
            'peak'    => $peakMemory,
        ];

        $responseJson = json_encode($response, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE);
        if ($responseJson === false) {
            bakpia_log("[PHP Worker] Failed to encode response: " . json_last_error_msg());
            fclose($client);
            continue;
        }
        
        $responsePayload = pack('N', strlen($responseJson)) . $responseJson;
        
        $written = @fwrite($client, $responsePayload);
        if ($written === false) {
            bakpia_log("[PHP Worker] Failed to write response");
        } else {
            bakpia_log("[PHP Worker] Response sent: " . strlen($responsePayload) . " bytes");
        }

        fflush($client);
        fclose($client);
        
        bakpia_log("[PHP Worker] Request complete");
        
    } catch (Throwable $e) {
        bakpia_log("[PHP Worker] FATAL ERROR: " . $e->getMessage());
        bakpia_log("[PHP Worker] Stack trace: " . $e->getTraceAsString());
        if (isset($client) && is_resource($client)) {
            @fclose($client);
        }
    }
}

if (file_exists($socketPath)) {
    unlink($socketPath);
}
bakpia_log("[PHP Worker] Shutting down gracefully.");

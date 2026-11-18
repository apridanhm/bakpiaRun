<?php
// src-worker/worker.php - High-Performance Persistent Keep-Alive & Modern Framework Worker

error_reporting(E_ALL);
ini_set('display_errors', '1');

$bakpiaDebug = strtolower(getenv('BAKPIARUN_LOG_LEVEL') ?: 'info') === 'debug';

function bakpia_log(string $msg): void {
    fwrite(STDERR, $msg . "\n");
}

function bakpia_debug(string $msg): void {
    global $bakpiaDebug;
    if ($bakpiaDebug) {
        fwrite(STDERR, $msg . "\n");
    }
}

// State per-request untuk response headers, Set-Cookie multi-value, dan raw HTTP body
$GLOBALS['__BAKPIARUN_HEADERS'] = [];
$GLOBALS['__BAKPIARUN_COOKIES'] = [];
$GLOBALS['__BAKPIARUN_RAW_BODY'] = '';

/**
 * Mengambil raw HTTP request body (untuk JSON REST API PUT/PATCH/POST/DELETE di mode CLI).
 */
function bakpiarun_raw_body(): string {
    return $GLOBALS['__BAKPIARUN_RAW_BODY'] ?? '';
}

/**
 * Helper decode JSON request body langsung menjadi associative array.
 */
function bakpiarun_json_body(): array {
    $raw = $GLOBALS['__BAKPIARUN_RAW_BODY'] ?? '';
    if ($raw === '') {
        return [];
    }
    $decoded = json_decode($raw, true);
    return is_array($decoded) ? $decoded : [];
}

/**
 * CLI-compatible pengganti header() yang mendukung Location redirect (auto 302) & multi Set-Cookie.
 */
function bakpiarun_header(string $header_line, bool $replace = true, int $response_code = 0): void {
    if ($response_code > 0) {
        http_response_code($response_code);
    }

    if (str_starts_with(strtoupper($header_line), 'HTTP/')) {
        if (preg_match('#^HTTP/\S+\s+(\d{3})#i', $header_line, $m)) {
            http_response_code((int)$m[1]);
        }
        return;
    }

    $parts = explode(':', $header_line, 2);
    if (count($parts) !== 2) {
        return;
    }

    $name = trim($parts[0]);
    $value = trim($parts[1]);
    $lower = strtolower($name);

    if ($lower === 'set-cookie') {
        if ($replace) {
            $GLOBALS['__BAKPIARUN_COOKIES'] = [$value];
        } else {
            $GLOBALS['__BAKPIARUN_COOKIES'][] = $value;
        }
        return;
    }

    if ($lower === 'location' && $response_code === 0 && http_response_code() === 200) {
        http_response_code(302);
    }

    if (!$replace && isset($GLOBALS['__BAKPIARUN_HEADERS'][$name])) {
        $GLOBALS['__BAKPIARUN_HEADERS'][$name] .= ', ' . $value;
    } else {
        $GLOBALS['__BAKPIARUN_HEADERS'][$name] = $value;
    }
}

/**
 * CLI-compatible pengganti setcookie() sesuai RFC 6265.
 */
function bakpiarun_setcookie(
    string $name,
    string $value = '',
    int $expires = 0,
    string $path = '/',
    string $domain = '',
    bool $secure = false,
    bool $httponly = false,
    string $samesite = 'Lax'
): void {
    $cookie = rawurlencode($name) . '=' . rawurlencode($value);
    if ($expires > 0) {
        $cookie .= '; Expires=' . gmdate('D, d M Y H:i:s T', $expires);
        $cookie .= '; Max-Age=' . max(0, $expires - time());
    }
    if ($path !== '') {
        $cookie .= '; Path=' . $path;
    }
    if ($domain !== '') {
        $cookie .= '; Domain=' . $domain;
    }
    if ($secure) {
        $cookie .= '; Secure';
    }
    if ($httponly) {
        $cookie .= '; HttpOnly';
    }
    if ($samesite !== '') {
        $cookie .= '; SameSite=' . $samesite;
    }
    $GLOBALS['__BAKPIARUN_COOKIES'][] = $cookie;
}

/**
 * Isolated Static Closure Sandbox untuk PHP Native / Micro-frameworks
 */
$__bakpiaRunIsolated = static function (string $__bakpia_target_file): void {
    include $__bakpia_target_file;
};

$running = true;
pcntl_async_signals(true);
pcntl_signal(SIGTERM, function () use (&$running) {
    bakpia_log("[PHP Worker] Received SIGTERM, shutting down...");
    $running = false;
});
pcntl_signal(SIGINT, function () use (&$running) {
    bakpia_log("[PHP Worker] Received SIGINT, shutting down...");
    $running = false;
});
pcntl_signal(SIGPIPE, SIG_IGN);

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

$context = stream_context_create([
    'socket' => [
        'backlog' => 1024,
    ],
]);
$server = stream_socket_server(
    "unix://{$socketPath}",
    $errno,
    $errstr,
    STREAM_SERVER_BIND | STREAM_SERVER_LISTEN,
    $context
);
if (!$server) {
    die("Gagal membuat socket: {$errstr} ({$errno})\n");
}

$docroot = getcwd();
$frontIndexPath = realpath($docroot . '/index.php') ?: ($docroot . '/index.php');
$projectRoot = dirname($docroot);

// Deteksi 1x di awal (0% overhead di hot-path jika bukan Laravel!)
$laravelBridge = null;
$isLaravelApp = is_file($projectRoot . '/artisan')
    && is_file($projectRoot . '/vendor/autoload.php')
    && is_file($projectRoot . '/bootstrap/app.php');

if ($isLaravelApp) {
    $bridgeFile = __DIR__ . '/../lib/BakpiaLaravelBridge.php';
    if (is_file($bridgeFile)) {
        require_once $bridgeFile;
        $laravelBridge = new BakpiaLaravelBridge($projectRoot);
        bakpia_log("[PHP Worker] Laravel application detected at {$projectRoot} (Persistent Bridge Ready)");
    }
}

bakpia_log("[PHP Worker] Listening on unix://{$socketPath} (PID: " . getmypid() . ")");

while ($running) {
    try {
        $client = @stream_socket_accept($server, 1.0);
        if (!$client) {
            continue;
        }

        stream_set_timeout($client, max(1, (int)$timeoutSec));
        stream_set_blocking($client, true);

        while ($running && is_resource($client)) {
            $read = [$client];
            $write = null;
            $except = null;
            $ready = @stream_select($read, $write, $except, 1, 0);

            if ($ready === false) {
                break;
            }
            if ($ready === 0) {
                if (feof($client)) {
                    break;
                }
                continue;
            }

            // 1. Baca 4-byte Big-Endian length header
            $lengthHeader = '';
            while (strlen($lengthHeader) < 4) {
                $chunk = @fread($client, 4 - strlen($lengthHeader));
                if ($chunk === false || $chunk === '') {
                    break;
                }
                $lengthHeader .= $chunk;
            }

            if ($lengthHeader === '') {
                break;
            }

            if (strlen($lengthHeader) < 4) {
                bakpia_log("[PHP Worker] Incomplete length header");
                break;
            }

            $payloadLength = unpack('N', $lengthHeader)[1];
            if ($payloadLength > $maxPayloadBytes) {
                bakpia_log("[PHP Worker] Payload too large: {$payloadLength} bytes (max: {$maxPayloadBytes})");
                break;
            }

            // 2. Baca JSON request payload
            $payload = '';
            $bytesRead = 0;
            $readFailed = false;

            while ($bytesRead < $payloadLength) {
                $chunk = @fread($client, min(65536, $payloadLength - $bytesRead));
                if ($chunk === false || $chunk === '') {
                    $readFailed = true;
                    break;
                }
                $payload .= $chunk;
                $bytesRead += strlen($chunk);
            }

            if ($readFailed || $bytesRead !== $payloadLength) {
                bakpia_log("[PHP Worker] Failed to read full payload ({$bytesRead}/{$payloadLength} bytes)");
                break;
            }

            $request = json_decode($payload, true);
            unset($payload);
            if (!is_array($request)) {
                bakpia_log("[PHP Worker] Invalid JSON: " . json_last_error_msg());
                break;
            }

            $method = $request['method'] ?? 'GET';
            $reqUri = $request['uri'] ?? '/';
            bakpia_debug("[PHP Worker] Handling request: {$method} {$reqUri}");

            // 3. Reset peak memory per-request & siapkan superglobals
            if (function_exists('memory_reset_peak_usage')) {
                memory_reset_peak_usage();
            }

            $uriParts = parse_url($reqUri);
            $uriPath = $uriParts['path'] ?? '/';
            $filePath = $request['file_path'] ?? '';
            $scriptName = $uriPath;
            if ($filePath !== '' && str_starts_with($filePath, $docroot)) {
                $scriptName = substr($filePath, strlen($docroot));
            }

            $hostHeader = $request['headers']['host'] ?? "{$defaultServerHost}:{$defaultServerPort}";
            $hostParts = explode(':', $hostHeader);
            $serverName = $hostParts[0] ?: $defaultServerHost;
            $serverPort = $hostParts[1] ?? $defaultServerPort;
            $rawBody = (string)($request['body'] ?? '');
            $GLOBALS['__BAKPIARUN_RAW_BODY'] = $rawBody;

            $_SERVER = [
                'REQUEST_METHOD'     => $method,
                'REQUEST_URI'        => $reqUri,
                'SERVER_NAME'        => $serverName,
                'SERVER_PORT'        => $serverPort,
                'SERVER_PROTOCOL'    => 'HTTP/1.1',
                'HTTP_HOST'          => $hostHeader,
                'SCRIPT_NAME'        => $scriptName,
                'PHP_SELF'           => $scriptName,
                'SCRIPT_FILENAME'    => $filePath,
                'QUERY_STRING'       => $request['query_string'] ?? '',
                'CONTENT_TYPE'       => $request['content_type'] ?? '',
                'HTTP_CONTENT_TYPE'  => $request['content_type'] ?? '',
                'CONTENT_LENGTH'     => $request['content_length'] ?? '0',
                'HTTP_CONTENT_LENGTH'=> $request['content_length'] ?? '0',
                'DOCUMENT_ROOT'      => $docroot,
                'PATH_INFO'          => $uriPath,
                'REQUEST_TIME'       => time(),
                'REQUEST_TIME_FLOAT' => microtime(true),
            ];

            if (isset($request['headers']) && is_array($request['headers'])) {
                foreach ($request['headers'] as $key => $value) {
                    $headerName = 'HTTP_' . strtoupper(str_replace('-', '_', $key));
                    $_SERVER[$headerName] = $value;
                }
            }

            $_GET = $request['query_params'] ?? [];
            $_POST = $request['post_params'] ?? [];
            $_COOKIE = $request['cookies'] ?? [];
            $_REQUEST = array_merge($_GET, $_POST, $_COOKIE);

            $_FILES = [];
            if (isset($request['files']) && is_array($request['files'])) {
                foreach ($request['files'] as $fieldName => $fileList) {
                    $isArrayField = str_ends_with($fieldName, '[]');
                    $cleanName = rtrim($fieldName, '[]');

                    if ($isArrayField) {
                        $_FILES[$cleanName] = [
                            'name' => [],
                            'type' => [],
                            'tmp_name' => [],
                            'error' => [],
                            'size' => [],
                        ];
                        foreach ($fileList as $fileInfo) {
                            $tmpPath = $fileInfo['tmp_path'] ?? '';
                            if ($tmpPath !== '' && file_exists($tmpPath)) {
                                $_FILES[$cleanName]['name'][] = $fileInfo['name'];
                                $_FILES[$cleanName]['type'][] = $fileInfo['type'];
                                $_FILES[$cleanName]['tmp_name'][] = realpath($tmpPath);
                                $_FILES[$cleanName]['error'][] = UPLOAD_ERR_OK;
                                $_FILES[$cleanName]['size'][] = $fileInfo['size'];
                            }
                        }
                    } elseif (isset($fileList[0])) {
                        $fileInfo = $fileList[0];
                        $tmpPath = $fileInfo['tmp_path'] ?? '';
                        if ($tmpPath !== '' && file_exists($tmpPath)) {
                            $_FILES[$cleanName] = [
                                'name' => $fileInfo['name'],
                                'type' => $fileInfo['type'],
                                'tmp_name' => realpath($tmpPath),
                                'error' => UPLOAD_ERR_OK,
                                'size' => $fileInfo['size'],
                            ];
                            if (PHP_VERSION_ID >= 80100) {
                                $_FILES[$cleanName]['full_path'] = $fileInfo['name'];
                            }
                        }
                    }
                }
            }
            unset($request);

            // 4. Eksekusi: Jalur Persistent Laravel Bridge ATAU Jalur Cepat Isolated Closure
            $GLOBALS['__BAKPIARUN_HEADERS'] = [];
            $GLOBALS['__BAKPIARUN_COOKIES'] = [];

            $resolvedTarget = ($filePath !== '') ? (realpath($filePath) ?: $filePath) : '';
            if ($laravelBridge !== null && $resolvedTarget === $frontIndexPath) {
                try {
                    $laravelResult = $laravelBridge->handle(
                        $reqUri,
                        $method,
                        $_GET,
                        $_POST,
                        $_COOKIE,
                        $_FILES,
                        $_SERVER,
                        $rawBody
                    );
                    $status = $laravelResult['status'];
                    $respHeaders = $laravelResult['headers'];
                    $respCookies = $laravelResult['cookies'];
                    $body = $laravelResult['body'];
                    unset($laravelResult);
                } catch (Throwable $e) {
                    $status = 500;
                    $respHeaders = [];
                    $respCookies = [];
                    $body = "<h1>500 Laravel Worker Error</h1><p>" . htmlspecialchars($e->getMessage()) . "</p>";
                }
            } else {
                http_response_code(200);
                if (function_exists('header_remove')) {
                    @header_remove();
                }

                $baseObLevel = ob_get_level();
                ob_start();

                $status = 200;
                try {
                    if ($filePath !== '' && file_exists($filePath)) {
                        $__bakpiaRunIsolated($filePath);
                        $code = http_response_code();
                        if (is_int($code) && $code >= 100 && $code <= 599) {
                            $status = $code;
                        }
                    } else {
                        $status = 404;
                        echo "<h1>404 Not Found</h1><p>File tidak ditemukan: " . htmlspecialchars($filePath) . "</p>";
                    }
                } catch (Throwable $e) {
                    $status = 500;
                    echo "<h1>500 Error</h1><p>" . htmlspecialchars($e->getMessage()) . "</p>";
                }

                while (ob_get_level() > $baseObLevel + 1) {
                    ob_end_flush();
                }
                $body = (string)ob_get_clean();

                $respHeaders = $GLOBALS['__BAKPIARUN_HEADERS'];
                $respCookies = $GLOBALS['__BAKPIARUN_COOKIES'];
                if (function_exists('headers_list')) {
                    foreach (headers_list() as $rawHeader) {
                        $parts = explode(':', $rawHeader, 2);
                        if (count($parts) === 2) {
                            $hName = trim($parts[0]);
                            $hVal = trim($parts[1]);
                            if (strtolower($hName) === 'set-cookie') {
                                $respCookies[] = $hVal;
                            } else {
                                $respHeaders[$hName] = $hVal;
                            }
                        }
                    }
                }
            }

            // Rekam peak memory, bersihkan state request, & kembalikan chunk besar (>8MB) ke kernel
            $peakMemory = memory_get_peak_usage(true);
            $_GET = $_POST = $_COOKIE = $_REQUEST = $_FILES = [];
            $GLOBALS['__BAKPIARUN_HEADERS'] = [];
            $GLOBALS['__BAKPIARUN_COOKIES'] = [];
            $GLOBALS['__BAKPIARUN_RAW_BODY'] = '';
            unset($rawBody);

            if (memory_get_usage(true) > 8388608) {
                gc_collect_cycles();
                if (function_exists('gc_mem_caches')) {
                    gc_mem_caches();
                }
            }
            $currentMemory = memory_get_usage(true);

            // 5. Kirim response JSON utuh melalui persistent UDS stream
            $responsePayloadData = [
                'status'  => $status,
                'headers' => (object)$respHeaders,
                'body'    => $body,
                'memory'  => $currentMemory,
                'peak'    => $peakMemory,
            ];
            if (!empty($respCookies)) {
                $responsePayloadData['cookies'] = $respCookies;
            }

            $responseJson = json_encode(
                $responsePayloadData,
                JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE
            );
            unset($responsePayloadData, $body, $respHeaders, $respCookies);

            if ($responseJson === false) {
                bakpia_log("[PHP Worker] Failed to encode response: " . json_last_error_msg());
                break;
            }

            $responsePayload = pack('N', strlen($responseJson)) . $responseJson;
            unset($responseJson);

            $totalLen = strlen($responsePayload);
            $offset = 0;
            $writeFailed = false;
            while ($offset < $totalLen) {
                $written = @fwrite($client, substr($responsePayload, $offset));
                if ($written === false || $written === 0) {
                    $writeFailed = true;
                    break;
                }
                $offset += $written;
            }
            unset($responsePayload);

            if ($writeFailed) {
                bakpia_log("[PHP Worker] Failed to write full response to UDS");
                break;
            }

            @fflush($client);
            bakpia_debug("[PHP Worker] Response sent ({$totalLen} bytes)");
        }

        if (is_resource($client)) {
            @fclose($client);
        }
    } catch (Throwable $e) {
        bakpia_log("[PHP Worker] FATAL ERROR: " . $e->getMessage());
        if (isset($client) && is_resource($client)) {
            @fclose($client);
        }
    }
}

if (file_exists($socketPath)) {
    @unlink($socketPath);
}
bakpia_log("[PHP Worker] Shutting down gracefully.");

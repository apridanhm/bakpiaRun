<?php
// src-worker/worker.php - Robust Version

error_reporting(E_ALL);
ini_set('display_errors', '1');

/**
 * Stream wrapper that makes `php://input` return the current request body in
 * CLI SAPI (where it is otherwise empty). All other `php://` streams
 * (temp, memory, stdout, stderr, filter, fd, ...) are delegated to the native
 * implementation, so existing behaviour is preserved.
 *
 * The wrapper is only installed for the duration of script execution (see the
 * request loop), so the worker's own bookkeeping uses the native `php://`.
 */
class BakpiaPhpStreamWrapper
{
    /** Raw body of the request currently being handled. */
    public static string $input = '';

    public $context;

    private bool $isInput = false;
    private int $pos = 0;
    /** @var resource|null */
    private $delegate = null;

    public function stream_open($path, $mode, $options, &$opened_path): bool
    {
        if (strtolower($path) === 'php://input') {
            $this->isInput = true;
            $this->pos = 0;
            return true;
        }

        // Open the underlying php:// stream (temp/memory/filter/...) using the
        // native wrapper. The returned resource carries its own stream ops, so
        // subsequent read/write/seek operate on it directly regardless of which
        // wrapper is currently registered for the "php" scheme.
        $this->isInput = false;
        $usePath = (bool) ($options & STREAM_USE_PATH);
        @stream_wrapper_restore('php');
        $this->delegate = @fopen($path, $mode, $usePath);
        @stream_wrapper_unregister('php');
        @stream_wrapper_register('php', self::class);
        return is_resource($this->delegate);
    }

    public function stream_read($count)
    {
        if ($this->isInput) {
            $chunk = substr(self::$input, $this->pos, $count);
            $this->pos += strlen($chunk);
            return $chunk;
        }
        return fread($this->delegate, $count);
    }

    public function stream_write($data)
    {
        if ($this->isInput) {
            return 0; // php://input is read-only
        }
        return fwrite($this->delegate, $data);
    }

    public function stream_eof(): bool
    {
        if ($this->isInput) {
            return $this->pos >= strlen(self::$input);
        }
        return feof($this->delegate);
    }

    public function stream_tell()
    {
        if ($this->isInput) {
            return $this->pos;
        }
        return ftell($this->delegate);
    }

    public function stream_seek($offset, $whence = SEEK_SET): bool
    {
        if ($this->isInput) {
            $len = strlen(self::$input);
            switch ($whence) {
                case SEEK_SET: $new = $offset; break;
                case SEEK_CUR: $new = $this->pos + $offset; break;
                case SEEK_END: $new = $len + $offset; break;
                default: return false;
            }
            if ($new < 0) {
                return false;
            }
            $this->pos = $new;
            return true;
        }
        return fseek($this->delegate, $offset, $whence) === 0;
    }

    public function stream_stat()
    {
        if ($this->isInput) {
            return ['size' => strlen(self::$input)];
        }
        return fstat($this->delegate);
    }

    public function stream_close(): void
    {
        if (!$this->isInput && is_resource($this->delegate)) {
            fclose($this->delegate);
        }
    }

    public function stream_set_option($option, $arg1, $arg2): bool
    {
        return false;
    }

    public function url_stat($path, $flags)
    {
        return false;
    }
}

$running = true;
pcntl_async_signals(true);
pcntl_signal(SIGTERM, function() use (&$running) {
    echo "[PHP Worker] Received SIGTERM, shutting down...\n";
    $running = false;
});
pcntl_signal(SIGINT, function() use (&$running) {
    echo "[PHP Worker] Received SIGINT, shutting down...\n";
    $running = false;
});
pcntl_signal(SIGPIPE, SIG_IGN); // Ignore SIGPIPE

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
echo "[PHP Worker] PID: " . getmypid() . "\n";

while ($running) {
    try {
        $client = @stream_socket_accept($server, 1.0);
        if (!$client) {
            continue;
        }

        stream_set_timeout($client, 30.0); // 30 detik timeout
        stream_set_blocking($client, true);

        // Baca length header (4 bytes)
        $lengthHeader = @fread($client, 4);
        if ($lengthHeader === false || strlen($lengthHeader) < 4) {
            echo "[PHP Worker] Failed to read length header\n";
            fclose($client);
            continue;
        }

        $payloadLength = unpack('N', $lengthHeader)[1];

        // Sanity check: jangan terima payload > 100MB
        if ($payloadLength > 100 * 1024 * 1024) {
            echo "[PHP Worker] Payload too large: {$payloadLength} bytes\n";
            fclose($client);
            continue;
        }

        $payload = '';
        $bytesRead = 0;
        $readFailed = false;

        while ($bytesRead < $payloadLength) {
            $chunk = @fread($client, min(8192, $payloadLength - $bytesRead));

            if ($chunk === false) {
                echo "[PHP Worker] fread() returned false\n";
                $readFailed = true;
                break;
            }

            if (strlen($chunk) === 0) {
                echo "[PHP Worker] Connection closed (EOF)\n";
                $readFailed = true;
                break;
            }

            $payload .= $chunk;
            $bytesRead += strlen($chunk);
        }

        if ($readFailed || $bytesRead !== $payloadLength) {
            echo "[PHP Worker] Failed to read full payload (got {$bytesRead} of {$payloadLength} bytes)\n";
            fclose($client);
            continue;
        }

        $request = json_decode($payload, true);
        if (!$request) {
            echo "[PHP Worker] Invalid JSON: " . json_last_error_msg() . "\n";
            fclose($client);
            continue;
        }

        echo "[PHP Worker] Handling request: " . ($request['method'] ?? '?') . " " . ($request['uri'] ?? '?') . "\n";

        // --- RESET PER-REQUEST STATE ---
        // Clear any response headers / status carried over from a prior request
        // handled by this long-running process.
        if (function_exists('header_remove')) {
            header_remove();
        }
        http_response_code(200);

        // --- INJECT SUPERGLOBALS ---
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
        $_SERVER['DOCUMENT_ROOT']  = getcwd();

        $uri_parts = parse_url($request['uri'] ?? '/');
        $_SERVER['PATH_INFO'] = $uri_parts['path'] ?? '/';

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
        $_SESSION = []; // don't leak session data between requests

        // Handle $_FILES dengan struktur array (support multiple files)
        $_FILES = [];
        $uploadedTmpFiles = []; // tracked so we can remove them after the request
        if (isset($request['files']) && is_array($request['files'])) {
            foreach ($request['files'] as $field_name => $file_list) {
                // Cek apakah field ini array (misal name="files[]") atau single
                $is_array_field = str_ends_with($field_name, '[]');
                $clean_name = rtrim($field_name, '[]');

                if ($is_array_field) {
                    // Format array untuk multiple files
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
                            $uploadedTmpFiles[] = $tmp_path;
                        }
                    }
                } else {
                    // Format single file
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
                            $uploadedTmpFiles[] = $tmp_path;
                        }
                    }
                }
            }
        }

        // Expose the raw body to php://input via the stream wrapper.
        BakpiaPhpStreamWrapper::$input = $request['body'] ?? '';

        // --- EKSEKUSI PHP ---
        ob_start();

        $filePath = $request['file_path'] ?? '';
        $status = 200;
        $exec_error = null;

        // Activate the php:// wrapper only while the user script runs.
        $wrapperActive = @stream_wrapper_unregister('php')
            && @stream_wrapper_register('php', BakpiaPhpStreamWrapper::class);

        try {
            if ($filePath && file_exists($filePath)) {
                include $filePath;
                $code = http_response_code();
                $status = is_int($code) ? $code : 200;
            } else {
                $status = 404;
                http_response_code(404);
                echo "<h1>404 Not Found</h1><p>File tidak ditemukan: " . htmlspecialchars($filePath) . "</p>";
            }
        } catch (Throwable $e) {
            $exec_error = $e->getMessage();
            $status = 500;
            http_response_code(500);
            echo "<h1>500 Error</h1><p>" . htmlspecialchars($exec_error) . "</p>";
        } finally {
            if ($wrapperActive) {
                @stream_wrapper_restore('php');
            }
        }

        $body = ob_get_clean();

        // Capture response headers emitted by the script (header(), setcookie()).
        $responseHeaders = headers_list();

        $currentMemory = memory_get_usage(true);
        $peakMemory = memory_get_peak_usage(true);

        // --- KIRIM RESPONSE ---
        $response = [
            'status'  => $status,
            'body'    => $body,
            'headers' => $responseHeaders,
            'memory'  => $currentMemory,
            'peak'    => $peakMemory,
        ];

        $responseJson = json_encode($response, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE);
        if ($responseJson === false) {
            echo "[PHP Worker] Failed to encode response: " . json_last_error_msg() . "\n";
            fclose($client);
            continue;
        }

        $responsePayload = pack('N', strlen($responseJson)) . $responseJson;

        $written = @fwrite($client, $responsePayload);
        if ($written === false) {
            echo "[PHP Worker] Failed to write response\n";
        } else {
            echo "[PHP Worker] Response sent: " . strlen($responsePayload) . " bytes\n";
        }

        fflush($client);
        fclose($client);

        // Remove upload temp files so they don't accumulate on disk. (Note:
        // move_uploaded_file() does not work for these in CLI SAPI — apps must
        // use rename()/copy(); see README.)
        foreach ($uploadedTmpFiles as $tmpFile) {
            if (is_string($tmpFile) && $tmpFile !== '' && file_exists($tmpFile)) {
                @unlink($tmpFile);
            }
        }

        echo "[PHP Worker] Request complete\n";

    } catch (Throwable $e) {
        echo "[PHP Worker] FATAL ERROR: " . $e->getMessage() . "\n";
        echo "[PHP Worker] Stack trace: " . $e->getTraceAsString() . "\n";
        // Make sure the wrapper is restored even if something blew up.
        if (in_array('php', stream_get_wrappers(), true) === false) {
            @stream_wrapper_restore('php');
        }
        // Jangan exit, lanjut ke request berikutnya
        if (isset($client) && is_resource($client)) {
            @fclose($client);
        }
    }
}

if (file_exists($socketPath)) {
    unlink($socketPath);
}
echo "[PHP Worker] Shutting down gracefully.\n";

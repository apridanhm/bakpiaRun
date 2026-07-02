<?php
// P0 self-test endpoint. Exercises the response-header bridge and php://input.
//
//   curl -i  'http://localhost:8080/p0-selftest.php?mode=json'
//   curl -i  'http://localhost:8080/p0-selftest.php?mode=redirect'
//   curl -i  'http://localhost:8080/p0-selftest.php?mode=cookie'
//   curl -i  'http://localhost:8080/p0-selftest.php?mode=status'
//   curl -s -X POST --data '{"hello":"world"}' \
//        'http://localhost:8080/p0-selftest.php?mode=input'

$mode = $_GET['mode'] ?? 'help';

switch ($mode) {
    case 'json': // Fix #1: Content-Type from PHP must reach the client
        header('Content-Type: application/json');
        echo json_encode(['ok' => true, 'mode' => 'json']);
        break;

    case 'redirect': // Fix #1: Location header + 302 must reach the client
        header('Location: /test.php');
        echo 'redirecting';
        break;

    case 'cookie': // Fix #1: Set-Cookie must reach the client
        setcookie('bakpia_test', 'value123', ['path' => '/']);
        header('X-Custom-Header: hello');
        echo 'cookie set';
        break;

    case 'status': // Fix #1: http_response_code() must reach the client
        http_response_code(418);
        echo "I'm a teapot";
        break;

    case 'input': // Fix #2: php://input must return the raw request body
        $raw = file_get_contents('php://input');
        header('Content-Type: application/json');
        echo json_encode([
            'received_bytes' => strlen($raw),
            'raw'            => $raw,
            'json_decoded'   => json_decode($raw, true),
        ]);
        break;

    default:
        header('Content-Type: text/plain');
        echo "modes: json | redirect | cookie | status | input\n";
}

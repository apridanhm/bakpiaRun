<?php
/**
 * BakpiaRun Persistent Worker Bridge for Laravel (8 / 9 / 10 / 11 / 12)
 *
 * Mem-boot instans aplikasi Laravel & Composer Autoloader HANYA SATU KALI
 * di memori pekerja persisten (serupa Laravel Octane / RoadRunner), kemudian
 * melayani setiap HTTP request secara langsung melalui Http\Kernel tanpa
 * overhead bootstrapping ulang.
 */
class BakpiaLaravelBridge
{
    private string $projectRoot;
    /** @var mixed */
    private $app = null;
    /** @var mixed */
    private $kernel = null;

    public function __construct(string $projectRoot)
    {
        $this->projectRoot = rtrim($projectRoot, '/');
    }

    private function ensureBooted(): void
    {
        if ($this->app !== null && $this->kernel !== null) {
            return;
        }

        if (!defined('LARAVEL_START')) {
            define('LARAVEL_START', microtime(true));
        }

        require_once $this->projectRoot . '/vendor/autoload.php';
        $this->app = require $this->projectRoot . '/bootstrap/app.php';
        $this->kernel = $this->app->make(\Illuminate\Contracts\Http\Kernel::class);
    }

    /**
     * @param array<string, mixed> $queryParams
     * @param array<string, mixed> $postParams
     * @param array<string, mixed> $cookies
     * @param array<string, mixed> $files
     * @param array<string, mixed> $server
     * @return array{status:int, headers:array<string,string>, cookies:list<string>, body:string}
     */
    public function handle(
        string $uri,
        string $method,
        array $queryParams,
        array $postParams,
        array $cookies,
        array $files,
        array $server,
        string $rawBody
    ): array {
        $this->ensureBooted();

        $parameters = in_array(strtoupper($method), ['GET', 'HEAD'], true) ? $queryParams : $postParams;

        /** @var \Illuminate\Http\Request $illuminateReq */
        $illuminateReq = \Illuminate\Http\Request::create(
            $uri,
            strtoupper($method),
            $parameters,
            $cookies,
            $files,
            $server,
            $rawBody
        );

        if (class_exists(\Illuminate\Support\Facades\Facade::class)) {
            \Illuminate\Support\Facades\Facade::clearResolvedInstance('request');
            \Illuminate\Support\Facades\Facade::clearResolvedInstance('auth');
        }
        $this->app->instance('request', $illuminateReq);

        $response = $this->kernel->handle($illuminateReq);

        $status = (int)$response->getStatusCode();
        $outHeaders = [];
        $outCookies = [];

        if (isset($response->headers)) {
            if (method_exists($response->headers, 'allPreserveCaseWithoutCookies')) {
                foreach ($response->headers->allPreserveCaseWithoutCookies() as $name => $values) {
                    if (is_array($values) && !empty($values)) {
                        $outHeaders[(string)$name] = implode(', ', $values);
                    }
                }
            }
            if (method_exists($response->headers, 'getCookies')) {
                foreach ($response->headers->getCookies() as $cookieObj) {
                    $outCookies[] = (string)$cookieObj;
                }
            }
        }

        // Tangani StreamedResponse / BinaryFileResponse maupun Response biasa
        if ($response instanceof \Symfony\Component\HttpFoundation\BinaryFileResponse) {
            $file = $response->getFile();
            $body = $file->isReadable() ? (string)file_get_contents($file->getPathname()) : '';
        } elseif ($response instanceof \Symfony\Component\HttpFoundation\StreamedResponse) {
            ob_start();
            $response->sendContent();
            $body = (string)ob_get_clean();
        } else {
            $body = (string)$response->getContent();
        }

        $this->kernel->terminate($illuminateReq, $response);

        if (method_exists($this->app, 'forgetScopedInstances')) {
            $this->app->forgetScopedInstances();
        }

        return [
            'status'  => $status,
            'headers' => $outHeaders,
            'cookies' => $outCookies,
            'body'    => $body,
        ];
    }
}

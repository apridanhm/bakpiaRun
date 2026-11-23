<?php
/**
 * BakpiaRun CLI phpinfo() HTML Formatter
 *
 * Mengubah output teks datar `phpinfo()` pada mode Persistent PHP CLI menjadi
 * halaman tabel HTML resmi bergaya PHP 8.3 yang rapi, dilengkapi logo resmi
 * bakpiaRun (tanpa menampilkan PID proses demi keamanan & kerapian).
 */
function bakpiarun_format_cli_phpinfo(string $raw): string
{
    static $cachedLogoUri = null;
    if ($cachedLogoUri === null) {
        $logoFile = __DIR__ . '/assets/bakpiarun-logo.png';
        if (is_file($logoFile) && is_readable($logoFile)) {
            $bytes = @file_get_contents($logoFile);
            $cachedLogoUri = ($bytes !== false)
                ? ('data:image/png;base64,' . base64_encode($bytes))
                : '';
        } else {
            $cachedLogoUri = '';
        }
    }

    $lines = preg_split('/\r?\n/', $raw) ?: [];
    $phpVersion = PHP_VERSION;

    // Gabungkan baris lanjutan (contoh daftar file .ini yang berakhir dengan koma)
    $merged = [];
    foreach ($lines as $line) {
        if (
            !empty($merged)
            && !str_contains($line, ' => ')
            && trim($line) !== ''
            && str_ends_with(rtrim($merged[count($merged) - 1]), ',')
        ) {
            $merged[count($merged) - 1] .= ' ' . trim($line);
        } else {
            $merged[] = $line;
        }
    }

    $esc = static function (string $s): string {
        if (str_starts_with(trim($s), '<font style="color:')) {
            return $s;
        }
        return htmlspecialchars($s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
    };

    $html = [];
    $html[] = '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">';
    $html[] = '<meta name="viewport" content="width=device-width, initial-scale=1">';
    $html[] = '<title>PHP ' . $esc($phpVersion) . ' - phpinfo() | bakpiaRun</title>';
    $html[] = '<style>
        body { background-color: #f4f5fa; color: #222; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 0; padding: 24px 12px 48px; }
        .center { max-width: 960px; margin: 0 auto; }
        table { border-collapse: collapse; border: 0; width: 100%; box-shadow: 0 1px 4px rgba(0,0,0,0.10); margin-bottom: 18px; table-layout: fixed; background: #fff; }
        td, th { border: 1px solid #666; font-size: 13px; vertical-align: baseline; padding: 6px 9px; word-wrap: break-word; overflow-wrap: anywhere; }
        th { position: sticky; top: 0; background-color: #8892bf; color: #fff; font-weight: 700; text-align: left; }
        .h { background: linear-gradient(135deg, #4f5b93 0%, #7a86b8 100%); color: #fff; border-radius: 8px; padding: 14px 22px; display: flex; justify-content: space-between; align-items: center; gap: 16px; margin-bottom: 20px; box-shadow: 0 3px 10px rgba(79,91,147,0.25); }
        .h-title h1 { font-size: 24px; margin: 0 0 4px 0; font-weight: 700; letter-spacing: -0.2px; }
        .h-sub { font-size: 13px; color: #e6e9f8; font-weight: 500; letter-spacing: 0.2px; }
        .logo-card { background: #ffffff; border-radius: 10px; padding: 4px 12px; overflow: hidden; display: flex; align-items: center; justify-content: center; box-shadow: 0 2px 8px rgba(0,0,0,0.18); flex-shrink: 0; }
        .logo-card img { height: 68px; width: auto; display: block; object-fit: contain; transform: scale(1.22); }
        h1.sec { font-size: 20px; text-align: center; margin: 28px 0 14px; color: #2d3250; }
        h2.ext { font-size: 17px; text-align: center; margin: 22px 0 8px; color: #2d3250; background: #e8ebf7; padding: 7px; border: 1px solid #8892bf; border-radius: 3px; }
        .e { background-color: #ccf; width: 32%; font-weight: 700; color: #111; }
        .v { background-color: #e9e9ee; width: 68%; color: #111; }
        .v3 { background-color: #e9e9ee; width: 34%; color: #111; }
        .box { background: #ffffff; border: 1px solid #8892bf; padding: 12px 16px; font-size: 13px; line-height: 1.55; margin-bottom: 18px; white-space: pre-wrap; border-radius: 4px; }
    </style></head><body><div class="center">';

    $logoHtml = ($cachedLogoUri !== '')
        ? '<div class="logo-card"><img src="' . $cachedLogoUri . '" alt="bakpiaRun Logo"></div>'
        : '';

    $html[] = '<div class="h">'
        . '<div class="h-title">'
        . '<h1>PHP Version ' . $esc($phpVersion) . '</h1>'
        . '<div class="h-sub">Powered by <strong>bakpiaRun</strong> Cloud-Native Runtime</div>'
        . '</div>'
        . $logoHtml
        . '</div>';

    $openCols = 0;
    $paraBuf = [];

    $closeTable = static function () use (&$openCols, &$html): void {
        if ($openCols > 0) {
            $html[] = '</tbody></table>';
            $openCols = 0;
        }
    };

    $flushPara = static function () use (&$paraBuf, &$html, $esc): void {
        if (!empty($paraBuf)) {
            $text = trim(implode("\n", $paraBuf));
            if ($text !== '') {
                $html[] = '<div class="box">' . $esc($text) . '</div>';
            }
            $paraBuf = [];
        }
    };

    foreach ($merged as $idx => $line) {
        $trimmed = trim($line);

        if ($idx === 0 && $trimmed === 'phpinfo()') {
            continue;
        }

        if ($trimmed === '' || preg_match('/^_+$/', $trimmed)) {
            $flushPara();
            continue;
        }

        if (str_contains($line, ' => ')) {
            $flushPara();
            $parts = explode(' => ', $line, 3);

            if (count($parts) === 3) {
                $c1 = trim($parts[0]);
                $c2 = trim($parts[1]);
                $c3 = trim($parts[2]);

                if ($c1 === 'Directive' && $c2 === 'Local Value') {
                    $closeTable();
                    $html[] = '<table><thead><tr><th style="width:32%">Directive</th><th style="width:34%">Local Value</th><th style="width:34%">Master Value</th></tr></thead><tbody>';
                    $openCols = 3;
                    continue;
                }

                if ($openCols !== 3) {
                    $closeTable();
                    $html[] = '<table><tbody>';
                    $openCols = 3;
                }
                $html[] = '<tr><td class="e">' . $esc($c1) . '</td><td class="v3">' . $esc($c2) . '</td><td class="v3">' . $esc($c3) . '</td></tr>';
            } else {
                $c1 = trim($parts[0]);
                $c2 = trim($parts[1] ?? '');

                if ($c1 === 'Variable' && $c2 === 'Value') {
                    $closeTable();
                    $html[] = '<table><thead><tr><th style="width:32%">Variable</th><th style="width:68%">Value</th></tr></thead><tbody>';
                    $openCols = 2;
                    continue;
                }

                if ($openCols !== 2) {
                    $closeTable();
                    $html[] = '<table><tbody>';
                    $openCols = 2;
                }
                $html[] = '<tr><td class="e">' . $esc($c1) . '</td><td class="v">' . $esc($c2) . '</td></tr>';
            }
            continue;
        }

        if (in_array($trimmed, ['Configuration', 'Additional Modules', 'Environment', 'PHP Variables', 'PHP Credits', 'PHP License'], true)) {
            $flushPara();
            $closeTable();
            $html[] = '<h1 class="sec">' . $esc($trimmed) . '</h1>';
        } elseif (strlen($trimmed) <= 42 && !str_contains($trimmed, '.') && !str_contains($trimmed, ':')) {
            $flushPara();
            $closeTable();
            $html[] = '<h2 class="ext">' . $esc($trimmed) . '</h2>';
        } else {
            $closeTable();
            $paraBuf[] = $trimmed;
        }
    }

    $flushPara();
    $closeTable();
    $html[] = '</div></body></html>';

    return implode("\n", $html);
}

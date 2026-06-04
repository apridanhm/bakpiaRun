<?php
echo "<h1>Request Handler Test</h1>";
echo "<h2>GET Parameters:</h2>";
echo "<pre>";
print_r($_GET);
echo "</pre>";

echo "<h2>POST Parameters:</h2>";
echo "<pre>";
print_r($_POST);
echo "</pre>";

echo "<h2>Cookies:</h2>";
echo "<pre>";
print_r($_COOKIE);
echo "</pre>";

echo "<h2>Headers:</h2>";
echo "<pre>";
print_r(array_filter($_SERVER, function($key) {
    return strpos($key, 'HTTP_') === 0;
}, ARRAY_FILTER_USE_KEY));
echo "</pre>";

echo "<h2>Raw Body:</h2>";
echo "<pre>" . htmlspecialchars(file_get_contents('php://input')) . "</pre>";
?>
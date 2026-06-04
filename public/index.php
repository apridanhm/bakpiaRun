<!DOCTYPE html>
<html>
<head>
    <title>bakpiaRun Test</title>
    <link rel="stylesheet" href="/style.css">
</head>
<body>
    <div class="container">
        <h1>Static File Serving Test</h1>
        <p>If you see this with purple gradient background, CSS is working!</p>
        <p>Check browser console for JavaScript message.</p>
        
        <h2>Request Info:</h2>
        <ul>
            <li>Method: <?php echo $_SERVER['REQUEST_METHOD']; ?></li>
            <li>URI: <?php echo $_SERVER['REQUEST_URI']; ?></li>
            <li>PHP Version: <?php echo phpversion(); ?></li>
        </ul>
    </div>
    <script src="/script.js"></script>
</body>
</html>
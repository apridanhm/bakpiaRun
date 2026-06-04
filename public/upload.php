<!DOCTYPE html>
<html>
<head>
    <title>File Upload Test - bakpiaRun</title>
    <link rel="stylesheet" href="/style.css">
    <style>
        .upload-form {
            margin: 20px 0;
            padding: 20px;
            background: #f8f9fa;
            border-radius: 8px;
        }
        .file-info {
            margin: 10px 0;
            padding: 10px;
            background: white;
            border-radius: 4px;
        }
        .success {
            color: #27ae60;
            font-weight: bold;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>File Upload Test</h1>
        
        <?php
        if ($_SERVER['REQUEST_METHOD'] === 'POST' && !empty($_FILES)) {
            echo '<h2 class="success">Upload Berhasil!</h2>';
            
            foreach ($_FILES as $field_name => $file_data) {
                echo '<div class="file-info">';
                echo '<h3>Field: ' . htmlspecialchars($field_name) . '</h3>';
                
                // Cek apakah ini multiple upload (array) atau single
                if (is_array($file_data['name'])) {
                    // MULTIPLE FILES
                    $count = count($file_data['name']);
                    for ($i = 0; $i < $count; $i++) {
                        if ($file_data['error'][$i] === UPLOAD_ERR_OK) {
                            $upload_dir = __DIR__ . '/uploads/';
                            if (!is_dir($upload_dir)) mkdir($upload_dir, 0755, true);
                            
                            $upload_path = $upload_dir . basename($file_data['name'][$i]);
                            
                            if (rename($file_data['tmp_name'][$i], $upload_path)) {
                                echo '<p><strong>' . htmlspecialchars($file_data['name'][$i]) . '</strong> saved!</p>';
                            } else {
                                echo '<p>Failed to save <strong>' . htmlspecialchars($file_data['name'][$i]) . '</strong></p>';
                            }
                        }
                    }
                } else {
                    // SINGLE FILE
                    if ($file_data['error'] === UPLOAD_ERR_OK) {
                        $upload_dir = __DIR__ . '/uploads/';
                        if (!is_dir($upload_dir)) mkdir($upload_dir, 0755, true);
                        
                        $upload_path = $upload_dir . basename($file_data['name']);
                        
                        if (rename($file_data['tmp_name'], $upload_path)) {
                            echo '<p><strong>' . htmlspecialchars($file_data['name']) . '</strong> saved!</p>';
                        } else {
                            echo '<p>Failed to save <strong>' . htmlspecialchars($file_data['name']) . '</strong></p>';
                        }
                    }
                }
                echo '</div>';
            }
        }
        ?>
        
        <div class="upload-form">
            <h2>Upload File</h2>
            <form method="POST" enctype="multipart/form-data">
                <div style="margin: 15px 0;">
                    <label><strong>Single File:</strong></label><br>
                    <input type="file" name="single_file" style="margin: 10px 0;">
                </div>
                
                <div style="margin: 15px 0;">
                    <label><strong>Multiple Files:</strong></label><br>
                    <input type="file" name="files[]" multiple style="margin: 10px 0;">
                </div>
                
                <div style="margin: 15px 0;">
                    <label><strong>Text Field:</strong></label><br>
                    <input type="text" name="description" placeholder="Description" style="width: 100%; padding: 8px; margin: 10px 0;">
                </div>
                
                <button type="submit" style="background: #667eea; color: white; padding: 10px 20px; border: none; border-radius: 5px; cursor: pointer;">
                    Upload Files
                </button>
            </form>
        </div>
        
        <h2>$_POST Data:</h2>
        <pre><?php print_r($_POST); ?></pre>
        
        <h2>$_FILES Data:</h2>
        <pre><?php print_r($_FILES); ?></pre>
    </div>
</body>
</html>
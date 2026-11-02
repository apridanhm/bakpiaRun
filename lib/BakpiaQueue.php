<?php
/**
 * bakpiaRun Queue Client Library
 * 
 * helper class untuk submit dan monitor background jobs
 */

class BakpiaQueue {
    private static $baseUrl = null;
    private static $adminToken = null;
    private static $timeout = 5;
    
    private static function getBaseUrl() {
        if (self::$baseUrl !== null) {
            return self::$baseUrl;
        }
        return rtrim(getenv('BAKPIARUN_BASE_URL') ?: 'http://127.0.0.1:8080', '/');
    }

    private static function getAdminToken() {
        if (self::$adminToken !== null) {
            return self::$adminToken;
        }
        return getenv('BAKPIARUN_ADMIN_TOKEN') ?: '';
    }

    /**
     * set base URL untuk queue API
     */
    public static function setBaseUrl($url) {
        self::$baseUrl = rtrim($url, '/');
    }

    /**
     * set admin token untuk autentikasi queue API
     */
    public static function setAdminToken($token) {
        self::$adminToken = $token;
    }
    
    /**
     * set timeout untuk HTTP request (detik)
     */
    public static function setTimeout($seconds) {
        self::$timeout = $seconds;
    }
    
    /**
     * submit job baru ke queue
     * 
     * @param string $task nama task (contoh: 'send_email', 'generate_report')
     * @param array $payload data yang akan diproses
     * @return array response dari server (job_id, status, message)
     * @throws Exception jika request gagal
     */
    public static function push($task, $payload = []) {
        $url = self::getBaseUrl() . '/api/queue/submit';
        
        $data = [
            'task' => $task,
            'payload' => $payload
        ];
        
        $headers = [
            'Content-Type: application/json',
            'Accept: application/json'
        ];
        $token = self::getAdminToken();
        if (!empty($token)) {
            $headers[] = 'X-Admin-Token: ' . $token;
        }

        $ch = curl_init($url);
        curl_setopt($ch, CURLOPT_POST, true);
        curl_setopt($ch, CURLOPT_POSTFIELDS, json_encode($data));
        curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
        curl_setopt($ch, CURLOPT_HTTPHEADER, $headers);
        curl_setopt($ch, CURLOPT_TIMEOUT, self::$timeout);
        
        $response = curl_exec($ch);
        $httpCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
        $error = curl_error($ch);
        curl_close($ch);
        
        if ($error) {
            throw new Exception("Queue API request failed: $error");
        }
        
        if ($httpCode !== 200) {
            throw new Exception("Queue API returned HTTP $httpCode: $response");
        }
        
        $result = json_decode($response, true);
        
        if (json_last_error() !== JSON_ERROR_NONE) {
            throw new Exception("Invalid JSON response from Queue API");
        }
        
        return $result;
    }
    
    /**
     * cek status job
     * 
     * @param string $jobId ID job yang ingin dicek
     * @return array status job (id, task, payload, status, created_at, completed_at, result)
     * @throws Exception jika job tidak ditemukan atau request gagal
     */
    public static function status($jobId) {
        $url = self::getBaseUrl() . '/api/queue/status/' . urlencode($jobId);
        
        $headers = [
            'Accept: application/json'
        ];
        $token = self::getAdminToken();
        if (!empty($token)) {
            $headers[] = 'X-Admin-Token: ' . $token;
        }

        $ch = curl_init($url);
        curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
        curl_setopt($ch, CURLOPT_HTTPHEADER, $headers);
        curl_setopt($ch, CURLOPT_TIMEOUT, self::$timeout);
        
        $response = curl_exec($ch);
        $httpCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
        $error = curl_error($ch);
        curl_close($ch);
        
        if ($error) {
            throw new Exception("Queue API request failed: $error");
        }
        
        if ($httpCode === 404) {
            throw new Exception("Job not found: $jobId");
        }
        
        if ($httpCode !== 200) {
            throw new Exception("Queue API returned HTTP $httpCode: $response");
        }
        
        $result = json_decode($response, true);
        
        if (json_last_error() !== JSON_ERROR_NONE) {
            throw new Exception("Invalid JSON response from Queue API");
        }
        
        return $result;
    }
    
    /**
     * tunggu sampai job selesai (blocking)
     * 
     * @param string $jobId ID job yang ingin ditunggu
     * @param int $maxWait maksimal waktu tunggu (detik)
     * @param int $interval interval pengecekan (detik)
     * @return array status job terakhir
     * @throws Exception jika timeout atau job gagal
     */
    public static function waitUntilComplete($jobId, $maxWait = 60, $interval = 1) {
        $startTime = time();
        
        while (true) {
            $status = self::status($jobId);
            
            if ($status['status'] === 'Completed') {
                return $status;
            }
            
            if ($status['status'] === 'Failed') {
                throw new Exception("Job failed: " . json_encode($status));
            }
            
            if (time() - $startTime > $maxWait) {
                throw new Exception("Timeout waiting for job completion");
            }
            
            sleep($interval);
        }
    }
}
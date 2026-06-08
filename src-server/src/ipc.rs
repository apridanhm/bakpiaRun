use crate::types::{PhpRequest, PhpResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use std::sync::Arc;

// Helper: Ambil koneksi (SELALU buat baru untuk sekarang)
async fn acquire_connection(
    socket_path: &str,
    _pool: &Arc<Mutex<Vec<UnixStream>>>,
) -> Result<UnixStream, String> {
    // DISABLE POOLING: Selalu buat koneksi baru
    UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("Failed to connect to PHP worker: {}", e))
}

// Helper: Kembalikan koneksi (DO NOTHING - pooling disabled)
async fn release_connection(
    _stream: UnixStream,
    _pool: &Arc<Mutex<Vec<UnixStream>>>,
    _max_size: usize,
) {
    // DISABLE POOLING: Socket otomatis di-drop
}

// Main function: Kirim request ke PHP Worker
pub async fn send_to_php_worker(
    socket_path: &str,
    conn_pool: &Arc<Mutex<Vec<UnixStream>>>,
    _pool_size: usize,
    request: PhpRequest,
) -> Result<PhpResponse, String> {
    // 1. Ambil koneksi (selalu baru)
    let mut stream = acquire_connection(socket_path, conn_pool).await?;

    // 2. Serialize request
    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    let payload_length = request_json.len() as u32;
    let length_bytes = payload_length.to_be_bytes();

    // 3. Kirim data
    stream
        .write_all(&length_bytes)
        .await
        .map_err(|e| format!("Failed to write length: {}", e))?;
    stream
        .write_all(request_json.as_bytes())
        .await
        .map_err(|e| format!("Failed to write payload: {}", e))?;

    // 4. Baca response
    let mut length_buf = [0u8; 4];
    stream
        .read_exact(&mut length_buf)
        .await
        .map_err(|e| format!("Failed to read response length: {}", e))?;

    let response_length = u32::from_be_bytes(length_buf) as usize;

    let mut response_buf = vec![0u8; response_length];
    stream
        .read_exact(&mut response_buf)
        .await
        .map_err(|e| format!("Failed to read response payload: {}", e))?;

    let response: PhpResponse = serde_json::from_slice(&response_buf)
        .map_err(|e| format!("Failed to deserialize response: {}", e))?;

    // 5. Kembalikan koneksi (do nothing - pooling disabled)
    release_connection(stream, conn_pool, _pool_size).await;

    Ok(response)
}
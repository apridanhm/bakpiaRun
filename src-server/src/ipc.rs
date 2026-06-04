use crate::types::{PhpRequest, PhpResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub async fn send_to_php_worker(
    socket_path: &str,
    request: PhpRequest,
) -> Result<PhpResponse, String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("Failed to connect to PHP worker: {}", e))?;

    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    let payload_length = request_json.len() as u32;
    let length_bytes = payload_length.to_be_bytes();

    stream
        .write_all(&length_bytes)
        .await
        .map_err(|e| format!("Failed to write length: {}", e))?;
    stream
        .write_all(request_json.as_bytes())
        .await
        .map_err(|e| format!("Failed to write payload: {}", e))?;

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

    Ok(response)
}

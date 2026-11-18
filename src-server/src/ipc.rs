use crate::types::{PhpRequest, PhpResponse};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

/// Sends a framed JSON request to a persistent PHP worker over a keep-alive Unix Domain Socket.
///
/// Cancel-safety & Self-healing:
/// - Takes ownership of the cached `UnixStream` (`guard.take()`) during I/O and only places it
///   back into the slot once the complete response frame has been read. If a timeout cancels
///   the future mid-flight or an I/O error occurs, the stream is automatically dropped/closed
///   so subsequent requests never read stale bytes.
/// - If a cached keep-alive connection was closed by the OS/worker, transparently reconnects once.
pub async fn send_to_php_worker(
    socket_path: &str,
    ipc_stream: &Arc<Mutex<Option<UnixStream>>>,
    request: PhpRequest,
) -> Result<PhpResponse, String> {
    // 1. Serialize request payload outside the IPC stream lock
    let request_json = serde_json::to_vec(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;
    let length_bytes = (request_json.len() as u32).to_be_bytes();

    // 2. Lock this worker's dedicated persistent UDS channel
    let mut guard = ipc_stream.lock().await;

    let (mut stream, was_reused) = match guard.take() {
        Some(existing) => (existing, true),
        None => {
            let fresh = UnixStream::connect(socket_path)
                .await
                .map_err(|e| format!("Failed to connect to PHP worker ({}): {}", socket_path, e))?;
            (fresh, false)
        }
    };

    // 3. Write frame (with automatic 1x reconnect if a cached keep-alive socket went stale)
    if let Err(write_err) = write_frame(&mut stream, &length_bytes, &request_json).await {
        if was_reused {
            stream = UnixStream::connect(socket_path).await.map_err(|e| {
                format!(
                    "Failed to reconnect to PHP worker ({}) after write error ({}): {}",
                    socket_path, write_err, e
                )
            })?;
            write_frame(&mut stream, &length_bytes, &request_json)
                .await
                .map_err(|e| format!("Failed to write request frame after reconnect: {}", e))?;
        } else {
            return Err(format!("Failed to write request frame: {}", write_err));
        }
    }

    // 4. Read 4-byte Big-Endian response length header (reconnect once if cached socket hit immediate EOF)
    let mut length_buf = [0u8; 4];
    if let Err(read_err) = stream.read_exact(&mut length_buf).await {
        if was_reused && read_err.kind() == std::io::ErrorKind::UnexpectedEof {
            stream = UnixStream::connect(socket_path).await.map_err(|e| {
                format!(
                    "Failed to reconnect to PHP worker ({}) after EOF: {}",
                    socket_path, e
                )
            })?;
            write_frame(&mut stream, &length_bytes, &request_json)
                .await
                .map_err(|e| format!("Failed to write request frame after EOF reconnect: {}", e))?;
            stream
                .read_exact(&mut length_buf)
                .await
                .map_err(|e| format!("Failed to read response length after reconnect: {}", e))?;
        } else {
            return Err(format!("Failed to read response length: {}", read_err));
        }
    }

    let response_length = u32::from_be_bytes(length_buf) as usize;

    // 5. Read JSON response body
    let mut response_buf = vec![0u8; response_length];
    stream
        .read_exact(&mut response_buf)
        .await
        .map_err(|e| format!("Failed to read response payload: {}", e))?;

    // 6. Return the healthy warm stream back to the slot for the next request (Keep-Alive!)
    *guard = Some(stream);
    drop(guard);

    // 7. Deserialize response outside the IPC stream lock
    let response: PhpResponse = serde_json::from_slice(&response_buf)
        .map_err(|e| format!("Failed to deserialize response: {}", e))?;

    Ok(response)
}

async fn write_frame(
    stream: &mut UnixStream,
    length_bytes: &[u8; 4],
    payload: &[u8],
) -> std::io::Result<()> {
    stream.write_all(length_bytes).await?;
    stream.write_all(payload).await?;
    Ok(())
}

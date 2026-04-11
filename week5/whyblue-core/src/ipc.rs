//! IPC protocol for communication between the whyblued daemon and whyblue-tui.
//!
//! Uses a Unix domain socket with length-prefixed JSON messages.
//! Simple and debuggable for V1.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::types::{StateSnapshot, WbTransport};

/// Default IPC socket path.
pub const DEFAULT_IPC_PATH: &str = "/tmp/whyblue.sock";

// ─── IPC Message Types ─────────────────────────────────────────────────────────

/// Request from TUI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    /// Request current system status.
    GetStatus,
    /// Force a specific transport as primary (manual override).
    ForceTransport(WbTransport),
    /// Return to automatic transport selection.
    AutoMode,
    /// Update a config value at runtime.
    SetConfig { key: String, value: String },
    /// Send a test message through the system.
    SendTestMessage { payload: String },
    /// Subscribe to real-time events (TUI keeps connection open).
    Subscribe,
}

/// Response from daemon to TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Full system status snapshot.
    Status(StateSnapshot),
    /// Acknowledgment of a command.
    Ack { message: String },
    /// Error response.
    Error { message: String },
    /// A state transition event (for subscribers).
    Event(crate::types::TransitionEvent),
}

// ─── Wire Format ───────────────────────────────────────────────────────────────
//
// Each message is:
//   [4 bytes: big-endian u32 length] [N bytes: JSON payload]

/// Write a message to an IPC stream.
pub async fn ipc_write<T: Serialize>(stream: &mut UnixStream, msg: &T) -> Result<()> {
    let json = serde_json::to_vec(msg).context("serializing IPC message")?;
    let len = json.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .context("writing IPC length")?;
    stream.write_all(&json).await.context("writing IPC payload")?;
    stream.flush().await.context("flushing IPC stream")?;
    Ok(())
}

/// Read a message from an IPC stream.
pub async fn ipc_read<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .context("reading IPC length")?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 1_000_000 {
        anyhow::bail!("IPC message too large: {} bytes", len);
    }

    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .context("reading IPC payload")?;

    serde_json::from_slice(&payload).context("deserializing IPC message")
}

// ─── IPC Server (daemon side) ──────────────────────────────────────────────────

/// Create an IPC server listener.
pub async fn create_ipc_server(path: &str) -> Result<UnixListener> {
    // Remove stale socket file
    let _ = tokio::fs::remove_file(path).await;
    let listener = UnixListener::bind(path).context("binding IPC socket")?;
    
    // Ensure the socket is accessible even if the daemon is run via sudo
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = tokio::fs::metadata(path).await.map(|m| m.permissions()) {
            perms.set_mode(0o666); // Read/write for all users
            let _ = tokio::fs::set_permissions(path, perms).await;
        }
    }

    tracing::info!(path, "IPC server listening");
    Ok(listener)
}

// ─── IPC Client (TUI side) ─────────────────────────────────────────────────────

/// Connect to the daemon's IPC socket.
pub async fn connect_ipc(path: &str) -> Result<UnixStream> {
    let stream = UnixStream::connect(path)
        .await
        .context(format!("connecting to IPC at {path}"))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ipc_roundtrip() {
        let path = "/tmp/whyblue_test.sock";
        let _ = tokio::fs::remove_file(path).await;

        let listener = create_ipc_server(path).await.unwrap();

        // Client sends a request
        let client_handle = tokio::spawn(async move {
            let mut stream = connect_ipc(path).await.unwrap();
            ipc_write(&mut stream, &IpcRequest::GetStatus).await.unwrap();

            let response: IpcResponse = ipc_read(&mut stream).await.unwrap();
            match response {
                IpcResponse::Ack { message } => assert_eq!(message, "ok"),
                _ => panic!("unexpected response"),
            }
        });

        // Server handles request
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: IpcRequest = ipc_read(&mut stream).await.unwrap();
        assert!(matches!(request, IpcRequest::GetStatus));

        ipc_write(&mut stream, &IpcResponse::Ack { message: "ok".into() })
            .await
            .unwrap();

        client_handle.await.unwrap();

        let _ = tokio::fs::remove_file(path).await;
    }
}

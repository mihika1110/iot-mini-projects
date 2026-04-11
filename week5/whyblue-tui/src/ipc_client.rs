//! IPC client for connecting to the whyblued daemon.

use anyhow::Result;
use tokio::net::UnixStream;

use whyblue_core::ipc::{self, IpcRequest, IpcResponse};
use whyblue_core::types::StateSnapshot;

/// Client that communicates with the whyblued daemon over Unix socket IPC.
pub struct IpcClient {
    stream: Option<UnixStream>,
    path: String,
}

impl IpcClient {
    pub fn new(path: String) -> Self {
        Self {
            stream: None,
            path,
        }
    }

    /// Connect to the daemon. Returns true if successful.
    pub async fn connect(&mut self) -> Result<()> {
        let stream = ipc::connect_ipc(&self.path).await?;
        self.stream = Some(stream);
        Ok(())
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Send a request and receive a response.
    async fn request(&mut self, req: &IpcRequest) -> Result<IpcResponse> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("not connected"))?;
        ipc::ipc_write(stream, req).await?;
        let resp: IpcResponse = ipc::ipc_read(stream).await?;
        Ok(resp)
    }

    /// Fetch the current status from the daemon.
    pub async fn get_status(&mut self) -> Result<StateSnapshot> {
        match self.request(&IpcRequest::GetStatus).await? {
            IpcResponse::Status(snap) => Ok(snap),
            IpcResponse::Error { message } => anyhow::bail!("daemon error: {message}"),
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Force a specific transport.
    pub async fn force_transport(
        &mut self,
        transport: whyblue_core::types::WbTransport,
    ) -> Result<String> {
        match self
            .request(&IpcRequest::ForceTransport(transport))
            .await?
        {
            IpcResponse::Ack { message } => Ok(message),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected: {other:?}"),
        }
    }

    /// Return to auto mode.
    pub async fn auto_mode(&mut self) -> Result<String> {
        match self.request(&IpcRequest::AutoMode).await? {
            IpcResponse::Ack { message } => Ok(message),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected: {other:?}"),
        }
    }

    /// Send a test message through the daemon.
    pub async fn send_test(&mut self, payload: String) -> Result<String> {
        match self
            .request(&IpcRequest::SendTestMessage { payload })
            .await?
        {
            IpcResponse::Ack { message } => Ok(message),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected: {other:?}"),
        }
    }
}

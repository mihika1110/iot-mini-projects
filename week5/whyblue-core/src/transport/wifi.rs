//! Wi-Fi UDP transport implementation.
//!
//! Wraps a tokio UdpSocket bound to wlan0 (or configured interface).
//! Provides raw send/recv and RSSI reading by parsing /proc/net/wireless.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::net::UdpSocket;

use super::Transport;
use crate::types::WbTransport;

/// Wi-Fi transport using UDP sockets.
pub struct WifiTransport {
    /// Peer address (IP:port)
    peer_addr: String,
    /// Local bind port
    local_port: u16,
    /// Network interface name (e.g., "wlan0")
    iface: String,
    /// The UDP socket (set after open)
    socket: Option<Arc<UdpSocket>>,
    /// Alive flag
    alive: Arc<AtomicBool>,
}

impl WifiTransport {
    pub fn new(peer_addr: String, local_port: u16, iface: String) -> Self {
        Self {
            peer_addr,
            local_port,
            iface,
            socket: None,
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get a clone of the socket for external use (e.g., probe tasks).
    pub fn socket(&self) -> Option<Arc<UdpSocket>> {
        self.socket.clone()
    }

    /// Read Wi-Fi RSSI from /proc/net/wireless.
    ///
    /// Format example:
    /// ```text
    /// Inter-| sta-|   Quality        |   Discarded packets               | Missed
    ///  face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon
    ///  wlan0: 0000   70.  -40.  -256        0      0      0      0      0        0
    /// ```
    async fn read_proc_wireless_rssi(&self) -> Result<i32> {
        let contents = tokio::fs::read_to_string("/proc/net/wireless").await?;

        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(&self.iface) {
                // Parse the line: "wlan0: 0000   70.  -40.  -256 ..."
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 4 {
                    // The signal level is typically the 4th field (index 3)
                    let level_str = parts[3].trim_end_matches('.');
                    let rssi: i32 = level_str
                        .parse()
                        .context("parsing RSSI from /proc/net/wireless")?;
                    return Ok(rssi);
                }
            }
        }

        anyhow::bail!("interface {} not found in /proc/net/wireless", self.iface)
    }
}

#[async_trait]
impl Transport for WifiTransport {
    async fn open(&mut self) -> Result<()> {
        let bind_addr = format!("0.0.0.0:{}", self.local_port);
        let socket = UdpSocket::bind(&bind_addr)
            .await
            .context("binding Wi-Fi UDP socket")?;

        // Connect to peer so we can use send/recv instead of send_to/recv_from
        socket
            .connect(&self.peer_addr)
            .await
            .context("connecting to Wi-Fi peer")?;

        // Attempt to bind to interface (requires CAP_NET_ADMIN)
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            let iface_bytes = self.iface.as_bytes();
            let mut ifname = [0u8; 16];
            let len = iface_bytes.len().min(15);
            ifname[..len].copy_from_slice(&iface_bytes[..len]);

            unsafe {
                let ret = libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_BINDTODEVICE,
                    ifname.as_ptr() as *const libc::c_void,
                    ifname.len() as libc::socklen_t,
                );
                if ret != 0 {
                    tracing::warn!(
                        iface = %self.iface,
                        "SO_BINDTODEVICE failed (need CAP_NET_ADMIN), continuing without interface binding"
                    );
                }
            }
        }

        tracing::info!(
            bind = %bind_addr,
            peer = %self.peer_addr,
            iface = %self.iface,
            "Wi-Fi transport opened"
        );

        self.socket = Some(Arc::new(socket));
        self.alive.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        self.socket = None;
        tracing::info!("Wi-Fi transport closed");
        Ok(())
    }

    async fn send(&self, buf: &[u8]) -> Result<usize> {
        let socket = self
            .socket
            .as_ref()
            .context("Wi-Fi socket not open")?;
        let n = socket.send(buf).await.context("Wi-Fi send")?;
        Ok(n)
    }

    async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let socket = self
            .socket
            .as_ref()
            .context("Wi-Fi socket not open")?;
        let n = socket.recv(buf).await.context("Wi-Fi recv")?;
        Ok(n)
    }

    async fn get_rssi(&self) -> Result<i32> {
        self.read_proc_wireless_rssi().await
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn transport_type(&self) -> WbTransport {
        WbTransport::Wifi
    }
}

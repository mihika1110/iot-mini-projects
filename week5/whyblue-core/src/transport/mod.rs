//! Transport trait and module declarations.
//!
//! Defines the async Transport trait that both Wi-Fi and Bluetooth
//! implementations must satisfy, plus shared transport utilities.

pub mod bluetooth;
pub mod wifi;

use anyhow::Result;
use async_trait::async_trait;

use crate::types::WbTransport;

/// Async transport interface for Wi-Fi and Bluetooth data paths.
///
/// Each transport manages its own connection lifecycle, provides
/// raw send/recv over UDP, and reports RSSI readings.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Open the transport: bind sockets, establish connections.
    async fn open(&mut self) -> Result<()>;

    /// Close the transport: unbind sockets, disconnect.
    async fn close(&mut self) -> Result<()>;

    /// Send raw bytes through this transport. Returns bytes sent.
    async fn send(&self, buf: &[u8]) -> Result<usize>;

    /// Receive raw bytes. Blocks until data arrives or timeout.
    async fn recv(&self, buf: &mut [u8]) -> Result<usize>;

    /// Get the current RSSI reading for this transport (dBm).
    async fn get_rssi(&self) -> Result<i32>;

    /// Check if this transport is currently alive and connected.
    fn is_alive(&self) -> bool;

    /// Return which transport type this is.
    fn transport_type(&self) -> WbTransport;
}

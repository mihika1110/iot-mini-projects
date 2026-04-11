//! Bluetooth PAN/BNEP transport implementation.
//!
//! Establishes a Bluetooth PAN connection using multiple methods
//! (D-Bus via bluetoothctl, bt-network, or direct bnep setup),
//! which creates a `bnep0` network interface. Then opens a UDP socket
//! bound to that interface for data exchange.
//!
//! On the "server" side (NAP role), this registers a PAN network server.
//! On the "client" side (PANU role), this connects to the server's PAN.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::net::UdpSocket;

use super::Transport;
use crate::types::WbTransport;

/// Bluetooth PAN transport using BNEP + UDP.
pub struct BluetoothTransport {
    /// Peer BT MAC address (e.g., "AA:BB:CC:DD:EE:FF")
    peer_addr: String,
    /// UDP port for data exchange over bnep0
    data_port: u16,
    /// Peer UDP address over BT PAN (IP assigned to peer on bnep0)
    peer_data_addr: String,
    /// Role: "server" (NAP) or "client" (PANU)
    role: String,
    /// BNEP interface name (typically "bnep0")
    bnep_iface: String,
    /// The UDP socket for data (set after BT PAN is up)
    socket: Option<Arc<UdpSocket>>,
    /// Whether the BT PAN is connected
    alive: Arc<AtomicBool>,
    /// Last known RSSI
    last_rssi: i32,
}

impl BluetoothTransport {
    pub fn new(peer_addr: String, data_port: u16, role: String) -> Self {
        // Default PAN IP addressing:
        // Server (NAP): 10.0.0.1
        // Client (PANU): 10.0.0.2
        let peer_data_addr = if role == "server" {
            format!("10.0.0.2:{}", data_port)
        } else {
            format!("10.0.0.1:{}", data_port)
        };

        let bnep_iface = if role == "server" {
            "br0".to_string()
        } else {
            "bnep0".to_string()
        };

        Self {
            peer_addr,
            data_port,
            peer_data_addr,
            role,
            bnep_iface,
            socket: None,
            alive: Arc::new(AtomicBool::new(false)),
            last_rssi: -100,
        }
    }

    /// Get a clone of the socket for external use (e.g., probe tasks).
    pub fn socket(&self) -> Option<Arc<UdpSocket>> {
        self.socket.clone()
    }

    /// Establish Bluetooth PAN connection, trying multiple methods.
    async fn setup_pan(&self) -> Result<()> {
        // Ensure bnep module is loaded
        run_cmd_quiet("modprobe", &["bnep"]).await.ok();

        // Step 0: Ensure the device is paired and trusted
        self.ensure_paired().await?;

        if self.role == "server" {
            self.setup_pan_server().await
        } else {
            self.setup_pan_client().await
        }
    }

    /// Ensure the BT peer is paired and trusted (idempotent).
    async fn ensure_paired(&self) -> Result<()> {
        // Check if already paired
        let output = run_cmd_quiet("bluetoothctl", &["info", &self.peer_addr]).await;
        if let Ok(info) = output {
            if info.contains("Paired: yes") && info.contains("Trusted: yes") {
                tracing::debug!(peer = %self.peer_addr, "BT peer already paired and trusted");
                return Ok(());
            }

            // Trust the device (pairing should be done once manually)
            if !info.contains("Trusted: yes") {
                tracing::info!(peer = %self.peer_addr, "Trusting BT peer");
                run_cmd_quiet("bluetoothctl", &["trust", &self.peer_addr]).await.ok();
            }
        }

        Ok(())
    }

    /// Set up as PAN NAP server.
    async fn setup_pan_server(&self) -> Result<()> {
        tracing::info!("Setting up BT PAN NAP server");

        // Create a bridge interface for NAP (ignore errors if it exists)
        run_cmd_quiet("ip", &["link", "add", "br0", "type", "bridge"]).await.ok();
        run_cmd_quiet("ip", &["link", "set", "br0", "up"]).await.ok();
        run_cmd_quiet("ip", &["addr", "add", "10.0.0.1/24", "dev", "br0"]).await.ok();

        // Method 1: Try bt-network (from bluez-tools)
        if which_exists("bt-network").await {
            tracing::info!("Cleaning up any orphaned bt-network processes...");
            run_cmd_quiet("killall", &["-q", "bt-network"]).await.ok();
            
            tracing::info!("Using bt-network for NAP server");
            let mut child = tokio::process::Command::new("bt-network")
                .args(["-s", "nap", "br0"])
                .spawn()
                .context("starting bt-network NAP server")?;

            tokio::select! {
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::error!("bt-network exited early with code {}", exit_code);
                    anyhow::bail!("bt-network failed to start properly. Check syslog or if bluetoothd has network plugin active");
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {
                    tracing::info!("BT PAN NAP server registered on br0 via bt-network");
                }
            }

            tokio::spawn(async move {
                let status = child.wait().await;
                tracing::warn!("bt-network NAP server background process exited: {:?}", status);
            });

            return Ok(());
        }

        // Method 2: Use dbus-send to register NAP via BlueZ NetworkServer1
        tracing::info!("bt-network not found, using D-Bus directly for NAP server");
        let result = run_cmd_quiet(
            "dbus-send",
            &[
                "--system",
                "--type=method_call",
                "--dest=org.bluez",
                "/org/bluez/hci0",
                "org.bluez.NetworkServer1.Register",
                "string:nap",
                "string:br0",
            ],
        )
        .await;

        match result {
            Ok(_) => {
                tracing::info!("BT PAN NAP server registered on br0 via D-Bus");
                Ok(())
            }
            Err(e) => {
                tracing::warn!("D-Bus NAP registration failed: {e}");
                anyhow::bail!(
                    "Could not set up BT PAN server. Install bluez-tools (apt install bluez-tools) or ensure bluetoothd supports NAP"
                )
            }
        }
    }

    /// Set up as PAN PANU client — connect to the server's NAP.
    async fn setup_pan_client(&self) -> Result<()> {
        tracing::info!(peer = %self.peer_addr, "Connecting BT PAN client to NAP");

        // Force a basic Bluetooth connection first to refresh the SDP cache
        // If we don't do this, BlueZ might use stale cached profiles and say "Network1 doesn't exist"
        tracing::info!("Refreshing BlueZ SDP profiles via bluetoothctl connect...");
        run_cmd_quiet("bluetoothctl", &["connect", &self.peer_addr]).await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // Method 1: Try D-Bus via dbus-send (most reliable on standard BlueZ)
        let dbus_path = mac_to_dbus_path(&self.peer_addr);
        let dbus_result = run_cmd_quiet(
            "dbus-send",
            &[
                "--system",
                "--type=method_call",
                "--print-reply",
                "--dest=org.bluez",
                &dbus_path,
                "org.bluez.Network1.Connect",
                "string:nap",
            ],
        )
        .await;

        match dbus_result {
            Ok(output) => {
                tracing::info!(output = %output.trim(), "BT PAN connected via D-Bus");
                // The output contains the interface name, e.g. "bnep0"
                if output.contains("bnep") {
                    // Extract interface name from D-Bus reply
                    for word in output.split_whitespace() {
                        if word.contains("bnep") {
                            let iface = word.trim_matches('"');
                            tracing::info!(iface, "BNEP interface from D-Bus");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("D-Bus PAN connect failed: {e}, trying bt-network fallback");

                // Method 2: Try bt-network (from bluez-tools)
                if which_exists("bt-network").await {
                    let output = tokio::process::Command::new("bt-network")
                        .args(["-c", &self.peer_addr, "nap"])
                        .output()
                        .await
                        .context("bt-network PANU connect")?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!("bt-network also failed: {stderr}");
                    }
                }
            }
        }

        // Wait for bnep interface to appear
        tracing::info!("Waiting for {} interface...", self.bnep_iface);
        let mut appeared = false;
        for i in 0..20 {
            if iface_exists(&self.bnep_iface).await {
                appeared = true;
                tracing::info!(
                    iface = %self.bnep_iface,
                    wait_ms = i * 500,
                    "BNEP interface appeared"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if !appeared {
            anyhow::bail!(
                "{} interface did not appear. Ensure peer {} is running as PAN NAP server \
                 and this device is paired with it. Try: bluetoothctl connect {}",
                self.bnep_iface,
                self.peer_addr,
                self.peer_addr
            );
        }

        // Assign IP to the BNEP interface
        let local_ip = if self.role == "server" {
            "10.0.0.1/24"
        } else {
            "10.0.0.2/24"
        };
        run_cmd_quiet("ip", &["addr", "add", local_ip, "dev", &self.bnep_iface])
            .await
            .ok(); // ok() — might already be assigned
        run_cmd_quiet("ip", &["link", "set", &self.bnep_iface, "up"]).await?;

        tracing::info!(
            iface = %self.bnep_iface,
            ip = local_ip,
            "BT PAN client connected"
        );
        Ok(())
    }

    /// Read BT RSSI via hcitool (best for classic BT connections) or bluetoothctl.
    async fn read_bt_rssi(&self) -> Result<i32> {
        // Method 1: hcitool rssi (works for active ACL connections)
        if let Ok(output) = run_cmd_quiet("hcitool", &["rssi", &self.peer_addr]).await {
            // Output: "RSSI return value: -42"
            if let Some(val) = output.split(':').last() {
                if let Ok(rssi) = val.trim().parse::<i32>() {
                    return Ok(rssi);
                }
            }
        }

        // Method 2: bluetoothctl info (RSSI field)
        if let Ok(info) = run_cmd_quiet("bluetoothctl", &["info", &self.peer_addr]).await {
            for line in info.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("RSSI:") {
                    // Format varies:
                    //   "RSSI: 0xffffffd6 (-42)"  -> extract from parens
                    //   "RSSI: -42"                -> direct parse
                    if let Some(paren_start) = trimmed.find('(') {
                        let inside = &trimmed[paren_start + 1..];
                        if let Some(paren_end) = inside.find(')') {
                            if let Ok(rssi) = inside[..paren_end].trim().parse::<i32>() {
                                return Ok(rssi);
                            }
                        }
                    } else if let Some(val) = trimmed.split(':').nth(1) {
                        if let Ok(rssi) = val.trim().parse::<i32>() {
                            return Ok(rssi);
                        }
                    }
                }
            }
        }

        // Method 3: Try reading from D-Bus property directly
        let dbus_path = mac_to_dbus_path(&self.peer_addr);
        if let Ok(output) = run_cmd_quiet(
            "dbus-send",
            &[
                "--system",
                "--print-reply",
                "--dest=org.bluez",
                &dbus_path,
                "org.freedesktop.DBus.Properties.Get",
                "string:org.bluez.Device1",
                "string:RSSI",
            ],
        )
        .await
        {
            // Parse the int16 value from D-Bus reply
            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("int16") || trimmed.starts_with("variant") {
                    for word in trimmed.split_whitespace() {
                        if let Ok(rssi) = word.parse::<i32>() {
                            return Ok(rssi);
                        }
                    }
                }
            }
        }

        anyhow::bail!("could not read BT RSSI for {}", self.peer_addr)
    }
}

#[async_trait]
impl Transport for BluetoothTransport {
    async fn open(&mut self) -> Result<()> {
        // Step 1: Set up the Bluetooth PAN connection
        self.setup_pan().await?;

        // Step 2: Open UDP socket on the BNEP interface
        let bind_addr = format!("0.0.0.0:{}", self.data_port);
        let socket = UdpSocket::bind(&bind_addr)
            .await
            .context("binding BT UDP socket")?;

        socket
            .connect(&self.peer_data_addr)
            .await
            .context("connecting to BT peer")?;

        // Bind to bnep interface
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            let iface_bytes = self.bnep_iface.as_bytes();
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
                        "SO_BINDTODEVICE to {} failed (need CAP_NET_ADMIN)",
                        self.bnep_iface
                    );
                }
            }
        }

        tracing::info!(
            bind = %bind_addr,
            peer = %self.peer_data_addr,
            iface = %self.bnep_iface,
            "Bluetooth PAN transport opened"
        );

        self.socket = Some(Arc::new(socket));
        self.alive.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        self.socket = None;

        // Disconnect BT PAN via D-Bus
        if self.role == "client" {
            let dbus_path = mac_to_dbus_path(&self.peer_addr);
            run_cmd_quiet(
                "dbus-send",
                &[
                    "--system",
                    "--type=method_call",
                    "--dest=org.bluez",
                    &dbus_path,
                    "org.bluez.Network1.Disconnect",
                ],
            )
            .await
            .ok();
        }

        tracing::info!("Bluetooth transport closed");
        Ok(())
    }

    async fn send(&self, buf: &[u8]) -> Result<usize> {
        let socket = self
            .socket
            .as_ref()
            .context("BT socket not open")?;
        let n = socket.send(buf).await.context("BT send")?;
        Ok(n)
    }

    async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let socket = self
            .socket
            .as_ref()
            .context("BT socket not open")?;
        let n = socket.recv(buf).await.context("BT recv")?;
        Ok(n)
    }

    async fn get_rssi(&self) -> Result<i32> {
        self.read_bt_rssi().await
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn transport_type(&self) -> WbTransport {
        WbTransport::Bluetooth
    }
}

// ─── Utility Functions ─────────────────────────────────────────────────────────

/// Run a system command, returning stdout. Does NOT bail on stderr noise.
async fn run_cmd_quiet(cmd: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .context(format!("running {cmd}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{} {} failed: {}", cmd, args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Check if a network interface exists.
async fn iface_exists(name: &str) -> bool {
    let path = format!("/sys/class/net/{}", name);
    tokio::fs::metadata(&path).await.is_ok()
}

/// Check if a command is available on PATH.
async fn which_exists(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Convert a BT MAC address to a BlueZ D-Bus object path.
/// "DC:A6:32:4A:B0:11" → "/org/bluez/hci0/dev_DC_A6_32_4A_B0_11"
fn mac_to_dbus_path(mac: &str) -> String {
    let escaped = mac.replace(':', "_");
    format!("/org/bluez/hci0/dev_{}", escaped)
}
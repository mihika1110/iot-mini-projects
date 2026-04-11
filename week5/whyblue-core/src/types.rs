//! Core types for the WhyBlue dual-transport networking system.
//!
//! All shared enums, structs, and constants used across the daemon,
//! transport layer, FSM, and TUI dashboard.

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Transport Identifier ──────────────────────────────────────────────────────

/// Identifies which transport is active or preferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WbTransport {
    None,
    Bluetooth,
    Wifi,
}

impl std::fmt::Display for WbTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WbTransport::None => write!(f, "None"),
            WbTransport::Bluetooth => write!(f, "Bluetooth"),
            WbTransport::Wifi => write!(f, "Wi-Fi"),
        }
    }
}

// ─── System States (FSM) ───────────────────────────────────────────────────────

/// Finite state machine states for the WhyBlue system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WbState {
    /// Initial startup, no transports active
    Init,
    /// Scanning for peer devices on BT and Wi-Fi
    Discovering,
    /// Bluetooth is the sole active transport
    BtOnly,
    /// Wi-Fi is the sole active transport
    WifiOnly,
    /// Both transports are connected and ready
    DualReady,
    /// Transitioning primary from BT → Wi-Fi
    HandoverBtToWifi,
    /// Transitioning primary from Wi-Fi → BT
    HandoverWifiToBt,
    /// Both transports are degraded or failing
    Degraded,
    /// Recovering from a degraded state
    Recovery,
}

impl std::fmt::Display for WbState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WbState::Init => write!(f, "INIT"),
            WbState::Discovering => write!(f, "DISCOVERING"),
            WbState::BtOnly => write!(f, "BT_ONLY"),
            WbState::WifiOnly => write!(f, "WIFI_ONLY"),
            WbState::DualReady => write!(f, "DUAL_READY"),
            WbState::HandoverBtToWifi => write!(f, "HANDOVER_BT→WIFI"),
            WbState::HandoverWifiToBt => write!(f, "HANDOVER_WIFI→BT"),
            WbState::Degraded => write!(f, "DEGRADED"),
            WbState::Recovery => write!(f, "RECOVERY"),
        }
    }
}

// ─── Proximity Classification ──────────────────────────────────────────────────

/// Distance classification based on RSSI with hysteresis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Proximity {
    /// BT RSSI consistently strong → Bluetooth preferred
    Near,
    /// Intermediate range → keep current link
    Mid,
    /// BT RSSI weak or absent → Wi-Fi preferred
    Far,
    /// Insufficient data to classify
    Unknown,
}

impl std::fmt::Display for Proximity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Proximity::Near => write!(f, "NEAR"),
            Proximity::Mid => write!(f, "MID"),
            Proximity::Far => write!(f, "FAR"),
            Proximity::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

// ─── Traffic Classes ───────────────────────────────────────────────────────────

/// Traffic classification for routing policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficClass {
    /// Control plane: handover signals, probes — duplicated on both during handover
    Control,
    /// Interactive: low-latency preferred (commands, small requests)
    Interactive,
    /// Streaming: stability preferred (audio, video, telemetry)
    Stream,
    /// Bulk: throughput preferred — always Wi-Fi
    Bulk,
}

// ─── Link Metrics ──────────────────────────────────────────────────────────────

/// Per-transport link quality metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkMetrics {
    /// Round-trip time in milliseconds (EWMA)
    pub rtt_ms: f64,
    /// Packet loss percentage (windowed, 0.0–100.0)
    pub loss_pct: f64,
    /// Estimated throughput in kbps (windowed)
    pub throughput_kbps: f64,
    /// RTT jitter in milliseconds (variance of recent RTTs)
    pub jitter_ms: f64,
    /// Most recent RSSI reading in dBm
    pub rssi_dbm: i32,
    /// Derived stability score (0.0–1.0, higher = more stable)
    pub stability_score: f64,
    /// Derived cost score (0.0–1.0, lower = better value)
    pub cost_score: f64,
    /// Whether this transport is currently reachable
    pub alive: bool,
    /// Milliseconds since the last successful packet exchange
    pub last_success_ms: u64,
    /// Number of reconnections since startup
    pub reconnect_count: u32,
}

impl Default for LinkMetrics {
    fn default() -> Self {
        Self {
            rtt_ms: 0.0,
            loss_pct: 100.0,
            throughput_kbps: 0.0,
            jitter_ms: 0.0,
            rssi_dbm: -100,
            stability_score: 0.0,
            cost_score: 1.0,
            alive: false,
            last_success_ms: u64::MAX,
            reconnect_count: 0,
        }
    }
}

// ─── Decision Types ────────────────────────────────────────────────────────────

/// Input snapshot for the FSM transition evaluator.
pub struct DecisionInput {
    pub bt: LinkMetrics,
    pub wifi: LinkMetrics,
    pub proximity: Proximity,
    pub proximity_confidence: f64,
    pub bt_available: bool,
    pub wifi_available: bool,
    pub current_primary: WbTransport,
    pub current_state: WbState,
    pub now: Instant,
    pub last_switch: Instant,
    pub state_entered: Instant,
    pub traffic_class_hint: TrafficClass,
}

/// Output from the FSM transition evaluator.
#[derive(Debug, Clone)]
pub struct Decision {
    pub next_state: WbState,
    pub preferred_primary: WbTransport,
    pub start_handover: bool,
    pub keep_secondary_alive: bool,
    pub reason: String,
}

// ─── Handover Protocol ─────────────────────────────────────────────────────────

/// Handover control message types exchanged between peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandoverMsg {
    Hello { session_id: u32 },
    HelloAck { session_id: u32 },
    SwitchPrepare { generation: u32, new_transport: WbTransport },
    SwitchAck { generation: u32 },
    PrimaryOn { generation: u32 },
    PrimaryConfirm { generation: u32 },
}

// ─── Configuration ─────────────────────────────────────────────────────────────

/// Runtime-tunable configuration for the WhyBlue system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WbConfig {
    // ── Proximity thresholds ──
    /// BT RSSI above this → candidate for NEAR (dBm)
    pub bt_rssi_near_threshold: i32,
    /// BT RSSI below this → candidate for FAR (dBm)
    pub bt_rssi_far_threshold: i32,
    /// Wi-Fi RSSI below this → FAR_WIFI_WEAK (dBm)
    pub wifi_rssi_weak_threshold: i32,

    // ── Hysteresis ──
    /// Number of consecutive samples required to change proximity band
    pub hysteresis_samples: u32,
    /// Duration in ms that RSSI must sustain to trigger band change
    pub hysteresis_duration_ms: u64,

    // ── Switching ──
    /// Minimum ms between transport switches (cooldown)
    pub switch_cooldown_ms: u64,
    /// Minimum ms to stay on a transport after switching (dwell)
    pub dwell_time_ms: u64,
    /// Duration of overlapping transmissions during handover
    pub handover_overlap_ms: u64,

    // ── Probing ──
    /// Interval between metric probe packets (ms)
    pub probe_interval_ms: u64,

    // ── Scoring weights ──
    /// Weight for latency in link scoring
    pub w_latency: f64,
    /// Weight for packet loss in link scoring
    pub w_loss: f64,
    /// Weight for stability in link scoring
    pub w_stability: f64,
    /// Weight for energy cost in link scoring
    pub w_energy: f64,
    /// Weight for proximity bonus in link scoring
    pub w_proximity: f64,

    // ── Thresholds for transport quality ──
    /// BT considered "bad" if score below this
    pub bt_bad_threshold: f64,
    /// Transport considered "good" if score above this
    pub good_threshold: f64,
    /// Duration (ms) transport must be bad before triggering handover
    pub bad_duration_ms: u64,
    /// Duration (ms) transport must be good before qualifying as alternative
    pub good_duration_ms: u64,

    // ── Network ──
    /// Wi-Fi peer IP address
    pub wifi_peer_addr: String,
    /// Wi-Fi data port
    pub wifi_port: u16,
    /// BT peer MAC address
    pub bt_peer_addr: String,
    /// BT data port (used after BNEP is up)
    pub bt_port: u16,
    /// Wi-Fi interface name
    pub wifi_iface: String,
    /// Role: "server" or "client" (for BT PAN NAP vs PANU)
    pub role: String,
    /// Unix socket path for IPC
    pub ipc_socket_path: String,
}

impl Default for WbConfig {
    fn default() -> Self {
        Self {
            bt_rssi_near_threshold: -65,
            bt_rssi_far_threshold: -78,
            wifi_rssi_weak_threshold: -75,
            hysteresis_samples: 5,
            hysteresis_duration_ms: 3000,
            switch_cooldown_ms: 5000,
            dwell_time_ms: 3000,
            handover_overlap_ms: 2000,
            probe_interval_ms: 500,
            w_latency: 0.30,
            w_loss: 0.25,
            w_stability: 0.25,
            w_energy: 0.10,
            w_proximity: 0.10,
            bt_bad_threshold: 0.35,
            good_threshold: 0.60,
            bad_duration_ms: 2000,
            good_duration_ms: 1000,
            bt_port: 9877,
            wifi_peer_addr: "192.168.4.2".to_string(),
            wifi_port: 9876,
            bt_peer_addr: "".to_string(),
            wifi_iface: "wlan0".to_string(),
            role: "client".to_string(),
            ipc_socket_path: "/tmp/whyblue.sock".to_string(),
        }
    }
}

// ─── State Snapshot (for IPC) ──────────────────────────────────────────────────

/// Serializable snapshot of the full system state, sent to the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub state: WbState,
    pub active_transport: WbTransport,
    pub standby_transport: WbTransport,
    pub bt_metrics: LinkMetrics,
    pub wifi_metrics: LinkMetrics,
    pub proximity: Proximity,
    pub proximity_confidence: f64,
    pub session_id: u32,
    pub handover_generation: u32,
    pub uptime_secs: u64,
    pub total_tx_bytes: u64,
    pub total_rx_bytes: u64,
    pub handover_count: u32,
    pub chat_log: Vec<(String, String)>,
    pub last_transition: Option<TransitionEvent>,
}

/// A recorded FSM transition event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionEvent {
    pub timestamp: String,
    pub from_state: WbState,
    pub to_state: WbState,
    pub reason: String,
}

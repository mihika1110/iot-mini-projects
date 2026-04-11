//! App state and event handling for the WhyBlue TUI dashboard.

use std::collections::VecDeque;

use whyblue_core::types::{
    LinkMetrics, Proximity, StateSnapshot, TransitionEvent, WbState, WbTransport,
};

use crate::ipc_client::IpcClient;

/// Maximum number of sparkline data points to retain.
const SPARKLINE_LEN: usize = 60;

/// Maximum transition log entries to show.
const MAX_LOG_ENTRIES: usize = 50;

/// Active panel in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    Dashboard,
    TransportDetail,
    StateView,
    Chat,
    Control,
}

impl ActivePanel {
    pub fn next(self) -> Self {
        match self {
            Self::Dashboard => Self::TransportDetail,
            Self::TransportDetail => Self::StateView,
            Self::StateView => Self::Chat,
            Self::Chat => Self::Control,
            Self::Control => Self::Dashboard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::TransportDetail => "Transport",
            Self::StateView => "FSM State",
            Self::Chat => "Stream Log",
            Self::Control => "Control",
        }
    }
}

/// Main application state.
pub struct App {
    /// IPC client for daemon communication
    pub ipc: IpcClient,
    /// Latest snapshot from daemon
    pub snapshot: StateSnapshot,
    /// Whether IPC is connected
    pub connected: bool,
    /// Currently active panel
    pub panel: ActivePanel,
    /// BT RTT sparkline data
    pub bt_rtt_history: VecDeque<u64>,
    /// Wi-Fi RTT sparkline data
    pub wifi_rtt_history: VecDeque<u64>,
    /// BT RSSI history
    pub bt_rssi_history: VecDeque<u64>,
    /// Wi-Fi RSSI history
    pub wifi_rssi_history: VecDeque<u64>,
    /// Transition event log
    pub transition_log: VecDeque<TransitionEvent>,
    /// Status bar message
    pub status_msg: String,
    /// Should quit
    pub should_quit: bool,
    /// Command input buffer
    pub command_input: String,
    /// Command input active
    pub command_active: bool,
    /// Tick count (for animations)
    pub tick: u64,
}

impl App {
    pub fn new(ipc_path: String) -> Self {
        Self {
            ipc: IpcClient::new(ipc_path),
            snapshot: default_snapshot(),
            connected: false,
            panel: ActivePanel::Dashboard,
            bt_rtt_history: VecDeque::with_capacity(SPARKLINE_LEN),
            wifi_rtt_history: VecDeque::with_capacity(SPARKLINE_LEN),
            bt_rssi_history: VecDeque::with_capacity(SPARKLINE_LEN),
            wifi_rssi_history: VecDeque::with_capacity(SPARKLINE_LEN),
            transition_log: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            status_msg: "Connecting to daemon...".into(),
            should_quit: false,
            command_input: String::new(),
            command_active: false,
            tick: 0,
        }
    }

    /// Try to connect/reconnect to the daemon.
    pub async fn try_connect(&mut self) {
        match self.ipc.connect().await {
            Ok(()) => {
                self.connected = true;
                self.status_msg = "Connected to whyblued".into();
            }
            Err(e) => {
                self.connected = false;
                self.status_msg = format!("Connection failed: {e}");
            }
        }
    }

    /// Poll daemon for latest state.
    pub async fn poll_status(&mut self) {
        if !self.connected {
            return;
        }

        match self.ipc.get_status().await {
            Ok(snap) => {
                // Update sparkline histories
                push_bounded(
                    &mut self.bt_rtt_history,
                    snap.bt_metrics.rtt_ms.max(0.0) as u64,
                    SPARKLINE_LEN,
                );
                push_bounded(
                    &mut self.wifi_rtt_history,
                    snap.wifi_metrics.rtt_ms.max(0.0) as u64,
                    SPARKLINE_LEN,
                );
                push_bounded(
                    &mut self.bt_rssi_history,
                    (snap.bt_metrics.rssi_dbm + 100).max(0) as u64, // Normalize to 0-100
                    SPARKLINE_LEN,
                );
                push_bounded(
                    &mut self.wifi_rssi_history,
                    (snap.wifi_metrics.rssi_dbm + 100).max(0) as u64,
                    SPARKLINE_LEN,
                );

                // Check for new transition events
                if let Some(ref event) = snap.last_transition {
                    let is_new = self
                        .transition_log
                        .back()
                        .map_or(true, |last| last.timestamp != event.timestamp || last.to_state != event.to_state);
                    if is_new {
                        push_bounded_item(
                            &mut self.transition_log,
                            event.clone(),
                            MAX_LOG_ENTRIES,
                        );
                    }
                }

                self.snapshot = snap;
                self.status_msg = "Connected ●".into();
            }
            Err(e) => {
                self.connected = false;
                self.status_msg = format!("Lost connection: {e}");
            }
        }
    }

    /// Handle a keyboard event.
    pub async fn handle_key(&mut self, key: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;

        if self.command_active {
            match key {
                KeyCode::Esc => {
                    self.command_active = false;
                    self.command_input.clear();
                }
                KeyCode::Enter => {
                    let cmd = self.command_input.clone();
                    self.command_input.clear();
                    self.command_active = false;
                    self.execute_command(&cmd).await;
                }
                KeyCode::Backspace => {
                    self.command_input.pop();
                }
                KeyCode::Char(c) => {
                    self.command_input.push(c);
                }
                _ => {}
            }
            return;
        }

        match key {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Tab => self.panel = self.panel.next(),
            KeyCode::F(1) => {
                if self.connected {
                    match self.ipc.force_transport(WbTransport::Bluetooth).await {
                        Ok(msg) => self.status_msg = msg,
                        Err(e) => self.status_msg = format!("Error: {e}"),
                    }
                }
            }
            KeyCode::F(2) => {
                if self.connected {
                    match self.ipc.force_transport(WbTransport::Wifi).await {
                        Ok(msg) => self.status_msg = msg,
                        Err(e) => self.status_msg = format!("Error: {e}"),
                    }
                }
            }
            KeyCode::F(3) => {
                if self.connected {
                    match self.ipc.auto_mode().await {
                        Ok(msg) => self.status_msg = msg,
                        Err(e) => self.status_msg = format!("Error: {e}"),
                    }
                }
            }
            KeyCode::Char(':') => {
                self.command_active = true;
                self.command_input.clear();
            }
            _ => {}
        }
    }

    /// Execute a typed command.
    async fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        match parts[0] {
            "test" => {
                let payload = parts[1..].join(" ");
                if self.connected {
                    match self.ipc.send_test(payload).await {
                        Ok(msg) => self.status_msg = msg,
                        Err(e) => self.status_msg = format!("Error: {e}"),
                    }
                }
            }
            "force" if parts.len() > 1 => match parts[1] {
                "bt" | "bluetooth" => {
                    if self.connected {
                        match self.ipc.force_transport(WbTransport::Bluetooth).await {
                            Ok(msg) => self.status_msg = msg,
                            Err(e) => self.status_msg = format!("Error: {e}"),
                        }
                    }
                }
                "wifi" | "wi-fi" => {
                    if self.connected {
                        match self.ipc.force_transport(WbTransport::Wifi).await {
                            Ok(msg) => self.status_msg = msg,
                            Err(e) => self.status_msg = format!("Error: {e}"),
                        }
                    }
                }
                _ => self.status_msg = "Usage: force bt|wifi".into(),
            },
            "auto" => {
                if self.connected {
                    match self.ipc.auto_mode().await {
                        Ok(msg) => self.status_msg = msg,
                        Err(e) => self.status_msg = format!("Error: {e}"),
                    }
                }
            }
            "reconnect" => {
                self.try_connect().await;
            }
            _ => {
                self.status_msg = format!("Unknown command: {}", parts[0]);
            }
        }
    }

    /// Increment tick counter for animations.
    pub fn tick(&mut self) {
        self.tick += 1;
    }
}

fn push_bounded<T>(deque: &mut VecDeque<T>, item: T, max: usize) {
    deque.push_back(item);
    while deque.len() > max {
        deque.pop_front();
    }
}

fn push_bounded_item<T>(deque: &mut VecDeque<T>, item: T, max: usize) {
    deque.push_back(item);
    while deque.len() > max {
        deque.pop_front();
    }
}

fn default_snapshot() -> StateSnapshot {
    StateSnapshot {
        state: WbState::Init,
        active_transport: WbTransport::None,
        standby_transport: WbTransport::None,
        bt_metrics: LinkMetrics::default(),
        wifi_metrics: LinkMetrics::default(),
        proximity: Proximity::Unknown,
        proximity_confidence: 0.0,
        session_id: 0,
        handover_generation: 0,
        uptime_secs: 0,
        total_tx_bytes: 0,
        total_rx_bytes: 0,
        handover_count: 0,
        chat_log: Vec::new(),
        last_transition: None,
    }
}

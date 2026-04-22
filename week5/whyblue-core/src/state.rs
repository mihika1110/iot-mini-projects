//! State manager — single source of truth for the WhyBlue system.
//!
//! Owns the current FSM state, active/standby transports, session tracking,
//! handover generation, and broadcasts state change events.

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

use crate::types::{
    LinkMetrics, Proximity, StateSnapshot, TransitionEvent, WbState, WbTransport,
};

/// Internal state data behind the RwLock.
#[derive(Debug)]
struct StateInner {
    state: WbState,
    active_transport: WbTransport,
    standby_transport: WbTransport,
    bt_metrics: LinkMetrics,
    wifi_metrics: LinkMetrics,
    proximity: Proximity,
    proximity_confidence: f64,
    session_id: u32,
    handover_generation: u32,
    start_time: Instant,
    last_switch_time: Instant,
    state_entered_at: Instant,
    peer_present: bool,
    total_tx_bytes: u64,
    total_rx_bytes: u64,
    handover_count: u32,
    /// True when the last transport change was initiated by the peer (prevents echo)
    peer_initiated_switch: bool,
    chat_log: Vec<(String, String)>,
    transition_log: Vec<TransitionEvent>,
}

/// Thread-safe state manager with event broadcasting.
#[derive(Clone)]
pub struct StateManager {
    inner: Arc<RwLock<StateInner>>,
    event_tx: broadcast::Sender<TransitionEvent>,
}

impl StateManager {
    /// Create a new state manager in the Init state.
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(RwLock::new(StateInner {
                state: WbState::Init,
                active_transport: WbTransport::None,
                standby_transport: WbTransport::None,
                bt_metrics: LinkMetrics::default(),
                wifi_metrics: LinkMetrics::default(),
                proximity: Proximity::Unknown,
                proximity_confidence: 0.0,
                session_id: rand_session_id(),
                handover_generation: 0,
                start_time: Instant::now(),
                last_switch_time: Instant::now(),
                state_entered_at: Instant::now(),
                peer_present: false,
                total_tx_bytes: 0,
                total_rx_bytes: 0,
                handover_count: 0,
                peer_initiated_switch: false,
                chat_log: Vec::new(),
                transition_log: Vec::new(),
            })),
            event_tx,
        }
    }

    /// Transition to a new FSM state with a reason string.
    pub async fn transition(&self, next_state: WbState, reason: &str) {
        let mut inner = self.inner.write().await;
        let from = inner.state;
        if from == next_state {
            return; // No-op
        }

        let event = TransitionEvent {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            from_state: from,
            to_state: next_state,
            reason: reason.to_string(),
        };

        tracing::info!(from = %from, to = %next_state, reason, "FSM transition");

        inner.state = next_state;
        inner.state_entered_at = Instant::now();
        inner.transition_log.push(event.clone());

        // Keep only last 100 events
        if inner.transition_log.len() > 100 {
            let excess = inner.transition_log.len() - 100;
            inner.transition_log.drain(..excess);
        }

        // Count handovers
        if matches!(
            next_state,
            WbState::HandoverBtToWifi | WbState::HandoverWifiToBt
        ) {
            inner.handover_count += 1;
        }

        let _ = self.event_tx.send(event);
    }

    /// Set the active primary transport.
    pub async fn set_active(&self, transport: WbTransport) {
        let mut inner = self.inner.write().await;
        inner.active_transport = transport;
        inner.last_switch_time = Instant::now();
    }

    /// Set the standby transport.
    pub async fn set_standby(&self, transport: WbTransport) {
        let mut inner = self.inner.write().await;
        inner.standby_transport = transport;
    }

    /// Increment the handover generation number.
    pub async fn increment_generation(&self) -> u32 {
        let mut inner = self.inner.write().await;
        inner.handover_generation += 1;
        inner.handover_generation
    }

    /// Update cached metrics for a transport.
    pub async fn update_metrics(&self, transport: WbTransport, metrics: LinkMetrics) {
        let mut inner = self.inner.write().await;
        match transport {
            WbTransport::Bluetooth => inner.bt_metrics = metrics,
            WbTransport::Wifi => inner.wifi_metrics = metrics,
            WbTransport::None => {}
        }
    }

    /// Update the proximity classification.
    pub async fn update_proximity(&self, proximity: Proximity, confidence: f64) {
        let mut inner = self.inner.write().await;
        inner.proximity = proximity;
        inner.proximity_confidence = confidence;
    }

    /// Set peer presence flag.
    pub async fn set_peer_present(&self, present: bool) {
        let mut inner = self.inner.write().await;
        inner.peer_present = present;
    }

    /// Mark that the last transport switch was initiated by the peer.
    pub async fn set_peer_initiated_switch(&self, val: bool) {
        let mut inner = self.inner.write().await;
        inner.peer_initiated_switch = val;
    }

    /// Check if the last transport switch was peer-initiated.
    pub async fn is_peer_initiated_switch(&self) -> bool {
        self.inner.read().await.peer_initiated_switch
    }

    /// Add to TX byte counter.
    pub async fn add_tx_bytes(&self, bytes: u64) {
        let mut inner = self.inner.write().await;
        inner.total_tx_bytes += bytes;
    }

    /// Add to RX byte counter.
    pub async fn add_rx_bytes(&self, bytes: u64) {
        let mut inner = self.inner.write().await;
        inner.total_rx_bytes += bytes;
    }

    /// Add a message to the chat log.
    pub async fn push_chat_message(&self, sender: String, msg: String) {
        let mut inner = self.inner.write().await;
        inner.chat_log.push((sender, msg));
        // Keep the last 100 messages
        if inner.chat_log.len() > 100 {
            let excess = inner.chat_log.len() - 100;
            inner.chat_log.drain(..excess);
        }
    }

    /// Get a full serializable snapshot for IPC/TUI.
    pub async fn snapshot(&self) -> StateSnapshot {
        let inner = self.inner.read().await;
        StateSnapshot {
            state: inner.state,
            active_transport: inner.active_transport,
            standby_transport: inner.standby_transport,
            bt_metrics: inner.bt_metrics.clone(),
            wifi_metrics: inner.wifi_metrics.clone(),
            proximity: inner.proximity,
            proximity_confidence: inner.proximity_confidence,
            session_id: inner.session_id,
            handover_generation: inner.handover_generation,
            uptime_secs: inner.start_time.elapsed().as_secs(),
            total_tx_bytes: inner.total_tx_bytes,
            total_rx_bytes: inner.total_rx_bytes,
            handover_count: inner.handover_count,
            chat_log: inner.chat_log.clone(),
            last_transition: inner.transition_log.last().cloned(),
        }
    }

    /// Get the current FSM state.
    pub async fn current_state(&self) -> WbState {
        self.inner.read().await.state
    }

    /// Get when the current state was entered.
    pub async fn state_entered_at(&self) -> Instant {
        self.inner.read().await.state_entered_at
    }

    /// Get the active transport.
    pub async fn active_transport(&self) -> WbTransport {
        self.inner.read().await.active_transport
    }

    /// Get the time of the last transport switch.
    pub async fn last_switch_time(&self) -> Instant {
        self.inner.read().await.last_switch_time
    }

    /// Get the session ID.
    pub async fn session_id(&self) -> u32 {
        self.inner.read().await.session_id
    }

    /// Get the handover generation.
    pub async fn generation(&self) -> u32 {
        self.inner.read().await.handover_generation
    }

    /// Subscribe to state change events.
    pub fn subscribe(&self) -> broadcast::Receiver<TransitionEvent> {
        self.event_tx.subscribe()
    }

    /// Get the full transition log.
    pub async fn transition_log(&self) -> Vec<TransitionEvent> {
        self.inner.read().await.transition_log.clone()
    }
}

/// Generate a random session ID.
fn rand_session_id() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    Instant::now().hash(&mut h);
    std::process::id().hash(&mut h);
    h.finish() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_initial_state() {
        let sm = StateManager::new();
        assert_eq!(sm.current_state().await, WbState::Init);
        assert_eq!(sm.active_transport().await, WbTransport::None);
    }

    #[tokio::test]
    async fn test_transition() {
        let sm = StateManager::new();
        let mut rx = sm.subscribe();

        sm.transition(WbState::Discovering, "startup").await;
        assert_eq!(sm.current_state().await, WbState::Discovering);

        let event = rx.recv().await.unwrap();
        assert_eq!(event.from_state, WbState::Init);
        assert_eq!(event.to_state, WbState::Discovering);
    }

    #[tokio::test]
    async fn test_noop_same_state() {
        let sm = StateManager::new();
        let mut rx = sm.subscribe();

        sm.transition(WbState::Init, "same").await;
        // Should not broadcast for no-op
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_snapshot() {
        let sm = StateManager::new();
        sm.transition(WbState::BtOnly, "bt connected").await;
        sm.set_active(WbTransport::Bluetooth).await;

        let snap = sm.snapshot().await;
        assert_eq!(snap.state, WbState::BtOnly);
        assert_eq!(snap.active_transport, WbTransport::Bluetooth);
        assert_eq!(snap.handover_count, 0);
    }
}

//! TX/RX session engine.
//!
//! Manages message submission, transport routing based on traffic class
//! and current active transport, sequence numbering, deduplication,
//! and the periodic probe loop for metrics collection.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::protocol::{
    self, WbHeader, FLAG_CONTROL, FLAG_PROBE,
};
use crate::state::StateManager;
use crate::types::{TrafficClass, WbTransport};

/// A message queued for transmission.
#[derive(Debug)]
pub struct TxMessage {
    pub payload: Vec<u8>,
    pub class: TrafficClass,
}

/// The session engine that routes messages to the appropriate transport.
pub struct SessionEngine {
    state: StateManager,
    seq_counter: Arc<AtomicU32>,
    tx_queue: mpsc::Sender<TxMessage>,
    rx_queue: mpsc::Receiver<TxMessage>,
}

impl SessionEngine {
    pub fn new(state: StateManager) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            state,
            seq_counter: Arc::new(AtomicU32::new(0)),
            tx_queue: tx,
            rx_queue: rx,
        }
    }

    /// Get a sender handle for submitting messages.
    pub fn sender(&self) -> mpsc::Sender<TxMessage> {
        self.tx_queue.clone()
    }

    /// Submit a message for transmission.
    pub async fn tx_submit(&self, payload: Vec<u8>, class: TrafficClass) -> Result<()> {
        self.tx_queue
            .send(TxMessage { payload, class })
            .await
            .map_err(|_| anyhow::anyhow!("tx queue closed"))
    }

    /// Drain the receive queue (call from the TX loop).
    pub async fn next_message(&mut self) -> Option<TxMessage> {
        self.rx_queue.recv().await
    }

    /// Build a complete frame for a message, ready to send on a transport.
    pub async fn build_frame(&self, payload: &[u8], class: TrafficClass) -> Vec<u8> {
        let seq = self.seq_counter.fetch_add(1, Ordering::SeqCst);
        let session_id = self.state.session_id().await;
        let active = self.state.active_transport().await;

        let flags = match class {
            TrafficClass::Control => FLAG_CONTROL,
            _ => 0,
        };

        let header = WbHeader::new(
            session_id,
            seq,
            class,
            active,
            payload.len() as u16,
            flags,
        );

        protocol::encode_frame(&header, payload)
    }

    /// Build a probe ping frame.
    pub async fn build_probe(&self) -> Vec<u8> {
        let seq = self.seq_counter.fetch_add(1, Ordering::SeqCst);
        let session_id = self.state.session_id().await;
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let probe = crate::protocol::PingProbe {
            seq,
            send_ts_ns: now_ns,
        };
        let probe_bytes = probe.encode();

        let header = WbHeader::new(
            session_id,
            seq,
            TrafficClass::Control,
            WbTransport::None,
            probe_bytes.len() as u16,
            FLAG_PROBE,
        );

        protocol::encode_frame(&header, &probe_bytes)
    }

    /// Build a probe pong (response) frame.
    pub fn build_pong(session_id: u32, seq: u32, original_ts: u64) -> Vec<u8> {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let pong = crate::protocol::PongProbe {
            seq,
            send_ts_ns: now_ns,
            echo_ts_ns: original_ts,
        };
        let pong_bytes = pong.encode();

        let header = WbHeader::new(
            session_id,
            seq,
            TrafficClass::Control,
            WbTransport::None,
            pong_bytes.len() as u16,
            FLAG_PROBE | crate::protocol::FLAG_ACK,
        );

        protocol::encode_frame(&header, &pong_bytes)
    }

    /// Determine which transport(s) a message should be sent on.
    pub async fn route(&self, class: TrafficClass) -> Vec<WbTransport> {
        let snapshot = self.state.snapshot().await;
        let active = snapshot.active_transport;

        match class {
            // Control messages: duplicate on both during handover
            TrafficClass::Control => {
                if matches!(
                    snapshot.state,
                    crate::types::WbState::HandoverBtToWifi
                        | crate::types::WbState::HandoverWifiToBt
                ) {
                    vec![WbTransport::Bluetooth, WbTransport::Wifi]
                } else {
                    vec![active]
                }
            }

            // Interactive: prefer lower RTT transport
            TrafficClass::Interactive => {
                if snapshot.bt_metrics.alive
                    && snapshot.wifi_metrics.alive
                    && snapshot.bt_metrics.rtt_ms < snapshot.wifi_metrics.rtt_ms
                {
                    vec![WbTransport::Bluetooth]
                } else {
                    vec![active]
                }
            }

            // Stream: prefer more stable link
            TrafficClass::Stream => {
                if snapshot.bt_metrics.alive
                    && snapshot.wifi_metrics.alive
                    && snapshot.bt_metrics.stability_score > snapshot.wifi_metrics.stability_score
                {
                    vec![WbTransport::Bluetooth]
                } else {
                    vec![active]
                }
            }

            // Bulk: prefer Wi-Fi almost always
            TrafficClass::Bulk => {
                if snapshot.wifi_metrics.alive {
                    vec![WbTransport::Wifi]
                } else {
                    vec![active]
                }
            }
        }
    }

    /// Get the current sequence number.
    pub fn current_seq(&self) -> u32 {
        self.seq_counter.load(Ordering::SeqCst)
    }
}

/// Deduplication buffer to avoid processing the same message twice
/// (important during handover when control messages are duplicated).
pub struct DeduplicationBuffer {
    seen: std::collections::HashSet<(u32, u32)>, // (session_id, seq_no)
    order: std::collections::VecDeque<(u32, u32)>,
    max_size: usize,
}

impl DeduplicationBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            seen: std::collections::HashSet::new(),
            order: std::collections::VecDeque::new(),
            max_size,
        }
    }

    /// Returns true if this is a NEW message (not seen before).
    pub fn check(&mut self, session_id: u32, seq_no: u32) -> bool {
        let key = (session_id, seq_no);
        if self.seen.contains(&key) {
            return false;
        }
        self.seen.insert(key);
        self.order.push_back(key);

        // Evict old entries
        while self.order.len() > self.max_size {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedup_buffer() {
        let mut dedup = DeduplicationBuffer::new(10);
        assert!(dedup.check(1, 1)); // New
        assert!(!dedup.check(1, 1)); // Duplicate
        assert!(dedup.check(1, 2)); // New
        assert!(dedup.check(2, 1)); // New (different session)
    }

    #[test]
    fn test_dedup_eviction() {
        let mut dedup = DeduplicationBuffer::new(3);
        assert!(dedup.check(1, 1));
        assert!(dedup.check(1, 2));
        assert!(dedup.check(1, 3));
        assert!(dedup.check(1, 4)); // Evicts (1,1)
        assert!(dedup.check(1, 1)); // Should be treated as new now
    }
}

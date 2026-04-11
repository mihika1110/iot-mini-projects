//! Link metrics engine.
//!
//! Tracks per-transport performance indicators (RTT, loss, throughput, jitter)
//! using exponentially weighted moving averages and sliding windows.
//! Produces a composite score for each transport to feed the FSM.

use std::collections::VecDeque;
use std::time::Instant;

use crate::types::{LinkMetrics, Proximity, WbConfig, WbTransport};

/// Maximum number of RTT samples retained.
const RTT_WINDOW: usize = 128;

/// Maximum number of probe results for loss calculation.
const LOSS_WINDOW: usize = 64;

/// EWMA smoothing factor (α). Higher = more responsive, lower = smoother.
const EWMA_ALPHA: f64 = 0.2;

/// Metrics tracker for a single transport.
#[derive(Debug)]
struct TransportTracker {
    transport: WbTransport,
    rtt_samples: VecDeque<f64>,
    rtt_ewma: f64,
    probe_results: VecDeque<bool>, // true = success, false = lost
    bytes_window: VecDeque<(Instant, u64)>,
    rssi_dbm: i32,
    alive: bool,
    last_success: Option<Instant>,
    reconnect_count: u32,
    /// Timestamp when the transport first went "bad"
    bad_since: Option<Instant>,
    /// Timestamp when the transport first went "good"
    good_since: Option<Instant>,
}

impl TransportTracker {
    fn new(transport: WbTransport) -> Self {
        Self {
            transport,
            rtt_samples: VecDeque::with_capacity(RTT_WINDOW),
            rtt_ewma: 0.0,
            probe_results: VecDeque::with_capacity(LOSS_WINDOW),
            bytes_window: VecDeque::new(),
            rssi_dbm: -100,
            alive: false,
            last_success: None,
            reconnect_count: 0,
            bad_since: None,
            good_since: None,
        }
    }
}

/// Metrics engine tracking both transports.
pub struct MetricsEngine {
    bt: TransportTracker,
    wifi: TransportTracker,
    config: WbConfig,
}

impl MetricsEngine {
    pub fn new(config: WbConfig) -> Self {
        Self {
            bt: TransportTracker::new(WbTransport::Bluetooth),
            wifi: TransportTracker::new(WbTransport::Wifi),
            config,
        }
    }

    /// Record the result of a probe on a transport.
    pub fn record_probe(
        &mut self,
        transport: WbTransport,
        rtt_ms: Option<f64>,
        success: bool,
        bytes: u64,
        now: Instant,
    ) {
        let tracker = self.tracker_mut(transport);

        // Record success/loss
        tracker.probe_results.push_back(success);
        if tracker.probe_results.len() > LOSS_WINDOW {
            tracker.probe_results.pop_front();
        }

        if success {
            tracker.alive = true;
            tracker.last_success = Some(now);

            // RTT
            if let Some(rtt) = rtt_ms {
                tracker.rtt_samples.push_back(rtt);
                if tracker.rtt_samples.len() > RTT_WINDOW {
                    tracker.rtt_samples.pop_front();
                }
                // EWMA update
                if tracker.rtt_ewma == 0.0 {
                    tracker.rtt_ewma = rtt;
                } else {
                    tracker.rtt_ewma = EWMA_ALPHA * rtt + (1.0 - EWMA_ALPHA) * tracker.rtt_ewma;
                }
            }

            // Throughput tracking
            if bytes > 0 {
                tracker.bytes_window.push_back((now, bytes));
                // Keep only last 5 seconds
                let cutoff = now - std::time::Duration::from_secs(5);
                while tracker
                    .bytes_window
                    .front()
                    .map_or(false, |(t, _)| *t < cutoff)
                {
                    tracker.bytes_window.pop_front();
                }
            }
        } else {
            // Check if transport has been dead for a while
            if let Some(last) = tracker.last_success {
                if now.duration_since(last).as_secs() > 10 {
                    tracker.alive = false;
                }
            }
        }
    }

    /// Update RSSI for a transport.
    pub fn update_rssi(&mut self, transport: WbTransport, rssi: i32) {
        self.tracker_mut(transport).rssi_dbm = rssi;
    }

    /// Record a reconnection event.
    pub fn record_reconnect(&mut self, transport: WbTransport) {
        self.tracker_mut(transport).reconnect_count += 1;
    }

    /// Mark transport as alive or dead.
    pub fn set_alive(&mut self, transport: WbTransport, alive: bool) {
        self.tracker_mut(transport).alive = alive;
    }

    /// Get a snapshot of metrics for a transport.
    pub fn get_metrics(&self, transport: WbTransport) -> LinkMetrics {
        let tracker = self.tracker(transport);
        let now = Instant::now();

        let rtt_ms = tracker.rtt_ewma;

        // Loss percentage
        let total_probes = tracker.probe_results.len();
        let loss_pct = if total_probes > 0 {
            let lost = tracker.probe_results.iter().filter(|&&s| !s).count();
            (lost as f64 / total_probes as f64) * 100.0
        } else {
            100.0
        };

        // Throughput (kbps) over last 5 seconds
        let throughput_kbps = {
            if tracker.bytes_window.len() < 2 {
                0.0
            } else {
                let first_time = tracker.bytes_window.front().unwrap().0;
                let elapsed = now.duration_since(first_time).as_secs_f64();
                if elapsed > 0.0 {
                    let total_bytes: u64 = tracker.bytes_window.iter().map(|(_, b)| b).sum();
                    (total_bytes as f64 * 8.0) / (elapsed * 1000.0)
                } else {
                    0.0
                }
            }
        };

        // Jitter (standard deviation of recent RTTs)
        let jitter_ms = if tracker.rtt_samples.len() >= 2 {
            let mean =
                tracker.rtt_samples.iter().sum::<f64>() / tracker.rtt_samples.len() as f64;
            let variance = tracker
                .rtt_samples
                .iter()
                .map(|x| (x - mean).powi(2))
                .sum::<f64>()
                / (tracker.rtt_samples.len() - 1) as f64;
            variance.sqrt()
        } else {
            0.0
        };

        // Stability score: composite of loss, jitter, reconnects
        let stability_score = {
            let loss_factor = 1.0 - (loss_pct / 100.0);
            let jitter_factor = if jitter_ms < 5.0 {
                1.0
            } else if jitter_ms < 20.0 {
                0.7
            } else if jitter_ms < 50.0 {
                0.4
            } else {
                0.1
            };
            let reconnect_factor = if tracker.reconnect_count == 0 {
                1.0
            } else {
                (1.0 / (1.0 + tracker.reconnect_count as f64)).max(0.1)
            };
            (loss_factor * 0.4 + jitter_factor * 0.3 + reconnect_factor * 0.3).clamp(0.0, 1.0)
        };

        // Cost score: lower = better (inverse of quality)
        let cost_score = 1.0 - stability_score;

        let last_success_ms = tracker
            .last_success
            .map(|t| now.duration_since(t).as_millis() as u64)
            .unwrap_or(u64::MAX);

        LinkMetrics {
            rtt_ms,
            loss_pct,
            throughput_kbps,
            jitter_ms,
            rssi_dbm: tracker.rssi_dbm,
            stability_score,
            cost_score,
            alive: tracker.alive,
            last_success_ms,
            reconnect_count: tracker.reconnect_count,
        }
    }

    /// Compute a composite score for a transport, factoring in proximity.
    pub fn score(&self, transport: WbTransport, proximity: Proximity) -> f64 {
        let m = self.get_metrics(transport);
        if !m.alive {
            return 0.0;
        }

        let c = &self.config;

        // Normalize RTT: 0ms=1.0, 100ms=0.0 (clamped)
        let latency_score = (1.0 - (m.rtt_ms / 100.0)).clamp(0.0, 1.0);

        // Normalize loss: 0%=1.0, 50%=0.0
        let loss_score = (1.0 - (m.loss_pct / 50.0)).clamp(0.0, 1.0);

        // Proximity bonus: BT gets bonus when near, Wi-Fi when far
        let proximity_bonus = match (transport, proximity) {
            (WbTransport::Bluetooth, Proximity::Near) => 1.0,
            (WbTransport::Bluetooth, Proximity::Mid) => 0.5,
            (WbTransport::Bluetooth, Proximity::Far) => 0.0,
            (WbTransport::Wifi, Proximity::Far) => 1.0,
            (WbTransport::Wifi, Proximity::Mid) => 0.7,
            (WbTransport::Wifi, Proximity::Near) => 0.3,
            _ => 0.5,
        };

        // Energy: BT is lower energy than Wi-Fi
        let energy_score = match transport {
            WbTransport::Bluetooth => 0.9,
            WbTransport::Wifi => 0.5,
            WbTransport::None => 0.0,
        };

        c.w_latency * latency_score
            + c.w_loss * loss_score
            + c.w_stability * m.stability_score
            + c.w_energy * energy_score
            + c.w_proximity * proximity_bonus
    }

    /// Check how long a transport has been continuously "bad" (score below threshold).
    pub fn bad_duration_ms(&self, transport: WbTransport, now: Instant) -> u64 {
        let tracker = self.tracker(transport);
        tracker
            .bad_since
            .map(|t| now.duration_since(t).as_millis() as u64)
            .unwrap_or(0)
    }

    /// Check how long a transport has been continuously "good" (score above threshold).
    pub fn good_duration_ms(&self, transport: WbTransport, now: Instant) -> u64 {
        let tracker = self.tracker(transport);
        tracker
            .good_since
            .map(|t| now.duration_since(t).as_millis() as u64)
            .unwrap_or(0)
    }

    /// Update the good/bad duration tracking based on current scores.
    pub fn update_quality_tracking(&mut self, proximity: Proximity, now: Instant) {
        // Pre-compute scores and cache config thresholds to avoid borrow conflicts
        let bad_threshold = self.config.bt_bad_threshold;
        let good_threshold = self.config.good_threshold;
        let bt_score = self.score(WbTransport::Bluetooth, proximity);
        let wifi_score = self.score(WbTransport::Wifi, proximity);

        let scores = [
            (WbTransport::Bluetooth, bt_score),
            (WbTransport::Wifi, wifi_score),
        ];

        for (transport, score) in scores {
            let tracker = self.tracker_mut(transport);

            if score < bad_threshold {
                if tracker.bad_since.is_none() {
                    tracker.bad_since = Some(now);
                }
                tracker.good_since = None;
            } else if score > good_threshold {
                if tracker.good_since.is_none() {
                    tracker.good_since = Some(now);
                }
                tracker.bad_since = None;
            } else {
                // In the middle — don't change state
            }
        }
    }

    fn tracker(&self, transport: WbTransport) -> &TransportTracker {
        match transport {
            WbTransport::Bluetooth => &self.bt,
            WbTransport::Wifi | WbTransport::None => &self.wifi,
        }
    }

    fn tracker_mut(&mut self, transport: WbTransport) -> &mut TransportTracker {
        match transport {
            WbTransport::Bluetooth => &mut self.bt,
            WbTransport::Wifi | WbTransport::None => &mut self.wifi,
        }
    }

    /// Update config at runtime.
    pub fn update_config(&mut self, config: WbConfig) {
        self.config = config;
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_metrics_dead() {
        let engine = MetricsEngine::new(WbConfig::default());
        let m = engine.get_metrics(WbTransport::Bluetooth);
        assert!(!m.alive);
        assert_eq!(m.loss_pct, 100.0);
    }

    #[test]
    fn test_probe_recording() {
        let mut engine = MetricsEngine::new(WbConfig::default());
        let now = Instant::now();

        // Record successful probes
        for i in 0..10 {
            let t = now + std::time::Duration::from_millis(i * 100);
            engine.record_probe(WbTransport::Wifi, Some(5.0), true, 100, t);
        }

        let m = engine.get_metrics(WbTransport::Wifi);
        assert!(m.alive);
        assert!(m.rtt_ms > 0.0);
        assert_eq!(m.loss_pct, 0.0);
    }

    #[test]
    fn test_loss_tracking() {
        let mut engine = MetricsEngine::new(WbConfig::default());
        let now = Instant::now();

        // 5 success, 5 failure = 50% loss
        for i in 0..10 {
            let t = now + std::time::Duration::from_millis(i * 100);
            let success = i % 2 == 0;
            engine.record_probe(WbTransport::Bluetooth, Some(10.0), success, 50, t);
        }

        let m = engine.get_metrics(WbTransport::Bluetooth);
        assert!((m.loss_pct - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_score_near_bt_preferred() {
        let mut engine = MetricsEngine::new(WbConfig::default());
        let now = Instant::now();

        // Both transports alive with decent metrics
        for i in 0..10 {
            let t = now + std::time::Duration::from_millis(i * 100);
            engine.record_probe(WbTransport::Bluetooth, Some(12.0), true, 100, t);
            engine.record_probe(WbTransport::Wifi, Some(8.0), true, 200, t);
        }

        let bt_score = engine.score(WbTransport::Bluetooth, Proximity::Near);
        let wifi_score = engine.score(WbTransport::Wifi, Proximity::Near);

        // BT should score higher when near due to proximity bonus + energy
        assert!(
            bt_score > wifi_score,
            "BT score ({bt_score}) should be > Wi-Fi ({wifi_score}) when NEAR"
        );
    }

    #[test]
    fn test_score_far_wifi_preferred() {
        let mut engine = MetricsEngine::new(WbConfig::default());
        let now = Instant::now();

        for i in 0..10 {
            let t = now + std::time::Duration::from_millis(i * 100);
            engine.record_probe(WbTransport::Bluetooth, Some(12.0), true, 100, t);
            engine.record_probe(WbTransport::Wifi, Some(8.0), true, 200, t);
        }

        let bt_score = engine.score(WbTransport::Bluetooth, Proximity::Far);
        let wifi_score = engine.score(WbTransport::Wifi, Proximity::Far);

        assert!(
            wifi_score > bt_score,
            "Wi-Fi score ({wifi_score}) should be > BT ({bt_score}) when FAR"
        );
    }
}

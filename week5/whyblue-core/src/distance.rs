//! Proximity estimator with hysteresis.
//!
//! Classifies the relative distance between two WhyBlue peers into
//! bands (Near/Mid/Far) based on Bluetooth and Wi-Fi RSSI readings.
//! Uses sliding windows and sustained-duration thresholds to prevent
//! flapping on noisy radio signals.

use std::collections::VecDeque;
use std::time::Instant;

use crate::types::{Proximity, WbConfig};

/// Maximum number of RSSI samples retained in the sliding window.
const MAX_WINDOW: usize = 64;

/// A single RSSI sample with timestamp.
#[derive(Debug, Clone, Copy)]
struct RssiSample {
    dbm: i32,
    at: Instant,
}

/// Proximity estimator with hysteresis-based classification.
pub struct DistanceEstimator {
    bt_samples: VecDeque<RssiSample>,
    wifi_samples: VecDeque<RssiSample>,
    current: Proximity,
    confidence: f64,
    /// Timestamp when we first started seeing evidence for a new band
    band_entry_time: Option<Instant>,
    /// The band we're trending toward
    trending_band: Proximity,
    config: WbConfig,
}

impl DistanceEstimator {
    pub fn new(config: WbConfig) -> Self {
        Self {
            bt_samples: VecDeque::with_capacity(MAX_WINDOW),
            wifi_samples: VecDeque::with_capacity(MAX_WINDOW),
            current: Proximity::Unknown,
            confidence: 0.0,
            band_entry_time: None,
            trending_band: Proximity::Unknown,
            config,
        }
    }

    /// Feed a new BT RSSI reading (or None if BT is unavailable).
    pub fn update_bt_rssi(&mut self, rssi: Option<i32>, now: Instant) {
        if let Some(dbm) = rssi {
            self.bt_samples.push_back(RssiSample { dbm, at: now });
            if self.bt_samples.len() > MAX_WINDOW {
                self.bt_samples.pop_front();
            }
        }
    }

    /// Feed a new Wi-Fi RSSI reading (or None if Wi-Fi is unavailable).
    pub fn update_wifi_rssi(&mut self, rssi: Option<i32>, now: Instant) {
        if let Some(dbm) = rssi {
            self.wifi_samples.push_back(RssiSample { dbm, at: now });
            if self.wifi_samples.len() > MAX_WINDOW {
                self.wifi_samples.pop_front();
            }
        }
    }

    /// Run the classification algorithm and update internal state.
    /// Call this after feeding new RSSI samples.
    pub fn classify(&mut self, now: Instant) -> (Proximity, f64) {
        let raw_band = self.raw_classify(now);
        let hysteresis_ms = self.config.hysteresis_duration_ms;

        if raw_band == self.current {
            // Stable in current band — reset trending
            self.band_entry_time = None;
            self.trending_band = self.current;
            self.confidence = self.compute_confidence(now);
            return (self.current, self.confidence);
        }

        // We're seeing a different band than current
        if raw_band != self.trending_band {
            // New trend direction — reset timer
            self.trending_band = raw_band;
            self.band_entry_time = Some(now);
        }

        // Check if we've sustained the trend long enough
        if let Some(entry_time) = self.band_entry_time {
            let elapsed_ms = now.duration_since(entry_time).as_millis() as u64;
            if elapsed_ms >= hysteresis_ms && self.check_sample_count(raw_band, now) {
                // Transition confirmed
                self.current = raw_band;
                self.band_entry_time = None;
                self.trending_band = raw_band;
                self.confidence = self.compute_confidence(now);
                return (self.current, self.confidence);
            }
        }

        // Not yet sustained — stay in current band with reduced confidence
        self.confidence = self.compute_confidence(now) * 0.7;
        (self.current, self.confidence)
    }

    /// Raw classification without hysteresis — what the current samples suggest.
    fn raw_classify(&self, now: Instant) -> Proximity {
        let bt_avg = self.recent_average(&self.bt_samples, now, 3000);
        let wifi_avg = self.recent_average(&self.wifi_samples, now, 3000);
        let bt_present = bt_avg.is_some();

        match (bt_avg, wifi_avg) {
            // BT is strong → NEAR
            (Some(bt), _) if bt > self.config.bt_rssi_near_threshold as f64 => Proximity::Near,

            // BT is weak → FAR
            (Some(bt), _) if bt < self.config.bt_rssi_far_threshold as f64 => Proximity::Far,

            // BT absent and Wi-Fi weak → FAR
            (None, Some(wifi)) if wifi < self.config.wifi_rssi_weak_threshold as f64 => {
                Proximity::Far
            }

            // BT absent but Wi-Fi ok → FAR (no BT means not near)
            (None, Some(_)) => Proximity::Far,

            // BT in middle range → MID
            (Some(_), _) if bt_present => Proximity::Mid,

            // No data at all
            _ => Proximity::Unknown,
        }
    }

    /// Compute average RSSI from samples within the last `window_ms` milliseconds.
    fn recent_average(
        &self,
        samples: &VecDeque<RssiSample>,
        now: Instant,
        window_ms: u64,
    ) -> Option<f64> {
        let cutoff = now - std::time::Duration::from_millis(window_ms);
        let recent: Vec<f64> = samples
            .iter()
            .filter(|s| s.at >= cutoff)
            .map(|s| s.dbm as f64)
            .collect();

        if recent.is_empty() {
            None
        } else {
            Some(recent.iter().sum::<f64>() / recent.len() as f64)
        }
    }

    /// Check if we have enough consecutive samples supporting the new band.
    fn check_sample_count(&self, band: Proximity, now: Instant) -> bool {
        let required = self.config.hysteresis_samples as usize;
        let window_ms = self.config.hysteresis_duration_ms;

        match band {
            Proximity::Near | Proximity::Mid | Proximity::Far => {
                let cutoff = now - std::time::Duration::from_millis(window_ms);
                let bt_recent: Vec<&RssiSample> =
                    self.bt_samples.iter().filter(|s| s.at >= cutoff).collect();

                match band {
                    Proximity::Near => bt_recent
                        .iter()
                        .filter(|s| s.dbm > self.config.bt_rssi_near_threshold)
                        .count()
                        >= required,
                    Proximity::Far => {
                        let bt_far = bt_recent
                            .iter()
                            .filter(|s| s.dbm < self.config.bt_rssi_far_threshold)
                            .count()
                            >= required;
                        let bt_absent = bt_recent.is_empty();
                        bt_far || bt_absent
                    }
                    Proximity::Mid => bt_recent.len() >= required,
                    _ => false,
                }
            }
            Proximity::Unknown => true,
        }
    }

    /// Compute a confidence score (0.0–1.0) based on sample freshness and consistency.
    fn compute_confidence(&self, now: Instant) -> f64 {
        let cutoff = now - std::time::Duration::from_millis(self.config.hysteresis_duration_ms);

        // How many recent BT samples do we have?
        let bt_count = self
            .bt_samples
            .iter()
            .filter(|s| s.at >= cutoff)
            .count() as f64;
        let bt_freshness = (bt_count / self.config.hysteresis_samples as f64).min(1.0);

        // RSSI standard deviation — lower variance = higher confidence
        let bt_std = self.rssi_std_dev(&self.bt_samples, now);
        let stability = if bt_std < 3.0 {
            1.0
        } else if bt_std < 8.0 {
            0.7
        } else if bt_std < 15.0 {
            0.4
        } else {
            0.2
        };

        (bt_freshness * 0.5 + stability * 0.5).min(1.0)
    }

    /// Standard deviation of recent RSSI samples.
    fn rssi_std_dev(&self, samples: &VecDeque<RssiSample>, now: Instant) -> f64 {
        let cutoff = now - std::time::Duration::from_millis(self.config.hysteresis_duration_ms);
        let recent: Vec<f64> = samples
            .iter()
            .filter(|s| s.at >= cutoff)
            .map(|s| s.dbm as f64)
            .collect();

        if recent.len() < 2 {
            return 20.0; // High uncertainty with few samples
        }

        let mean = recent.iter().sum::<f64>() / recent.len() as f64;
        let variance =
            recent.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (recent.len() - 1) as f64;
        variance.sqrt()
    }

    /// Get the current proximity state without re-evaluating.
    pub fn current_proximity(&self) -> (Proximity, f64) {
        (self.current, self.confidence)
    }

    /// Update config at runtime (e.g., from TUI).
    pub fn update_config(&mut self, config: WbConfig) {
        self.config = config;
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> WbConfig {
        let mut config = WbConfig::default();
        config.hysteresis_duration_ms = 100; // Fast for tests
        config.hysteresis_samples = 3;
        config
    }

    #[test]
    fn test_initial_state_unknown() {
        let est = DistanceEstimator::new(test_config());
        let (prox, _conf) = est.current_proximity();
        assert_eq!(prox, Proximity::Unknown);
    }

    #[test]
    fn test_strong_bt_becomes_near() {
        let mut est = DistanceEstimator::new(test_config());
        let start = Instant::now();

        // Feed strong BT signals
        for i in 0..5 {
            let t = start + std::time::Duration::from_millis(i * 30);
            est.update_bt_rssi(Some(-55), t);
            est.classify(t);
        }

        // After enough sustained samples, should be NEAR
        let t = start + std::time::Duration::from_millis(200);
        est.update_bt_rssi(Some(-55), t);
        let (prox, _) = est.classify(t);
        assert_eq!(prox, Proximity::Near);
    }

    #[test]
    fn test_weak_bt_becomes_far() {
        let mut est = DistanceEstimator::new(test_config());
        let start = Instant::now();

        // Feed weak BT signals
        for i in 0..5 {
            let t = start + std::time::Duration::from_millis(i * 30);
            est.update_bt_rssi(Some(-85), t);
            est.classify(t);
        }

        let t = start + std::time::Duration::from_millis(200);
        est.update_bt_rssi(Some(-85), t);
        let (prox, _) = est.classify(t);
        assert_eq!(prox, Proximity::Far);
    }

    #[test]
    fn test_single_spike_does_not_switch() {
        let mut est = DistanceEstimator::new(test_config());
        let start = Instant::now();

        // Establish NEAR
        for i in 0..5 {
            let t = start + std::time::Duration::from_millis(i * 30);
            est.update_bt_rssi(Some(-55), t);
            est.classify(t);
        }
        let t = start + std::time::Duration::from_millis(200);
        est.update_bt_rssi(Some(-55), t);
        est.classify(t);

        // Single weak reading should NOT switch away from NEAR
        let t2 = t + std::time::Duration::from_millis(10);
        est.update_bt_rssi(Some(-85), t2);
        let (prox, _) = est.classify(t2);
        assert_eq!(prox, Proximity::Near, "single spike should not cause transition");
    }
}

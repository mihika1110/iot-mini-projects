//! FSM transition logic.
//!
//! Pure-function evaluator that takes current system state (metrics, proximity,
//! timers) and returns a decision about what state to move to and which
//! transport to prefer. Does NOT mutate state — that's the state manager's job.
//!
//! Implements hysteresis (cooldown timers, dwell times) to prevent oscillation.

use crate::types::*;

/// Evaluate the current system inputs and produce a transition decision.
///
/// This is a pure function: it reads the decision input and config,
/// and returns what the system should do next, without side effects.
pub fn evaluate(input: &DecisionInput, config: &WbConfig) -> Decision {
    let now = input.now;
    let since_last_switch = now.duration_since(input.last_switch).as_millis() as u64;

    // ── Guard: enforce cooldown period after a switch ──
    let in_cooldown = since_last_switch < config.switch_cooldown_ms;

    match input.current_state {
        // ────────────────────────────────────────────────────────────────────
        // INIT → discover transports and move to an active state
        // ────────────────────────────────────────────────────────────────────
        WbState::Init => {
            if input.bt_available && input.wifi_available {
                Decision {
                    next_state: WbState::DualReady,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Both transports discovered".into(),
                }
            } else if input.bt_available {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Only Bluetooth available".into(),
                }
            } else if input.wifi_available {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Only Wi-Fi available".into(),
                }
            } else {
                Decision {
                    next_state: WbState::Discovering,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "No transports available, scanning".into(),
                }
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // DISCOVERING → keep scanning until something comes up
        // ────────────────────────────────────────────────────────────────────
        WbState::Discovering => {
            if input.bt_available && input.wifi_available {
                Decision {
                    next_state: WbState::DualReady,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Both transports discovered".into(),
                }
            } else if input.bt_available {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Bluetooth discovered".into(),
                }
            } else if input.wifi_available {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Wi-Fi discovered".into(),
                }
            } else {
                stay(input, "Still scanning for transports")
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // DUAL_READY → choose initial primary based on proximity
        // ────────────────────────────────────────────────────────────────────
        WbState::DualReady => {
            if input.proximity == Proximity::Near
                && input.bt.alive
                && input.bt.stability_score > config.good_threshold
            {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Proximity NEAR, BT is healthy → prefer BT".into(),
                }
            } else if input.wifi.alive {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Default to Wi-Fi as primary".into(),
                }
            } else if input.bt.alive {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Only BT alive in dual-ready".into(),
                }
            } else {
                Decision {
                    next_state: WbState::Degraded,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Both transports failed in dual-ready".into(),
                }
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // BT_ONLY → stay on BT unless quality degrades or distance increases
        // ────────────────────────────────────────────────────────────────────
        WbState::BtOnly => {
            if !input.bt.alive {
                // BT died
                if input.wifi_available && input.wifi.alive {
                    Decision {
                        next_state: WbState::WifiOnly,
                        preferred_primary: WbTransport::Wifi,
                        start_handover: false, // Hard switch, BT is dead
                        keep_secondary_alive: false,
                        reason: "BT transport dead, failing over to Wi-Fi".into(),
                    }
                } else {
                    Decision {
                        next_state: WbState::Degraded,
                        preferred_primary: WbTransport::None,
                        start_handover: false,
                        keep_secondary_alive: false,
                        reason: "BT dead, Wi-Fi unavailable".into(),
                    }
                }
            } else if !in_cooldown && should_leave_bt(input, config) {
                if input.wifi_available && input.wifi.alive {
                    Decision {
                        next_state: WbState::HandoverBtToWifi,
                        preferred_primary: WbTransport::Bluetooth, // Still primary during handover
                        start_handover: true,
                        keep_secondary_alive: true,
                        reason: format!(
                            "BT quality degraded or proximity={}, initiating handover to Wi-Fi",
                            input.proximity
                        ),
                    }
                } else {
                    // Can't switch, stay but note degradation
                    stay(input, "BT degraded but Wi-Fi not available for handover")
                }
            } else {
                stay(input, "BT stable, staying")
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // WIFI_ONLY → stay on Wi-Fi unless BT becomes attractive
        // ────────────────────────────────────────────────────────────────────
        WbState::WifiOnly => {
            if !input.wifi.alive {
                if input.bt_available && input.bt.alive {
                    Decision {
                        next_state: WbState::BtOnly,
                        preferred_primary: WbTransport::Bluetooth,
                        start_handover: false,
                        keep_secondary_alive: false,
                        reason: "Wi-Fi dead, failing over to BT".into(),
                    }
                } else {
                    Decision {
                        next_state: WbState::Degraded,
                        preferred_primary: WbTransport::None,
                        start_handover: false,
                        keep_secondary_alive: false,
                        reason: "Wi-Fi dead, BT unavailable".into(),
                    }
                }
            } else if !in_cooldown && should_switch_to_bt(input, config) {
                if input.bt_available && input.bt.alive {
                    Decision {
                        next_state: WbState::HandoverWifiToBt,
                        preferred_primary: WbTransport::Wifi, // Still primary during handover
                        start_handover: true,
                        keep_secondary_alive: true,
                        reason: "Proximity NEAR, BT healthy, switching to BT".into(),
                    }
                } else {
                    stay(input, "Want BT but it's not available")
                }
            } else {
                stay(input, "Wi-Fi stable, staying")
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // HANDOVER states → wait for protocol confirmation
        // ────────────────────────────────────────────────────────────────────
        WbState::HandoverBtToWifi => {
            // Faked protocol: automatically transition to target state after overlap timeout
            if !input.wifi.alive {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Handover aborted: Wi-Fi died during transition".into(),
                }
            } else if input.now.duration_since(input.state_entered).as_millis() as u64 > config.handover_overlap_ms {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Handover overlap complete, switching to Wi-Fi".into(),
                }
            } else {
                stay(input, "Handover BT→Wi-Fi in progress")
            }
        }

        WbState::HandoverWifiToBt => {
            if !input.bt.alive {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Handover aborted: BT died during transition".into(),
                }
            } else if input.now.duration_since(input.state_entered).as_millis() as u64 > config.handover_overlap_ms {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Handover overlap complete, switching to BT".into(),
                }
            } else {
                stay(input, "Handover Wi-Fi→BT in progress")
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // DEGRADED → retry until something recovers
        // ────────────────────────────────────────────────────────────────────
        WbState::Degraded => {
            if input.bt_available && input.bt.alive && input.wifi_available && input.wifi.alive {
                Decision {
                    next_state: WbState::Recovery,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Both transports recovered".into(),
                }
            } else if input.bt_available && input.bt.alive {
                Decision {
                    next_state: WbState::Recovery,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "BT recovered from degraded".into(),
                }
            } else if input.wifi_available && input.wifi.alive {
                Decision {
                    next_state: WbState::Recovery,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Wi-Fi recovered from degraded".into(),
                }
            } else {
                stay(input, "Both transports still down")
            }
        }

        // ────────────────────────────────────────────────────────────────────
        // RECOVERY → probe and decide where to go
        // ────────────────────────────────────────────────────────────────────
        WbState::Recovery => {
            if input.bt_available && input.wifi_available && input.bt.alive && input.wifi.alive {
                Decision {
                    next_state: WbState::DualReady,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: true,
                    reason: "Recovery complete, both transports ready".into(),
                }
            } else if input.bt.alive {
                Decision {
                    next_state: WbState::BtOnly,
                    preferred_primary: WbTransport::Bluetooth,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Recovery with BT only".into(),
                }
            } else if input.wifi.alive {
                Decision {
                    next_state: WbState::WifiOnly,
                    preferred_primary: WbTransport::Wifi,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Recovery with Wi-Fi only".into(),
                }
            } else {
                Decision {
                    next_state: WbState::Degraded,
                    preferred_primary: WbTransport::None,
                    start_handover: false,
                    keep_secondary_alive: false,
                    reason: "Recovery failed, back to degraded".into(),
                }
            }
        }
    }
}

// ─── Helper predicates ─────────────────────────────────────────────────────────

/// Should we leave BT as primary?
/// True if BT quality has been bad for long enough, OR proximity is no longer NEAR.
fn should_leave_bt(input: &DecisionInput, config: &WbConfig) -> bool {
    // Proximity no longer near
    if input.proximity != Proximity::Near && input.proximity != Proximity::Unknown {
        return true;
    }

    // BT quality below threshold for sustained period
    if input.bt.stability_score < config.bt_bad_threshold
        && input.bt.last_success_ms > config.bad_duration_ms
    {
        return true;
    }

    // High loss on BT
    if input.bt.loss_pct > 30.0 {
        return true;
    }

    false
}

/// Should we switch from Wi-Fi to BT?
/// True if proximity is NEAR, BT is good, and we're not doing bulk traffic.
fn should_switch_to_bt(input: &DecisionInput, config: &WbConfig) -> bool {
    // Must be near
    if input.proximity != Proximity::Near {
        return false;
    }

    // Don't switch during bulk transfers
    if input.traffic_class_hint == TrafficClass::Bulk {
        return false;
    }

    // BT must be alive and good
    if !input.bt.alive || input.bt.stability_score < config.good_threshold {
        return false;
    }

    // BT must have been good for long enough
    if input.bt.last_success_ms > config.good_duration_ms {
        return false;
    }

    true
}

/// Stay in the current state.
fn stay(input: &DecisionInput, reason: &str) -> Decision {
    Decision {
        next_state: input.current_state,
        preferred_primary: input.current_primary,
        start_handover: false,
        keep_secondary_alive: false,
        reason: reason.to_string(),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn make_metrics(alive: bool, rtt: f64, loss: f64, stability: f64) -> LinkMetrics {
        LinkMetrics {
            rtt_ms: rtt,
            loss_pct: loss,
            throughput_kbps: 100.0,
            jitter_ms: 2.0,
            rssi_dbm: -55,
            stability_score: stability,
            cost_score: 1.0 - stability,
            alive,
            last_success_ms: if alive { 100 } else { 30000 },
            reconnect_count: 0,
        }
    }

    fn base_input() -> DecisionInput {
        let now = Instant::now();
        DecisionInput {
            bt: make_metrics(true, 12.0, 0.0, 0.9),
            wifi: make_metrics(true, 8.0, 0.0, 0.95),
            proximity: Proximity::Near,
            proximity_confidence: 0.9,
            bt_available: true,
            wifi_available: true,
            current_primary: WbTransport::None,
            current_state: WbState::Init,
            now,
            last_switch: now - Duration::from_secs(60), // Long ago
            traffic_class_hint: TrafficClass::Interactive,
        }
    }

    #[test]
    fn test_init_both_available() {
        let input = base_input();
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::DualReady);
    }

    #[test]
    fn test_init_bt_only() {
        let mut input = base_input();
        input.wifi_available = false;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::BtOnly);
        assert_eq!(decision.preferred_primary, WbTransport::Bluetooth);
    }

    #[test]
    fn test_init_nothing_available() {
        let mut input = base_input();
        input.bt_available = false;
        input.wifi_available = false;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::Discovering);
    }

    #[test]
    fn test_dual_ready_near_prefers_bt() {
        let mut input = base_input();
        input.current_state = WbState::DualReady;
        input.proximity = Proximity::Near;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::BtOnly);
        assert_eq!(decision.preferred_primary, WbTransport::Bluetooth);
    }

    #[test]
    fn test_dual_ready_far_prefers_wifi() {
        let mut input = base_input();
        input.current_state = WbState::DualReady;
        input.proximity = Proximity::Far;
        input.bt.stability_score = 0.3; // Below threshold
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::WifiOnly);
    }

    #[test]
    fn test_bt_only_far_triggers_handover() {
        let mut input = base_input();
        input.current_state = WbState::BtOnly;
        input.current_primary = WbTransport::Bluetooth;
        input.proximity = Proximity::Far;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::HandoverBtToWifi);
        assert!(decision.start_handover);
    }

    #[test]
    fn test_cooldown_prevents_switch() {
        let mut input = base_input();
        input.current_state = WbState::BtOnly;
        input.current_primary = WbTransport::Bluetooth;
        input.proximity = Proximity::Far;
        input.last_switch = Instant::now(); // Just switched
        let decision = evaluate(&input, &WbConfig::default());
        // Should stay due to cooldown
        assert_eq!(decision.next_state, WbState::BtOnly);
    }

    #[test]
    fn test_wifi_only_near_bt_good_triggers_handover() {
        let mut input = base_input();
        input.current_state = WbState::WifiOnly;
        input.current_primary = WbTransport::Wifi;
        input.proximity = Proximity::Near;
        input.bt.alive = true;
        input.bt.stability_score = 0.9;
        input.bt.last_success_ms = 100; // Recent
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::HandoverWifiToBt);
        assert!(decision.start_handover);
    }

    #[test]
    fn test_no_handover_during_bulk() {
        let mut input = base_input();
        input.current_state = WbState::WifiOnly;
        input.current_primary = WbTransport::Wifi;
        input.proximity = Proximity::Near;
        input.traffic_class_hint = TrafficClass::Bulk;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::WifiOnly);
    }

    #[test]
    fn test_degraded_recovery() {
        let mut input = base_input();
        input.current_state = WbState::Degraded;
        input.bt.alive = true;
        input.wifi.alive = false;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::Recovery);
    }

    #[test]
    fn test_handover_abort_on_target_death() {
        let mut input = base_input();
        input.current_state = WbState::HandoverBtToWifi;
        input.wifi.alive = false;
        let decision = evaluate(&input, &WbConfig::default());
        assert_eq!(decision.next_state, WbState::BtOnly);
    }
}

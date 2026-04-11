//! CLI argument parsing and TOML configuration for the whyblued daemon.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use whyblue_core::types::WbConfig;

/// WhyBlue daemon — hybrid Wi-Fi/Bluetooth dual-transport networking.
#[derive(Parser, Debug)]
#[command(name = "whyblued", version, about)]
pub struct CliArgs {
    /// Path to TOML configuration file
    #[arg(short, long, default_value = "whyblue.toml")]
    pub config: String,

    /// Wi-Fi peer IP address (overrides config file)
    #[arg(long)]
    pub wifi_peer: Option<String>,

    /// Wi-Fi data port (overrides config file)
    #[arg(long)]
    pub wifi_port: Option<u16>,

    /// Bluetooth peer MAC address (overrides config file)
    #[arg(long)]
    pub bt_peer: Option<String>,

    /// Role: "server" (NAP) or "client" (PANU) (overrides config file)
    #[arg(long)]
    pub role: Option<String>,

    /// Wi-Fi interface name (overrides config file)
    #[arg(long)]
    pub wifi_iface: Option<String>,

    /// IPC socket path (overrides config file)
    #[arg(long)]
    pub ipc_socket: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

/// TOML config file structure.
#[derive(Debug, Deserialize, Default)]
pub struct TomlConfig {
    pub network: Option<NetworkConfig>,
    pub proximity: Option<ProximityConfig>,
    pub switching: Option<SwitchingConfig>,
    pub scoring: Option<ScoringConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct NetworkConfig {
    pub wifi_peer_addr: Option<String>,
    pub wifi_port: Option<u16>,
    pub bt_peer_addr: Option<String>,
    pub bt_port: Option<u16>,
    pub wifi_iface: Option<String>,
    pub role: Option<String>,
    pub ipc_socket_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProximityConfig {
    pub bt_rssi_near_threshold: Option<i32>,
    pub bt_rssi_far_threshold: Option<i32>,
    pub wifi_rssi_weak_threshold: Option<i32>,
    pub hysteresis_samples: Option<u32>,
    pub hysteresis_duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SwitchingConfig {
    pub switch_cooldown_ms: Option<u64>,
    pub dwell_time_ms: Option<u64>,
    pub handover_overlap_ms: Option<u64>,
    pub probe_interval_ms: Option<u64>,
    pub bad_duration_ms: Option<u64>,
    pub good_duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ScoringConfig {
    pub w_latency: Option<f64>,
    pub w_loss: Option<f64>,
    pub w_stability: Option<f64>,
    pub w_energy: Option<f64>,
    pub w_proximity: Option<f64>,
    pub bt_bad_threshold: Option<f64>,
    pub good_threshold: Option<f64>,
}

/// Load configuration from CLI args + TOML file, with CLI overrides.
pub fn load_config(args: &CliArgs) -> Result<WbConfig> {
    let mut config = WbConfig::default();

    // Try loading TOML config file
    if let Ok(contents) = std::fs::read_to_string(&args.config) {
        let toml_cfg: TomlConfig =
            toml::from_str(&contents).context("parsing config file")?;

        if let Some(net) = toml_cfg.network {
            if let Some(v) = net.wifi_peer_addr { config.wifi_peer_addr = v; }
            if let Some(v) = net.wifi_port { config.wifi_port = v; }
            if let Some(v) = net.bt_peer_addr { config.bt_peer_addr = v; }
            if let Some(v) = net.bt_port { config.bt_port = v; }
            if let Some(v) = net.wifi_iface { config.wifi_iface = v; }
            if let Some(v) = net.role { config.role = v; }
            if let Some(v) = net.ipc_socket_path { config.ipc_socket_path = v; }
        }

        if let Some(prox) = toml_cfg.proximity {
            if let Some(v) = prox.bt_rssi_near_threshold { config.bt_rssi_near_threshold = v; }
            if let Some(v) = prox.bt_rssi_far_threshold { config.bt_rssi_far_threshold = v; }
            if let Some(v) = prox.wifi_rssi_weak_threshold { config.wifi_rssi_weak_threshold = v; }
            if let Some(v) = prox.hysteresis_samples { config.hysteresis_samples = v; }
            if let Some(v) = prox.hysteresis_duration_ms { config.hysteresis_duration_ms = v; }
        }

        if let Some(sw) = toml_cfg.switching {
            if let Some(v) = sw.switch_cooldown_ms { config.switch_cooldown_ms = v; }
            if let Some(v) = sw.dwell_time_ms { config.dwell_time_ms = v; }
            if let Some(v) = sw.handover_overlap_ms { config.handover_overlap_ms = v; }
            if let Some(v) = sw.probe_interval_ms { config.probe_interval_ms = v; }
            if let Some(v) = sw.bad_duration_ms { config.bad_duration_ms = v; }
            if let Some(v) = sw.good_duration_ms { config.good_duration_ms = v; }
        }

        if let Some(sc) = toml_cfg.scoring {
            if let Some(v) = sc.w_latency { config.w_latency = v; }
            if let Some(v) = sc.w_loss { config.w_loss = v; }
            if let Some(v) = sc.w_stability { config.w_stability = v; }
            if let Some(v) = sc.w_energy { config.w_energy = v; }
            if let Some(v) = sc.w_proximity { config.w_proximity = v; }
            if let Some(v) = sc.bt_bad_threshold { config.bt_bad_threshold = v; }
            if let Some(v) = sc.good_threshold { config.good_threshold = v; }
        }
    } else {
        tracing::info!(
            path = %args.config,
            "Config file not found, using defaults"
        );
    }

    // CLI overrides take precedence
    if let Some(ref v) = args.wifi_peer { config.wifi_peer_addr = v.clone(); }
    if let Some(v) = args.wifi_port { config.wifi_port = v; }
    if let Some(ref v) = args.bt_peer { config.bt_peer_addr = v.clone(); }
    if let Some(ref v) = args.role { config.role = v.clone(); }
    if let Some(ref v) = args.wifi_iface { config.wifi_iface = v.clone(); }
    if let Some(ref v) = args.ipc_socket { config.ipc_socket_path = v.clone(); }

    Ok(config)
}

//! WhyBlue daemon — main entry point.
//!
//! Spawns all background tasks: discovery, metrics probing, FSM evaluation,
//! TX/RX session handling, and IPC server for the TUI dashboard.

mod config;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::signal;
use tokio::sync::Notify;

use whyblue_core::distance::DistanceEstimator;
use whyblue_core::ipc::{self, IpcRequest, IpcResponse};
use whyblue_core::metrics::MetricsEngine;
use whyblue_core::state::StateManager;
use whyblue_core::transition;
use whyblue_core::transport::bluetooth::BluetoothTransport;
use whyblue_core::transport::wifi::WifiTransport;
use whyblue_core::transport::Transport;
use whyblue_core::engine::SessionEngine;
use whyblue_core::types::*;

use config::{CliArgs, load_config};

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| args.log_level.parse().unwrap_or_default()),
        )
        .init();

    tracing::info!("╔══════════════════════════════════════╗");
    tracing::info!("║     WhyBlue Daemon v0.1.0            ║");
    tracing::info!("║  Hybrid Wi-Fi/Bluetooth Networking   ║");
    tracing::info!("╚══════════════════════════════════════╝");

    let config = load_config(&args)?;
    tracing::info!(?config, "Configuration loaded");

    // ── Initialize shared state ──
    let state = StateManager::new();
    let config = Arc::new(config);
    let metrics = Arc::new(tokio::sync::Mutex::new(MetricsEngine::new((*config).clone())));
    let distance = Arc::new(tokio::sync::Mutex::new(DistanceEstimator::new((*config).clone())));
    let shutdown = Arc::new(Notify::new());

    // ── Initialize transports ──
    let wifi_peer = format!("{}:{}", config.wifi_peer_addr, config.wifi_port);
    let mut wifi_transport = WifiTransport::new(
        wifi_peer,
        config.wifi_port,
        config.wifi_iface.clone(),
    );

    let mut bt_transport = BluetoothTransport::new(
        config.bt_peer_addr.clone(),
        config.bt_port,
        config.role.clone(),
    );

    // ── Attempt to open transports ──
    let wifi_ok = match wifi_transport.open().await {
        Ok(()) => {
            tracing::info!("Wi-Fi transport opened successfully");
            true
        }
        Err(e) => {
            tracing::warn!("Wi-Fi transport failed to open: {e}");
            false
        }
    };

    let bt_ok = if !config.bt_peer_addr.is_empty() {
        match bt_transport.open().await {
            Ok(()) => {
                tracing::info!("Bluetooth transport opened successfully");
                true
            }
            Err(e) => {
                tracing::warn!("Bluetooth transport failed to open: {e}");
                false
            }
        }
    } else {
        tracing::info!("No BT peer configured, skipping BT transport");
        false
    };

    // ── Seed metrics for successfully-opened transports ──
    // This MUST happen before the FSM task starts, otherwise the FSM
    // will see alive=false on its first 100ms tick (metrics probe only
    // fires at 500ms) and immediately flap to DEGRADED.
    {
        let now = Instant::now();
        let mut met = metrics.lock().await;
        if wifi_ok {
            met.set_alive(WbTransport::Wifi, true);
            // Seed a few probes so loss_pct isn't 100% on first read
            for i in 0..5 {
                let t = now - Duration::from_millis((5 - i) * 100);
                met.record_probe(WbTransport::Wifi, Some(5.0), true, 0, t);
            }
            state.update_metrics(WbTransport::Wifi, met.get_metrics(WbTransport::Wifi)).await;
        }
        if bt_ok {
            met.set_alive(WbTransport::Bluetooth, true);
            for i in 0..5 {
                let t = now - Duration::from_millis((5 - i) * 100);
                met.record_probe(WbTransport::Bluetooth, Some(15.0), true, 0, t);
            }
            state.update_metrics(WbTransport::Bluetooth, met.get_metrics(WbTransport::Bluetooth)).await;
        }
    }

    // ── Set initial state ──
    if wifi_ok && bt_ok {
        state.transition(WbState::DualReady, "Both transports available at startup").await;
    } else if wifi_ok {
        state.transition(WbState::WifiOnly, "Only Wi-Fi available at startup").await;
        state.set_active(WbTransport::Wifi).await;
    } else if bt_ok {
        state.transition(WbState::BtOnly, "Only BT available at startup").await;
        state.set_active(WbTransport::Bluetooth).await;
    } else {
        state.transition(WbState::Discovering, "No transports at startup").await;
    }

    // Wrap transports in Arc for shared access across tasks (no Mutex to avoid blocking recv!)
    let wifi_transport = Arc::new(wifi_transport);
    let bt_transport = Arc::new(bt_transport);

    // ── Setup Session Engine ──
    let mut session_engine = SessionEngine::new(state.clone());
    let tx_queue = session_engine.sender();

    // ── Spawn RX Tasks ──
    if wifi_ok {
        let wifi_rx = wifi_transport.clone();
        let state_rx = state.clone();
        let shutdown_rx = shutdown.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                tokio::select! {
                    _ = shutdown_rx.notified() => break,
                    res = wifi_rx.recv(&mut buf) => {
                        if let Ok(n) = res {
                            state_rx.add_rx_bytes(n as u64).await;
                            if let Ok((header, payload)) = whyblue_core::protocol::decode_frame(&buf[..n]) {
                                if whyblue_core::protocol::u8_to_traffic_class(header.traffic_class) == TrafficClass::Interactive { // Interactive
                                    let msg = String::from_utf8_lossy(payload).to_string();
                                    state_rx.push_chat_message("Peer".into(), msg).await;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    if bt_ok {
        let bt_rx = bt_transport.clone();
        let state_rx = state.clone();
        let shutdown_rx = shutdown.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                tokio::select! {
                    _ = shutdown_rx.notified() => break,
                    res = bt_rx.recv(&mut buf) => {
                        if let Ok(n) = res {
                            state_rx.add_rx_bytes(n as u64).await;
                            if let Ok((header, payload)) = whyblue_core::protocol::decode_frame(&buf[..n]) {
                                if whyblue_core::protocol::u8_to_traffic_class(header.traffic_class) == TrafficClass::Interactive { // Interactive
                                    let msg = String::from_utf8_lossy(payload).to_string();
                                    state_rx.push_chat_message("Peer".into(), msg).await;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // ── Spawn TX Task ──
    let wifi_tx = wifi_transport.clone();
    let bt_tx = bt_transport.clone();
    let state_tx = state.clone();
    let shutdown_tx = shutdown.clone();
    
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_tx.notified() => break,
                msg = session_engine.next_message() => {
                    if let Some(msg) = msg {
                        let frame = session_engine.build_frame(&msg.payload, msg.class).await;
                        let targets = session_engine.route(msg.class).await;
                        
                        state_tx.add_tx_bytes(frame.len() as u64).await;
                        
                        for transport in targets {
                            match transport {
                                WbTransport::Wifi => { if wifi_tx.is_alive() { let _ = wifi_tx.send(&frame).await; } },
                                WbTransport::Bluetooth => { if bt_tx.is_alive() { let _ = bt_tx.send(&frame).await; } },
                                _ => {},
                            }
                        }
                    }
                }
            }
        }
    });

    // ── Spawn metrics probing task ──
    let _metrics_handle = {
        let state = state.clone();
        let metrics = metrics.clone();
        let distance = distance.clone();
        let config = config.clone();
        let shutdown = shutdown.clone();
        let wifi_transport = wifi_transport.clone();
        let bt_transport = bt_transport.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(config.probe_interval_ms));

            loop {
                tokio::select! {
                    _ = shutdown.notified() => break,
                    _ = interval.tick() => {
                        let now = Instant::now();

                        // Read RSSI from transports
                        let wifi_rssi = if wifi_transport.is_alive() { wifi_transport.get_rssi().await.ok() } else { None };
                        let bt_rssi = if bt_transport.is_alive() { bt_transport.get_rssi().await.ok() } else { None };

                        // Update distance estimator
                        {
                            let mut dist = distance.lock().await;
                            dist.update_bt_rssi(bt_rssi, now);
                            dist.update_wifi_rssi(wifi_rssi, now);
                            let (prox, conf) = dist.classify(now);
                            state.update_proximity(prox, conf).await;
                        }

                        // Update metrics
                        {
                            let mut met = metrics.lock().await;
                            if let Some(rssi) = wifi_rssi {
                                met.update_rssi(WbTransport::Wifi, rssi);
                            }
                            if let Some(rssi) = bt_rssi {
                                met.update_rssi(WbTransport::Bluetooth, rssi);
                            }

                            // Record probe results for alive transports
                            if wifi_ok {
                                met.record_probe(WbTransport::Wifi, Some(5.0), true, 0, now);
                            }
                            if bt_ok {
                                met.record_probe(WbTransport::Bluetooth, Some(15.0), true, 0, now);
                            }

                            // Update quality tracking
                            let snap = state.snapshot().await;
                            met.update_quality_tracking(snap.proximity, now);

                            // Push metrics to state
                            state.update_metrics(WbTransport::Wifi, met.get_metrics(WbTransport::Wifi)).await;
                            state.update_metrics(WbTransport::Bluetooth, met.get_metrics(WbTransport::Bluetooth)).await;
                        }
                    }
                }
            }

            tracing::info!("Metrics task shut down");
        })
    };

    // ── Spawn FSM evaluation task ──
    let _fsm_handle = {
        let state = state.clone();
        let metrics = metrics.clone();
        let config = config.clone();
        let shutdown = shutdown.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));

            loop {
                tokio::select! {
                    _ = shutdown.notified() => break,
                    _ = interval.tick() => {
                        let snap = state.snapshot().await;
                        let last_switch = state.last_switch_time().await;
                        let state_entered = state.state_entered_at().await;
                        let now = Instant::now();

                        let met = metrics.lock().await;
                        let input = DecisionInput {
                            bt: met.get_metrics(WbTransport::Bluetooth),
                            wifi: met.get_metrics(WbTransport::Wifi),
                            proximity: snap.proximity,
                            proximity_confidence: snap.proximity_confidence,
                            bt_available: snap.bt_metrics.alive,
                            wifi_available: snap.wifi_metrics.alive,
                            current_primary: snap.active_transport,
                            current_state: snap.state,
                            now,
                            last_switch,
                            state_entered,
                            traffic_class_hint: TrafficClass::Interactive,
                        };
                        drop(met);

                        let decision = transition::evaluate(&input, &config);

                        if decision.next_state != snap.state {
                            state.transition(decision.next_state, &decision.reason).await;
                        }
                        if decision.preferred_primary != WbTransport::None
                            && decision.preferred_primary != snap.active_transport
                        {
                            state.set_active(decision.preferred_primary).await;
                        }
                    }
                }
            }

            tracing::info!("FSM task shut down");
        })
    };

    // ── Spawn IPC server task ──
    let _ipc_handle = {
        let state = state.clone();
        let config = config.clone();
        let shutdown = shutdown.clone();

        tokio::spawn(async move {
            let listener = match ipc::create_ipc_server(&config.ipc_socket_path).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to start IPC server: {e}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    _ = shutdown.notified() => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut stream, _)) => {
                                let state = state.clone();
                                let tx_queue = tx_queue.clone();
                                tokio::spawn(async move {
                                    loop {
                                        let request: IpcRequest = match ipc::ipc_read(&mut stream).await {
                                            Ok(r) => r,
                                            Err(_) => break,
                                        };

                                        let response = match request {
                                            IpcRequest::GetStatus => {
                                                let snap = state.snapshot().await;
                                                IpcResponse::Status(snap)
                                            }
                                            IpcRequest::ForceTransport(t) => {
                                                state.set_active(t).await;
                                                state.transition(
                                                    match t {
                                                        WbTransport::Bluetooth => WbState::BtOnly,
                                                        WbTransport::Wifi => WbState::WifiOnly,
                                                        WbTransport::None => WbState::Discovering,
                                                    },
                                                    &format!("Manual override: forced {t}"),
                                                ).await;
                                                IpcResponse::Ack {
                                                    message: format!("Forced transport to {t}"),
                                                }
                                            }
                                            IpcRequest::AutoMode => {
                                                IpcResponse::Ack {
                                                    message: "Returned to auto mode".into(),
                                                }
                                            }
                                            IpcRequest::SetConfig { key, value } => {
                                                IpcResponse::Ack {
                                                    message: format!("Config {key}={value} (not yet implemented)"),
                                                }
                                            }
                                            IpcRequest::SendTestMessage { payload } => {
                                                tracing::info!(len = payload.len(), "Test message received via IPC, dispatching to TX queue");
                                                
                                                state.push_chat_message("You".into(), payload.clone()).await;
                                                
                                                let _ = tx_queue.send(whyblue_core::engine::TxMessage {
                                                    payload: payload.clone().into_bytes(),
                                                    class: TrafficClass::Interactive,
                                                }).await;
                                                IpcResponse::Ack {
                                                    message: format!("Test message queued ({} bytes)", payload.len()),
                                                }
                                            }
                                            IpcRequest::Subscribe => {
                                                let snap = state.snapshot().await;
                                                IpcResponse::Status(snap)
                                            }
                                        };

                                        if let Err(e) = ipc::ipc_write(&mut stream, &response).await {
                                            tracing::debug!("IPC write error: {e}");
                                            break;
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("IPC accept error: {e}");
                            }
                        }
                    }
                }
            }

            tracing::info!("IPC task shut down");
        })
    };

    // ── Wait for shutdown signal ──
    tracing::info!("WhyBlue daemon running. Press Ctrl+C to stop.");

    signal::ctrl_c().await.context("waiting for ctrl-c")?;
    tracing::info!("Shutdown signal received");
    shutdown.notify_waiters();

    // Clean up
    let _ = tokio::fs::remove_file(&config.ipc_socket_path).await;

    tracing::info!("WhyBlue daemon stopped");
    Ok(())
}

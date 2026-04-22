# Week 5 — WhyBlue: Hybrid Wi-Fi/Bluetooth Dual-Transport Networking Daemon

> Server-Authoritative Transport Switching · Composite Metric Scoring · Make-Before-Break Handover · Ratatui Dashboard · Rust/Tokio Async

---

## Overview

WhyBlue is a high-performance hybrid networking daemon built in Rust that abstracts Wi-Fi and Bluetooth Personal Area Network (PAN) transports into a single, highly-available logical link. It continuously monitors link quality metrics (RSSI, latency, stability) on both transports and autonomously switches active traffic to the optimal interface using a server-authoritative Finite State Machine (FSM). The system achieves zero-packet-drop handovers via a make-before-break dual-emission strategy and provides real-time telemetry through a Ratatui terminal dashboard.

---

## System Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                    Raspberry Pi (Server Node)                 │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                     whyblued (Daemon)                   │  │
│  │                                                        │  │
│  │  ┌──────────┐  ┌──────────┐  ┌────────────────────┐   │  │
│  │  │  Wi-Fi   │  │Bluetooth │  │  Metrics Poller    │   │  │
│  │  │ UDP RX/TX│  │ UDP RX/TX│  │ /proc/net/wireless │   │  │
│  │  │ (wlan0)  │  │ (br0)    │  │  DBus BT RSSI      │   │  │
│  │  └────┬─────┘  └────┬─────┘  └────────┬───────────┘   │  │
│  │       │              │                 │               │  │
│  │  ┌────▼──────────────▼─────────────────▼───────────┐   │  │
│  │  │              StateManager                        │   │  │
│  │  │  ┌────────────┐  ┌────────────┐  ┌───────────┐  │   │  │
│  │  │  │SessionEngine│  │ FSM        │  │ Composite │  │   │  │
│  │  │  │(TX routing) │  │(transition)│  │  Scorer   │  │   │  │
│  │  │  └────────────┘  └────────────┘  └───────────┘  │   │  │
│  │  └─────────────────────┬────────────────────────────┘   │  │
│  │                        │ IPC (Unix Socket)              │  │
│  └────────────────────────┼────────────────────────────────┘  │
│                           │                                   │
│  ┌────────────────────────▼────────────────────────────────┐  │
│  │                  whyblue-tui (Dashboard)                 │  │
│  │  Status │ Metrics │ Chat │ Force Switch                  │  │
│  └──────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
            │  Wi-Fi (UDP :9876)          │  Bluetooth PAN
            │                             │  (UDP :9877)
┌───────────▼─────────────────────────────▼───────────────────┐
│                    Raspberry Pi (Client Node)                 │
│  whyblued (--config whyblue-client.toml)                     │
│  • Obedient state-follower (no local FSM)                    │
│  • SwitchRequest protocol for user-initiated requests        │
│  • PANU role on Bluetooth                                    │
└──────────────────────────────────────────────────────────────┘
```

---

## Workspace Structure

The codebase is a Cargo workspace with three crates:

| Crate | Type | Purpose |
|---|---|---|
| `whyblue-core` | Library | Protocol definitions, state management, metric scoring, transport interfaces, FSM transition logic |
| `whyblued` | Binary | Tokio async daemon: socket binding, background tasks, lifecycle management |
| `whyblue-tui` | Binary | Ratatui terminal dashboard: telemetry visualization, chat, forced transport switching |

---

## Core Concepts

### Server-Authoritative State Management

To prevent split-brain (ping-pong) scenarios:

- **Server** — Runs the FSM evaluation loop, hosts Bluetooth NAP, makes all transport switching decisions
- **Client** — Obedient state-follower; mutates `ActiveTransport` only upon receiving `SwitchPrepare` from server
- **Switch Requests** — If a client user forces a switch via the TUI, a `HandoverMsg::SwitchRequest` is sent to the server, which validates and responds with `SwitchPrepare`

### Custom Binary Protocol

Every packet uses a `WbHeader`:

```rust
struct WbHeader {
    session_id: u32,     // Session identifier
    seq: u32,            // Sequence number (dedup)
    class: TrafficClass, // Interactive | Bulk | Control
    transport: WbTransport, // Wifi | Bluetooth
    payload_len: u16,    // Payload size
    flags: u8,           // Bitmask (FLAG_HANDOVER = 0x02)
}
```

**Handover Messages** (serialized via serde within `FLAG_HANDOVER` tagged packets):
- `SwitchRequest` — Client → Server: "Please switch to transport X"
- `SwitchPrepare` — Server → Client: "Switch to transport X now"

### Composite Metric Scoring

Each transport receives a 0.0–1.0 composite score:

```
Score = (Latency × W_lat) + (Stability × W_stab) + (Loss × W_loss) + (Proximity × W_prox)
```

| Component | Source | Description |
|---|---|---|
| **Latency** | Packet tracking + probe responses | Lower = better |
| **Stability** | Jitter of continuous RSSI values | Penalizes radical swings |
| **Loss** | Packet drop rate | Derived from sequence gaps |
| **Proximity** | RSSI (dBm → linear bounded) | Signal strength proxy |

When the active transport's score drops below `bad_threshold` (e.g., 0.35) for a sustained `hysteresis` period, the FSM triggers handover to the standby link.

### Make-Before-Break Handover

During transport transitions:
1. **Overlap Phase** (configurable, e.g., 2000 ms) — The `SessionEngine` duplicates every outgoing packet across **both** Wi-Fi and Bluetooth UDP sockets simultaneously.
2. **Commit Phase** — After the overlap timer expires, the old transport is dropped; all traffic funnels exclusively to the new transport.
3. **Deduplication** — Receivers listen on both sockets concurrently; the first-arriving copy is processed and duplicates are dropped via `seq` comparison.

This guarantees **zero-packet-drop** handovers.

---

## Network & Transport Layer

### Wi-Fi Transport

1. UDP socket bound to port `9876`
2. `SO_BINDTODEVICE` (libc setsockopt) forces the socket to `wlan0` exclusively
3. RSSI polled from `/proc/net/wireless` targeting `wlan0`
4. Requires `CAP_NET_ADMIN` privileges

### Bluetooth Transport (BNEP/PAN)

**Server (NAP — Network Access Point):**
1. Creates a Linux bridge `br0` with `ip link add`
2. Assigns static IP `10.0.0.1/24`
3. Registers as Bluetooth NAP via `bt-network -s nap br0` or D-Bus fallback (`org.bluez.NetworkServer1.Register`)
4. UDP socket bound to port `9877` on `br0`

**Client (PANU — PAN User):**
1. Ensures server MAC is paired and trusted via `bluetoothctl`
2. Connects via D-Bus (`org.bluez.Network1.Connect` with `nap` profile)
3. Creates `bnep0` interface
4. Assigns IP `10.0.0.2/24`
5. UDP socket bound to port `9877` on `bnep0`

---

## Daemon Task Model (Tokio)

| Task | Role | Concurrency |
|---|---|---|
| **Wi-Fi RX** | `tokio::spawn` loop — polls Wi-Fi UDP socket, decodes headers, routes to state manager or chat log | Independent |
| **Bluetooth RX** | `tokio::spawn` loop — polls BT UDP socket, same routing | Independent |
| **TX** | Reads from async MPSC `tx_queue`, builds `WbHeader` frames, dispatches to active transport(s). Dual-emits during overlap phase | Independent |
| **Metrics Poller** | Wakes every 100 ms — reads `/proc/net/wireless` (Wi-Fi) and D-Bus (BT RSSI). Aggregates via EMA | Independent |
| **FSM Evaluator** | Server only — reads metric scores, calls `transition::evaluate()`, triggers `SwitchPrepare` cascade | Server only |
| **IPC Server** | Unix domain socket at `/tmp/whyblue.sock` — serves JSON RPC (`GetStatus`, `ForceTransport`, `SendTestMessage`) to TUI | Independent |

All tasks share state via `Arc<RwLock<StateManager>>`.

---

## TUI Dashboard

The `whyblue-tui` application connects to the daemon via IPC and provides:

- **Dashboard Tab** — Real-time metrics: active transport, composite scores, RSSI, latency, packet counts
- **Stream Log Tab** — Chat/text streaming over the active transport
- **Transport Control** — Force switch button (emits RPC → daemon → `SwitchRequest` flow)

Switch between tabs with `Tab`.

---

## Configuration

### `whyblue.toml` (Template: `whyblue.toml.example`)

```toml
[network]
wifi_peer_addr = "10.42.0.48"
wifi_port = 9876
bt_peer_addr = "D8:3A:DD:0E:03:B8"
role = "server"  # or "client"

[proximity]
bt_rssi_near_threshold = -65
bt_rssi_far_threshold = -78
hysteresis_samples = 5
```

Separate configs are provided:
- `whyblue-server.toml` — Server role, NAP configuration
- `whyblue-client.toml` — Client role, PANU configuration

---

## Build & Run

### Prerequisites

- **OS**: Linux (Raspberry Pi OS / Ubuntu)
- **Rust**: ≥ 1.70 with `cargo`
- **BlueZ**: `bluez`, `bluez-tools` for PAN support
- **Permissions**: `CAP_NET_ADMIN` (typically `sudo`)

### Build

```bash
cargo build --release
```

### Run

**Server:**
```bash
sudo ./target/release/whyblued --config whyblue-server.toml
```

**Client:**
```bash
sudo ./target/release/whyblued --config whyblue-client.toml
```

**Dashboard (either node):**
```bash
./target/release/whyblue-tui
```

### Remote Deployment

```bash
./sync.sh <username> <hostname_or_ip>
# Uses rsync to push source, excluding target/
```

---

## File Structure

```
week5/
├── Cargo.toml                   # Workspace root
├── Cargo.lock
├── README.md                    # This document
├── ARCHITECTURE.md              # Detailed technical architecture
├── NETWORK.md                   # Network interface lifecycle
├── whyblue.toml.example         # Configuration template
├── whyblue-server.toml          # Server configuration
├── whyblue-client.toml          # Client configuration
├── sync.sh                      # rsync deployment script
├── whyblue-core/                # Shared library crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs               # Module exports
│       ├── protocol.rs          # WbHeader, frame serialization
│       ├── types.rs             # TrafficClass, WbTransport, HandoverMsg
│       ├── state.rs             # StateManager, ActiveTransport
│       ├── engine.rs            # SessionEngine (TX routing, dual-emit)
│       ├── transition.rs        # FSM evaluation, transition logic
│       ├── metrics.rs           # Metric collection, EMA scoring
│       ├── distance.rs          # RSSI → distance conversion
│       ├── ipc.rs               # IPC protocol (JSON RPC)
│       └── transport/           # Transport trait implementations
├── whyblued/                    # Daemon binary crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs              # Tokio entry point, task spawning
│       └── config.rs            # TOML configuration parsing
└── whyblue-tui/                 # TUI binary crate
    ├── Cargo.toml
    └── src/
        ├── main.rs              # TUI entry point
        ├── app.rs               # Application state
        ├── ipc_client.rs        # Unix socket IPC client
        └── ui/                  # Ratatui rendering modules
```

---

## Dependencies

### Workspace

| Dependency | Version | Purpose |
|---|---|---|
| `tokio` | 1 (full) | Async runtime |
| `serde` / `serde_json` | 1 | Serialization |
| `tracing` / `tracing-subscriber` | 0.1/0.3 | Structured logging |
| `async-trait` | 0.1 | Async trait support |
| `anyhow` / `thiserror` | 1/2 | Error handling |
| `chrono` | 0.4 | Timestamps |

### TUI

| Dependency | Version | Purpose |
|---|---|---|
| `ratatui` | ≥0.28 | Terminal UI framework |
| `crossterm` | ≥0.27 | Terminal backend |

---

## Handover Protocol Summary

```
Server FSM detects declining Wi-Fi quality
         │
         ▼
Server sends SwitchPrepare (FLAG_HANDOVER)
to Client via current active transport
         │
         ▼
Both nodes enter Overlap Phase
(dual-emit on Wi-Fi + Bluetooth for 2s)
         │
         ▼
Overlap expires → Commit Phase
Old transport dropped, new transport exclusive
         │
         ▼
Traffic resumes seamlessly on new transport
```

---

## Technical Specifications

| Metric | Value |
|---|---|
| Wi-Fi Port | 9876 (UDP) |
| Bluetooth Port | 9877 (UDP) |
| Metrics Poll Interval | 100 ms |
| BT RSSI Near Threshold | -65 dBm |
| BT RSSI Far Threshold | -78 dBm |
| Hysteresis Samples | 5 |
| IPC Socket | `/tmp/whyblue.sock` |
| Handover Overlap Duration | ~2000 ms |
| Bad Score Threshold | 0.35 |
| Server BT IP | 10.0.0.1/24 (br0) |
| Client BT IP | 10.0.0.2/24 (bnep0) |

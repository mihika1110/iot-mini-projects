# Week 3 — Multi-Node Object Localization System with PIR + Ultrasonic Sensor Fusion over MQTT

> 4× ESP32 Sensor Nodes → PIR Debounce + Ultrasonic Ranging → MQTT → Rust Subscriber → Weighted Sensor Fusion → Ratatui Terminal Dashboard

---

## Overview

This project builds a real-time indoor object localization system using four ESP32 sensor nodes, each equipped with an HC-SR04 ultrasonic distance sensor and an HC-SR501 passive infrared (PIR) motion detector. The nodes are positioned at known fixed coordinates within a room and publish sensor readings over MQTT at 200 ms intervals. A Rust-based subscriber application collects data from all nodes, applies a weighted statistical sensor fusion model to estimate the object's 2D position, and renders a live terminal dashboard using Ratatui showing per-node status, time-series charts, a spatial position map, and system logs.

---

## System Architecture

```
┌────────────────────────────────────────────────────────────┐
│                      Physical Space                         │
│                                                            │
│   Node-1 ●────────────────────────────── ● Node-2         │
│   (s-001)                                  (s-002)         │
│                          ★ Object                          │
│   Node-3 ●────────────────────────────── ● Node-4         │
│   (s-003)                                  (s-004)         │
└────────────────────────────────────────────────────────────┘
              │        │        │        │
              └────────┴────────┴────────┘
                          WiFi / MQTT
                              │
                   ┌──────────┴──────────┐
                   │   MQTT Broker        │
                   │   (Mosquitto)        │
                   │   192.168.50.100     │
                   └──────────┬──────────┘
                              │
               ┌──────────────▼──────────────┐
               │  Rust Subscriber (Tokio)     │
               │                              │
               │  ┌────────────────────────┐  │
               │  │  AgentStore            │  │
               │  │  (Arc<Mutex<...>>)     │  │
               │  └────────┬───────────────┘  │
               │           │                  │
               │  ┌────────▼───────────────┐  │
               │  │  Sensor Fusion         │  │
               │  │  (weighted barycentric │  │
               │  │   estimation)          │  │
               │  └────────┬───────────────┘  │
               │           │                  │
               │  ┌────────▼───────────────┐  │
               │  │  Ratatui TUI Dashboard │  │
               │  └────────────────────────┘  │
               └──────────────────────────────┘
```

---

## Hardware Components

| Component | Model / Spec | Quantity | Pin Assignment |
|---|---|---|---|
| MCU | ESP32 DevKit | 4 | — |
| Ultrasonic Sensor | HC-SR04 | 4 | TRIG: GPIO 13, ECHO: GPIO 12 |
| PIR Sensor | HC-SR501 | 4 | Signal: GPIO 14 |
| LED (Status) | Built-in | 4 | GPIO 2 |

---

## ESP32 Node Firmware

**`node_firmware.ino`** — Identical firmware flashed to all 4 nodes, differentiated by `CLIENT_ID` and MQTT topics.

### State Machine

```
STATE_CONNECTING_WIFI ──▶ STATE_CONNECTING_MQTT ──▶ STATE_RUNNING
         ▲                         ▲                      │
         └─────────────────────────┴──────────────────────┘
                    (on WiFi loss or MQTT disconnect)
```

### Sensor Fusion Logic (Per-Node)

1. **PIR Debounce** — Requires `PIR_DEBOUNCE_COUNT` (default: 2) consecutive HIGH digital reads before confirming motion. A single LOW immediately resets the counter.
2. **Motion Hold Window** — Once motion is confirmed, the ultrasonic sensor is activated for `MOTION_HOLD_MS` (default: 5000 ms). This prevents spurious distance readings when no object is present.
3. **Ultrasonic Ranging** — HC-SR04 is triggered at `SONIC_DELAY` intervals (400 ms). Uses `pulseIn()` with 30 ms timeout, converting echo duration to distance in cm: `d = duration × 0.0172`.
4. **Window Expiry** — When the motion window closes, distance is cleared to `0.0` to signal inactivity.

### MQTT Communication

| Direction | Topic Pattern | Payload |
|---|---|---|
| Publish | `<node-id>/send` | `{"timestamp": <millis>, "distance": <cm>, "movement": <0\|1>}` |
| Subscribe | `<node-id>/receive` | Control commands (extensible) |

**Publish interval**: 200 ms

### LED Status Patterns

| Pattern | Meaning |
|---|---|
| Slow blink (300 ms) | Searching for WiFi |
| Fast blink (100 ms) | Connecting to MQTT broker |
| Solid ON | Fully operational |
| Short pulse (30 ms) | Data published to broker |

### Disconnect Recovery

The firmware implements automatic reconnection:
- WiFi loss → Transitions to `STATE_CONNECTING_WIFI`, LED slow blink, calls `WiFi.reconnect()`
- MQTT loss (WiFi OK) → Transitions to `STATE_CONNECTING_MQTT`, LED fast blink, retries every 3 seconds

---

## MQTT Broker

A Mosquitto broker running on a local machine at `192.168.50.100:1883`.

```bash
./run.sh
# or: mosquitto -c mosquitto.conf
```

---

## Rust Subscriber — `avant_garde`

The subscriber is a multi-threaded async Rust application built with Tokio.

### Module Structure

| Module | File | Responsibility |
|---|---|---|
| **main** | `main.rs` | Entry point: spawns MQTT and UI tasks, manages shared `AgentStore` |
| **agent** | `agent.rs` | `AgentStore`, `AgentRecord`, `AgentInfo`, `Coordinate` types; per-node state with sample queue |
| **sensor** | `sensor.rs` | `SensorData` struct (deserialized from MQTT JSON: `timestamp`, `distance`, `movement`) |
| **comm** | `comm.rs` | Async MQTT event loop (rumqttc): subscribes to `+/send`, routes payloads to `AgentStore` |
| **localizer** | `localizer.rs` | Statistical position estimation: weighted barycentric sensor fusion model |
| **logger** | `logger.rs` | Thread-safe singleton ring-buffer logger (last 100 entries) |
| **ui** | `ui.rs` | Ratatui TUI: agent list, time-series charts, position canvas, log panel |

### Concurrency Model

```
      main thread              tokio background task
     ─────────────             ──────────────────────
     Ratatui UI loop   ←── Arc<Mutex<AgentStore>> ←── comm.rs MQTT loop
```

The `AgentStore` is protected by `Arc<Mutex<...>>` and shared between the MQTT receiver task (writes) and the Ratatui render loop (reads).

### Agent Lifecycle

- **ACTIVE** — Data received within the last `AGENT_EXPIRATION_TIME` (5000 ms)
- **EXPIRED** — No data received beyond the expiration threshold
- Each agent maintains a circular queue of up to `MAX_QUEUE_LENGTH` (128) sensor samples

---

## Statistical Position Estimation Model

### Node Geometry

Nodes are placed at known fixed coordinates (configurable per deployment):

| Node | ID | X (m) | Y (m) |
|---|---|---|---|
| 1 | s-001 | 0.0 | 0.0 |
| 2 | s-002 | 10.0 | 0.0 |
| 3 | s-003 | 0.0 | 6.0 |
| 4 | s-004 | 10.0 | 6.0 |

### Algorithm

**Step 1 — Active Node Filtering**
Only nodes with `movement == 1` are considered; their ultrasonic readings are valid.

**Step 2 — Circular Intersection**
Each active node `nᵢ` at position `(xᵢ, yᵢ)` defines a circle of radius `dᵢ` (measured distance). The object lies at or near the intersection of these circles.

**Step 3 — Weighted Barycentric Estimation**
```
            Σ (wᵢ · pᵢ)
P_est  =  ──────────────
               Σ wᵢ

where:
  pᵢ  = (xᵢ, yᵢ) + dᵢ · unit_vector_toward_center
  wᵢ  = 1 / (dᵢ² + ε)   (inverse-distance weighting)
```

**Step 4 — Confidence Scoring**
Confidence = (active nodes / total nodes). Position is displayed only when confidence ≥ 0.5 (at least 2 of 4 nodes detect motion).

### Graceful Degradation

| Active Nodes | Output |
|---|---|
| 0 | No estimate |
| 1 | Circle (radius = measured distance from single node) |
| 2 | Arc (intersection locus of two circles) |
| 3+ | Point estimate (weighted intersection) |

---

## Dashboard Controls

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate agent list |
| `p` | Open position input panel for selected agent |
| `Tab` | Switch X ↔ Y input field |
| `Enter` | Confirm coordinate entry |
| `Esc` | Cancel input / close panel |
| `q` | Quit the dashboard |

### Dashboard Layout

```
┌──────────────────┬─────────────────────────────────────┐
│                  │                                      │
│  Agent List      │       Sensor Data Chart              │
│  🟢 s-001 (...)  │  (Distance & Motion over time)       │
│  🟢 s-002 (...)  │                                      │
│  🔴 s-003        ├─────────────────────────────────────┤
│  🟢 s-004 (...)  │                                      │
│                  │       Agent Position Map             │
│                  │  (canvas: dots at (x,y) per node)   │
│                  ├─────────────────────────────────────┤
│                  │       System Logs                    │
│                  │  [INFO] Received data from s-001...  │
└──────────────────┴─────────────────────────────────────┘
```

---

## Setup & Running

### Prerequisites

- **Arduino IDE** with ESP32 board support
- **Rust** (≥1.79) with `cargo`
- **Mosquitto** MQTT broker

### 1. Flash Firmware

1. Open `node_firmware.ino` in Arduino IDE.
2. Set `CLIENT_ID`, `WIFI_SSID`, `WIFI_PASSWORD`, `MQTT_BROKER_ADDRESS` for each node.
3. Flash to each ESP32 board.

### 2. Start MQTT Broker

```bash
cd src/node_firmware
./run.sh
# or: mosquitto -c mosquitto.conf
```

### 3. Run Dashboard

```bash
cd src/avant_garde
cargo run
```

---

## Configuration Reference

| Parameter | Default | Description |
|---|---|---|
| `SONIC_DELAY` | 400 ms | Min interval between ultrasonic readings |
| `PUBLISH_INTERVAL` | 200 ms | MQTT publish rate |
| `RECONNECT_DELAY` | 3000 ms | Time between reconnect attempts |
| `MOTION_HOLD_MS` | 5000 ms | Ultrasonic active window after PIR motion |
| `PIR_DEBOUNCE_COUNT` | 2 | Consecutive HIGH reads for confirmed motion |
| `AGENT_EXPIRATION_TIME` | 5000 ms | Agent marked EXPIRED if no data |
| `MAX_QUEUE_LENGTH` | 128 | Max sensor samples stored per agent |

---

## File Structure

```
week3/
├── README.md                       # This document
├── screenshot.png                  # Dashboard screenshot
├── docs/
│   └── HC SR501 PIR Sensor Datasheet.pdf
└── src/
    ├── node_firmware/              # ESP32 firmware
    │   ├── node_firmware.ino       # Arduino sketch (state machine + sensors)
    │   ├── mosquitto.conf          # MQTT broker configuration
    │   ├── run.sh                  # Broker start script
    │   └── reflash.sh              # Firmware flash helper
    └── avant_garde/                # Rust MQTT subscriber + dashboard
        ├── Cargo.toml
        └── src/
            ├── main.rs             # Entry point, Arc<Mutex<AgentStore>>
            ├── agent.rs            # Per-node state, sample queue
            ├── sensor.rs           # SensorData deserialization
            ├── comm.rs             # Async MQTT event loop
            ├── localizer.rs        # Weighted sensor fusion model
            ├── logger.rs           # Ring-buffer system logger
            └── ui.rs               # Ratatui TUI rendering
```

---

## Dependencies

### Firmware (Arduino)
- `WiFi.h` — ESP32 WiFi
- `MQTTClient.h` — MQTT client
- `ArduinoJson.h` — JSON serialization

### Subscriber (Rust)
- `rumqttc 0.24` — Async MQTT client
- `tokio 1` — Async runtime (full features)
- `serde 1.0` / `serde_json 1.0` — JSON deserialization
- `ratatui 0.30` — Terminal UI framework
- `crossterm 0.28` — Terminal backend

---

## Technical Specifications

| Metric | Value |
|---|---|
| Number of Nodes | 4 |
| MQTT Publish Rate | 200 ms (5 Hz) |
| Ultrasonic Sample Rate | 400 ms (2.5 Hz) |
| PIR Debounce | 2 consecutive HIGH |
| Motion Hold Window | 5 seconds |
| Agent Expiry Timeout | 5 seconds |
| Max Samples per Agent | 128 |
| Position Confidence Threshold | ≥ 0.5 (2/4 nodes) |
| MQTT QoS | 0 (fire-and-forget) |
| Localization Dimensions | 2D (X, Y) |

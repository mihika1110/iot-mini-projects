# IoT Multi-Node Object Localization System

> **Week 3 Mini-Project** — Passive-Infrared + Ultrasonic Sensor Fusion over MQTT with a Rust Terminal Dashboard

---

## Overview

This project builds a real-time object localization system using **4 ESP32 sensor nodes**, each equipped with a PIR (Passive-Infrared) and an HC-SR04 ultrasonic sensor. The nodes are physically arranged in a known geometry inside a room. A **central Rust subscriber** collects MQTT readings from all nodes, applies a statistical sensor-fusion model, and displays an estimated object position on a live terminal dashboard powered by **Ratatui**.

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
                   ┌──────────┴──────────┐
                   │  mqtt_subscriber     │
                   │  (Rust / Tokio)      │
                   │                     │
                   │  ┌───────────────┐  │
                   │  │  AgentStore   │  │
                   │  │  (per-node)   │  │
                   │  └───────┬───────┘  │
                   │          │          │
                   │  ┌───────▼───────┐  │
                   │  │ Sensor Fusion │  │
                   │  │ (statistical  │  │
                   │  │  model)       │  │
                   │  └───────┬───────┘  │
                   │          │          │
                   │  ┌───────▼───────┐  │
                   │  │ Ratatui TUI   │  │
                   │  │  Dashboard    │  │
                   │  └───────────────┘  │
                   └─────────────────────┘
```

---

## Components

### 1. ESP32 Firmware — `esp_mqtt.ino`

Each node runs an identical firmware sketch with a 3-state machine:

```
STATE_CONNECTING_WIFI → STATE_CONNECTING_MQTT → STATE_RUNNING
                             ↑_________________________________|
                             (on WiFi loss or MQTT drop)
```

**Sensors:**

| Sensor | Pin | Purpose |
|---|---|---|
| HC-SR04 TRIG | GPIO 13 | Trigger ultrasonic pulse |
| HC-SR04 ECHO | GPIO 12 | Receive echo pulse |
| PIR | GPIO 14 | Detect passive infrared motion |
| LED | GPIO 2 | Status indicator |

**Sensor Fusion Logic (per-node):**

- The **PIR** is continuously sampled; **2 consecutive HIGH** reads are required to confirm motion (debouncing).
- Ultrasonic ranging is **only activated for 5 seconds** after motion is confirmed, saving power and avoiding spurious readings.
- Once motion hold-window expires, the distance reading is cleared to `0.0`.

**MQTT Payload (published every 200ms):**

```json
{
  "timestamp": 12345,
  "distance": 34.2,
  "movement": 1
}
```

**MQTT Topics:**

| Direction | Topic Pattern |
|---|---|
| Publish (sensor data) | `<node-id>/send` |
| Subscribe (commands) | `<node-id>/receive` |

**LED Status Codes:**

| Pattern | Meaning |
|---|---|
| Slow blink (300ms) | Searching for WiFi |
| Fast blink (100ms) | Connecting to MQTT |
| Solid ON | Fully operational |
| Short pulse (30ms) | Data published to broker |

---

### 2. MQTT Broker — Mosquitto

A Mosquitto broker running on a local machine at `192.168.50.100:1883`. The included `mosquitto.conf` handles configuration. Start it with:

```bash
./run.sh
```

---

### 3. Rust Subscriber — `mqtt_subscriber/`

The subscriber is a multi-threaded async Rust application.

**Structure:**

```
src/
├── main.rs      — Entry point, sets up Arc<Mutex<AgentStore>> and spawns tasks
├── agent.rs     — AgentStore, AgentRecord, AgentInfo, Coordinate types
├── sensor.rs    — SensorData struct (deserialized from MQTT JSON)
├── comm.rs      — Async MQTT event loop, updates AgentStore
├── logger.rs    — Thread-safe singleton logger (ring buffer, last 100 entries)
└── ui.rs        — Ratatui TUI: agent list, charts, position map, log panel
```

**Concurrency model:**

```
       main thread          tokio background task
      ─────────────         ──────────────────────
      Ratatui UI loop   ←── Arc<Mutex<AgentStore>> ←── comm.rs MQTT loop
```

---

## Statistical Position Estimation Model

> **This is the core scientific contribution of the project.**

### Node Geometry

The 4 nodes are placed at known fixed coordinates in the room. Example configuration (1 unit = 1 meter):

```
Node  │ ID    │ X    │ Y
──────┼───────┼──────┼─────
1     │ s-001 │  0.0 │  0.0
2     │ s-002 │ 10.0 │  0.0
3     │ s-003 │  0.0 │  6.0
4     │ s-004 │ 10.0 │  6.0
```

### Data Used Per Node

- **`distance`** (cm): Direct range measurement from ultrasonic sensor.
- **`movement`** (0/1): Binary motion confirmation from PIR (post-debounce).

### Estimation Approach

Each node provides a noisy distance reading `dᵢ` to the object and a binary motion flag `mᵢ`. 

**Step 1 — Filter active nodes:**  
Only nodes with `movement == 1` contribute to position estimation, as their ultrasonic readings are valid.

**Step 2 — Circular intersection (range-based):**  
Each active node `nᵢ` at position `(xᵢ, yᵢ)` defines a circle of radius `dᵢ`. The object lies at the intersection of these circles.

**Step 3 — Weighted least-squares:**  
Since readings are noisy, a weighted barycentric estimate is used:

```
            Σ (wᵢ · pᵢ)
P_est  =  ──────────────
               Σ wᵢ

where:
  pᵢ  = (xᵢ, yᵢ) + dᵢ * unit_vector_toward_center
  wᵢ  = 1 / (dᵢ² + ε)   (inverse-distance weighting)
```

**Step 4 — Confidence score:**  
The confidence is the fraction of active nodes over total nodes. A position is only displayed when confidence ≥ 0.5 (at least 2 of 4 nodes detect motion).

### Future Improvements

- Kalman filter for temporal smoothing of position over multiple samples.
- Angle-of-arrival estimation using multiple echo timings.
- Machine learning model trained on ground-truth positions.

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
- **Rust** (>=1.79) with `cargo`
- **Mosquitto** MQTT broker installed

### 1. Flash Firmware

1. Open `esp_mqtt.ino` in Arduino IDE.
2. Set `CLIENT_ID`, `WIFI_SSID`, `WIFI_PASSWORD`, `MQTT_BROKER_ADDRESS` for each node.
3. Flash to each ESP32 board.

```bash
./reflash.sh  # optional helper script
```

### 2. Start MQTT Broker

```bash
./run.sh
# or: mosquitto -c mosquitto.conf
```

### 3. Run Dashboard

```bash
cd mqtt_subscriber
cargo run
```

---

## Configuration Reference

| Parameter | Default | Description |
|---|---|---|
| `SONIC_DELAY` | 400 ms | Min time between ultrasonic readings |
| `PUBLISH_INTERVAL` | 200 ms | MQTT publish rate |
| `RECONNECT_DELAY` | 3000 ms | Time between reconnect attempts |
| `MOTION_HOLD_MS` | 5000 ms | Ultrasonic active window after PIR |
| `PIR_DEBOUNCE_COUNT` | 2 | Consecutive HIGH reads for confirmed motion |
| `AGENT_EXPIRATION_TIME` | 5000 ms | Agent marked EXPIRED if no data received |
| `MAX_QUEUE_LENGTH` | 128 | Max sensor samples stored per agent |

---

## File Structure

```
esp_mqtt/
├── esp_mqtt.ino           — Arduino firmware for each sensor node
├── mosquitto.conf         — MQTT broker configuration
├── run.sh                 — Broker start script
├── reflash.sh             — Firmware flash helper
└── mqtt_subscriber/       — Rust MQTT subscriber + dashboard
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── agent.rs
        ├── sensor.rs
        ├── comm.rs
        ├── logger.rs
        └── ui.rs
```

---

## Dependencies

### Firmware (Arduino)
- `WiFi.h` — ESP32 WiFi
- `MQTTClient.h` — MQTT client
- `ArduinoJson.h` — JSON serialization

### Subscriber (Rust)
- `rumqttc` — Async MQTT client
- `tokio` — Async runtime
- `serde` / `serde_json` — JSON deserialization
- `ratatui` — Terminal UI
- `crossterm` — Terminal backend

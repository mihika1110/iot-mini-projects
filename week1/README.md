# Week 1 — LoRa Environmental Monitoring System with Predictive Analytics

> End-to-end IoT pipeline: BME680 + MQ-135 + Anemometer → LoRa → Base Station → Flask API → Streamlit Dashboard with Ridge Regression Predictions

---

## Overview

This project implements a complete environmental monitoring system that collects air-quality and weather data from multiple sensors on an ESP32-based LoRa node, transmits the readings wirelessly via LoRa (865 MHz ISM band), receives and decodes them on a Heltec WiFi LoRa 32 base station, persists the data via a Flask REST API, and presents real-time visualizations alongside machine learning predictions on a Streamlit dashboard. A secondary comparison against the OpenWeatherMap API is used to validate sensor accuracy.

---

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Sensor Node (ESP32)                          │
│  BME680 (I²C)  ──┐                                                 │
│  MQ-135 (A/D)  ──┤──▶ Sensor ──▶ Encoder ──▶ Queue ──▶ Framing    │
│  Anemometer    ──┘         (BSEC2)    (delta)          (SOF+CRC)   │
│                                                    │                │
│                                            Transmission (LoRa)     │
└────────────────────────────────────────────────┬────────────────────┘
                                                 │  865 MHz RF
┌────────────────────────────────────────────────▼────────────────────┐
│                   Base Station (Heltec WiFi LoRa 32)                │
│  LoRa RX ──▶ Packet Decoder ──▶ JSON Payload ──▶ WiFi HTTP POST   │
│                  │                                                  │
│             OLED Display                    FreeRTOS API Queue      │
│          (live stats)                     (async, pinned to Core 0) │
└────────────────────────────────────────────────┬────────────────────┘
                                                 │  HTTP POST
┌────────────────────────────────────────────────▼────────────────────┐
│                       Backend Server (RPi / PC)                     │
│                                                                     │
│  ┌──────────────┐    ┌──────────────────┐    ┌──────────────────┐  │
│  │  Flask API   │    │  Data Logger     │    │  Ridge Regressor │  │
│  │  (port 5000) │    │  (serial backup) │    │  (scikit-learn)  │  │
│  └──────┬───────┘    └──────────────────┘    └────────┬─────────┘  │
│         │  CSV/JSONL                                  │             │
│  ┌──────▼───────────────────────────────────────────▼─────────┐    │
│  │                  Streamlit Dashboard                        │    │
│  │  Live Metrics │ Time-Series Plots │ Model Predictions      │    │
│  │               │                   │ Weather API Comparison  │    │
│  └────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Hardware Components

| Component | Model / Spec | Interface | Purpose |
|---|---|---|---|
| MCU (Sensor Node) | ESP32 (Heltec) | — | Sensor acquisition + LoRa TX |
| MCU (Base Station) | Heltec WiFi LoRa 32 | — | LoRa RX + WiFi relay |
| Environmental Sensor | BME680 | I²C (SDA: GPIO 41, SCL: GPIO 42) | Temperature, humidity, pressure, IAQ, CO₂, VOC |
| Air Quality Sensor | MQ-135 | Analog (GPIO 6) + Digital (GPIO 2) | Raw gas concentration / threshold |
| Wind Sensor | Anemometer (analog) | Analog (GPIO 4) | Wind speed proxy |
| Display (Base Station) | SSD1306 OLED (128×64) | I²C | Live packet stats |

---

## Firmware Architecture

The sensor node firmware follows a modular **subsystem** architecture. Each subsystem inherits from a common `Subsystem` base class:

| Subsystem | File | Responsibility |
|---|---|---|
| **Cadence** | `cadence.cpp/h` | Non-blocking timer manager for sensor and transmission intervals |
| **Sensor** | `sensor.cpp/h` | BME680 (via BSEC2 library), MQ-135 (analog+digital), and anemometer |
| **Encoder** | `encoder.cpp/h` | Delta-compression encoder with 25-bit flag field for presence/delta/sign |
| **Queue** | `queue.cpp/h` | Circular buffer for encoded data awaiting transmission |
| **Framing** | `framing.cpp/h` | SOF (0x7E) delimited frames with byte-stuffing (ESC 0x7F), CRC-16, and sequence numbering |
| **Transmission** | `transmission.cpp/h` | LoRa PHY management at 865 MHz, SF7, BW 125 kHz, CR 4/5, 21 dBm TX |

### LoRa Radio Configuration

| Parameter | Value |
|---|---|
| Frequency | 865 MHz (ISM India) |
| TX Power | 21 dBm |
| Bandwidth | 125 kHz (index 0) |
| Spreading Factor | 7 |
| Coding Rate | 4/5 |
| Preamble Length | 8 symbols |

### Packet Protocol (Base Station Decode)

Two packet types are defined with CRC-16/CCITT integrity:

| Packet Type | ID | Size | Contents |
|---|---|---|---|
| ENV (Environmental) | `0x01` | 31 bytes | Temperature, humidity, pressure, IAQ, CO₂, VOC, gas %, stabilization status |
| ANALOG | `0x02` | 16 bytes | MQ-135 raw, anemometer raw |

All multi-byte fields use **big-endian** (network byte order).

---

## Base Station

The base station firmware runs on a Heltec WiFi LoRa 32 V3 and performs:

1. **LoRa Reception** — Continuous RX mode with interrupt-driven packet handling.
2. **Packet Decoding** — Supports both ENV and ANALOG packet types with CRC validation.
3. **WiFi Relay** — Constructs a JSON payload and enqueues it to a FreeRTOS queue.
4. **Async HTTP POST** — A dedicated FreeRTOS task (`apiSenderTask`) pinned to Core 0 dequeues payloads and POSTs them to the Flask API. This decouples radio reception from network I/O.
5. **OLED Dashboard** — Displays live RX count, queue depth, temperature, humidity, IAQ, CO₂, and API response status.

---

## Backend API Server

**`bstation/api_server.py`** — A Flask-based REST API optimized for high-throughput ingestion:

- **Endpoint**: `POST /api/sensor` — Accepts JSON sensor payloads.
- **Endpoint**: `GET /api/stats` — Returns server statistics (received, queued, written, errors).
- **Async Buffered Writer** — A background thread accumulates records in a buffer (100 records or 1-second flush interval) and batch-writes to:
  - `bsec_data.csv` — BSEC environmental data
  - `analog_data.csv` — Analog sensor data
  - `sensor_data.jsonl` — Raw JSON Lines backup

### Data Logger (Serial Fallback)

**`bstation/data_logger.py`** — Reads JSON directly from the base station's serial port as a fallback data path. Features checkpointing for crash recovery.

---

## Machine Learning Models

### Ridge Regressor (Primary — Server-side)

A `MultiOutputRegressor(Ridge)` model trained on 19 engineered features to predict the **next** reading of 4 targets:

| Target | Description |
|---|---|
| `temp_target` | Next temperature (°C) |
| `pres_target` | Next pressure (Pa) |
| `hum_target` | Next humidity (%) |
| `iaq_target` | Next IAQ index (0–500) |

**Feature Engineering:**
- Lag features: `lag1_*`, `lag2_*` for temperature, pressure, humidity, IAQ
- Time-aware EWMA: Using `τ = 15 min` decay constant
- Rolling mean: 15-minute window
- Temporal context: `delta_t`, `lag1_time_diff`, `lag2_time_diff`

### LSTM (Experimental — Edge Deployment)

A TensorFlow Lite LSTM model (`model.tflite`) compiled to a C byte array (`model_data.cc`) for on-device inference on the base station ESP32. Predicts the same 4 targets from a sequence of sensor readings.

---

## Streamlit Dashboard

**`dashboard.py`** — A real-time monitoring dashboard with:

- **Live Metrics Panel** — Configurable via `config.yaml` (temperature, humidity, pressure, IAQ, CO₂, VOC, MQ-135, anemometer).
- **Time-Series Charts** — Interactive Plotly charts with selectable time ranges (1h, 6h, 24h, 7d, all).
- **Model Predictions** — Displays computed feature vector and predicted next readings from the Ridge Regressor.
- **Weather API Comparison** — Fetches real-time data from OpenWeatherMap (free tier) and computes MAE for sensor validation.
- **Device Filtering** — Multi-device support with per-device views.
- **Auto-Refresh** — Optional 2-second polling mode.
- **Dark Theme** — Custom CSS with `#0b0f1a` background, styled metrics, and Plotly chart theming.

---

## Configuration

### `config.yaml`
```yaml
bsec_file: bstation/data/bsec_data.csv
analog_file: bstation/data/analog_data.csv

header:
  - temperature
  - humidity
  - pressure
  - iaq
  - co2_ppm
  - voc_ppm
  - mq135_raw
  - anemometer_raw

plots:
  - temperature
  - humidity
  - pressure
  - iaq
```

### `.env`
```
OPENWEATHER_API_KEY=<your_key>
```

---

## Getting Started

### Prerequisites
- Arduino IDE with Heltec ESP32 board support
- Python 3.8+ with `pip`
- BSEC2 Arduino library (Bosch)

### 1. Flash Sensor Node
```bash
cd firmware
./run.sh   # or use Arduino IDE
```

### 2. Flash Base Station
```bash
cd bstation/firmware
./run.sh
```

### 3. Start API Server
```bash
cd bstation
pip install flask
python api_server.py
```

### 4. Train Model (optional)
```bash
cd model/ridge_regressor
python train.py
```

### 5. Launch Dashboard
```bash
pip install -r requirements.txt
streamlit run dashboard.py
```

---

## File Structure

```
week1/
├── firmware/                    # Sensor node firmware (ESP32)
│   ├── firmware.ino             # Main Arduino sketch
│   ├── meta.h                   # Shared data structures
│   └── subsystems/              # Modular subsystem implementations
│       ├── cadence.cpp/h        # Timer management
│       ├── sensor.cpp/h         # BME680 + MQ-135 + Anemometer
│       ├── encoder.cpp/h        # Delta compression encoder
│       ├── decoder.cpp/h        # Decoder (base station side)
│       ├── queue.cpp/h          # Circular buffer
│       ├── framing.cpp/h       # SOF/CRC framing
│       └── transmission.cpp/h  # LoRa PHY
├── bstation/                    # Base station
│   ├── firmware/                # Heltec WiFi LoRa 32 firmware
│   │   ├── firmware.ino         # Receiver + WiFi relay + OLED
│   │   ├── packet.cpp/h         # Packet encode/decode (CRC-16)
│   │   ├── display.cpp/h        # SSD1306 OLED driver
│   │   ├── lora_config.h        # LoRa radio parameters
│   │   └── model.cc/h           # TFLite LSTM (edge inference)
│   ├── api_server.py            # Flask REST API (async buffered)
│   ├── data_logger.py           # Serial data logger (fallback)
│   └── data/                    # Persisted CSV + JSONL
├── model/                       # Machine learning models
│   ├── ridge_regressor/         # Server-side Ridge Regressor
│   │   ├── train.py             # Training script
│   │   ├── model.py             # Inference wrapper
│   │   ├── util.py              # Time-aware EWMA utility
│   │   └── model.bin            # Serialized model (joblib)
│   └── lstm/                    # Edge LSTM model
│       ├── model.ipynb          # Training notebook
│       ├── model.tflite         # TFLite model
│       ├── model_data.cc/h      # C byte array for ESP32
│       └── model_test.cc        # ESP32 inference test
├── dashboard.py                 # Streamlit real-time dashboard
├── model_example.py             # Standalone model inference example
├── config.yaml                  # Dashboard configuration
├── requirements.txt             # Python dependencies
└── .env                         # API keys
```

---

## Dependencies

### Firmware (Arduino)
- `bsec2` — Bosch BSEC2 library for BME680
- `LoRaWan_APP` — Heltec LoRa library
- `WiFi` — ESP32 WiFi
- `HTTPClient` — HTTP POST client
- `ArduinoJson` — JSON serialization (base station)
- `FreeRTOS` — Task management (base station)

### Python
- `streamlit ≥ 1.28.0`
- `pandas ≥ 2.0.0`
- `plotly ≥ 5.17.0`
- `flask`
- `pyserial`
- `scikit-learn`
- `joblib`
- `numpy`
- `pyyaml`
- `python-dotenv`
- `requests`

---

## Technical Specifications

| Metric | Value |
|---|---|
| Sensor Sampling Interval | 1000 ms |
| LoRa Transmission Interval | 10000 ms |
| API Ingestion Buffer | 100 records / 1s flush |
| HTTP Timeout (Base Station) | 2000 ms |
| API Queue Depth (Base Station) | 50 packets |
| Model Feature Dimensionality | 19 |
| Model Target Dimensionality | 4 |
| Dashboard Refresh Rate | 2s (optional) |

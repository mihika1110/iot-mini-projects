# Week 2 — Modbus RS-485 Soil Sensor Service with ML-Based Soil Classification

> 9-in-1 Soil Sensor → Modbus RTU → Raspberry Pi (C++ Daemon) → Named Pipe → Flask API + CSV Logger + Random Forest Classifier

---

## Overview

This project implements a complete soil health monitoring pipeline that interfaces with an industrial 9-in-1 soil sensor over the Modbus RTU protocol via RS-485. A C++ daemon running on a Raspberry Pi continuously polls the sensor holding registers, decodes the raw readings, and streams them through a Unix named pipe (FIFO). A Python companion process reads from the pipe, persists data to CSV, and exposes the latest readings through a Flask REST API. A machine learning model (Random Forest Classifier) trained on the collected data classifies soil condition into four categories: Saline, Balanced, Dry, or Acidic.

---

## System Architecture

```
┌───────────────────────────────────────────────────────────┐
│               9-in-1 Soil Sensor (Modbus Slave)           │
│  Moisture │ Temperature │ EC │ pH │ N │ P │ K │ Sal │ TDS │
└──────────────────────────┬────────────────────────────────┘
                           │  RS-485 (4800 baud, 8N1)
                           │
┌──────────────────────────▼────────────────────────────────┐
│              Raspberry Pi (Modbus Master)                  │
│                                                            │
│  ┌─────────────────┐      ┌─────────────────────────┐     │
│  │  C++ Daemon      │      │  Python Reader (pread)  │     │
│  │  (ModbusMaster)  │      │                         │     │
│  │                  │      │  ┌────────────────────┐ │     │
│  │  Read Holding    │─FIFO─▶│  │  CSV Logger        │ │     │
│  │  Registers       │      │  └────────────────────┘ │     │
│  │  (0x0000..0x0008)│      │  ┌────────────────────┐ │     │
│  │                  │      │  │  Flask API          │ │     │
│  └─────────────────┘      │  │  GET /latest        │ │     │
│                            │  └────────────────────┘ │     │
│                            └─────────────────────────┘     │
│                                                            │
│  ┌─────────────────────────────────────────────────────┐  │
│  │  ML Classifier (Random Forest)                       │  │
│  │  Input: moisture, ec, ph, salinity, tds              │  │
│  │  Output: Saline │ Balanced │ Dry │ Acidic            │  │
│  └─────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
```

---

## Hardware Components

| Component | Specification | Interface |
|---|---|---|
| Soil Sensor | 9-in-1 (Modbus RTU) | RS-485 via USB-TTL adapter |
| Single Board Computer | Raspberry Pi | USB (`/dev/ttyUSB0`) |

### Sensor Measurements

| Register | Parameter | Unit | Scaling |
|---|---|---|---|
| 0x0000 | Moisture | % | ÷ 10 |
| 0x0001 | Temperature | °C | ÷ 10 |
| 0x0002 | Electrical Conductivity (EC) | µS/cm | raw |
| 0x0003 | pH | — | ÷ 10 |
| 0x0004 | Nitrogen (N) | mg/kg | raw |
| 0x0005 | Phosphorus (P) | mg/kg | raw |
| 0x0006 | Potassium (K) | mg/kg | raw |
| 0x0007 | Salinity | ppt | ÷ 10 |
| 0x0008 | Total Dissolved Solids (TDS) | ppm | raw |

---

## C++ Modbus Master Daemon

### Architecture

The daemon is built around a ported `ModbusMaster` library (originally Arduino-based, adapted for Linux POSIX serial I/O). Key adaptations:

1. **`Stream` Class** (`stream.cpp`) — A Linux-native serial I/O abstraction replacing Arduino's `Stream`:
   - Opens `/dev/ttyUSB0` with `O_RDWR | O_NOCTTY | O_NONBLOCK`
   - Configures via `termios`: 4800 baud, 8N1, no flow control, raw mode
   - Implements `select()`-based non-blocking read with configurable timeout
   - Uses a ring buffer for incoming data

2. **`ModbusMaster` Class** (`ModbusMaster.cpp/h`) — Full Modbus RTU implementation:
   - Function codes: Read coils (0x01), Read discrete inputs (0x02), Read holding registers (0x03), Read input registers (0x04), Write single coil/register (0x05/0x06), Write multiple (0x0F/0x10), Mask write (0x16), Read-write multiple (0x17)
   - CRC-16 validation on all transactions
   - 2-second response timeout

3. **Utility Headers** (`util/`):
   - `crc16.h` — Modbus CRC-16 computation
   - `ring-buffer.h` — Generic ring buffer implementation
   - `millis.h` — POSIX `clock_gettime` substitute for Arduino `millis()`
   - `bitwrite.h`, `byte.h`, `word.h` — Bit/byte manipulation macros

### Data Flow

```
main.cpp loop:
  1. readHoldingRegisters(0x0000, 9)  →  Modbus RTU request frame
  2. Parse 9 response registers       →  Response_t struct
  3. dprintf(pipe_fd, CSV_LINE)        →  Named pipe (soil.sock)
  4. sleep(100ms)                      →  Next poll cycle
```

The daemon writes CSV-formatted lines to a Unix named pipe (`soil.sock`), using `O_WRONLY | O_NONBLOCK` to avoid blocking when no reader is connected. `SIGPIPE` is explicitly ignored to handle reader disconnections gracefully.

---

## Python Data Reader & API

**`pread.py`** — A Flask application that:

1. **FIFO Reader Thread** — Opens `soil.sock`, parses CSV lines, validates field count, and:
   - Appends rows to `soil_data.csv` with immediate flush
   - Maintains a `deque(maxlen=5)` of latest readings in memory

2. **REST Endpoint** — `GET /latest` returns the 5 most recent readings as JSON.

---

## Machine Learning — Soil Classification

### Model

A **Random Forest Classifier** trained in Google Colab (`Final_soil_training.ipynb`), serialized with `joblib`:

| File | Description |
|---|---|
| `soil_model.pkl` | Trained Random Forest model (~5 MB) |
| `scaler.pkl` | Feature scaler (StandardScaler) |

### Features & Classes

| Feature | Description |
|---|---|
| `moisture` | Soil moisture (%) |
| `ec` | Electrical conductivity (µS/cm) |
| `ph` | Soil pH |
| `salinity` | Salinity (ppt) |
| `tds` | Total dissolved solids (ppm) |

| Class | Label |
|---|---|
| 0 | Saline Soil |
| 1 | Balanced Soil |
| 2 | Dry Soil |
| 3 | Acidic Soil |

### Inference

```python
import joblib, pandas as pd

model = joblib.load("soil_model.pkl")
scaler = joblib.load("scaler.pkl")

sensor_input = pd.DataFrame([{
    "moisture": 35, "ec": 1.2, "ph": 6.8,
    "salinity": 0.3, "tds": 600,
}])

prediction = model.predict(sensor_input)
# → "Balanced Soil"
```

---

## Deployment as systemd Service

The project includes a `systemd` service unit for automatic startup:

### `soil.service`
```ini
[Unit]
Description=Soil Sensor Startup Script
After=network.target

[Service]
Type=simple
ExecStart=/opt/soil/run.sh
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

### Installation

```bash
cd modbus-service
sudo ./install.sh
```

This script:
1. Builds the C++ binary via `make`
2. Generates `run.sh` (launches both C++ daemon and Python reader)
3. Copies to `/opt/soil/`
4. Installs and enables the systemd service

---

## Build & Run

### Prerequisites
- Linux (Raspberry Pi OS / Debian)
- `g++` with C++11 support
- Python 3 with `flask`
- RS-485 USB adapter connected as `/dev/ttyUSB0`

### Manual Build
```bash
cd modbus-service
make clean && make
```

### Manual Run
```bash
./main 1>out.log 2>err.log &
python3 pread.py
```

---

## File Structure

```
week2/
├── modbus-service/
│   ├── main.cpp                 # C++ Modbus polling daemon
│   ├── ModbusMaster.cpp/h       # Modbus RTU master (ported from Arduino)
│   ├── stream.cpp               # POSIX serial I/O (Linux Stream)
│   ├── stream.h                 # Stream header (stub)
│   ├── Makefile                 # Build system (g++)
│   ├── pread.py                 # Python FIFO reader + Flask API
│   ├── install.sh               # Deployment installer
│   ├── run.sh                   # Runtime launcher
│   ├── soil.service             # systemd service unit
│   ├── soil_data.csv            # Collected sensor data
│   ├── util/                    # Utility headers
│   │   ├── stream.h             # Stream class definition
│   │   ├── crc16.h              # Modbus CRC-16
│   │   ├── ring-buffer.h        # Generic ring buffer
│   │   ├── millis.h             # POSIX millis() replacement
│   │   ├── bitwrite.h           # Bit manipulation
│   │   ├── byte.h               # Byte extraction
│   │   └── word.h               # Word manipulation
│   ├── machine_learning/        # ML model artifacts
│   │   ├── Final_soil_training.ipynb  # Training notebook
│   │   ├── DatasetScript.ipynb        # Dataset preparation
│   │   ├── model_final_soil.py        # Inference script
│   │   ├── soil_model.pkl             # Trained Random Forest
│   │   ├── scaler.pkl                 # Feature scaler
│   │   └── soil_data.csv             # Training data
│   └── data/                    # Runtime data directory
└── drive/                       # (Google Drive mount - data sync)
```

---

## Communication Protocol

### Modbus RTU Frame Format

```
┌──────────┬───────────────┬────────────┬─────────┐
│ Slave ID │ Function Code │   Data     │ CRC-16  │
│ (1 byte) │   (1 byte)    │ (N bytes)  │ (2 byte)│
└──────────┴───────────────┴────────────┴─────────┘
```

- **Baud Rate**: 4800
- **Data Bits**: 8
- **Parity**: None
- **Stop Bits**: 1
- **Flow Control**: None
- **Response Timeout**: 2000 ms

---

## Technical Specifications

| Metric | Value |
|---|---|
| Polling Interval | 100 ms |
| Serial Baud Rate | 4800 |
| Modbus Slave ID | 1 |
| Registers Read | 9 (0x0000–0x0008) |
| IPC Mechanism | Unix Named Pipe (FIFO) |
| API Port | 5000 |
| In-Memory Buffer | 5 readings (deque) |
| ML Model | Random Forest Classifier (4 classes) |
| Service Restart Delay | 3 seconds |

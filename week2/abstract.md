# Abstract

## Modbus RTU-Based Soil Health Monitoring Pipeline with Automated Classification

This project presents the design and implementation of an automated soil health monitoring system that bridges industrial-grade Modbus RTU sensors with modern data analytics and machine learning classification. The system addresses the need for continuous, automated soil analysis in agricultural and environmental monitoring contexts where manual sampling is impractical and existing IoT solutions lack support for legacy industrial protocols.

### Problem Statement

Soil health assessment traditionally requires laboratory analysis or periodic manual measurements, introducing significant delays between data collection and actionable insights. Industrial-grade multi-parameter soil sensors communicate via Modbus RTU over RS-485, a protocol well-suited for harsh field environments but lacking native integration with modern cloud or edge computing pipelines. There is a need for a lightweight, always-on data acquisition daemon that bridges the RS-485 physical layer to IP-accessible analytics while also providing automated soil classification.

### Approach

The system is structured into three tightly coupled components:

1. **Data Acquisition Layer** — A C++ daemon running on a Raspberry Pi implements a full Modbus RTU master, ported from the Arduino `ModbusMaster` library to POSIX-compliant serial I/O. The daemon polls 9 holding registers from a multi-parameter soil sensor (measuring moisture, temperature, electrical conductivity, pH, nitrogen, phosphorus, potassium, salinity, and total dissolved solids) at 100 ms intervals over a 4800-baud RS-485 link. The serial abstraction layer uses `select()` for non-blocking reads with configurable timeouts, and a ring buffer for reliable byte-level processing.

2. **Data Streaming & Persistence Layer** — The daemon writes CSV-formatted readings to a Unix named pipe (FIFO), implementing `SIGPIPE` tolerance and non-blocking writes for resilient operation when no consumer is attached. A companion Python process (`pread.py`) reads from the FIFO, validates and parses each line, persists records to a timestamped CSV file with immediate flush, and maintains a rolling in-memory cache of the 5 most recent readings. A Flask REST API (`GET /latest`) exposes this cache for integration with external dashboards or monitoring systems.

3. **Machine Learning Classification Layer** — A Random Forest Classifier trained on historical soil data classifies each reading into one of four agronomically significant soil categories: **Saline**, **Balanced**, **Dry**, or **Acidic**. The model uses five engineered features (moisture, EC, pH, salinity, TDS) and is serialized with `joblib` alongside a separate `StandardScaler` for feature normalization. Training is performed in Google Colab; inference runs locally on the Raspberry Pi.

The entire pipeline is packaged as a `systemd` service with automatic restart, enabling unattended deployment in field conditions.

### Key Results

- Continuous, sub-second soil parameter polling with reliable Modbus RTU communication over RS-485.
- Clean decoupling of data acquisition (C++) and analytics (Python) via Unix FIFO, allowing independent process lifecycle management.
- The Random Forest Classifier provides four-class soil type categorization from five chemical/physical parameters.
- Deployment as a systemd service enables autonomous operation on standard Raspberry Pi hardware.
- The REST API enables real-time integration with external monitoring dashboards and alerting systems.

### Technologies Used

- **Hardware**: Raspberry Pi, 9-in-1 Modbus soil sensor, RS-485 USB adapter
- **Data Acquisition**: C++ (Modbus RTU master, POSIX serial I/O, `select()`, ring buffer)
- **IPC**: Unix named pipe (FIFO)
- **API**: Python (Flask)
- **Machine Learning**: scikit-learn (Random Forest Classifier), joblib, pandas
- **Deployment**: systemd, Make
- **Protocols**: Modbus RTU, RS-485 (4800 baud, 8N1)

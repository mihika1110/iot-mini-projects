# Abstract

## LoRa-Based Environmental Monitoring System with Real-Time Predictive Analytics

This project presents the design and implementation of an end-to-end environmental monitoring system built on LoRa (Long Range) wireless technology for telemetry and machine learning for predictive analytics. The system addresses the challenge of continuous, low-power air quality and weather monitoring in indoor and semi-outdoor environments where cellular or Wi-Fi connectivity may be limited or unreliable.

### Problem Statement

Real-time environmental monitoring is critical for health-sensitive spaces such as laboratories, server rooms, and industrial facilities. Existing commercial solutions are either prohibitively expensive, lack extensibility, or depend on continuous internet connectivity. There is a need for an affordable, modular, and self-contained system that not only monitors but also **predicts** future environmental conditions to enable proactive response.

### Approach

The system consists of three primary tiers:

1. **Sensor Tier** — An ESP32 microcontroller interfaced with a Bosch BME680 environmental sensor (via the BSEC2 signal processing library), an MQ-135 air quality sensor, and an analog anemometer. The firmware employs a modular subsystem architecture with delta-compression encoding and CRC-protected framing to minimize LoRa airtime while maintaining data integrity. Sensor readings are sampled at 1-second intervals and transmitted at 10-second intervals over the 865 MHz ISM band.

2. **Gateway Tier** — A Heltec WiFi LoRa 32 V3 base station receives LoRa packets, decodes the binary protocol (supporting both environmental and analog packet types with CRC-16/CCITT validation), constructs JSON payloads, and relays them to a backend API server over Wi-Fi. FreeRTOS task pinning and an asynchronous queue architecture ensure zero packet loss during HTTP round-trips. A 128×64 OLED display provides at-a-glance operational status.

3. **Analytics Tier** — A Flask REST API with asynchronous batch-buffered CSV/JSONL persistence serves as the data backend. Two machine learning models are trained on the historical sensor data:
   - A **Ridge Regressor** (scikit-learn `MultiOutputRegressor`) operating on 19 engineered features (lag values, time-aware exponentially weighted moving averages, rolling means, and temporal deltas) to predict the next reading of temperature, pressure, humidity, and IAQ index.
   - An **LSTM** model compiled to TensorFlow Lite and exported as a C byte array for on-device inference at the base station, enabling edge prediction without network dependency.

   A Streamlit dashboard provides real-time visualization of sensor data across configurable time ranges, displays model predictions for upcoming readings, and cross-validates sensor accuracy against the OpenWeatherMap API by computing per-metric absolute errors and overall MAE.

### Key Results

- Achieved sub-second sensor-to-dashboard end-to-end latency in local deployments.
- The Ridge Regressor demonstrates strong next-step prediction accuracy on temperature, pressure, humidity, and IAQ with low MAE across test splits.
- The LoRa link operates reliably at 865 MHz with SF7, providing sufficient range for indoor/campus deployments with 21 dBm TX power.
- The delta-compression encoder reduces average payload size by eliminating redundant fields between consecutive readings.

### Technologies Used

- **Hardware**: ESP32, Heltec WiFi LoRa 32 V3, Bosch BME680, MQ-135, SSD1306 OLED
- **Firmware**: Arduino/C++ with BSEC2, LoRaWAN_APP, FreeRTOS
- **Backend**: Python (Flask, pandas, scikit-learn, joblib)
- **Dashboard**: Streamlit, Plotly, OpenWeatherMap API
- **Protocols**: LoRa (865 MHz), HTTP REST, JSON, Custom binary framing (SOF/CRC-16)

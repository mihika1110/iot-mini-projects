# Abstract

## Multi-Node Indoor Object Localization Using PIR and Ultrasonic Sensor Fusion over MQTT

This project presents a real-time indoor object localization system built on a distributed network of four ESP32 sensor nodes communicating over MQTT, with a central Rust-based subscriber performing weighted statistical sensor fusion and rendering live position estimates on a terminal dashboard.

### Problem Statement

Indoor localization remains a challenging problem, particularly in environments where GPS signals are unavailable and infrastructure-heavy solutions (UWB anchors, camera arrays) are cost-prohibitive. Low-cost ultrasonic and motion sensors are widely available but individually insufficient for accurate positioning — ultrasonic sensors suffer from reflections and limited angular coverage, while PIR sensors provide only binary motion detection without distance or direction information. A fusion approach that leverages the complementary strengths of both sensor types across multiple spatially distributed nodes can yield useful position estimates at minimal hardware cost.

### Approach

The system comprises three layers:

1. **Sensor Nodes (4× ESP32)** — Each node is equipped with an HC-SR04 ultrasonic ranging sensor and an HC-SR501 passive infrared motion detector. The firmware implements a 3-state machine (WiFi connecting → MQTT connecting → Running) with automatic reconnection and LED-based status indication. A power-efficient sensing strategy is employed: the PIR sensor is continuously sampled with a 2-read debounce filter, and the ultrasonic sensor is activated for a 5-second window only upon confirmed motion detection. This avoids wasted ultrasonic cycles when no object is present. Sensor data (distance and motion flag) is published as JSON to per-node MQTT topics at 200 ms intervals.

2. **Communication Layer** — A Mosquitto MQTT broker running on a local server facilitates pub/sub communication. Each node publishes to `<node-id>/send` and subscribes to `<node-id>/receive` for extensible command reception. The lightweight JSON payload (`timestamp`, `distance`, `movement`) enables low-latency processing.

3. **Subscriber & Localization Engine (Rust)** — A multi-threaded asynchronous Rust application built on Tokio subscribes to all node topics via the `rumqttc` MQTT client. Incoming sensor data is deserialized and stored in a shared `AgentStore` protected by `Arc<Mutex<...>>`. The localization module (`localizer.rs`) implements a weighted barycentric estimation algorithm:
   - Only nodes with confirmed motion contribute to position estimation.
   - Each active node defines a circle of radius equal to its measured distance, centered at its known fixed coordinates.
   - An inverse-distance-squared weighting scheme prioritizes closer, more reliable readings.
   - The system degrades gracefully from point estimates (≥3 nodes) to arc constraints (2 nodes) to circle bounds (1 node).
   - A confidence metric (fraction of active nodes) gates the display of position estimates.

   A Ratatui-based terminal dashboard renders the agent list with online/expired status, time-series distance and motion charts, a 2D spatial position map showing node locations and estimated object position, and a scrollable system log.

### Key Results

- Real-time object position estimation with sub-second latency from sensor triggering to dashboard display.
- The PIR debounce + motion hold window strategy effectively eliminates false-positive ultrasonic readings, significantly reducing noise in the position estimate.
- The weighted barycentric model provides reasonable position estimates when ≥2 nodes observe the same object, with accuracy bounded by the ultrasonic sensor's precision (~2 cm at short range).
- Graceful degradation maintains useful output even with partial node coverage or node failures.
- The async Rust architecture (Tokio) handles concurrent 5 Hz data streams from 4 nodes without observable latency.

### Technologies Used

- **Hardware**: ESP32 DevKit, HC-SR04 ultrasonic sensor, HC-SR501 PIR sensor
- **Firmware**: Arduino/C++ with WiFi, MQTTClient, ArduinoJson
- **Communication**: MQTT (Mosquitto broker, QoS 0)
- **Subscriber**: Rust (Tokio, rumqttc, serde_json)
- **Visualization**: Ratatui, crossterm
- **Localization**: Weighted barycentric estimation with inverse-distance² weighting

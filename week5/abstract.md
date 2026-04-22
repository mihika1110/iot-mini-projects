# Abstract

## WhyBlue: Server-Authoritative Hybrid Wi-Fi/Bluetooth Networking Daemon with Autonomous Transport Switching

This project presents the design and implementation of WhyBlue, a hybrid dual-transport networking daemon built in Rust that provides seamless, autonomous switching between Wi-Fi and Bluetooth communication channels based on real-time link quality assessment. The system addresses the fundamental challenge of maintaining reliable wireless connectivity in mobile or spatially distributed IoT deployments where no single transport can guarantee consistent availability.

### Problem Statement

In local-area IoT networks (e.g., between Raspberry Pi nodes in a building), single-transport reliance creates a single point of failure: Wi-Fi may degrade due to interference, distance, or congestion, while Bluetooth offers shorter range but often superior reliability in close proximity. Naive dual-transport approaches suffer from the "ping-pong" effect — rapid, oscillatory transport switching when both links are near their quality boundaries — and from split-brain scenarios where nodes disagree on the active transport, causing packet loss and state inconsistency. There is a need for a principled, server-authoritative protocol that coordinates transport transitions across all participants while maintaining application-layer transparency.

### Approach

WhyBlue establishes a logical networking layer that abstracts Wi-Fi (UDP over `wlan0`) and Bluetooth Personal Area Network (UDP over BNEP/`br0`) into a single session managed by a custom binary protocol. The system is structured into three Rust crates within a Cargo workspace:

1. **`whyblue-core`** — The shared library implementing:
   - A **custom binary protocol** with a `WbHeader` supporting session tracking, sequence numbering, traffic classification (Interactive/Bulk/Control), transport tagging, and a `FLAG_HANDOVER` bitmask for control plane multiplexing.
   - A **composite metric scoring algorithm** that evaluates each transport on a [0.0, 1.0] scale across four weighted dimensions: latency, RSSI stability (jitter), packet loss rate, and proximity (bounded RSSI linear conversion). Scores are smoothed via exponential moving average.
   - A **server-authoritative FSM** where only the designated Server node evaluates transport health and initiates switching. The Client node operates as an obedient state-follower, mutating its active transport exclusively upon receiving a `SwitchPrepare` command. Clients may request switches via a `SwitchRequest` protocol that the Server validates before executing.
   - A **SessionEngine** implementing make-before-break handover: during a configurable overlap window (~2 seconds), every outgoing packet is dual-emitted across both Wi-Fi and Bluetooth UDP sockets simultaneously. Receivers listen on both sockets concurrently and deduplicate based on sequence numbers, achieving zero-packet-drop transport transitions.

2. **`whyblued`** — The async Tokio daemon responsible for:
   - Binding UDP sockets to specific interfaces via `SO_BINDTODEVICE` (requiring `CAP_NET_ADMIN`)
   - Establishing the Bluetooth PAN infrastructure (Server: NAP via `br0` bridge + BlueZ D-Bus; Client: PANU via `bnep0`)
   - Spawning independent tasks for Wi-Fi RX, Bluetooth RX, TX dispatch, metrics polling (reading `/proc/net/wireless` and D-Bus RSSI), FSM evaluation (Server only), and IPC service
   - Shared state coordination via `Arc<RwLock<StateManager>>`

3. **`whyblue-tui`** — A Ratatui terminal dashboard communicating via Unix Domain Socket IPC to the running daemon. Provides real-time visualization of active transport, composite scores, RSSI, latency, and packet statistics. Includes a text-streaming (chat) interface and user-initiated transport switching.

### Key Results

- Achieved transparent, zero-packet-drop transport handovers via the make-before-break dual-emission strategy, verified through sustained text streaming during switching events.
- The server-authoritative FSM architecture eliminates ping-pong switching and split-brain state conflicts observed in symmetric peer-to-peer approaches.
- The composite metric scoring system (combining latency, stability, loss, and proximity) provides more stable switching decisions than raw RSSI thresholds alone.
- The `SwitchRequest` protocol enables client-side user interaction while preserving server authority over the network state.
- The system runs as a standard Linux daemon on Raspberry Pi hardware with no specialized networking equipment, requiring only standard BlueZ and Wi-Fi tools.

### Technologies Used

- **Language**: Rust (2021 edition)
- **Async Runtime**: Tokio (full features)
- **Hardware**: Raspberry Pi (server + client nodes)
- **Transports**: Wi-Fi (UDP, `wlan0`), Bluetooth PAN (BNEP over `br0`/`bnep0`)
- **Bluetooth Stack**: BlueZ (via D-Bus and `bt-network`)
- **Protocol**: Custom binary framing with serde-serialized control messages
- **UI**: Ratatui + crossterm
- **IPC**: Unix Domain Socket (JSON RPC)
- **System**: SO_BINDTODEVICE, `/proc/net/wireless`, systemd-compatible

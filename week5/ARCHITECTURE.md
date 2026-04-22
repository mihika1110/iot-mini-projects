# WhyBlue Technical Architecture & Design

WhyBlue is a decentralized, hybrid dual-transport wireless networking suite built in Rust. It abstracts Wi-Fi and Bluetooth Personal Area Networks (PAN) into a single, highly-available logical link. By continually probing and evaluating link metrics, the system can seamlessly handover active streaming traffic from a degrading interface to a more optimal one.

---

## 1. System Topology & Workspaces

The codebase is split into a monolithic workspace containing three primary crates:

1. **`whyblue-core` (Library Crate):** Contains the domain logic, protocol serialization, metric scoring algorithms, and the core `StateManager`. It defines the interfaces that physical transports must implement.
2. **`whyblued` (Daemon Crate):** The Tokio-driven async daemon that binds to raw hardware sockets, spins up background tasks, and manages the lifecycle of the network. It requires root privileges (`CAP_NET_ADMIN`) to bind to specific network interfaces and parse `/proc/net/wireless`.
3. **`whyblue-tui` (UI Crate):** A Ratatui-based terminal dashboard that communicates via Unix Domain Sockets (IPC) to the running daemon. It offers telemetry visualization and application-level features like text-streaming (chat).

---

## 2. Server-Authoritative State Management

To prevent split-brain scenarios where both nodes disagree on the active transport (the "ping-pong" effect), WhyBlue establishes a hierarchical **Server-Authoritative** architecture.

- **Role Definitions**: Nodes are explicitly configured as either `Server` or `Client`.
  - In a localized network, the Server typically hosts the Bluetooth NAP (Network Access Point), while the Client connects as a PANU.
- **FSM Isolation**: The automated Finite State Machine (FSM) evaluation loop *only* executes on the Server. 
- **Command Proxies**: If a Client desires a transport change (e.g., a user forces a switch via the TUI), it generates a `HandoverMsg::SwitchRequest` rather than mutating its local state. The Server intercepts this request, validates that the target transport is alive, executes the state mutation locally, and replies with a definitive `SwitchPrepare` command.
- **Obedient Clients**: The Client node operates as an obedient state-follower. Its `ActiveTransport` state mutates solely upon receiving a finalized `SwitchPrepare` packet originating from the Server.

---

## 3. The Protocol & Packet Structure

WhyBlue uses a custom, lightweight binary protocol. Data is structured into Frames, parsed explicitly to support control tagging and multiplexing.

### 3.1 Frame Header (`WbHeader`)
Each packet begins with an explicitly sized header:
```rust
struct WbHeader {
    session_id: u32,
    seq: u32,
    class: TrafficClass,
    transport: WbTransport,
    payload_len: u16,
    flags: u8,
}
```
- **class**: Maps packets to `Interactive`, `Bulk`, or `Control` priorities.
- **flags**: Bitmask operations allow packets to be tagged dynamically. Crucially, `FLAG_HANDOVER` (0x02) designates packets that carry transport-switching control logic (bypassing normal application routing).

### 3.2 Handover Protocol (`HandoverMsg`)
When FSM limits are pushed, control structures are serialized and injected into the stream via SERDE:
- `SwitchRequest`: Emitted by a Client to politely ask the Server to switch.
- `SwitchPrepare`: Broadcast by the Server to explicitly execute a state mutation on all connected nodes.

---

## 4. Internal Daemon Concurrency (Task Model)

The daemon (`whyblued`) utilizes Tokio to heavily parallelize its operations using `sync::Arc` and `RwLock` primitives.

1. **RX Tasks (Wi-Fi & Bluetooth)**: Independent `tokio::spawn` loops continuously poll their respective UDP sockets. When a frame arrives:
   - Statistics (bytes received) are incremented.
   - Headers are decoded. If `FLAG_HANDOVER` is detected, the payload is intercepted and routed to internal state mutation logic.
   - Otherwise, text payloads map to the `StateSnapshot::chat_log` array.
2. **TX Task**: Rather than transports sending data natively, the `SessionEngine` manages an asynchronous MPSC queue. Payloads entering the `tx_queue` are built into `WbHeader` frames. 
   - **Make-before-break routing**: During transitioning states, the TX Task intentionally duplicates and blasts the exact same frame across *both* Wi-Fi and Bluetooth to ensure absolute zero-packet-drop handovers.
3. **Metrics Polling Task**: Wakes dynamically (default 100ms) to read `/proc/net/wireless` for Wi-Fi RSSI and probes DBus for Bluetooth RSSI. It aggregates latency, drop rate, and signal strength via an Exponential Moving Average (EMA).
4. **FSM Evaluation Task (Server Only)**: Reads the computed values from the Metrics Task and feeds them into `transition::evaluate()`. If a new primary transport is selected, it triggers state mutation and kicks off the `SwitchPrepare` cascade.
5. **IPC Task**: Serves a local Unix Domain Socket at `/tmp/whyblue.sock`. Listens for structured JSON RPC requests from the TUI (`GetStatus`, `ForceTransport`, `SendTestMessage`).

---

## 5. Composite Metric Scoring Algorithm

Instead of relying purely on RSSI logic (which is wildly unstable in mobile environments), the system calculates a 0.0 to 1.0 floating-point score based on weighted categories:

```rust
Score = (Latency * Weight_Lat) 
      + (Stability * Weight_Stab)
      + (Loss * Weight_Loss)
      + (Proximity * Weight_Prox)
```

- **Latency**: Derived from internal packet tracking and probe response times.
- **Stability**: A jitter modifier heavily penalizing radical swings in continuous RSSI values.
- **Proximity**: A bounded linear conversion of the raw RSSI dbM value.

When the Active Transport's composite score plummets below a configurable `bad_threshold` (e.g., 0.35) for a sustained `hysteresis` period, the FSM formally declares the link unhealthy and initiates a handover sequence to the Standby link.

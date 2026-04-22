# WhyBlue Network Architecture & Interface Lifecycle

This document provides a low-level overview of how WhyBlue programmatically interacts with Linux networking stacks, creates logical interfaces, and manages the lifecycle of the underlying physical transports.

---

## 1. Network Topology Overview

WhyBlue treats both Wi-Fi and Bluetooth as active "pipes." However, because Bluetooth does not natively provide IP-based socket routing out of the box in the same way a Wi-Fi dongle does, WhyBlue establishes an IP abstraction layer over Bluetooth using **Bluetooth Personal Area Network (PAN)** profiles.

Once established, WhyBlue functionally bonds both transports using standard UDP sockets. A custom routing engine (the `SessionEngine`) handles multiplexing application data over one or both UDP sockets depending on the current state.

---

## 2. Interface Creation & Socket Binding

### 2.1 Wi-Fi Transport Setup

The Wi-Fi interface operates directly on top of the native WPA-supplicant managed stack.

1. **UDP Bind**: The daemon opens a standard UDP socket (bound to configurable ports, default `9876`).
2. **Device Isolation (`SO_BINDTODEVICE`)**: To prevent the Linux kernel from accidentally routing Wi-Fi packets over the Bluetooth interface (if standard IP subnets were ever to overlap or default paths get confused), the daemon uses the `libc::setsockopt` system call with the `SO_BINDTODEVICE` flag. 
   - *Requires `CAP_NET_ADMIN` privileges.*
   - Forces the raw file descriptor of the UDP socket to explicitly map exclusively to the `wlan0` hardware interface.
3. **Telemetry Polling**: Wi-Fi RSSI (signal strength) is not easily fetchable via standard sockets. The daemon launches an async task to continuously parse the raw outputs of `/proc/net/wireless` targeting the `wlan0` prefix to acquire dbM signal data.

### 2.2 Bluetooth Transport Setup (BNEP & PAN)

Bluetooth setup is significantly more complex because WhyBlue converts the raw Bluetooth hardware into an IP-routable network interface (BNEP).

#### **Server Role (NAP - Network Access Point)**
When configured as a Server, the daemon:
1. Provisions a virtual Linux bridge interface (`br0`) using `ip link add`.
2. Assigns a static IP gateway (`10.0.0.1/24`) to the bridge.
3. Attempts to execute `bt-network -s nap br0` (from `bluez-tools`).
4. *Fallback*: If `bt-network` is unavailable, it issues a raw D-Bus system call to `org.bluez.NetworkServer1.Register` passing `nap` and `br0` to the BlueZ daemon.
5. This exposes a Bluetooth Network server where remote Bluetooth clients can tether, effectively creating an IP local area network over radio frequencies.

#### **Client Role (PANU - PAN User)**
When configured as a Client, the daemon:
1. Programmatically probes `bluetoothctl info` to ensure the Server MAC address is Trusted and Paired.
2. Executes `dbus-send` to call `org.bluez.Network1.Connect` with the `nap` profile argument targeting the Server's MAC address.
3. This creates a `bnep0` network interface upon successful connection.
4. It manually bridges an IP (`10.0.0.2/24`) onto `bnep0` using `ip addr add` so that UDP sockets can bind to it.

Once the interface (`br0` on the server, `bnep0` on the client) is actively IP-routable, a secondary UDP Socket is bound to Port `9877`. Like the Wi-Fi stack, it is bonded securely via `SO_BINDTODEVICE` targeting the exact `bnep` or `br` interface name.

---

## 3. The Switching Mechanism (Data-Link Layer)

WhyBlue performs **Layer 4 (UDP) Application Switching**, bypassing the slower Linux kernel routing tables altogether.

### The "SessionEngine" Router
The `SessionEngine` manages the TX dispatch channel. It doesn't modify Linux `route` or `iptables` rules when a handover occurs. Instead, it maintains persistent `.send()` streams open to both the Wi-Fi UDP Socket and the BT UDP Socket simultaneously.

When the `StateManager` computes a transport transition:
1. **Selection**: It flips the active pointer (e.g., from `WbTransport::Wifi` to `WbTransport::Bluetooth`).
2. **Make-before-break Active Phase**: For the duration of configured overlap time (e.g., 2000ms), the `SessionEngine` takes arriving application packets, packages them, and emits `socket.send()` across *both* physical UDP pipes simultaneously.
3. **Commit Phase**: After the overlap timer expires, the Engine drops the old transport from the `targets` array, funneling all bytes purely into the newly elected hardware socket.

Because the receivers on the opposite node are listening to *both* UDP sockets simultaneously in separate `tokio::spawn` loops, whichever packet arrives first gets decoded, and duplicates are cleanly dropped using the `seq_num` property inside the `WbHeader`.

This results in a completely seamless, zero-drop transport swap.

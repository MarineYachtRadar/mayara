# Mayara Architecture

> This document describes the architecture of the Mayara radar system,
> showing what is shared between deployment modes and the path to maximum code reuse.

---

## FUNDAMENTAL PRINCIPLE: mayara-core is the Single Source of Truth

**This is the most important architectural concept in Mayara.**

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        mayara-core (THE DATABASE)                            │
│                                                                              │
│   Contains ALL knowledge about radars:                                       │
│   - Model database (ranges, spokes, capabilities per model)                  │
│   - Control definitions (what controls exist, their types, min/max, units)   │
│   - Protocol specifications (wire format, parsing, command dispatch)         │
│   - Feature flags (doppler, dual-range, no-transmit zones, etc.)            │
│   - Connection state machine (platform-independent)                          │
│   - I/O abstraction (IoProvider trait)                                      │
│   - RadarLocator (discovery logic)                                          │
│                                                                              │
│   THIS IS THE ONLY PLACE WHERE RADAR LOGIC IS DEFINED.                      │
│   SERVER AND WASM ARE THIN I/O ADAPTERS AROUND CORE.                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ adapters implement IoProvider
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           I/O Provider Layer                                 │
│                                                                              │
│  ┌─────────────────────────┐          ┌─────────────────────────┐           │
│  │    TokioIoProvider      │          │     WasmIoProvider      │           │
│  │    (mayara-server)      │          │  (mayara-signalk-wasm)  │           │
│  │                         │          │                         │           │
│  │  Wraps tokio sockets    │          │  Wraps SignalK FFI      │           │
│  │  in poll-based API      │          │  socket calls           │           │
│  └─────────────────────────┘          └─────────────────────────┘           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ exposes via
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           REST API (SignalK-compatible)                      │
│                                                                              │
│   GET /radars/{id}/capabilities    ← Returns model info from mayara-core    │
│   GET /radars/{id}/state           ← Current control values                 │
│   PUT /radars/{id}/controls/{id}   ← Set control values                     │
│                                                                              │
│   The API is the CONTRACT. All clients use ONLY the API.                    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ consumed by
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              ALL CLIENTS                                     │
│                                                                              │
│   - WebGUI (mayara-gui/)           - Reads /capabilities to know what       │
│   - mayara-server internal logic     controls to display                    │
│   - Future: mayara_opencpn         - Dynamically builds UI from API         │
│   - Future: mobile apps            - NEVER hardcodes radar capabilities     │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### What This Means in Practice

1. **mayara-core defines everything:**
   - All radar models and their specifications
   - All control types (gain, sea, rain, dopplerMode, etc.)
   - Valid ranges per model
   - Available features per model
   - Wire protocol encoding/decoding
   - **Command dispatch** (control ID → wire command)
   - **Connection state machine** (Disconnected → Connecting → Connected → Active)

2. **mayara-server and mayara-signalk-wasm are thin adapters:**
   - Implement `IoProvider` trait for their platform
   - Run the **same** RadarLocator code from mayara-core
   - Use the **same** dispatch functions for control commands
   - No hardcoded control names, range tables, or protocol details

3. **The REST API is the contract:**
   - `/capabilities` returns what the radar can do (from mayara-core)
   - Clients build their UI dynamically from this response
   - Same WebGUI works for ANY radar brand because it follows the API

4. **Adding a new control:**
   - Add definition to `mayara-core/capabilities/controls.rs`
   - Add dispatch entry in `mayara-core/protocol/{brand}/dispatch.rs`
   - Add to model's control list in `mayara-core/models/{brand}.rs`
   - **Server and WASM automatically pick it up - no changes needed!**

---

## Current Crate Structure (December 2025)

```
mayara/
├── mayara-core/                    # Platform-independent radar library
│   └── src/
│       ├── lib.rs                  # Re-exports: Brand, IoProvider, RadarLocator, controllers, etc.
│       ├── io.rs                   # IoProvider trait (UDP/TCP abstraction)
│       ├── locator.rs              # RadarLocator (multi-brand discovery)
│       ├── connection.rs           # ConnectionState, ConnectionManager, furuno login
│       ├── state.rs                # RadarState, PowerState (control values)
│       ├── brand.rs                # Brand enum (Furuno, Navico, Raymarine, Garmin)
│       ├── radar.rs                # RadarDiscovery struct
│       ├── error.rs                # ParseError type
│       ├── dual_range.rs           # Dual-range controller logic
│       │
│       ├── controllers/            # ★ UNIFIED BRAND CONTROLLERS ★
│       │   ├── mod.rs              # Re-exports all controllers
│       │   ├── furuno.rs           # FurunoController (TCP login + commands)
│       │   ├── navico.rs           # NavicoController (UDP multicast)
│       │   ├── raymarine.rs        # RaymarineController (Quantum/RD)
│       │   └── garmin.rs           # GarminController (UDP)
│       │
│       ├── protocol/               # Wire protocol (encoding/decoding)
│       │   ├── furuno/
│       │   │   ├── mod.rs          # Beacon parsing, spoke parsing, constants
│       │   │   ├── command.rs      # Format functions (format_gain_command, etc.)
│       │   │   ├── dispatch.rs     # Control dispatch (ID → wire command)
│       │   │   └── report.rs       # TCP response parsing
│       │   ├── navico.rs           # Navico protocol
│       │   ├── raymarine.rs        # Raymarine protocol
│       │   └── garmin.rs           # Garmin protocol
│       │
│       ├── models/                 # Radar model database
│       │   ├── furuno.rs           # DRS4D-NXT, DRS6A-NXT, etc. (ranges, controls)
│       │   ├── navico.rs           # HALO, 4G, 3G, BR24
│       │   ├── raymarine.rs        # Quantum, RD series
│       │   └── garmin.rs           # xHD series
│       │
│       ├── capabilities/           # Control definitions
│       │   ├── controls.rs         # 40+ control definitions (gain, sea, dopplerMode...)
│       │   └── builder.rs          # Capability manifest builder
│       │
│       ├── arpa/                   # ARPA target tracking
│       │   ├── detector.rs         # Contour detection
│       │   ├── tracker.rs          # Kalman filter tracking
│       │   ├── cpa.rs              # CPA/TCPA calculation
│       │   └── ...
│       │
│       ├── trails/                 # Target trail history
│       └── guard_zones/            # Guard zone alerting
│
├── mayara-server/                  # Standalone native server
│   └── src/
│       ├── main.rs                 # Entry point, tokio runtime
│       ├── lib.rs                  # Session, Cli, VERSION exports
│       ├── tokio_io.rs             # TokioIoProvider (implements IoProvider)
│       ├── core_locator.rs         # CoreLocatorAdapter (wraps mayara-core RadarLocator)
│       ├── locator.rs              # Legacy platform-specific locator
│       ├── web.rs                  # Axum HTTP/WebSocket handlers
│       ├── settings.rs             # Control factory using mayara-core definitions
│       ├── storage.rs              # Local applicationData storage
│       ├── navdata.rs              # NMEA/SignalK navigation input
│       │
│       └── brand/                  # Brand-specific controllers
│           ├── furuno/             # Furuno TCP controller
│           ├── navico/             # Navico UDP controller
│           ├── raymarine/          # Raymarine controller
│           └── garmin/             # Garmin controller
│
├── mayara-signalk-wasm/            # SignalK WASM plugin
│   └── src/
│       ├── lib.rs                  # WASM entry point, plugin exports
│       ├── wasm_io.rs              # WasmIoProvider (implements IoProvider)
│       ├── locator.rs              # Re-exports RadarLocator from mayara-core
│       ├── radar_provider.rs       # RadarProvider (uses controllers from mayara-core)
│       ├── spoke_receiver.rs       # UDP spoke data receiver
│       └── signalk_ffi.rs          # SignalK FFI bindings
│
└── mayara-gui/                     # Shared web GUI assets
    ├── index.html
    ├── viewer.html
    ├── control.html
    ├── api.js                      # Auto-detects SignalK vs Standalone
    └── ...
```

---

## The IoProvider Architecture

**Key Insight:** Both WASM and Server use the **exact same** radar logic from mayara-core.
The only difference is how sockets are implemented.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           mayara-core                                        │
│                    (Pure Rust, no I/O, WASM-compatible)                      │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       IoProvider Trait                               │    │
│  │  (mayara-core/io.rs)                                                 │    │
│  │                                                                      │    │
│  │  trait IoProvider {                                                  │    │
│  │      // UDP: create, bind, broadcast, multicast, send, recv, close   │    │
│  │      // TCP: create, connect, send, recv_line, recv_raw, close       │    │
│  │      // Utility: current_time_ms(), debug()                          │    │
│  │  }                                                                   │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       RadarLocator                                   │    │
│  │  (mayara-core/locator.rs)                                           │    │
│  │                                                                      │    │
│  │  - Multi-brand discovery (Furuno, Navico, Raymarine, Garmin)         │    │
│  │  - Beacon packet construction                                        │    │
│  │  - Multicast group management                                        │    │
│  │  - Radar identification and deduplication                            │    │
│  │                                                                      │    │
│  │  Uses IoProvider for all I/O:                                        │    │
│  │    fn start<I: IoProvider>(&mut self, io: &mut I)                    │    │
│  │    fn poll<I: IoProvider>(&mut self, io: &mut I) -> Vec<Discovery>   │    │
│  │    fn shutdown<I: IoProvider>(&mut self, io: &mut I)                 │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       ConnectionManager                              │    │
│  │  (mayara-core/connection.rs)                                         │    │
│  │                                                                      │    │
│  │  - ConnectionState enum (Disconnected → Connected → Active)          │    │
│  │  - Exponential backoff logic (1s, 2s, 4s, 8s, max 30s)              │    │
│  │  - Furuno login protocol constants and parsing                       │    │
│  │  - ReceiveSocketType (multicast/broadcast fallback)                  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       Dispatch Functions                             │    │
│  │  (mayara-core/protocol/furuno/dispatch.rs)                          │    │
│  │                                                                      │    │
│  │  - format_control_command(id, value, auto) → wire command            │    │
│  │  - format_request_command(id) → request command                      │    │
│  │  - parse_control_response(line) → ControlUpdate enum                 │    │
│  │                                                                      │    │
│  │  Controllers call dispatch, not individual format functions!         │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       Unified Brand Controllers                      │    │
│  │  (mayara-core/controllers/)                                         │    │
│  │                                                                      │    │
│  │  FurunoController   - TCP login + command, uses dispatch functions   │    │
│  │  NavicoController   - UDP multicast, BR24/3G/4G/HALO support        │    │
│  │  RaymarineController - UDP, Quantum (solid-state) / RD (magnetron)  │    │
│  │  GarminController   - UDP multicast, xHD series                     │    │
│  │                                                                      │    │
│  │  All controllers use IoProvider for I/O:                            │    │
│  │    fn poll<I: IoProvider>(&mut self, io: &mut I) -> bool            │    │
│  │    fn set_gain<I: IoProvider>(&mut self, io: &mut I, value, auto)   │    │
│  │    fn shutdown<I: IoProvider>(&mut self, io: &mut I)                │    │
│  │                                                                      │    │
│  │  SAME CODE runs on both server (tokio) and WASM (FFI)!              │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┴───────────────┐
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │      TokioIoProvider       │    │      WasmIoProvider        │
     │   (mayara-server)          │    │   (mayara-signalk-wasm)    │
     │                            │    │                            │
     │   impl IoProvider for      │    │   impl IoProvider for      │
     │   TokioIoProvider {        │    │   WasmIoProvider {         │
     │     fn udp_create() {      │    │     fn udp_create() {      │
     │       socket2::Socket::new │    │       sk_udp_create()      │
     │       tokio::UdpSocket     │    │     }                      │
     │     }                      │    │     fn udp_send_to() {     │
     │     fn udp_recv_from() {   │    │       sk_udp_send()        │
     │       socket.try_recv_from │    │     }                      │
     │     }                      │    │   }                        │
     │   }                        │    │                            │
     └────────────────────────────┘    └────────────────────────────┘
```

### Server's CoreLocatorAdapter

The server wraps mayara-core's sync RadarLocator in an async adapter:

```rust
// mayara-server/src/core_locator.rs

pub struct CoreLocatorAdapter {
    locator: RadarLocator,       // from mayara-core (sync)
    io: TokioIoProvider,         // platform I/O adapter
    discovery_tx: mpsc::Sender<LocatorMessage>,
    poll_interval: Duration,     // default: 100ms
}

impl CoreLocatorAdapter {
    pub async fn run(mut self, subsys: SubsystemHandle) -> Result<...> {
        self.locator.start(&mut self.io);  // Same code as WASM!

        loop {
            select! {
                _ = subsys.on_shutdown_requested() => break,
                _ = poll_timer.tick() => {
                    let discoveries = self.locator.poll(&mut self.io);  // Same!
                    for d in discoveries {
                        self.discovery_tx.send(LocatorMessage::RadarDiscovered(d)).await;
                    }
                }
            }
        }
        self.locator.shutdown(&mut self.io);
    }
}
```

---

## Implementation Status (December 2025)

### ✅ Fully Implemented

| Component | Location | Notes |
|-----------|----------|-------|
| **Protocol parsing** | mayara-core/protocol/ | All 4 brands: Furuno, Navico, Raymarine, Garmin |
| **Model database** | mayara-core/models/ | All models with ranges, spokes, capabilities |
| **Control definitions** | mayara-core/capabilities/ | 40+ controls (v5 API) |
| **IoProvider trait** | mayara-core/io.rs | Platform-independent I/O abstraction |
| **RadarLocator** | mayara-core/locator.rs | Multi-brand discovery via IoProvider |
| **ConnectionManager** | mayara-core/connection.rs | State machine, backoff, Furuno login |
| **RadarState types** | mayara-core/state.rs | Control values, update_from_response() |
| **Dispatch functions** | mayara-core/protocol/furuno/dispatch.rs | Control ID → wire command routing |
| **Unified Controllers** | mayara-core/controllers/ | Furuno, Navico, Raymarine, Garmin (all brands!) |
| **ARPA tracking** | mayara-core/arpa/ | Kalman filter, CPA/TCPA, contour detection |
| **Trails history** | mayara-core/trails/ | Target position storage |
| **Guard zones** | mayara-core/guard_zones/ | Zone alerting logic |
| **TokioIoProvider** | mayara-server/tokio_io.rs | Tokio sockets implementing IoProvider |
| **CoreLocatorAdapter** | mayara-server/core_locator.rs | Async wrapper for RadarLocator |
| **WasmIoProvider** | mayara-signalk-wasm/wasm_io.rs | SignalK FFI implementing IoProvider |
| **SignalK WASM plugin** | mayara-signalk-wasm/ | Working with Furuno |
| **Standalone server** | mayara-server/ | Full functionality |
| **Web GUI** | mayara-gui/ | Shared between WASM and Standalone |
| **Local storage API** | mayara-server/storage.rs | SignalK-compatible applicationData |

### 🚧 In Progress / Partial

| Component | Location | Status |
|-----------|----------|--------|
| Raymarine support | mayara-server/brand/raymarine/ | Partial (untested) |
| Garmin support | mayara-server/brand/garmin/ | Stub only |

### ❌ Not Yet Implemented

| Component | Notes |
|-----------|-------|
| mayara_opencpn plugin | OpenCPN integration (see Future section) |
| SignalK Provider Mode | Standalone → SignalK provider registration |

---

## Deployment Modes

### Mode 1: SignalK WASM Plugin

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    SignalK Server (Node.js)                                  │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │              WASM Runtime (wasmer)                                      │ │
│  │  ┌──────────────────────────────────────────────────────────────────┐  │ │
│  │  │         mayara-signalk-wasm                                       │  │ │
│  │  │                                                                   │  │ │
│  │  │  ┌──────────────────┐  ┌───────────────────────────────────────┐ │  │ │
│  │  │  │  WasmIoProvider  │  │   RadarLocator (from mayara-core)     │ │  │ │
│  │  │  │  (FFI sockets)   │──│   SAME CODE AS SERVER                 │ │  │ │
│  │  │  └──────────────────┘  └───────────────────────────────────────┘ │  │ │
│  │  │                                                                   │  │ │
│  │  │  ┌──────────────────────────────────────────────────────────┐    │  │ │
│  │  │  │         Unified Controllers (from mayara-core)            │    │  │ │
│  │  │  │  FurunoController   │ NavicoController   (SAME CODE!)     │    │  │ │
│  │  │  │  RaymarineController│ GarminController   (AS SERVER!)     │    │  │ │
│  │  │  └──────────────────────────────────────────────────────────┘    │  │ │
│  │  └──────────────────────────────────────────────────────────────────┘  │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Characteristics:**
- Runs inside SignalK's WASM sandbox
- Uses SignalK FFI for all network I/O via WasmIoProvider
- Poll-based (no async runtime in WASM)
- **Same RadarLocator AND Controllers as server** (all 4 brands!)

### Mode 2: Standalone Server

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    mayara-server (Rust)                                      │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                     CoreLocatorAdapter                               │    │
│  │  ┌──────────────────┐  ┌───────────────────────────────────────┐    │    │
│  │  │  TokioIoProvider │  │   RadarLocator (from mayara-core)     │    │    │
│  │  │  (tokio sockets) │──│   SAME CODE AS WASM                   │    │    │
│  │  └──────────────────┘  └───────────────────────────────────────┘    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │   Brand Controllers (can use mayara-core/controllers/ OR brand/)     │    │
│  │   - Unified controllers from mayara-core (FurunoController, etc.)    │    │
│  │   - OR async wrappers in brand/ that use core's controllers          │    │
│  │   - TokioIoProvider implements IoProvider for I/O                    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │              Axum Router (web.rs)                                    │    │
│  │   /radars/*, /targets/*, static files (rust-embed from mayara-gui/) │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Characteristics:**
- Native Rust binary with tokio async runtime
- Direct network I/O via TokioIoProvider
- Axum web server hosts API + GUI
- **Same RadarLocator AND Controllers as WASM** (from mayara-core)
- **Same API paths as SignalK** → same GUI works unchanged

---

## What Gets Shared

| Component | Location | WASM | Server | Notes |
|-----------|----------|:----:|:------:|-------|
| **Protocol parsing** | mayara-core/protocol/ | ✓ | ✓ | Packet encode/decode |
| **Model database** | mayara-core/models/ | ✓ | ✓ | Ranges, capabilities |
| **Control definitions** | mayara-core/capabilities/ | ✓ | ✓ | v5 API schemas |
| **IoProvider trait** | mayara-core/io.rs | ✓ | ✓ | Socket abstraction |
| **RadarLocator** | mayara-core/locator.rs | ✓ | ✓ | **Same discovery code!** |
| **Unified Controllers** | mayara-core/controllers/ | ✓ | ✓ | **ALL 4 brands!** |
| **ConnectionManager** | mayara-core/connection.rs | ✓ | ✓ | State machine, backoff |
| **Dispatch functions** | mayara-core/protocol/furuno/dispatch.rs | ✓ | ✓ | Control routing |
| **RadarState** | mayara-core/state.rs | ✓ | ✓ | update_from_response() |
| **ARPA** | mayara-core/arpa/ | ✓ | ✓ | Target tracking |
| **Trails** | mayara-core/trails/ | ✓ | ✓ | Position history |
| **Guard zones** | mayara-core/guard_zones/ | ✓ | ✓ | Alerting logic |
| **Web GUI** | mayara-gui/ | ✓ | ✓ | Shared assets |

**What's platform-specific:**
- TokioIoProvider (mayara-server) - wraps tokio sockets
- WasmIoProvider (mayara-signalk-wasm) - wraps SignalK FFI
- Axum web server (mayara-server only)
- Spoke data receivers (async in server, poll-based in WASM)

---

## Unified Controllers Architecture

The most significant architectural advancement is the **unified controller system** in `mayara-core/controllers/`. This eliminates code duplication between server and WASM, ensuring identical behavior across platforms.

### Controller Design Principles

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      Controller Design Pattern                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  1. Poll-based (not async) → works in WASM without runtime                  │
│  2. IoProvider abstraction → no direct socket calls                         │
│  3. State machine → handles connect/disconnect/reconnect                    │
│  4. Brand-specific protocol → TCP (Furuno) or UDP (Navico/Raymarine/Garmin) │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                      Controller Interface                               │ │
│  │                                                                         │ │
│  │  fn new(radar_id, address, ...) -> Self                                │ │
│  │  fn poll<I: IoProvider>(&mut self, io: &mut I) -> bool                 │ │
│  │  fn is_connected(&self) -> bool                                        │ │
│  │  fn state(&self) -> ControllerState                                    │ │
│  │                                                                         │ │
│  │  // Control setters (all take IoProvider)                              │ │
│  │  fn set_power<I: IoProvider>(&mut self, io: &mut I, transmit: bool)    │ │
│  │  fn set_range<I: IoProvider>(&mut self, io: &mut I, meters: u32)       │ │
│  │  fn set_gain<I: IoProvider>(&mut self, io: &mut I, value: u32, auto)   │ │
│  │  fn set_sea<I: IoProvider>(&mut self, io: &mut I, value: u32, auto)    │ │
│  │  fn set_rain<I: IoProvider>(&mut self, io: &mut I, value: u32, auto)   │ │
│  │  ...                                                                    │ │
│  │                                                                         │ │
│  │  fn shutdown<I: IoProvider>(&mut self, io: &mut I)                     │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Controller State Machines

Each controller manages its own connection state:

```
                    ┌──────────────┐
                    │ Disconnected │ ◄──────────────────────────────┐
                    └──────┬───────┘                                │
                           │ poll() creates sockets                 │
                           ▼                                        │
                    ┌──────────────┐                                │
                    │  Listening   │  (UDP: waiting for reports)    │
                    │  Connecting  │  (TCP: waiting for connect)    │
                    └──────┬───────┘                                │
                           │ reports received / TCP connected       │
                           ▼                                        │
                    ┌──────────────┐                                │
                    │  Connected   │  (ready for commands)          │
                    └──────┬───────┘                                │
                           │ connection lost / timeout              │
                           └────────────────────────────────────────┘
```

### Brand-Specific Details

| Brand | Protocol | Connection | Special Features |
|-------|----------|------------|------------------|
| **Furuno** | TCP | Login sequence (root) | NXT Doppler modes, ~30 controls |
| **Navico** | UDP multicast | Report multicast join | BR24/3G/4G/HALO, Doppler (HALO) |
| **Raymarine** | UDP | Report multicast | Quantum (solid-state) vs RD (magnetron) |
| **Garmin** | UDP multicast | Report multicast | xHD series, simple protocol |

### Usage Example (WASM)

```rust
// mayara-signalk-wasm/src/radar_provider.rs

use mayara_core::controllers::{
    FurunoController, NavicoController, RaymarineController, GarminController,
};
use mayara_core::Brand;

struct RadarProvider {
    io: WasmIoProvider,
    furuno_controllers: BTreeMap<String, FurunoController>,
    navico_controllers: BTreeMap<String, NavicoController>,
    raymarine_controllers: BTreeMap<String, RaymarineController>,
    garmin_controllers: BTreeMap<String, GarminController>,
}

impl RadarProvider {
    fn poll(&mut self) {
        // Poll all controllers - same code regardless of platform!
        for controller in self.furuno_controllers.values_mut() {
            controller.poll(&mut self.io);
        }
        for controller in self.navico_controllers.values_mut() {
            controller.poll(&mut self.io);
        }
        // ... etc
    }

    fn set_gain(&mut self, radar_id: &str, value: u32, auto: bool) {
        if let Some(c) = self.furuno_controllers.get_mut(radar_id) {
            c.set_gain(&mut self.io, value, auto);
        } else if let Some(c) = self.navico_controllers.get_mut(radar_id) {
            c.set_gain(&mut self.io, value, auto);
        }
        // ... etc
    }
}
```

### Benefits of Unified Controllers

| Benefit | Description |
|---------|-------------|
| **Single source of truth** | Fix bugs once, fixed everywhere |
| **Consistent behavior** | WASM and server behave identically |
| **Easier testing** | Mock IoProvider for unit tests |
| **Reduced code size** | ~1500 lines shared vs ~3000 lines duplicated |
| **Faster feature development** | Add control to core, works on all platforms |

---

## Adding a New Feature: The Workflow

### Example: Adding a New Control (e.g., "pulseWidth")

**Step 1: Add control definition (mayara-core)**
```rust
// mayara-core/src/capabilities/controls.rs
pub fn control_pulse_width() -> ControlDefinition {
    ControlDefinition {
        id: "pulseWidth",
        name: "Pulse Width",
        control_type: ControlType::Number,
        min: Some(0.0),
        max: Some(3.0),
        ...
    }
}
```

**Step 2: Add to model capabilities (mayara-core)**
```rust
// mayara-core/src/models/furuno.rs
static CONTROLS_NXT: &[&str] = &[
    "beamSharpening", "dopplerMode", ...,
    "pulseWidth",  // ← Add here
];
```

**Step 3: Add dispatch entry (mayara-core)**
```rust
// mayara-core/src/protocol/furuno/dispatch.rs
pub fn format_control_command(control_id: &str, value: i32, auto: bool) -> Option<String> {
    match control_id {
        ...
        "pulseWidth" => Some(format_pulse_width_command(value)),  // ← Add here
        _ => None,
    }
}
```

**Step 4: Done!**
- Server automatically uses new dispatch entry
- WASM automatically uses new dispatch entry
- GUI automatically shows control (reads from /capabilities)
- No server code changes needed!

---

## Architecture Diagram: Full Picture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              mayara-core                                     │
│                    (Pure Rust, no I/O, WASM-compatible)                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  protocol/    │ │   models/     │ │ capabilities/ │ │   state.rs    │   │
│  │  - furuno/    │ │ - furuno.rs   │ │ - controls.rs │ │   RadarState  │   │
│  │    - dispatch │ │ - navico.rs   │ │ - builder.rs  │ │   PowerState  │   │
│  │    - command  │ │ - raymarine   │ │               │ │               │   │
│  │    - report   │ │ - garmin.rs   │ │               │ │               │   │
│  │  - navico.rs  │ │               │ │               │ │               │   │
│  │  - raymarine  │ │               │ │               │ │               │   │
│  │  - garmin.rs  │ │               │ │               │ │               │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  io.rs        │ │ locator.rs    │ │ connection.rs │ │  arpa/        │   │
│  │  IoProvider   │ │ RadarLocator  │ │ ConnManager   │ │  trails/      │   │
│  │  trait        │ │ (discovery)   │ │ ConnState     │ │  guard_zones/ │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    controllers/  (★ UNIFIED ★)                       │   │
│  │   FurunoController │ NavicoController │ RaymarineController │ Garmin │   │
│  │   (TCP login)      │ (UDP multicast)  │ (Quantum/RD)        │ (UDP)  │   │
│  │                                                                      │   │
│  │   ALL controllers use IoProvider - SAME code on server AND WASM!    │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┴───────────────┐
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │   mayara-signalk-wasm      │    │       mayara-server        │
     │      (WASM + FFI)          │    │    (Native + tokio)        │
     ├────────────────────────────┤    ├────────────────────────────┤
     │                            │    │                            │
     │  wasm_io.rs:               │    │  tokio_io.rs:              │
     │  - WasmIoProvider          │    │  - TokioIoProvider         │
     │  - impl IoProvider         │    │  - impl IoProvider         │
     │                            │    │                            │
     │  locator.rs:               │    │  core_locator.rs:          │
     │  - Re-exports RadarLocator │    │  - CoreLocatorAdapter      │
     │    from mayara-core        │    │  - Wraps RadarLocator      │
     │                            │    │                            │
     │  radar_provider.rs:        │    │  brand/:                   │
     │  - Uses controllers from   │    │  - Can use core controllers│
     │    mayara-core directly!   │    │    with TokioIoProvider    │
     │  - FurunoController        │    │  - OR async wrappers       │
     │  - NavicoController        │    │                            │
     │  - RaymarineController     │    │  web.rs:                   │
     │  - GarminController        │    │  - Axum handlers           │
     │                            │    │                            │
     │  signalk_ffi.rs:           │    │  storage.rs:               │
     │  - FFI bindings            │    │  - Local applicationData   │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │     SignalK Server         │    │     Axum HTTP Server       │
     │                            │    │                            │
     │  Routes /radars/* to       │    │  /radars/*  (same API!)    │
     │  WASM RadarProvider        │    │  Static files (same GUI!)  │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    └───────────────┬───────────────┘
                                    │
                                    ▼
                     ┌────────────────────────────┐
                     │         mayara-gui/        │
                     │     (shared web assets)    │
                     │                            │
                     │  Works in ANY mode!        │
                     │  api.js auto-detects       │
                     └────────────────────────────┘
```

---

## Benefits of This Architecture

| Benefit | Description |
|---------|-------------|
| **Single source of truth** | All radar logic in mayara-core |
| **Fixes apply everywhere** | Bug fixed in core → fixed in WASM and Server |
| **No code duplication** | Same RadarLocator, same controllers, same dispatch |
| **All 4 brands everywhere** | Furuno, Navico, Raymarine, Garmin work on WASM AND Server |
| **Easy to add features** | Add to core, both platforms get it automatically |
| **Testable** | Core is pure Rust, mock IoProvider for unit tests |
| **WASM-compatible** | Core has zero tokio dependencies |
| **Same GUI** | Works unchanged with SignalK or Standalone |
| **Same API** | Clients don't know which backend they're talking to |

---

## Future: OpenCPN Integration (mayara_opencpn)

> Create a standalone OpenCPN plugin that connects to Mayara Standalone via HTTP/WebSocket.

```
┌─────────────────────────────────────────────────────────────────┐
│                     mayara_opencpn (OpenCPN Plugin)             │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    MayaraRadarPanel                       │   │
│  │  - PPI rendering (OpenGL/GLES with shaders)               │   │
│  │  - Guard zones, ARPA targets, trails display              │   │
│  │  - All data from mayara-server API                        │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    MayaraClient                           │   │
│  │  - HTTP: GET /radars, GET /capabilities, PUT /state       │   │
│  │  - WebSocket: /radars/{id}/spokes (protobuf stream)       │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │  Mayara Standalone  │
                    │  (localhost:6502)   │
                    └─────────────────────┘
                               │
                               ▼
                        Radar Hardware
                    (Furuno, Navico, etc.)
```

**Why this works well:**
- ARPA logic already in mayara-core
- No reimplementation needed in OpenCPN plugin
- Plugin is just a thin rendering client

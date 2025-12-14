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
│   - Protocol specifications                                                  │
│   - Feature flags (doppler, dual-range, no-transmit zones, etc.)            │
│                                                                              │
│   THIS IS THE ONLY PLACE WHERE RADAR CAPABILITIES ARE DEFINED.              │
│   NO OTHER COMPONENT MAY HAVE STATIC/HARDCODED RADAR DATA.                  │
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

2. **mayara-server (and WASM) have NO static radar data:**
   - No hardcoded range tables
   - No hardcoded control lists
   - No model-specific constants
   - They get ALL of this from mayara-core at runtime

3. **The REST API is the contract:**
   - `/capabilities` returns what the radar can do (from mayara-core)
   - Clients build their UI dynamically from this response
   - Same WebGUI works for ANY radar brand because it follows the API

4. **Adding a new radar model:**
   - Add it to mayara-core's model database
   - Implement wire protocol handling (if new brand)
   - That's it - the API automatically exposes the new capabilities
   - WebGUI automatically shows the right controls

5. **Control names are API names:**
   - Use strings like `"gain"`, `"dopplerMode"`, NOT enums
   - Control IDs in code match the API exactly
   - No translation layers, no mapping, no confusion

### Why This Matters

- **Consistency:** WASM and Standalone behave identically
- **Maintainability:** Change radar specs in ONE place (mayara-core)
- **Extensibility:** New features automatically available everywhere
- **Testability:** Test the core, API contract is verified
- **No drift:** Impossible for server to have different data than API

---

## Implementation Status (December 2025)

### Current Crate Structure

```
mayara-core (pure protocol, WASM-safe, ~10k LOC)
    │
    ├── mayara-server (native binary, tokio I/O, Axum web server)
    │   - Platform-specific locator (netlink, CoreFoundation, Win32)
    │   - Controller implementations (tokio TCP/UDP)
    │   - NMEA/SignalK navdata integration
    │   - Web GUI embedded via rust-embed from mayara-gui/
    │
    └── mayara-signalk-wasm (WASM plugin for SignalK)
        - WasmIoProvider using SignalK FFI
        - Re-exports RadarLocator from mayara-core
        - Web GUI copied to public/ at build time from mayara-gui/

mayara-gui/ (shared web assets)
    - viewer.html, control.html
    - JavaScript, CSS, protobuf files
    - Used by BOTH mayara-server and mayara-signalk-wasm
```

### ✅ Implemented

| Component | Location | Status |
|-----------|----------|--------|
| Protocol parsing (Furuno, Navico, Raymarine, Garmin) | mayara-core/protocol/ | ✅ Complete |
| Model database | mayara-core/models/ | ✅ Complete |
| Capability definitions (v5 API) | mayara-core/capabilities/ | ✅ Complete |
| Radar state types | mayara-core/state/ | ✅ Complete |
| **ARPA target tracking** | mayara-core/arpa/ | ✅ Complete |
| **Trails history** | mayara-core/trails/ | ✅ Complete |
| **Guard zones** | mayara-core/guard_zones/ | ✅ Complete |
| **IoProvider trait** | mayara-core/io.rs | ✅ Complete |
| **RadarLocator (generic)** | mayara-core/locator.rs | ✅ Complete |
| SignalK WASM plugin (v5 API) | mayara-signalk-wasm/ | ✅ Working (Furuno)* |
| **WasmIoProvider** | mayara-signalk-wasm/wasm_io.rs | ✅ Complete |
| v6 ARPA WASM exports | mayara-signalk-wasm/lib.rs | ✅ Complete |
| SignalK notification FFI | mayara-signalk-wasm/signalk_ffi.rs | ✅ Complete |
| mayara-server standalone | mayara-server/ | ✅ Complete |
| v6 ARPA endpoints | mayara-server/web.rs | ✅ Complete |
| SignalK-style API | mayara-server/web.rs | ✅ Complete |
| **mayara-gui shared package** | mayara-gui/ | ✅ Complete |
| **Local applicationData API** | mayara-server/storage.rs | ✅ Complete |

### 🚧 In Progress / Partial

| Component | Location | Status |
|-----------|----------|--------|
| Raymarine support | mayara-server/brand/raymarine/ | 🚧 Partial (untested) |
| Garmin support | mayara-server/brand/garmin/ | 🚧 Stub only |

### ❌ Not Yet Implemented

| Component | Planned Location | Notes |
|-----------|-----------------|-------|
| mayara_opencpn plugin | mayara_opencpn/ | OpenCPN integration |
| SignalK Provider Mode | mayara-server | Standalone → SignalK provider |
| WASM Navico controller | mayara-signalk-wasm/ | Navico uses UDP-based protocol |
| WASM Raymarine controller | mayara-signalk-wasm/ | Raymarine uses different protocol |
| WASM Garmin controller | mayara-signalk-wasm/ | Garmin uses different protocol |

> **Note on brand controllers:** Each radar brand uses a different control protocol
> (Furuno=TCP/NMEA-like, Navico=UDP/binary, etc.). The WASM plugin currently only
> implements FurunoController. mayara-server has controllers for all brands in
> `brand/*/`. To add more brands to WASM, each needs its own controller implementation.

---

## Design Principle: Unified SignalK-Compatible API

**Key Insight:** The SignalK WASM plugin has a fully tested, working implementation of the
SignalK Radar API v5 with Furuno. Instead of maintaining two different APIs, **Standalone
implements the same SignalK-compatible API** (without requiring SignalK itself) so that:

1. **Same GUI** works unchanged in WASM and Standalone modes
2. **Same locator and controller code** can be shared (only I/O layer differs)
3. **Standalone can optionally register as a SignalK provider** later

### The API Contract

Standalone implements a SignalK-compatible API surface. The GUI code doesn't know or care
whether it's talking to SignalK or Standalone - the endpoints behave identically.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                  SignalK-Compatible API (implemented by both)                │
│                                                                              │
│  a) RADAR API (v5)                                                           │
│  ───────────────────────────────────────────────────────────────────────────│
│  GET  /radars                         - List discovered radars              │
│  GET  /radars/{id}                    - Get radar info                      │
│  GET  /radars/{id}/capabilities       - Get capabilities manifest           │
│  GET  /radars/{id}/state              - Get current state                   │
│  PUT  /radars/{id}/state              - Update state (controls)             │
│  WS   /radars/{id}/spokes             - WebSocket spoke stream              │
│                                                                              │
│  b) ARPA TARGET API (v6)                                                     │
│  ───────────────────────────────────────────────────────────────────────────│
│  GET  /radars/{id}/targets            - List tracked ARPA targets           │
│  POST /radars/{id}/targets            - Manual target acquisition           │
│  DELETE /radars/{id}/targets/{tid}    - Cancel target tracking              │
│  GET  /radars/{id}/arpa/settings      - Get ARPA settings                   │
│  PUT  /radars/{id}/arpa/settings      - Update ARPA settings                │
│  WS   /radars/{id}/targets            - WebSocket target stream             │
│                                                                              │
│  c) APPLICATION DATA API (for settings/storage)                              │
│  ───────────────────────────────────────────────────────────────────────────│
│  GET  /signalk/v1/applicationData/global/{appid}/{version}/{*key}           │
│  PUT  /signalk/v1/applicationData/global/{appid}/{version}/{*key}           │
│  (See: https://demo.signalk.org/documentation/Developing/Plugins/WebApps)   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                ┌───────────────────┴───────────────────┐
                │                                       │
                ▼                                       ▼
┌───────────────────────────────────┐    ┌───────────────────────────────────┐
│         WASM Plugin               │    │           Standalone              │
│       (runs in SignalK)           │    │        (own Axum server)          │
├───────────────────────────────────┤    ├───────────────────────────────────┤
│                                   │    │                                   │
│  SignalK provides API endpoints   │    │  Axum provides SAME endpoints    │
│  SignalK provides storage API     │    │  Local file provides storage     │
│                                   │    │                                   │
│  Mayara WASM implements:          │    │  Mayara Standalone implements:   │
│  - RadarLocator (from core)       │    │  - Locator (tokio I/O)           │
│  - WasmIoProvider (FFI I/O)       │    │  - Controller (tokio I/O)        │
│  - RadarProvider trait            │    │  - web.rs handlers               │
│                                   │    │                                   │
│  GUI served by SignalK            │    │  GUI embedded via rust-embed     │
│  (copied from mayara-gui/)        │    │  (from mayara-gui/)              │
│                                   │    │                                   │
└───────────────────────────────────┘    └───────────────────────────────────┘
                │                                       │
                └───────────────────┬───────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────────┐
                    │         mayara-gui/               │
                    │     (shared web assets)           │
                    │                                   │
                    │  index.html, viewer.html          │
                    │  control.html, api.js             │
                    │  *.js, *.css, protobuf/           │
                    │                                   │
                    │  api.js auto-detects mode:        │
                    │  - SignalK: uses SK endpoints     │
                    │  - Standalone: uses local API     │
                    └───────────────────────────────────┘
```

---

## Deployment Modes

### Mode 1: SignalK WASM Plugin (Current, to be tested)

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
│  │  │  │  (FFI sockets)   │──│   Uses IoProvider for I/O             │ │  │ │
│  │  │  └──────────────────┘  └───────────────────────────────────────┘ │  │ │
│  │  │         │                      │                                  │  │ │
│  │  │         └──────────┬───────────┘                                  │  │ │
│  │  │                    ▼                                              │  │ │
│  │  │  ┌──────────────────────────────────────────────────────────┐    │  │ │
│  │  │  │                     RadarProvider                         │    │  │ │
│  │  │  │  - Brand Controllers* (TCP/UDP via IoProvider)            │    │  │ │
│  │  │  │  - SpokeReceiver (UDP multicast via IoProvider)           │    │  │ │
│  │  │  │  - ArpaProcessor (from mayara-core)                       │    │  │ │
│  │  │  │  *Currently: Furuno only. Each brand needs its own        │    │  │ │
│  │  │  │   controller due to different protocols.                  │    │  │ │
│  │  │  └──────────────────────────────────────────────────────────┘    │  │ │
│  │  └──────────────────────────┬───────────────────────────────────────┘  │ │
│  └─────────────────────────────┼──────────────────────────────────────────┘ │
│                                │ FFI calls                                   │
│  ┌─────────────────────────────┴──────────────────────────────────────────┐ │
│  │           SignalK Radar API v5/v6 Endpoints                             │ │
│  │  (SignalK routes requests to RadarProvider methods)                     │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
              │
              ▼
         Browser / GUI (from mayara-gui/)
```

**Characteristics:**
- Runs inside SignalK's WASM sandbox
- Uses SignalK FFI for all network I/O via WasmIoProvider
- Poll-based (no async runtime in WASM)
- SignalK handles HTTP routing, WebSocket management
- RadarLocator from mayara-core runs unchanged

### Mode 2: Standalone (Working)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    mayara-server (Rust)                                      │
│                                                                              │
│  ┌─────────────┐  ┌──────────────────────────────────────────────────────┐  │
│  │  Locator    │  │   Brand Controllers (brand/furuno/, etc.)            │  │
│  │  (tokio)    │  │   (tokio TCP/UDP)                                    │  │
│  └──────┬──────┘  └────────────────────────┬─────────────────────────────┘  │
│         │                                  │                                 │
│         └──────────┬───────────────────────┘                                │
│                    ▼                                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │              Axum Router (web.rs)                                    │    │
│  │  ┌─────────────────────────────────────────────────────────────┐    │    │
│  │  │         SignalK Radar API v5/v6 Handlers                     │    │    │
│  │  │                                                              │    │    │
│  │  │  GET  /radars                                                │    │    │
│  │  │  GET  /radars/{radar_id}/capabilities                        │    │    │
│  │  │  GET  /radars/{radar_id}/state                               │    │    │
│  │  │  PUT  /radars/{radar_id}/state                               │    │    │
│  │  │  WS   /radars/{radar_id}/spokes                              │    │    │
│  │  │  GET  /radars/{radar_id}/targets                             │    │    │
│  │  │  POST /radars/{radar_id}/targets                             │    │    │
│  │  │  DELETE /radars/{radar_id}/targets/{target_id}               │    │    │
│  │  └─────────────────────────────────────────────────────────────┘    │    │
│  │  ┌─────────────────────────────────────────────────────────────┐    │    │
│  │  │         Static File Server (GUI via rust-embed)              │    │    │
│  │  │  /                    → index.html (from mayara-gui/)        │    │    │
│  │  │  /viewer.html         → viewer.html                          │    │    │
│  │  │  /control.html        → control.html                         │    │    │
│  │  │  /style.css, etc.                                            │    │    │
│  │  └─────────────────────────────────────────────────────────────┘    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
              │
              ▼
         Browser / GUI (same files from mayara-gui/!)
```

**Characteristics:**
- Native Rust binary with tokio async runtime
- Direct network I/O (socket2, tokio, platform-specific netlink/CoreFoundation)
- Axum web server hosts API + GUI
- GUI embedded via `rust-embed` from `mayara-gui/`
- **Same API paths as SignalK** → same GUI works unchanged

### Mode 3: Standalone + SignalK Provider (Future)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    mayara-server (Rust)                                      │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │   (Same as Mode 2: Locator, Controller, web.rs)                      │    │
│  └──────────────────────────┬──────────────────────────────────────────┘    │
│                             │                                                │
│  ┌──────────────────────────┴──────────────────────────────────────────┐    │
│  │                    Axum Router                                       │    │
│  │  ┌────────────────────┐  ┌────────────────────────┐                 │    │
│  │  │  Local API (v5/v6) │  │  SignalK Provider      │                 │    │
│  │  │  /radars/*         │  │  Client                │                 │    │
│  │  │                    │  │                        │                 │    │
│  │  │  For local GUI     │  │  Registers with SK     │                 │    │
│  │  │  and direct access │  │  Forwards radar data   │                 │    │
│  │  └────────────────────┘  └───────────┬────────────┘                 │    │
│  └──────────────────────────────────────┼──────────────────────────────┘    │
└─────────────────────────────────────────┼───────────────────────────────────┘
              │                           │
              ▼                           ▼
         Browser / GUI          ┌─────────────────────────┐
         (local access)         │    SignalK Server       │
                                │                         │
                                │  Mayara registered as   │
                                │  radar provider         │
                                │                         │
                                │  Other SK clients       │
                                │  see radar via SignalK  │
                                └─────────────────────────┘
```

---

## Code Sharing Strategy

### Key Insight: IoProvider Abstraction

The WASM plugin and standalone share radar locator and controller logic through
the `IoProvider` trait. All socket operations are abstracted, allowing the same
discovery and control code to run on both platforms.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SHARED CODE (mayara-core)                          │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       RadarLocator                                   │    │
│  │  (mayara-core/locator.rs)                                           │    │
│  │                                                                      │    │
│  │  - Beacon packet construction (Furuno, Navico, Raymarine, Garmin)    │    │
│  │  - Discovery state machine                                           │    │
│  │  - Multicast group management                                        │    │
│  │  - Radar identification                                              │    │
│  │                                                                      │    │
│  │  Uses IoProvider for all I/O:                                        │    │
│  │    fn start<I: IoProvider>(&mut self, io: &mut I)                    │    │
│  │    fn poll<I: IoProvider>(&mut self, io: &mut I)                     │    │
│  │    fn send_furuno_announce<I: IoProvider>(&self, io: &mut I)         │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      ARPA / Trails / Guard Zones                     │    │
│  │  (mayara-core/arpa/, trails/, guard_zones/)                         │    │
│  │                                                                      │    │
│  │  - Target detection and tracking (Kalman filter)                     │    │
│  │  - CPA/TCPA calculation                                              │    │
│  │  - Trail history storage                                             │    │
│  │  - Guard zone alerting                                               │    │
│  │                                                                      │    │
│  │  Pure computation, no I/O - works identically on WASM and native     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
                    │                               │
                    ▼                               ▼
     ┌──────────────────────────┐     ┌──────────────────────────┐
     │   WasmIoProvider         │     │   Tokio I/O (direct)     │
     │   (mayara-signalk-wasm)  │     │   (mayara-server)        │
     │                          │     │                          │
     │   impl IoProvider for    │     │   tokio::net::UdpSocket  │
     │   WasmIoProvider {       │     │   tokio::net::TcpStream  │
     │     fn udp_create() {    │     │                          │
     │       sk_udp_create()    │     │   Platform-specific:     │
     │     }                    │     │   - netlink (Linux)      │
     │     fn udp_send_to() {   │     │   - CoreFoundation (Mac) │
     │       sk_udp_send()      │     │   - Win32 (Windows)      │
     │     }                    │     │                          │
     │   }                      │     │                          │
     └──────────────────────────┘     └──────────────────────────┘
```

### The IoProvider Trait

```rust
// mayara-core/src/io.rs

/// Platform-independent I/O provider.
///
/// All operations are non-blocking and poll-based.
pub trait IoProvider {
    // UDP Operations
    fn udp_create(&mut self) -> Result<UdpSocketHandle, IoError>;
    fn udp_bind(&mut self, socket: &UdpSocketHandle, port: u16) -> Result<(), IoError>;
    fn udp_set_broadcast(&mut self, socket: &UdpSocketHandle, enabled: bool) -> Result<(), IoError>;
    fn udp_join_multicast(&mut self, socket: &UdpSocketHandle, group: &str, interface: &str) -> Result<(), IoError>;
    fn udp_send_to(&mut self, socket: &UdpSocketHandle, data: &[u8], addr: &str, port: u16) -> Result<usize, IoError>;
    fn udp_recv_from(&mut self, socket: &UdpSocketHandle, buf: &mut [u8]) -> Option<(usize, String, u16)>;
    fn udp_pending(&self, socket: &UdpSocketHandle) -> i32;
    fn udp_close(&mut self, socket: UdpSocketHandle);

    // TCP Operations
    fn tcp_create(&mut self) -> Result<TcpSocketHandle, IoError>;
    fn tcp_connect(&mut self, socket: &TcpSocketHandle, addr: &str, port: u16) -> Result<(), IoError>;
    fn tcp_is_connected(&self, socket: &TcpSocketHandle) -> bool;
    fn tcp_send(&mut self, socket: &TcpSocketHandle, data: &[u8]) -> Result<usize, IoError>;
    fn tcp_recv_line(&mut self, socket: &TcpSocketHandle, buf: &mut [u8]) -> Option<usize>;
    fn tcp_close(&mut self, socket: TcpSocketHandle);

    // Utility
    fn current_time_ms(&self) -> u64;
    fn debug(&self, msg: &str);
}
```

### WASM IoProvider Implementation

```rust
// mayara-signalk-wasm/src/wasm_io.rs

pub struct WasmIoProvider {
    current_time_ms: u64,
}

impl IoProvider for WasmIoProvider {
    fn udp_create(&mut self) -> Result<UdpSocketHandle, IoError> {
        let id = unsafe { signalk_ffi::raw::sk_udp_create(0) };
        if id < 0 { Err(IoError::from_code(id)) }
        else { Ok(UdpSocketHandle(id)) }
    }

    fn udp_send_to(&mut self, socket: &UdpSocketHandle, data: &[u8], addr: &str, port: u16) -> Result<usize, IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_udp_send(socket.0, addr.as_ptr(), addr.len(), port, data.as_ptr(), data.len())
        };
        if result < 0 { Err(IoError::from_code(result)) }
        else { Ok(result as usize) }
    }

    // ... other methods wrap SignalK FFI calls
}
```

### RadarLocator Usage (WASM)

```rust
// mayara-signalk-wasm/src/radar_provider.rs

pub struct RadarProvider {
    io: WasmIoProvider,
    locator: RadarLocator,  // from mayara-core
    // ...
}

impl RadarProvider {
    pub fn new() -> Self {
        let mut io = WasmIoProvider::new();
        let mut locator = RadarLocator::new();
        locator.start(&mut io);  // Same locator code as native!

        Self { io, locator, /* ... */ }
    }

    pub fn poll(&mut self) -> i32 {
        self.io.set_time(/* timestamp from host */);
        let new_radars = self.locator.poll(&mut self.io);  // Same poll code!

        for discovery in &new_radars {
            self.emit_radar_discovered(discovery);
        }
        // ...
    }
}
```

---

## Architecture Diagram (Current State)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              mayara-core                                     │
│                    (Pure Rust, no I/O, WASM-compatible)                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  protocol/    │ │   models/     │ │ capabilities/ │ │   state.rs    │   │
│  │  - furuno/    │ │ - furuno.rs   │ │ - controls.rs │ │   (types)     │   │
│  │  - navico.rs  │ │ - navico.rs   │ │ - builder.rs  │ │               │   │
│  │  - raymarine  │ │ - raymarine   │ │               │ │               │   │
│  │  - garmin.rs  │ │ - garmin.rs   │ │               │ │               │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  arpa/        │ │  trails/      │ │ guard_zones/  │ │  io.rs        │   │
│  │  - types.rs   │ │ - history.rs  │ │ - zone.rs     │ │ (IoProvider   │   │
│  │  - detector   │ │               │ │               │ │  trait)       │   │
│  │  - tracker    │ │               │ │               │ │               │   │
│  │  - cpa.rs     │ │               │ │               │ │               │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │  locator.rs - Generic RadarLocator using IoProvider                    │  │
│  │                                                                        │  │
│  │  Discovers: Furuno, Navico (BR24, Gen3), Raymarine, Garmin             │  │
│  │  Methods: start(), poll(), send_furuno_announce(), shutdown()          │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
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
     │  wasm_io.rs:               │    │  locator.rs:               │
     │  - WasmIoProvider          │    │  - Native discovery        │
     │  - Implements IoProvider   │    │  - Platform netlink/CF     │
     │                            │    │                            │
     │  locator.rs:               │    │  brand/:                   │
     │  - Re-exports RadarLocator │    │  - furuno/ (tokio TCP)     │
     │    from mayara-core        │    │  - navico/                 │
     │                            │    │  - raymarine/              │
     │  furuno_controller.rs:     │    │                            │
     │  - TCP control via FFI     │    │  navdata.rs:               │
     │                            │    │  - NMEA/SignalK input      │
     │  radar_provider.rs:        │    │                            │
     │  - RadarProvider impl      │    │  web.rs:                   │
     │  - ArpaProcessor usage     │    │  - Axum handlers           │
     │                            │    │  - ArpaProcessor usage     │
     │  signalk_ffi.rs:           │    │                            │
     │  - FFI bindings            │    │  storage.rs:               │
     │  - Notifications           │    │  - Local applicationData   │
     │                            │    │                            │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │     SignalK Server         │    │     Axum HTTP Server       │
     │                            │    │                            │
     │  Routes /radars/* to       │    │  /radars/*  (same API!)    │
     │  WASM RadarProvider        │    │  Static files (same GUI!)  │
     │                            │    │                            │
     │  Serves GUI from           │    │  GUI embedded via          │
     │  plugin public/ dir        │    │  rust-embed                │
     │                            │    │                            │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    └───────────────┬───────────────┘
                                    │
                                    ▼
                     ┌────────────────────────────┐
                     │         mayara-gui/        │
                     │     (shared web assets)    │
                     │                            │
                     │  index.html, viewer.html   │
                     │  control.html, api.js      │
                     │  mayara.js, viewer.js      │
                     │  style.css                 │
                     │  protobuf/ (client lib)    │
                     │  proto/RadarMessage.proto  │
                     │                            │
                     │  api.js auto-detects:      │
                     │  - SignalK vs Standalone   │
                     │  Works in ANY mode!        │
                     └────────────────────────────┘
```

---

## What Gets Shared

| Component | Location | WASM | Standalone | mayara_opencpn | Notes |
|-----------|----------|:----:|:----------:|:--------------:|-------|
| Protocol parsing | mayara-core/protocol/ | ✓ | ✓ | ✓ | Packet encoding/decoding |
| Model database | mayara-core/models/ | ✓ | ✓ | ✓ | Radar specs, range tables |
| Control definitions | mayara-core/capabilities/ | ✓ | ✓ | ✓ | v5 API control schemas |
| RadarState types | mayara-core/state.rs | ✓ | ✓ | ✓ | State representation |
| **IoProvider trait** | mayara-core/io.rs | ✓ | - | - | I/O abstraction |
| **RadarLocator** | mayara-core/locator.rs | ✓ | - | - | Generic discovery |
| **ARPA** | mayara-core/arpa/ | ✓ | ✓ | ✓ | Target tracking, CPA/TCPA |
| **Trails** | mayara-core/trails/ | ✓ | ✓ | ✓ | Target position history |
| **Guard zones** | mayara-core/guard_zones/ | ✓ | ✓ | ✓ | Zone alerting logic |
| **Web GUI** | mayara-gui/ | ✓ | ✓ | - | Shared web assets |

---

## Build System

### mayara-signalk-wasm Build (build.js)

```bash
node build.js [--test] [--no-pack]

Steps:
1. (optional) Run cargo tests on mayara-core
2. Copy GUI assets from mayara-gui/ → public/
3. Build WASM: cargo build --target wasm32-wasip1 --release -p mayara-signalk-wasm
4. Copy plugin.wasm to package directory
5. (default) Create npm package: npm pack
```

### mayara-server Build

```bash
cargo build --release -p mayara-server

# build.rs:
# - Generates protobuf Rust code
# - Copies RadarMessage.proto to web output
# - Downloads protobuf.js for web clients
# - Triggers rebuild if mayara-gui/ changes

# rust-embed:
# - Embeds mayara-gui/ directory at compile time
# - Served via axum_embed::ServeEmbed<Assets>
```

---

## SignalK Notifications from ARPA

The WASM plugin publishes collision warnings to SignalK's notification system,
enabling chart plotters to display radar-based alerts alongside AIS warnings.

### Notification Paths

```
notifications.navigation.closestApproach.radar:{radarId}:target:{targetId}
notifications.navigation.radarGuardZone.radar:{radarId}:zone:{zoneId}
notifications.navigation.radarTargetLost.radar:{radarId}:target:{targetId}
```

### How It Works

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          mayara-signalk-wasm                                 │
│                                                                              │
│   Spokes ──► ArpaProcessor (mayara-core) ──► Targets with CPA/TCPA          │
│                    │                                                         │
│                    ▼                                                         │
│         ┌─────────────────────┐                                             │
│         │  Notification Logic │                                             │
│         │  - CPA < threshold? │                                             │
│         │  - Guard zone hit?  │                                             │
│         │  - Target lost?     │                                             │
│         └─────────┬───────────┘                                             │
│                   │                                                          │
│                   ▼ SignalK FFI: publish_notification()                      │
└───────────────────┼─────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          SignalK Server                                      │
│                                                                              │
│   notifications.navigation.closestApproach.radar:furuno-1:target:3          │
│   { "state": "warn", "message": "ARPA target 3: CPA 320m in 5m 24s" }       │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Chart Plotters / SignalK Clients                          │
│                                                                              │
│   Same collision warning UI as AIS-based alerts                              │
│   (Freeboard-SK, WilhelmSK, etc.)                                           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Alert States

| State | CPA Threshold | Description |
|-------|---------------|-------------|
| `normal` | > 1000m | Target tracked, no danger |
| `alert` | < 1000m | Approaching, monitor closely |
| `warn` | < 500m | Getting close |
| `alarm` | < 200m | Danger, take action |
| `emergency` | < 100m | Imminent collision |

---

## Application Data Storage API

The GUI needs to persist settings (like guard zone configurations, display preferences).
SignalK provides this via the applicationData API. Standalone implements the same interface.

### API Endpoints

```
GET  /signalk/v1/applicationData/global/{appid}/{version}/{*key}
PUT  /signalk/v1/applicationData/global/{appid}/{version}/{*key}

Examples:
  GET  /signalk/v1/applicationData/global/mayara/1.0/guardZones
  PUT  /signalk/v1/applicationData/global/mayara/1.0/displaySettings
```

### Storage Backend

**WASM (SignalK provides storage):**
- SignalK stores data in its own database
- GUI calls SignalK's applicationData API

**Standalone (local storage via storage.rs):**
- Axum implements same endpoints
- Data stored in local file (`~/.config/mayara/appdata.json`)

### GUI Usage (same code works in both modes)

```javascript
// mayara-gui/api.js

const STORAGE_BASE = '/signalk/v1/applicationData/global/mayara/1.0';

async function loadSettings(key) {
    const response = await fetch(`${STORAGE_BASE}/${key}`);
    return response.json();
}

async function saveSettings(key, value) {
    await fetch(`${STORAGE_BASE}/${key}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(value)
    });
}

// Works identically whether talking to SignalK or Standalone
const guardZones = await loadSettings('guardZones');
await saveSettings('displaySettings', { colorScheme: 'night' });
```

---

## File Reference

| Path | Purpose | WASM | Native | Status |
|------|---------|:----:|:------:|:------:|
| `mayara-core/src/protocol/` | Protocol parsing | ✓ | ✓ | ✅ |
| `mayara-core/src/models/` | Model database | ✓ | ✓ | ✅ |
| `mayara-core/src/capabilities/` | Control definitions | ✓ | ✓ | ✅ |
| `mayara-core/src/state.rs` | State types | ✓ | ✓ | ✅ |
| `mayara-core/src/arpa/` | ARPA target tracking | ✓ | ✓ | ✅ |
| `mayara-core/src/trails/` | Target trail history | ✓ | ✓ | ✅ |
| `mayara-core/src/guard_zones/` | Guard zone logic | ✓ | ✓ | ✅ |
| `mayara-core/src/io.rs` | IoProvider trait | ✓ | - | ✅ |
| `mayara-core/src/locator.rs` | Generic RadarLocator | ✓ | - | ✅ |
| `mayara-gui/` | Web GUI assets | ✓ | ✓ | ✅ |
| `mayara-signalk-wasm/src/wasm_io.rs` | WasmIoProvider | WASM | - | ✅ |
| `mayara-signalk-wasm/src/locator.rs` | Re-exports RadarLocator | WASM | - | ✅ |
| `mayara-signalk-wasm/src/signalk_ffi.rs` | SignalK FFI bindings | WASM | - | ✅ |
| `mayara-signalk-wasm/src/lib.rs` | WASM entry point (v5+v6) | WASM | - | ✅ |
| `mayara-signalk-wasm/src/radar_provider.rs` | RadarProvider impl | WASM | - | ✅ |
| `mayara-server/src/main.rs` | Binary entry, Axum setup | - | Native | ✅ |
| `mayara-server/src/locator.rs` | Network radar discovery | - | Native | ✅ |
| `mayara-server/src/brand/` | Controller implementations | - | Native | ✅ |
| `mayara-server/src/network/` | Platform-specific sockets | - | Native | ✅ |
| `mayara-server/src/navdata.rs` | NMEA/SignalK integration | - | Native | ✅ |
| `mayara-server/src/web.rs` | Axum handlers (v5+v6 API) | - | Native | ✅ |
| `mayara-server/src/storage.rs` | Local applicationData | - | Native | ✅ |

---

## Future: OpenCPN Integration (mayara_opencpn)

> **Decision:** Create a standalone OpenCPN plugin (mayara_opencpn) that connects to Mayara Standalone.

### Background

OpenCPN users currently use [radar_pi](https://github.com/opencpn-radar-pi/radar_pi) for radar display.
While mature (10+ years), it lacks Furuno support and modern Garmin/Raymarine models.

**Decision Rationale (Option B - Standalone Plugin):**
- Clean slate implementation with full control over UI/UX
- No dependency on radar_pi maintainers for upstream changes
- ARPA/trails logic already in mayara-core - no reimplementation needed
- Leverages Mayara's proven WebSocket/protobuf protocol

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     mayara_opencpn (OpenCPN Plugin)             │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    MayaraRadarPanel                       │   │
│  │  - PPI rendering (OpenGL/GLES with shaders)               │   │
│  │  - Guard zones display                                    │   │
│  │  - ARPA target display (from /targets API)                │   │
│  │  - Trails display                                         │   │
│  │  - EBL/VRM tools                                          │   │
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

### What mayara_opencpn Gets "For Free"

Because ARPA and trails logic is in mayara-core, mayara_opencpn benefits:

| Feature | Source | Notes |
|---------|--------|-------|
| Target detection | mayara-core/arpa/ | Contour detection, blob tracking |
| Target tracking | mayara-core/arpa/ | Kalman filtering, prediction |
| CPA/TCPA calculation | mayara-core/arpa/ | Collision warnings |
| Target trails | mayara-core/trails/ | Historical position storage |
| Guard zones | mayara-core/guard_zones/ | Zone definition + alerting logic |

mayara_opencpn only needs to implement:
- OpenGL PPI rendering (shader-based, like radar_pi)
- wxWidgets UI integration
- HTTP/WebSocket client
- Protobuf parsing

### Rendering Strategy

**Use OpenGL/GLES with shader-based polar rendering** (same approach as radar_pi):

1. **Spoke texture:** Store all spokes in a 2D texture
2. **Fragment shader:** Rectangular → polar coordinate conversion
3. **Efficient updates:** Only changed spoke rows updated via `glTexSubImage2D`

Platform compatibility: Desktop OpenGL 2.0+, RPi5 GLESv2, RPi3/4 GLShim.

### Open Questions

1. **Discovery:** mDNS/Bonjour for automatic Mayara discovery, or manual configuration?
2. **Multiple radars:** One panel per radar, or single panel with selector?

---

## Benefits Summary

| Benefit | Description |
|---------|-------------|
| **One API to maintain** | SignalK Radar API v5/v6 is the standard, used everywhere |
| **One GUI to maintain** | Same HTML/JS/CSS in mayara-gui/ works in all modes |
| **Shared locator code** | RadarLocator in mayara-core runs unchanged on WASM and (future) native |
| **ARPA everywhere** | Collision warnings in WASM, Standalone, AND future mayara_opencpn |
| **Tested implementation** | WASM plugin proves the API and code design works |
| **Flexibility** | Users choose: WASM plugin OR standalone OR standalone+provider |
| **Code quality** | Shared logic means bugs fixed once, everywhere |

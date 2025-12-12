# Mayara Architecture

> This document describes the architecture of the Mayara radar system,
> showing what is shared between deployment modes and the path to maximum code reuse.

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
│  a) RADAR API                                                                │
│  ───────────────────────────────────────────────────────────────────────────│
│  GET  /radars                         - List discovered radars              │
│  GET  /radars/{id}                    - Get radar info                      │
│  GET  /radars/{id}/capabilities       - Get capabilities manifest           │
│  GET  /radars/{id}/state              - Get current state                   │
│  PUT  /radars/{id}/state              - Update state (controls)             │
│  WS   /radars/{id}/spokes             - WebSocket spoke stream              │
│                                                                              │
│  b) APPLICATION DATA API (for settings/storage)                              │
│  ───────────────────────────────────────────────────────────────────────────│
│  GET  /signalk/v1/applicationData/global/:appid/:version/:key               │
│  PUT  /signalk/v1/applicationData/global/:appid/:version/:key               │
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
│  SignalK provides storage API     │    │  Local file/DB provides storage  │
│                                   │    │                                   │
│  Mayara WASM implements:          │    │  Mayara Standalone implements:   │
│  - Locator (FFI I/O)              │    │  - Locator (tokio I/O)           │
│  - Controller (FFI I/O)           │    │  - Controller (tokio I/O)        │
│  - RadarProvider trait            │    │  - RadarProvider trait           │
│                                   │    │                                   │
│  GUI served by SignalK            │    │  GUI embedded via rust-embed     │
│                                   │    │                                   │
│                                   │    │  Optional: register to SignalK   │
│                                   │    │  as provider (future)            │
└───────────────────────────────────┘    └───────────────────────────────────┘
                │                                       │
                └───────────────────┬───────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────────┐
                    │           Same GUI                │
                    │     viewer.html, control.js       │
                    │                                   │
                    │  Uses:                            │
                    │  - /radars/* for radar control    │
                    │  - /signalk/v1/applicationData/*  │
                    │    for settings persistence       │
                    └───────────────────────────────────┘
```

---

## Deployment Modes

### Mode 1: SignalK WASM Plugin (Current, Working)

```
┌─────────────────────────────────────────────────────────────┐
│                    SignalK Server (Node.js)                  │
│  ┌────────────────────────────────────────────────────────┐ │
│  │              WASM Runtime (wasmer)                      │ │
│  │  ┌──────────────────────────────────────────────────┐  │ │
│  │  │         mayara-signalk-wasm                       │  │ │
│  │  │                                                   │  │ │
│  │  │  ┌─────────────┐  ┌──────────────────────────┐   │  │ │
│  │  │  │  Locator    │  │   FurunoController       │   │  │ │
│  │  │  │  (FFI I/O)  │  │   (FFI I/O)              │   │  │ │
│  │  │  └──────┬──────┘  └────────────┬─────────────┘   │  │ │
│  │  │         │                      │                  │  │ │
│  │  │         └──────────┬───────────┘                  │  │ │
│  │  │                    ▼                              │  │ │
│  │  │         ┌──────────────────────┐                  │  │ │
│  │  │         │   RadarProvider      │◄── Implements    │  │ │
│  │  │         │   (v5 API impl)      │    SignalK trait │  │ │
│  │  │         └──────────────────────┘                  │  │ │
│  │  └──────────────────────┬───────────────────────────┘  │ │
│  └─────────────────────────┼──────────────────────────────┘ │
│                            │ FFI calls                       │
│  ┌─────────────────────────┴──────────────────────────────┐ │
│  │           SignalK Radar API v5 Endpoints                │ │
│  │  (SignalK routes requests to RadarProvider methods)     │ │
│  └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
              │
              ▼
         Browser / GUI
```

**Characteristics:**
- Runs inside SignalK's WASM sandbox
- Uses SignalK FFI for all network I/O
- Poll-based (no async runtime in WASM)
- SignalK handles HTTP routing, WebSocket management

### Mode 2: Standalone (Target Architecture)

```
┌─────────────────────────────────────────────────────────────┐
│                    mayara-server (Rust)                      │
│                                                              │
│  ┌─────────────┐  ┌──────────────────────────┐              │
│  │  Locator    │  │   FurunoController       │              │
│  │  (tokio)    │  │   (tokio)                │              │
│  └──────┬──────┘  └────────────┬─────────────┘              │
│         │                      │                             │
│         └──────────┬───────────┘                             │
│                    ▼                                         │
│         ┌──────────────────────┐                             │
│         │   RadarProvider      │◄── Same trait as WASM!     │
│         │   (async impl)       │                             │
│         └──────────┬───────────┘                             │
│                    │                                         │
│  ┌─────────────────┴─────────────────────────────────────┐  │
│  │              Axum Router                               │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │         SignalK Radar API v5 Handlers            │  │  │
│  │  │  (SAME logic as WASM, different I/O backend)     │  │  │
│  │  │                                                   │  │  │
│  │  │  GET  /radars                                     │  │  │
│  │  │  GET  /radars/{id}/capabilities                   │  │  │
│  │  │  GET  /radars/{id}/state                          │  │  │
│  │  │  PUT  /radars/{id}/state                          │  │  │
│  │  │  WS   /radars/{id}/spokes                         │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │         Static File Server (GUI)                 │  │  │
│  │  │  /                    → viewer.html              │  │  │
│  │  │  /control.html        → control.html             │  │  │
│  │  │  /style.css, etc.                                │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
              │
              ▼
         Browser / GUI (same files!)
```

**Characteristics:**
- Native Rust binary with tokio async runtime
- Direct network I/O (socket2, tokio)
- Axum web server hosts API + GUI
- **Same API paths as SignalK** → same GUI works

### Mode 3: Standalone + SignalK Provider (Future)

```
┌─────────────────────────────────────────────────────────────┐
│                    mayara-server (Rust)                      │
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │   (Same as Mode 2: Locator, Controller, Provider)    │    │
│  └──────────────────────────┬──────────────────────────┘    │
│                             │                                │
│  ┌──────────────────────────┴──────────────────────────┐    │
│  │                    Axum Router                       │    │
│  │  ┌────────────────────┐  ┌────────────────────────┐ │    │
│  │  │  Local API (v5)    │  │  SignalK Provider      │ │    │
│  │  │  /radars/*         │  │  Client                │ │    │
│  │  │                    │  │                        │ │    │
│  │  │  For local GUI     │  │  Registers with SK     │ │    │
│  │  │  and direct access │  │  Forwards radar data   │ │    │
│  │  └────────────────────┘  └───────────┬────────────┘ │    │
│  └──────────────────────────────────────┼──────────────┘    │
└─────────────────────────────────────────┼───────────────────┘
              │                           │
              ▼                           ▼
         Browser / GUI          ┌─────────────────────────┐
         (local access)         │    SignalK Server       │
                                │                         │
                                │  Mayara registered as   │
                                │  radar provider         │
                                │                         │
                                │  Other SK clients see   │
                                │  radar via SignalK API  │
                                └─────────────────────────┘
```

**Characteristics:**
- All benefits of Mode 2
- Additionally registers with a SignalK server as a **provider**
- SignalK clients (chart plotters, other apps) can access radar through SignalK
- Mayara is the **source**, SignalK is the **hub**

---

## Code Sharing Strategy

### Key Insight: Share Locator and Controller

The WASM plugin has a fully working Furuno implementation with locator and controller.
Instead of extracting "state machines" separately, we share the **actual locator and
controller code** by abstracting only the I/O layer.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SHARED CODE (mayara-core)                          │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         Locator                                      │    │
│  │  - Beacon packet construction                                        │    │
│  │  - Discovery state machine                                           │    │
│  │  - Interface enumeration logic                                       │    │
│  │  - Radar identification                                              │    │
│  │                                                                      │    │
│  │  I/O abstracted via trait: fn send_beacon(), fn recv_response()     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      Controller (per vendor)                         │    │
│  │  - FurunoController                                                  │    │
│  │  - NavicoController (future)                                         │    │
│  │  - RaymarineController (future)                                      │    │
│  │                                                                      │    │
│  │  - Command encoding/decoding                                         │    │
│  │  - State synchronization logic                                       │    │
│  │  - Capability-based control handling                                 │    │
│  │                                                                      │    │
│  │  I/O abstracted via trait: fn send_cmd(), fn recv_response()        │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      API Handler Logic                               │    │
│  │  - Request/response transformation                                   │    │
│  │  - Validation                                                        │    │
│  │  - Error mapping                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
                    │                               │
                    ▼                               ▼
     ┌──────────────────────────┐     ┌──────────────────────────┐
     │   WASM I/O Layer         │     │   Tokio I/O Layer        │
     │                          │     │                          │
     │   impl IoProvider for    │     │   impl IoProvider for    │
     │   FfiSocket {            │     │   TokioSocket {          │
     │     fn send() {          │     │     fn send() {          │
     │       sk_socket_send()   │     │       socket.send()      │
     │     }                    │     │     }                    │
     │   }                      │     │   }                      │
     └──────────────────────────┘     └──────────────────────────┘
```

### The IoProvider Trait (Thin I/O Abstraction)

```rust
// In mayara-core (no I/O dependencies)

/// Minimal I/O abstraction - implementations are platform-specific
pub trait IoProvider {
    /// Send data to socket
    fn send(&mut self, data: &[u8]) -> Result<usize, IoError>;

    /// Receive data from socket (non-blocking)
    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;

    /// Check if data available
    fn poll_readable(&self) -> bool;
}

/// The actual Locator logic - shared between WASM and Standalone
pub struct Locator<I: IoProvider> {
    io: I,
    state: DiscoveryState,
    // ... all the discovery logic
}

impl<I: IoProvider> Locator<I> {
    pub fn new(io: I) -> Self { ... }

    pub fn poll(&mut self) -> Vec<DiscoveredRadar> {
        // This is the SAME code in WASM and Standalone
        // Only the IoProvider implementation differs
        if self.io.poll_readable() {
            let mut buf = [0u8; 1024];
            if let Ok(n) = self.io.recv(&mut buf) {
                return self.process_response(&buf[..n]);
            }
        }
        self.maybe_send_beacon();
        vec![]
    }
}
```

### RadarProvider Trait (API Layer)

```rust
// In mayara-core

/// Platform-agnostic radar provider interface
pub trait RadarProvider {
    fn list_radars(&self) -> Vec<RadarInfo>;
    fn get_capabilities(&self, id: &str) -> Option<CapabilityManifest>;
    fn get_state(&self, id: &str) -> Option<RadarState>;
    fn set_state(&self, id: &str, updates: StateUpdate) -> Result<(), ControlError>;
    fn poll(&mut self);
}

/// Shared implementation - works with any IoProvider
pub struct MayaraProvider<I: IoProvider> {
    locator: Locator<I>,
    controllers: HashMap<String, Controller<I>>,
}

impl<I: IoProvider> RadarProvider for MayaraProvider<I> {
    // All logic is shared - only I differs
}
```

### Platform-Specific I/O

**WASM (FFI to SignalK):**
```rust
// mayara-signalk-wasm/src/io.rs
pub struct FfiSocket { handle: u32 }

impl IoProvider for FfiSocket {
    fn send(&mut self, data: &[u8]) -> Result<usize, IoError> {
        unsafe { sk_socket_send(self.handle, data.as_ptr(), data.len()) }
    }
    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        unsafe { sk_socket_recv(self.handle, buf.as_mut_ptr(), buf.len()) }
    }
}

// Entry point
type WasmProvider = MayaraProvider<FfiSocket>;
```

**Native (Tokio):**
```rust
// mayara-lib/src/io.rs
pub struct TokioSocket { socket: Arc<UdpSocket> }

impl IoProvider for TokioSocket {
    fn send(&mut self, data: &[u8]) -> Result<usize, IoError> {
        // Uses try_send (non-blocking) to match poll-based interface
        self.socket.try_send(data).map_err(Into::into)
    }
    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.socket.try_recv(buf).map_err(Into::into)
    }
}

// Entry point
type NativeProvider = MayaraProvider<TokioSocket>;
```

---

## Architecture Diagram (Target State)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              mayara-core                                     │
│                    (Pure Rust, no I/O, WASM-compatible)                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         api/ (NEW)                                   │    │
│  │  handlers.rs      - Request/response logic (no I/O)                 │    │
│  │  types.rs         - API request/response types                      │    │
│  │  errors.rs        - API error types                                 │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  protocol/    │ │   models/     │ │ capabilities/ │ │   config/     │   │
│  │  - furuno     │ │ - furuno.rs   │ │ - controls.rs │ │ - schema.rs   │   │
│  │  - navico     │ │ - navico.rs   │ │ - builder.rs  │ │ - defaults.rs │   │
│  │  - raymarine  │ │ - raymarine   │ │               │ │               │   │
│  │  - garmin     │ │ - garmin.rs   │ │               │ │               │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐   │
│  │  discovery/   │ │  controller/  │ │   state.rs    │ │  provider.rs  │   │
│  │  - state.rs   │ │ - furuno.rs   │ │ (RadarState)  │ │ (trait def)   │   │
│  │  - beacon.rs  │ │ - navico.rs   │ │               │ │               │   │
│  │ (pure logic)  │ │ (pure logic)  │ │               │ │               │   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └───────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┴───────────────┐
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │   mayara-signalk-wasm      │    │       mayara-lib           │
     │      (WASM + FFI)          │    │    (Native + tokio)        │
     ├────────────────────────────┤    ├────────────────────────────┤
     │                            │    │                            │
     │  WasmRadarProvider         │    │  AsyncRadarProvider        │
     │  - Wraps core logic        │    │  - Wraps core logic        │
     │  - Uses FFI for I/O        │    │  - Uses tokio for I/O      │
     │                            │    │                            │
     │  SignalK FFI imports:      │    │  Network I/O:              │
     │  - socket_udp_bind         │    │  - tokio::net::UdpSocket   │
     │  - socket_tcp_connect      │    │  - tokio::net::TcpStream   │
     │  - socket_send/recv        │    │  - socket2 for multicast   │
     │                            │    │                            │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    ▼                               ▼
     ┌────────────────────────────┐    ┌────────────────────────────┐
     │     SignalK Server         │    │     mayara-server          │
     │                            │    │                            │
     │  Routes /radars/* to       │    │  ┌──────────────────────┐ │
     │  WASM RadarProvider        │    │  │   Axum Router        │ │
     │                            │    │  │                      │ │
     │  Serves GUI from           │    │  │  /radars/*           │ │
     │  plugin static files       │    │  │  (same API!)         │ │
     │                            │    │  │                      │ │
     │                            │    │  │  Static files        │ │
     │                            │    │  │  (same GUI!)         │ │
     │                            │    │  └──────────────────────┘ │
     │                            │    │                            │
     │                            │    │  Optional:                │
     │                            │    │  ┌──────────────────────┐ │
     │  ◄──────────────────────────────┤  │ SignalK Provider     │ │
     │  (Mayara registers as      │    │  │ Client               │ │
     │   radar provider to SK)    │    │  │ (registers to SK)    │ │
     │                            │    │  └──────────────────────┘ │
     └────────────────────────────┘    └────────────────────────────┘
                    │                               │
                    └───────────────┬───────────────┘
                                    │
                                    ▼
                     ┌────────────────────────────┐
                     │         mayara-gui         │
                     │     (shared web assets)    │
                     │                            │
                     │  viewer.html               │
                     │  control.html              │
                     │  control.js                │
                     │  mayara.js                 │
                     │  style.css                 │
                     │  van.js                    │
                     │                            │
                     │  All fetch from /radars/*  │
                     │  Works in ANY mode!        │
                     └────────────────────────────┘
```

---

## What Gets Shared

| Component | Location | WASM | Standalone | Notes |
|-----------|----------|:----:|:----------:|-------|
| Protocol parsing | mayara-core/protocol/ | ✓ | ✓ | Already shared |
| Model database | mayara-core/models/ | ✓ | ✓ | Already shared |
| Control definitions | mayara-core/capabilities/ | ✓ | ✓ | Already shared |
| RadarState types | mayara-core/state.rs | ✓ | ✓ | Already shared |
| **Locator** | mayara-core/locator/ | ✓ | ✓ | **Shared code, IoProvider abstraction** |
| **Controller** | mayara-core/controller/ | ✓ | ✓ | **Shared code, IoProvider abstraction** |
| **API handlers** | mayara-core/api/ | ✓ | ✓ | **Move from WASM** |
| **RadarProvider trait** | mayara-core/provider.rs | ✓ | ✓ | **Abstract interface** |
| **IoProvider trait** | mayara-core/io.rs | ✓ | ✓ | **Thin I/O abstraction** |
| **Web GUI** | mayara-gui/ | ✓ | ✓ | **Shared package** |

---

## Application Data Storage API

The GUI needs to persist settings (like guard zone configurations, display preferences).
SignalK provides this via the applicationData API. Standalone implements the same interface.

### API Endpoints

```
GET  /signalk/v1/applicationData/global/mayara/1.0/:key
PUT  /signalk/v1/applicationData/global/mayara/1.0/:key

Examples:
  GET  /signalk/v1/applicationData/global/mayara/1.0/guardZones
  PUT  /signalk/v1/applicationData/global/mayara/1.0/displaySettings
```

### Storage Backend

**WASM (SignalK provides storage):**
- SignalK stores data in its own database
- GUI calls SignalK's applicationData API

**Standalone (local storage):**
- Axum implements same endpoints
- Data stored in local file (`~/.mayara/settings.json`) or SQLite

```rust
// mayara-server/src/storage.rs

pub struct LocalStorage {
    path: PathBuf,
    data: HashMap<String, Value>,
}

impl LocalStorage {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn put(&mut self, key: &str, value: Value) -> Result<(), StorageError> {
        self.data.insert(key.to_owned(), value);
        self.persist()?;
        Ok(())
    }
}

// Axum routes
async fn get_app_data(
    Path((appid, version, key)): Path<(String, String, String)>,
    State(storage): State<Arc<RwLock<LocalStorage>>>,
) -> Json<Value> {
    let storage = storage.read().unwrap();
    Json(storage.get(&key).cloned().unwrap_or(Value::Null))
}

async fn put_app_data(
    Path((appid, version, key)): Path<(String, String, String)>,
    State(storage): State<Arc<RwLock<LocalStorage>>>,
    Json(value): Json<Value>,
) -> StatusCode {
    storage.write().unwrap().put(&key, value).unwrap();
    StatusCode::OK
}
```

### GUI Usage (same code works in both modes)

```javascript
// mayara-gui/settings.js

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

## SignalK Provider Mode

When running standalone with SignalK provider mode enabled:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                                                                              │
│                            Mayara Standalone                                 │
│                                                                              │
│   1. Discovers radars via UDP                                                │
│   2. Controls radars via TCP                                                 │
│   3. Hosts local API at http://localhost:6502/radars/*                      │
│   4. Connects to SignalK server                                              │
│   5. Registers as radar provider                                             │
│   6. Pushes radar data to SignalK                                            │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
         │                              │
         │ Local GUI                    │ Provider connection
         ▼                              ▼
┌─────────────────┐          ┌─────────────────────────────────┐
│    Browser      │          │        SignalK Server           │
│                 │          │                                 │
│  http://        │          │  Receives radar data from       │
│  localhost:6502 │          │  Mayara provider                │
│                 │          │                                 │
│  Uses local     │          │  Other SignalK clients          │
│  Mayara API     │          │  (Freeboard-SK, WilhelmSK, etc.)│
│                 │          │  can access radar via SignalK   │
└─────────────────┘          └─────────────────────────────────┘
```

**Provider Registration Protocol:**

```javascript
// Mayara connects to SignalK and registers as provider
POST /signalk/v2/api/radars/providers
{
  "name": "Mayara Standalone",
  "version": "0.3.0",
  "capabilities": ["furuno", "navico", "raymarine", "garmin"]
}

// SignalK assigns provider ID
Response: { "providerId": "mayara-1", "endpoints": { ... } }

// Mayara pushes radar discoveries
POST /signalk/v2/api/radars/providers/mayara-1/radars
{
  "id": "furuno-drs4d-nxt-172-31-3-212",
  "make": "Furuno",
  "model": "DRS4D-NXT",
  ...
}

// Mayara pushes state updates
PUT /signalk/v2/api/radars/providers/mayara-1/radars/{id}/state
{ ... current state ... }

// Mayara pushes spokes via WebSocket
WS /signalk/v2/api/radars/providers/mayara-1/radars/{id}/spokes
```

---

## Migration Path

### Phase 1: Define IoProvider Trait & Move Locator

1. **Define IoProvider trait** in mayara-core
   - Simple trait: `send()`, `recv()`, `poll_readable()`
   - No async - poll-based to match WASM constraints

2. **Move Locator to mayara-core**
   - Current WASM locator becomes `Locator<I: IoProvider>`
   - All discovery logic is shared
   - WASM: implement `IoProvider` for FFI sockets
   - Native: implement `IoProvider` for tokio sockets

3. **Move Controller to mayara-core**
   - `FurunoController<I: IoProvider>`
   - All control logic is shared

### Phase 2: Unified API in Standalone

1. **Add SignalK-compatible endpoints to mayara-server**
   - `/radars/*` - same as SignalK Radar API v5
   - `/signalk/v1/applicationData/*` - storage API

2. **Implement local storage backend**
   - File-based or SQLite for settings
   - Same API as SignalK's applicationData

3. **Native IoProvider implementation**
   - Wrapper around tokio sockets
   - Uses try_send/try_recv for poll-based interface

### Phase 3: Share GUI Package

1. **Create mayara-gui/** directory
   - Move web assets from `mayara-signalk-wasm/public/`
   - GUI uses relative API paths (works in both modes)

2. **Update build systems**
   - WASM: copy from mayara-gui to public/
   - Standalone: embed from mayara-gui via rust-embed

### Phase 4: SignalK Provider Mode (Future)

1. **Implement SignalK provider client**
   - WebSocket client to connect to SignalK
   - Provider registration protocol
   - Push radar data to SignalK

2. **Add CLI flag** `--signalk-provider ws://signalk.local:3000`

---

## Benefits Summary

| Benefit | Description |
|---------|-------------|
| **One API to maintain** | SignalK Radar API v5 is the standard, used everywhere |
| **One GUI to maintain** | Same HTML/JS/CSS works in all modes |
| **Tested implementation** | WASM plugin proves the API design works |
| **Flexibility** | Users choose: WASM plugin OR standalone OR standalone+provider |
| **SignalK ecosystem** | Standalone can participate in SignalK network as provider |
| **Code quality** | Shared logic means bugs fixed once, everywhere |

---

## File Reference (Target State)

| Path | Purpose | Shared? |
|------|---------|:-------:|
| `mayara-core/src/io.rs` | IoProvider trait | ✓ |
| `mayara-core/src/locator/` | Locator<I: IoProvider> | ✓ |
| `mayara-core/src/controller/` | Controller<I: IoProvider> per vendor | ✓ |
| `mayara-core/src/api/` | API handlers, types | ✓ |
| `mayara-core/src/provider.rs` | RadarProvider trait | ✓ |
| `mayara-core/src/protocol/` | Protocol parsing | ✓ |
| `mayara-core/src/models/` | Model database | ✓ |
| `mayara-core/src/capabilities/` | Control definitions | ✓ |
| `mayara-core/src/state.rs` | State types | ✓ |
| `mayara-gui/` | Web GUI assets | ✓ |
| `mayara-signalk-wasm/src/io.rs` | FfiSocket: IoProvider impl | WASM only |
| `mayara-signalk-wasm/src/lib.rs` | WASM entry point | WASM only |
| `mayara-lib/src/io.rs` | TokioSocket: IoProvider impl | Native only |
| `mayara-server/src/storage.rs` | Local applicationData storage | Native only |
| `mayara-server/src/main.rs` | Binary entry, Axum setup | Native only |

---

## Future: Mayara Standalone and OpenCPN Integration

> **Status:** Research/Planning phase. This section documents findings and open questions
> for potential integration between Mayara Standalone and OpenCPN.

### Background: The radar_pi Plugin

OpenCPN users currently use the [radar_pi](https://github.com/opencpn-radar-pi/radar_pi) plugin
for radar display. This is a mature C++ plugin with 10+ years of development.

**What radar_pi supports:**
- Navico: BR24, 3G, 4G, HALO series
- Garmin: HD, xHD (but NOT xHD2, Fantom)
- Raymarine: Quantum, RME120
- Emulator for testing

**What radar_pi does NOT support:**
- Furuno radars (any model)
- Modern Garmin (xHD2, Fantom)
- Newer Raymarine models

**radar_pi Architecture Highlights:**
- Multi-threaded: one receive thread per radar, wxWidgets main thread for UI
- Abstracted `RadarReceive` base class for data sources
- Built-in Emulator proves non-hardware sources work
- Dual rendering: CPU vertex buffers or GPU shaders
- Features: PPI display, guard zones, ARPA/MARPA tracking, trails, EBL/VRM

### The Opportunity

Mayara Standalone could serve as a **radar backend** for OpenCPN, providing:

1. **Furuno support** - Currently missing from radar_pi
2. **Unified protocol handling** - One implementation for all brands
3. **Future brands** - As Mayara adds support, OpenCPN benefits automatically
4. **Decoupled architecture** - Radar handling separate from chart plotter

### Architecture Comparison

```
Current radar_pi (Direct Hardware Access):
┌─────────────────────────────────────────────────────────────────┐
│                         radar_pi                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │NavicoReceive │  │GarminReceive │  │RaymarineRecv │          │
│  │(UDP multicast)│  │(UDP multicast)│  │(UDP multicast)│          │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
│         └─────────────────┼─────────────────┘                   │
│                           ▼                                      │
│                    ┌──────────────┐                             │
│                    │  RadarInfo   │                             │
│                    │ProcessSpoke()│                             │
│                    └──────┬───────┘                             │
│                           ▼                                      │
│         ┌─────────────────┴─────────────────┐                   │
│         │          Rendering                 │                   │
│         │  (Vertex / Shader / Guard Zones)   │                   │
│         └────────────────────────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
                           │
                           ▼
                    Radar Hardware
                  (Navico/Garmin/Raymarine)


Proposed: radar_pi with Mayara Backend:
┌─────────────────────────────────────────────────────────────────┐
│                         radar_pi                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │NavicoReceive │  │GarminReceive │  │MayaraReceive │ ◄── NEW  │
│  │(direct HW)   │  │(direct HW)   │  │(WebSocket)   │          │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
│         └─────────────────┼─────────────────┘                   │
│                           ▼                                      │
│                    ┌──────────────┐                             │
│                    │  RadarInfo   │  (unchanged)                │
│                    │ProcessSpoke()│                             │
│                    └──────┬───────┘                             │
│                           ▼                                      │
│         ┌─────────────────┴─────────────────┐                   │
│         │          Rendering                 │  (unchanged)      │
│         │  (Vertex / Shader / Guard Zones)   │                   │
│         └────────────────────────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
         │                              │
         ▼                              ▼
   Radar Hardware                Mayara Standalone
 (Navico/Garmin/Raymarine)              │
                                        ▼
                                  Radar Hardware
                                (Furuno + others)
```

### Integration Options

#### Option A: Mayara Connector in radar_pi (Minimal Change)

Add a new "radar type" to radar_pi that connects to Mayara instead of hardware.

**New files in radar_pi:**
```
radar_pi/src/mayara/
├── MayaraReceive.cpp      # WebSocket client for spoke stream
├── MayaraReceive.h
├── MayaraControl.cpp      # HTTP client for control commands
├── MayaraControl.h
├── MayaraLocate.cpp       # HTTP discovery of Mayara radars
├── MayaraLocate.h
└── mayaratype.h           # DEFINE_RADAR for RT_MAYARA
```

**Data flow:**
```
MayaraReceive : RadarReceive
        │
        ├── HTTP GET /radars              (discovery)
        ├── HTTP GET /capabilities        (once, on connect)
        ├── WebSocket /spokes/{id}        (binary protobuf stream)
        └── HTTP PUT /controls/{id}       (commands)
                │
                ▼
        Mayara Standalone
                │
                ▼
        Actual Radar Hardware
```

**Pros:**
- Minimal changes to radar_pi (add one new radar type)
- Leverages existing radar_pi rendering, guard zones, ARPA, trails
- OpenCPN users get immediate Furuno support
- Future Mayara improvements automatically available
- Emulator pattern proves this architecture works

**Cons:**
- Requires C++ dependencies in radar_pi (HTTP client, WebSocket, protobuf)
- Two processes running (OpenCPN + Mayara)
- Network latency (~1ms on localhost)

#### Option B: Standalone OpenCPN Plugin (mayara_pi)

Create a completely new OpenCPN plugin that only talks to Mayara.

**Pros:**
- Clean slate, modern implementation
- Full control over UI/UX
- Could share rendering code with Mayara web GUI
- No legacy dependencies

**Cons:**
- Must reimplement: PPI rendering, guard zones, trails, ARPA
- radar_pi has 10+ years of battle-tested code
- Duplicate effort, longer timeline

#### Option C: Refactor radar_pi as Library + Backend

Separate radar_pi into rendering library and data acquisition backends.

**Pros:**
- Clean architecture, reusable components
- Best of both worlds

**Cons:**
- Major refactor of radar_pi
- Breaking changes for existing users
- Requires radar_pi maintainer buy-in

### Control Mapping (Mayara → radar_pi)

| Mayara Control | radar_pi ControlType | Notes |
|----------------|---------------------|-------|
| `power` | CT_TRANSMIT | Partial: radar_pi has on/off, Mayara has off/standby/transmit |
| `range` | CT_RANGE | Direct mapping |
| `gain` | CT_GAIN | Direct mapping (mode + value) |
| `sea` | CT_SEA | Direct mapping (mode + value) |
| `rain` | CT_RAIN | Direct mapping |
| `interferenceRejection` | CT_INTERFERENCE_REJECTION | Direct mapping |
| `beamSharpening` | - | New control needed or ignore |
| `dopplerMode` | - | New control needed or ignore |
| `birdMode` | - | New control needed or ignore |
| `noTransmitZones` | CT_NO_TRANSMIT_START/END | Up to 4 zones |

### Spoke Data Mapping

**Mayara protobuf format:**
```protobuf
message RadarMessage {
    message Spoke {
        uint32 angle = 1;       // 0..spokes_per_revolution
        uint32 bearing = 2;     // optional, true north reference
        uint32 range = 3;       // meters
        uint64 time = 4;        // milliseconds since epoch
        int64 lat = 6;          // 1e-16 degrees
        int64 lon = 7;          // 1e-16 degrees
        bytes data = 5;         // intensity values (0-255)
    }
    repeated Spoke spokes = 2;
}
```

**radar_pi internal format:**
```cpp
struct line_history {
    uint8_t* line;           // intensity data
    wxLongLong time;         // timestamp
    GeoPosition pos;         // vessel position
};

// Called per spoke:
void RadarInfo::ProcessRadarSpoke(
    SpokeBearing angle,      // 0..m_spokes
    SpokeBearing bearing,    // true bearing
    uint8_t *data,           // intensity array
    size_t len,              // data length
    int range_meters,        // current range
    wxLongLong time_rec      // timestamp
);
```

**Conversion is straightforward** - the formats are nearly identical.

### Open Questions

1. **Is modifying radar_pi acceptable?**
   - Or should this be a completely separate plugin?
   - Who maintains radar_pi? Can we contribute upstream?

2. **Long-term vision for radar_pi:**
   - Should radar_pi eventually use Mayara as the *only* backend?
   - Or should both direct hardware and Mayara paths coexist?

3. **Priority:**
   - Quick Furuno support for OpenCPN users?
   - Or clean architecture first?

4. **Dependencies:**
   - What HTTP/WebSocket/protobuf libraries are acceptable for radar_pi?
   - libcurl? cpp-httplib? websocketpp? Boost.Beast?

5. **Discovery:**
   - How should radar_pi discover Mayara instances?
   - mDNS/Bonjour? Manual configuration? Both?

6. **Dual-mode operation:**
   - Can radar_pi simultaneously use direct hardware AND Mayara?
   - Example: Navico via direct, Furuno via Mayara

### Preliminary Recommendation

**Option A (Mayara Connector in radar_pi)** appears most practical because:

1. radar_pi's `RadarReceive` abstraction already supports non-hardware sources
2. The Emulator proves this pattern works
3. Minimal risk - additive change, doesn't break existing functionality
4. OpenCPN users immediately benefit from Mayara's Furuno support
5. Future: as Mayara adds Navico/Raymarine, radar_pi users get improvements

### Next Steps (When Ready)

1. **Prototype MayaraReceive** - WebSocket client that calls `ProcessRadarSpoke()`
2. **Test with Furuno radar** - Verify data flow and rendering
3. **Add MayaraControl** - HTTP client for control commands
4. **Discuss with radar_pi maintainers** - Upstream contribution or fork?
5. **Document user setup** - How to configure Mayara + radar_pi together

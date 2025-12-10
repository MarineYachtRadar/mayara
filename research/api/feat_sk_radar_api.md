# SignalK Radar API v2 Proposal

## Overview

This proposal extends the SignalK radar REST API to provide a unified interface for controlling
marine radars from multiple vendors while preserving access to vendor-specific features. The design
is informed by protocol analysis of Furuno, Navico, Raymarine, and Garmin radar implementations.

## Design Goals

1. **Common API surface** - Uniform endpoints for controls shared across all vendors
2. **Vendor extensibility** - Namespaced endpoints for vendor-specific features
3. **Discoverability** - Capabilities endpoint for runtime feature detection
4. **Documentation** - Human-readable explanations for controls and their values
5. **Real-time state** - WebSocket subscription for control state changes
6. **State awareness** - Support for disabled/read-only controls in preset modes

## Base Path

All radar endpoints are under:
```
/signalk/v2/api/vessels/self/radars/{radarId}
```

---

## Common Controls API

These endpoints are available on ALL radars regardless of vendor.

### Minimum Common Set

| Control | Endpoint | Description |
|---------|----------|-------------|
| Power | `PUT /power` | Transmit state: off, standby, transmit |
| Range | `PUT /range` | Detection range in meters |
| Gain | `PUT /gain` | Signal amplification |
| Sea Clutter | `PUT /sea` | Sea return suppression |
| Rain Clutter | `PUT /rain` | Rain/precipitation suppression |

### Extended Common Controls

These are supported by most (but not all) radars:

| Control | Endpoint | Supported By |
|---------|----------|--------------|
| Interference Rejection | `PUT /interferenceRejection` | Furuno, Navico, Raymarine, Garmin |
| Target Expansion | `PUT /targetExpansion` | Furuno, Navico, Raymarine |
| Bearing Alignment | `PUT /bearingAlignment` | All |
| Antenna Height | `PUT /antennaHeight` | Furuno, Navico |
| No Transmit Zones | `PUT /noTransmitZones` | Furuno, Navico, Garmin |
| Scan Speed | `PUT /scanSpeed` | Furuno, Navico |

### Common Control Schemas

#### Power State
```json
PUT /radars/{id}/power
{
  "value": "transmit"  // "off" | "standby" | "transmit" | "warming_up" (read-only)
}
```

#### Range
```json
PUT /radars/{id}/range
{
  "value": 5000  // meters
}
```

#### Gain Control
```json
PUT /radars/{id}/gain
{
  "mode": "auto",     // "manual" | "auto"
  "value": 50,        // 0-100 (percentage), only used when mode=manual
  "autoLevel": "high" // "low" | "high" (only for auto mode, optional)
}
```

#### Sea Clutter
```json
PUT /radars/{id}/sea
{
  "mode": "auto",     // "manual" | "auto"
  "value": 30,        // 0-100 (percentage)
  "autoLevel": "calm" // vendor-specific: "calm" | "moderate" | "rough"
}
```

#### Rain Clutter
```json
PUT /radars/{id}/rain
{
  "mode": "manual",
  "value": 25
}
```

#### No Transmit Zones
```json
PUT /radars/{id}/noTransmitZones
{
  "zones": [
    { "enabled": true, "start": 90, "end": 180 },
    { "enabled": false, "start": 0, "end": 0 }
  ]
}
```
Note: Number of zones varies by vendor (Furuno: 2, Navico: 2-4, Garmin: 1)

---

## Vendor-Specific Controls API

Vendor-specific features are namespaced under the vendor name.

### Endpoint Pattern
```
PUT /radars/{id}/{vendor}/{control}
GET /radars/{id}/{vendor}/{control}
```

### Furuno-Specific Controls

| Control | Endpoint | Description |
|---------|----------|-------------|
| RezBoost | `PUT /furuno/rezboost` | Beam sharpening (0=off, 1-3=levels) |
| Bird Mode | `PUT /furuno/birdMode` | Optimizes for bird detection |
| Target Analyzer | `PUT /furuno/targetAnalyzer` | Doppler-based threat highlighting |
| TX Channel | `PUT /furuno/txChannel` | Transmit frequency channel |
| Fast Target Tracking | `PUT /furuno/fastTargetTracking` | Quick target acquisition |
| Echo Trail | `PUT /furuno/echoTrail` | Historical echo persistence |
| Main Bang | `PUT /furuno/mainBang` | Suppresses transmitter pulse |
| Near/Middle/Far STC | `PUT /furuno/stc` | Range-based sensitivity control |

#### RezBoost Schema
```json
PUT /radars/{id}/furuno/rezboost
{
  "value": 2  // 0=off, 1=low, 2=medium, 3=max
}
```

#### Target Analyzer Schema
```json
PUT /radars/{id}/furuno/targetAnalyzer
{
  "enabled": true
}
```

### Navico-Specific Controls (Simrad/Lowrance/B&G)

| Control | Endpoint | Description |
|---------|----------|-------------|
| Doppler Mode | `PUT /navico/doppler` | VelocityTrack target motion |
| Doppler Speed | `PUT /navico/dopplerSpeed` | Minimum speed threshold |
| Mode Preset | `PUT /navico/mode` | Harbor/Offshore/Weather/Bird/Custom |
| Target Boost | `PUT /navico/targetBoost` | Enhances weak targets |
| Target Separation | `PUT /navico/targetSeparation` | Distinguishes close targets |
| Noise Rejection | `PUT /navico/noiseRejection` | Digital noise filtering |
| Sidelobe Suppression | `PUT /navico/sidelobeSuppression` | Reduces antenna side lobes |
| Sea State | `PUT /navico/seaState` | Auto sea clutter calibration |
| Accent Light | `PUT /navico/accentLight` | Scanner status LED |
| Local IR | `PUT /navico/localInterferenceRejection` | Nearby radar filtering |

#### Doppler Mode Schema
```json
PUT /radars/{id}/navico/doppler
{
  "mode": "approaching"  // "off" | "both" | "approaching"
}
```

#### Mode Preset Schema
```json
PUT /radars/{id}/navico/mode
{
  "value": "harbor"  // "custom" | "harbor" | "offshore" | "weather" | "bird"
}
```

### Raymarine-Specific Controls

| Control | Endpoint | Description |
|---------|----------|-------------|
| Doppler | `PUT /raymarine/doppler` | Motion detection (Quantum 2, Cyclone) |
| Color Gain | `PUT /raymarine/colorGain` | Echo color intensity |
| Display Timing | `PUT /raymarine/displayTiming` | Radar timing adjustment |
| Fast Scan | `PUT /raymarine/fastScan` | High-speed rotation mode |
| Sector Blanking | `PUT /raymarine/sectorBlanking` | Mast/structure blanking |

### Garmin-Specific Controls

| Control | Endpoint | Description |
|---------|----------|-------------|
| Crosstalk Rejection | `PUT /garmin/crosstalkRejection` | Multi-radar interference |
| Timed Idle | `PUT /garmin/timedIdle` | Power-saving transmit cycling |
| Auto Gain Level | `PUT /garmin/autoGainLevel` | High/Low auto sensitivity |

#### Timed Idle Schema
```json
PUT /radars/{id}/garmin/timedIdle
{
  "enabled": true,
  "idleTime": 300,    // seconds
  "runTime": 60       // seconds
}
```

---

## Capabilities Endpoint

Provides runtime discovery of radar features.

### Request
```
GET /radars/{id}/capabilities
```

### Response Schema
```json
{
  "id": "radar-0",
  "vendor": "navico",
  "model": "HALO24",
  "serialNumber": "1234567890",
  "firmwareVersion": "2.1.0",

  "characteristics": {
    "spokesPerRevolution": 2048,
    "maxSpokeLength": 1024,
    "pixelDepth": 4,
    "hasDoppler": true,
    "maxRange": 72000
  },

  "controls": {
    "common": [
      "power",
      "range",
      "gain",
      "sea",
      "rain",
      "interferenceRejection",
      "targetExpansion",
      "bearingAlignment",
      "noTransmitZones",
      "scanSpeed"
    ],
    "vendor": [
      "doppler",
      "dopplerSpeed",
      "mode",
      "targetBoost",
      "targetSeparation",
      "noiseRejection",
      "sidelobeSuppression",
      "seaState",
      "accentLight"
    ]
  },

  "ranges": [
    125, 250, 500, 750, 1000, 1500, 2000, 3000, 4000, 6000,
    8000, 12000, 16000, 24000, 36000, 48000, 72000
  ]
}
```

---

## Control Metadata Endpoint

Provides human-readable descriptions and valid values for controls.

### Request
```
GET /radars/{id}/controls
GET /radars/{id}/controls/{controlName}
```

### Response Schema
```json
{
  "controls": {
    "gain": {
      "name": "Gain",
      "description": "Controls the amplification of received radar signals. Higher values increase sensitivity but may also amplify noise.",
      "type": "ranged",
      "modes": ["manual", "auto"],
      "range": { "min": 0, "max": 100, "step": 1, "unit": "percent" },
      "autoLevels": ["low", "high"],
      "default": { "mode": "auto", "autoLevel": "high" }
    },

    "furuno/rezboost": {
      "name": "RezBoost",
      "vendor": "furuno",
      "description": "Furuno's proprietary beam sharpening technology that uses advanced signal processing to narrow the effective beam width. This produces resolution comparable to larger antenna arrays, improving target separation and reducing elongated echoes from distant targets.",
      "marketingNote": "RezBoost™ achieves the equivalent resolution of a larger antenna array in a compact radome.",
      "type": "enum",
      "values": [
        { "value": 0, "label": "Off", "description": "RezBoost disabled" },
        { "value": 1, "label": "Low", "description": "Mild beam sharpening" },
        { "value": 2, "label": "Medium", "description": "Moderate beam sharpening" },
        { "value": 3, "label": "Max", "description": "Maximum beam sharpening, 2° effective beam width" }
      ],
      "default": 2
    },

    "furuno/targetAnalyzer": {
      "name": "Target Analyzer",
      "vendor": "furuno",
      "description": "Uses Doppler processing to color-code radar targets based on their relative motion. Approaching targets are highlighted in red for immediate threat identification, while stationary or receding targets appear in green.",
      "marketingNote": "Target Analyzer™ is FURUNO's exclusive Doppler-based threat detection that works independent of vessel speed.",
      "type": "boolean",
      "default": true
    },

    "furuno/birdMode": {
      "name": "Bird Mode",
      "vendor": "furuno",
      "description": "Automatically adjusts gain and sea clutter settings optimized for detecting birds, which often indicate fish schools below. Essential for sportfishing applications.",
      "type": "boolean",
      "default": false
    },

    "navico/doppler": {
      "name": "VelocityTrack",
      "vendor": "navico",
      "description": "Simrad/Lowrance/B&G's Doppler motion detection that color-codes all radar targets based on relative motion. Unlike manual ARPA tracking, VelocityTrack automatically analyzes every target on screen without selection limits.",
      "marketingNote": "VelocityTrack™ provides instant visual feedback on whether targets are approaching or receding, with no manual target selection required.",
      "type": "enum",
      "values": [
        { "value": "off", "label": "Off", "description": "Doppler detection disabled" },
        { "value": "both", "label": "Both", "description": "Highlight approaching and receding targets" },
        { "value": "approaching", "label": "Approaching Only", "description": "Only highlight approaching targets" }
      ],
      "default": "approaching"
    },

    "navico/mode": {
      "name": "Radar Mode",
      "vendor": "navico",
      "description": "Preset operating modes that automatically configure gain, sea clutter, rain clutter, and other settings for specific conditions. In preset modes, individual controls become read-only.",
      "type": "enum",
      "values": [
        { "value": "custom", "label": "Custom", "description": "Full manual control of all settings" },
        { "value": "harbor", "label": "Harbor", "description": "Optimized for busy ports with fast scanning and clutter reduction" },
        { "value": "offshore", "label": "Offshore", "description": "Balanced settings for open water navigation" },
        { "value": "weather", "label": "Weather", "description": "Enhanced sensitivity for detecting precipitation cells" },
        { "value": "bird", "label": "Bird", "description": "Optimized for detecting birds indicating fish schools" }
      ],
      "default": "harbor",
      "controlsDisabledInPreset": ["gain", "sea", "rain", "interferenceRejection"]
    },

    "raymarine/doppler": {
      "name": "Doppler Target Detection",
      "vendor": "raymarine",
      "description": "Quantum 2 and Cyclone series feature Doppler-based collision avoidance that color-codes moving contacts to show if they are approaching or moving away.",
      "marketingNote": "Leveraging sophisticated Doppler processing, Quantum 2 clearly highlights moving radar contacts for enhanced situational awareness.",
      "type": "enum",
      "values": [
        { "value": "off", "label": "Off" },
        { "value": "approaching", "label": "Approaching" }
      ]
    },

    "garmin/crosstalkRejection": {
      "name": "Crosstalk Rejection",
      "vendor": "garmin",
      "description": "Filters interference from nearby radars operating on similar frequencies, particularly useful in crowded marina or commercial shipping environments.",
      "type": "ranged",
      "range": { "min": 0, "max": 100, "step": 1, "unit": "percent" }
    }
  }
}
```

---

## Control State Endpoint

Returns current state of all controls including disabled status.

### Request
```
GET /radars/{id}/state
```

### Response Schema
```json
{
  "id": "radar-0",
  "timestamp": "2025-01-15T10:30:00Z",

  "power": {
    "value": "transmit",
    "disabled": false
  },

  "range": {
    "value": 6000,
    "disabled": false
  },

  "gain": {
    "mode": "auto",
    "value": 50,
    "autoLevel": "high",
    "disabled": true,
    "disabledReason": "Controlled by Harbor mode preset"
  },

  "sea": {
    "mode": "auto",
    "value": 35,
    "disabled": true,
    "disabledReason": "Controlled by Harbor mode preset"
  },

  "navico/mode": {
    "value": "harbor",
    "disabled": false
  },

  "navico/doppler": {
    "mode": "approaching",
    "disabled": false
  }
}
```

### Disabled State

Controls can be disabled (read-only) when:
1. **Preset Mode Active**: HALO Harbor/Offshore/Weather modes lock dependent controls
2. **Hardware Limitation**: Feature unavailable at current range or configuration
3. **Dependency**: Another control's state disables this one

The `disabledReason` field provides human-readable explanation.

---

## WebSocket API

Real-time control state updates via WebSocket subscription.

### Connection
```
ws://{host}/signalk/v2/api/vessels/self/radars/{radarId}/state
```

### Message Format

#### Subscription Request
```json
{
  "type": "subscribe",
  "controls": ["*"]  // or specific: ["gain", "sea", "navico/mode"]
}
```

#### State Update Message
```json
{
  "type": "state",
  "timestamp": "2025-01-15T10:30:01Z",
  "changes": [
    {
      "control": "navico/mode",
      "value": "weather",
      "disabled": false
    },
    {
      "control": "gain",
      "mode": "auto",
      "value": 75,
      "autoLevel": "high",
      "disabled": true,
      "disabledReason": "Controlled by Weather mode preset"
    },
    {
      "control": "sea",
      "mode": "auto",
      "value": 20,
      "disabled": true,
      "disabledReason": "Controlled by Weather mode preset"
    },
    {
      "control": "rain",
      "mode": "auto",
      "value": 60,
      "disabled": true,
      "disabledReason": "Controlled by Weather mode preset"
    }
  ]
}
```

### Use Cases

1. **Multi-controller sync**: When a hardware MFD changes a setting, all connected clients receive the update
2. **Mode cascade**: Changing from Custom to Harbor mode triggers updates for all affected controls
3. **Auto-adjustment**: Auto gain/sea values change based on conditions - clients see real-time values

---

## Implementation Notes

### WASM Plugin Interface

The `mayara-signalk-wasm` plugin should export:

```rust
// Existing exports
#[wasm_bindgen]
pub fn radar_get_radars() -> String;
#[wasm_bindgen]
pub fn radar_get_info(radar_id: &str) -> String;
#[wasm_bindgen]
pub fn radar_set_power(radar_id: &str, state: &str) -> bool;
#[wasm_bindgen]
pub fn radar_set_range(radar_id: &str, range_m: u32) -> bool;
#[wasm_bindgen]
pub fn radar_set_gain(radar_id: &str, mode: &str, value: u32, auto_level: &str) -> bool;
#[wasm_bindgen]
pub fn radar_set_sea(radar_id: &str, mode: &str, value: u32) -> bool;
#[wasm_bindgen]
pub fn radar_set_rain(radar_id: &str, mode: &str, value: u32) -> bool;
#[wasm_bindgen]
pub fn radar_set_controls(radar_id: &str, controls_json: &str) -> bool;

// New exports for v2 API
#[wasm_bindgen]
pub fn radar_get_capabilities(radar_id: &str) -> String;
#[wasm_bindgen]
pub fn radar_get_controls_metadata(radar_id: &str) -> String;
#[wasm_bindgen]
pub fn radar_get_state(radar_id: &str) -> String;
#[wasm_bindgen]
pub fn radar_set_vendor_control(radar_id: &str, vendor: &str, control: &str, value_json: &str) -> bool;
#[wasm_bindgen]
pub fn radar_subscribe_state(radar_id: &str, callback: &js_sys::Function) -> u32;
#[wasm_bindgen]
pub fn radar_unsubscribe_state(subscription_id: u32) -> bool;
```

### SignalK Server Route Mapping

```javascript
// Common controls
router.put('/radars/:id/power', (req, res) => wasmPlugin.radar_set_power(req.params.id, req.body.value));
router.put('/radars/:id/range', (req, res) => wasmPlugin.radar_set_range(req.params.id, req.body.value));
router.put('/radars/:id/gain', (req, res) => wasmPlugin.radar_set_gain(...));
router.put('/radars/:id/sea', (req, res) => wasmPlugin.radar_set_sea(...));
router.put('/radars/:id/rain', (req, res) => wasmPlugin.radar_set_rain(...));

// Vendor-specific controls
router.put('/radars/:id/:vendor/:control', (req, res) => {
  wasmPlugin.radar_set_vendor_control(req.params.id, req.params.vendor, req.params.control, JSON.stringify(req.body));
});

// Discovery
router.get('/radars/:id/capabilities', ...);
router.get('/radars/:id/controls', ...);
router.get('/radars/:id/state', ...);

// WebSocket
router.ws('/radars/:id/state', ...);
```

---

## Vendor Feature Glossary

### Furuno

| Feature | Marketing Name | Technical Description |
|---------|---------------|----------------------|
| RezBoost™ | Beam Sharpening | Signal processing that narrows effective beam width for improved target separation |
| Target Analyzer™ | Doppler Threat Detection | Color-codes targets by relative motion (red=approaching, green=receding/stationary) |
| Bird Mode | Fish Finder Aid | Auto-tunes radar for detecting bird flocks indicating fish schools |
| Fast Target Tracking™ | Rapid ARPA | Generates target vectors in seconds vs traditional ARPA delay |
| Echo Trail | Target History | Displays historical echo positions showing target movement |

### Navico (Simrad/Lowrance/B&G)

| Feature | Marketing Name | Technical Description |
|---------|---------------|----------------------|
| VelocityTrack™ | Doppler Motion Detection | Automatic motion analysis of all targets without selection limits |
| Beam Sharpening | Resolution Enhancement | Digital processing to improve angular resolution |
| Target Separation | Close Target Distinction | Improves ability to distinguish closely-spaced targets |
| MARPA | Target Tracking | Manual Automatic Radar Plotting Aid for collision avoidance |
| Dual Range | Split Display | Simultaneous display of two range scales |

### Raymarine

| Feature | Marketing Name | Technical Description |
|---------|---------------|----------------------|
| CHIRP | Pulse Compression | Frequency-modulated pulse for improved range resolution |
| ATX™ | Advanced Target Separation | Enhanced target separation using multiple compressed pulses |
| Doppler | Collision Avoidance | Motion-based target highlighting (Quantum 2, Cyclone series) |

### Garmin

| Feature | Marketing Name | Technical Description |
|---------|---------------|----------------------|
| Timed Idle | Power Management | Cycles between transmit and standby to conserve power |
| Crosstalk Rejection | Multi-Radar Filtering | Reduces interference from other nearby radars |

---

## Migration Path

### Phase 1: Capabilities Discovery
- Add `/capabilities` endpoint
- Add `/controls` metadata endpoint
- Maintain backward compatibility with existing API

### Phase 2: Common Controls
- Implement standardized common control endpoints
- Add control state with disabled flag support
- Deprecate (but maintain) legacy endpoints

### Phase 3: Vendor Extensions
- Add vendor-namespaced endpoints
- Implement full control metadata
- Add WebSocket state subscription

### Phase 4: Full Migration
- Remove deprecated endpoints
- Complete vendor-specific control coverage
- Add comprehensive control documentation

---

## References

- [Furuno Radar Technology](https://www.furuno.com/en/technology/radar/)
- [Simrad HALO Radar](https://www.simrad-yachting.com/simrad/series/halo-radar/)
- [Raymarine Quantum Radar](https://www.raymarine.com/en-us/our-products/marine-radar/quantum)
- [mayara-lib Protocol Documentation](../../../mayara/docs/)
- [SignalK Radar API v1](../../../signalk-server/docs/develop/rest-api/radar_api.md)

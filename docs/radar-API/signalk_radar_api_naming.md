# SignalK Radar API - Naming Conventions

> This document defines the naming conventions for the SignalK Radar REST API.
> Follow these rules when adding new radars or controls to ensure consistency.

## Overview

The API uses **semantic control IDs** that are vendor-neutral. Each vendor's proprietary control names are mapped to standardized IDs that work across all radar brands.

```
Vendor-Specific Name     →    Semantic API ID
─────────────────────────────────────────────
Furuno "RezBoost"        →    beamSharpening
Navico "Beam Sharpening" →    beamSharpening
Furuno "Target Analyzer" →    dopplerMode
Navico "VelocityTrack"   →    dopplerMode
```

---

## Naming Rules

### 1. Control IDs: `camelCase`

All control identifiers use **lowerCamelCase**:

```
✓ beamSharpening
✓ dopplerMode
✓ interferenceRejection
✓ noTransmitZones
✓ mainBangSuppression

✗ beam_sharpening      (no underscores)
✗ BeamSharpening       (no PascalCase)
✗ BEAM_SHARPENING      (no SCREAMING_CASE)
```

### 2. Property Names: `camelCase`

Compound control properties also use lowerCamelCase:

```json
{
  "gain": {
    "mode": "auto",
    "value": 50
  },
  "dopplerMode": {
    "enabled": true,
    "mode": "approaching"
  }
}
```

### 3. Enum Values: `lowercase` or integers

String enum values use **lowercase**. Numeric enum values use **integers**:

```json
// String enums
"power": "standby"      // not "Standby" or "STANDBY"
"mode": "auto"          // not "Auto" or "AUTO"
"mode": "manual"        // not "Manual"

// Numeric enums (for hardware levels)
"beamSharpening": 0     // Off
"beamSharpening": 1     // Low
"beamSharpening": 2     // Medium
"beamSharpening": 3     // High
```

### 4. Display Names: Title Case

Human-readable names use **Title Case**:

```
name: "Beam Sharpening"
name: "Interference Rejection"
name: "No-Transmit Zones"
```

### 5. Units: lowercase

Unit strings use lowercase:

```
unit: "meters"
unit: "degrees"
unit: "percent"
unit: "knots"
unit: "hours"
```

---

## Control Categories

Controls are organized into categories:

| Category | Description | Examples |
|----------|-------------|----------|
| `base` | Required on all radars | power, range, gain, sea, rain |
| `extended` | Optional, model-specific | beamSharpening, dopplerMode, birdMode |
| `installation` | Configuration settings | bearingAlignment, antennaHeight, noTransmitZones |

---

## Standard Control IDs

### Base Controls (All Radars)

| ID | Type | Description |
|----|------|-------------|
| `power` | enum | Operational state: off, standby, transmit, warming |
| `range` | number | Detection range in meters |
| `gain` | compound | Signal amplification: {mode, value} |
| `sea` | compound | Sea clutter suppression: {mode, value} |
| `rain` | compound | Rain clutter suppression: {mode, value} |

### Read-Only Info

| ID | Type | Description |
|----|------|-------------|
| `serialNumber` | string | Hardware serial number |
| `firmwareVersion` | string | Firmware version string |
| `operatingHours` | number | Total hours of operation |

### Signal Processing

| ID | Type | Furuno | Navico | Raymarine | Garmin |
|----|------|--------|--------|-----------|--------|
| `beamSharpening` | enum | RezBoost | Beam Sharpening | - | - |
| `dopplerMode` | compound | Target Analyzer | VelocityTrack | Doppler | - |
| `dopplerSpeed` | number | - | Doppler Speed | - | - |
| `birdMode` | enum | Bird Mode | - | - | - |
| `noiseReduction` | boolean | Noise Reduction | - | - | - |
| `noiseRejection` | enum | - | Noise Rejection | - | - |
| `mainBangSuppression` | number | MBS | - | MBS | - |

### Interference Filtering

| ID | Type | Furuno | Navico | Raymarine | Garmin |
|----|------|--------|--------|-----------|--------|
| `interferenceRejection` | boolean/enum | IR (on/off) | IR (levels) | IR | IR |
| `localInterferenceRejection` | enum | - | Local IR | - | - |
| `crosstalkRejection` | enum | - | - | - | Crosstalk |
| `sidelobeSuppression` | compound | - | SLS | - | - |

### Target Processing

| ID | Type | Description |
|----|------|-------------|
| `targetSeparation` | enum | Distinguishes closely-spaced targets |
| `targetExpansion` | enum | Makes small targets more visible |
| `targetBoost` | enum | Amplifies weak targets |
| `autoAcquire` | boolean | Automatic ARPA target acquisition |

### Clutter Controls

| ID | Type | Description |
|----|------|-------------|
| `seaState` | enum | Sea state preset (calm/moderate/rough) |
| `ftc` | compound | Fast Time Constant for rain clutter |

### Operating Modes

| ID | Type | Description |
|----|------|-------------|
| `presetMode` | enum | Pre-configured mode (harbor/offshore/weather) |
| `scanSpeed` | enum | Antenna rotation speed |
| `txChannel` | enum | TX frequency channel selection |

### Receiver Controls

| ID | Type | Description |
|----|------|-------------|
| `tune` | compound | Receiver tuning: {mode, value} |
| `colorGain` | compound | Color intensity: {mode, value} |

### Installation Settings

| ID | Type | Description |
|----|------|-------------|
| `bearingAlignment` | number | Heading offset correction (degrees) |
| `antennaHeight` | number | Antenna height above waterline (meters) |
| `noTransmitZones` | compound | Sectors where radar won't transmit |

### Hardware Controls

| ID | Type | Description |
|----|------|-------------|
| `accentLight` | enum | Pedestal accent lighting |

---

## Adding a New Control

When adding a new control, follow this checklist:

### 1. Check if a semantic ID already exists

Before creating a new ID, check if an existing one covers the functionality:

```
New Furuno "Echo Enhance" feature
  → Is it similar to existing targetBoost? Use targetBoost
  → Is it unique? Create new ID: echoEnhance
```

### 2. Use semantic naming

Name the control by what it **does**, not what the vendor calls it:

```
✓ beamSharpening     (describes function)
✗ rezBoost           (vendor-specific name)

✓ dopplerMode        (describes technology)
✗ targetAnalyzer     (Furuno marketing name)
✗ velocityTrack      (Navico marketing name)
```

### 3. Add to controls.rs

Create a factory function in `mayara-core/src/capabilities/controls.rs`:

```rust
/// New feature: enhances echo visibility
///
/// Furuno: Echo Enhance
/// Navico: Target Boost
pub fn control_echo_enhance() -> ControlDefinition {
    ControlDefinition {
        id: "echoEnhance".into(),
        name: "Echo Enhance".into(),
        description: "Enhances visibility of weak echoes.".into(),
        category: ControlCategory::Extended,
        control_type: ControlType::Enum,
        // ... rest of definition
    }
}
```

### 4. Register in get_extended_control()

Add the new ID to the match statement:

```rust
pub fn get_extended_control(id: &str) -> Option<ControlDefinition> {
    match id {
        // ... existing controls
        "echoEnhance" => Some(control_echo_enhance()),
        _ => None,
    }
}
```

### 5. Add to model's control list

In the model database (e.g., `furuno.rs`):

```rust
static CONTROLS_NXT: &[&str] = &[
    "beamSharpening",
    "dopplerMode",
    "echoEnhance",    // Add new control
    // ...
];
```

---

## Compound Control Structure

Compound controls have multiple properties. Follow this structure:

### Mode + Value Pattern

For controls with auto/manual mode and a numeric value:

```json
{
  "gain": {
    "mode": "auto",    // "auto" | "manual"
    "value": 50        // 0-100
  }
}
```

### Enabled + Mode Pattern

For controls that can be toggled with sub-modes:

```json
{
  "dopplerMode": {
    "enabled": true,
    "mode": "approaching"  // "approaching" | "both" | "target" | "rain"
  }
}
```

### Array Pattern

For controls with multiple items:

```json
{
  "noTransmitZones": {
    "zones": [
      { "enabled": true, "start": 90, "end": 180 },
      { "enabled": false, "start": 0, "end": 0 }
    ]
  }
}
```

---

## REST API Paths

### Endpoint Structure

```
GET  /signalk/v2/api/radars                    # List all radars
GET  /signalk/v2/api/radars/{id}               # Get radar info
GET  /signalk/v2/api/radars/{id}/capabilities  # Get capabilities
GET  /signalk/v2/api/radars/{id}/state         # Get current state
PUT  /signalk/v2/api/radars/{id}/state         # Update state
GET  /signalk/v2/api/radars/{id}/spokes        # WebSocket for spokes
```

### Radar ID Format

Radar IDs are generated from network discovery:

```
furuno-drs4d-nxt-172-31-3-212     # brand-model-ip
navico-halo-192-168-1-100
raymarine-quantum-10-0-0-50
```

---

## For AI Agents

When implementing new radar support:

1. **Map vendor controls to semantic IDs** - Don't create new IDs for features that already exist
2. **Use existing control types** - boolean, number, enum, compound
3. **Follow camelCase** - All IDs and property names
4. **Document vendor mapping** - Comment which vendor feature maps to which semantic ID
5. **Test consistency** - Ensure same control ID produces same JSON structure across vendors

### Quick Reference

```
ID format:           camelCase         (beamSharpening)
Property format:     camelCase         (mode, value, enabled)
String enum values:  lowercase         ("auto", "manual")
Numeric enum values: integers          (0, 1, 2, 3)
Display names:       Title Case        ("Beam Sharpening")
Units:               lowercase         ("meters", "degrees")
```

---

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2024-12 | Initial naming conventions for Furuno DRS-NXT |

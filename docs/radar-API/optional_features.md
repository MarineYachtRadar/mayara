# SignalK Radar API - Optional Features

> How providers announce optional capabilities and how clients discover them.

---

## Problem

The SignalK Radar API v6 adds optional features like ARPA target tracking. Clients need to know:

1. Does this radar/provider support ARPA?
2. Does this radar/provider support guard zones?
3. What other optional features are available?

Currently, clients must try endpoints and check for 404/null responses - not ideal.

---

## Solution: `supportedFeatures` Array

Add a `supportedFeatures` field to `CapabilityManifest`:

```typescript
interface CapabilityManifest {
  id: string
  make: string
  model: string
  characteristics: RadarCharacteristics
  controls: ControlDefinition[]
  constraints?: ControlConstraint[]

  // NEW: Explicitly declare optional feature support
  supportedFeatures?: SupportedFeature[]
}

type SupportedFeature = 'arpa' | 'guardZones' | 'trails' | 'dualRange'
```

### Example Response

```json
{
  "id": "Furuno-6424",
  "make": "Furuno",
  "model": "DRS4D-NXT",
  "characteristics": {
    "maxRange": 74080,
    "hasDoppler": true,
    "hasDualRange": true
  },
  "supportedFeatures": ["arpa", "guardZones", "trails"],
  "controls": [...]
}
```

---

## Feature Definitions

| Feature | Description | API Endpoints |
|---------|-------------|---------------|
| `arpa` | ARPA target tracking with CPA/TCPA | `/targets`, `/arpa/settings` |
| `guardZones` | Guard zone alerting | `/guardZones` (future) |
| `trails` | Target trail history | Included in `/targets` response |
| `dualRange` | Simultaneous dual-range display | `characteristics.hasDualRange` already exists |

---

## Client Usage

```javascript
// Fetch capabilities once when radar connects
const caps = await fetch(`/radars/${radarId}/capabilities`).then(r => r.json());

// Check for ARPA support before showing ARPA UI
const hasArpa = caps.supportedFeatures?.includes('arpa') ?? false;

if (hasArpa) {
  // Show ARPA controls, target list, etc.
  initArpaPanel(radarId);
}

// Check for guard zones
const hasGuardZones = caps.supportedFeatures?.includes('guardZones') ?? false;

if (hasGuardZones) {
  initGuardZonePanel(radarId);
}
```

---

## Provider Implementation

### TypeScript (SignalK Plugin)

```typescript
const provider: RadarProvider = {
  name: 'Mayara Radar',
  methods: {
    getCapabilities: async (radarId) => ({
      id: radarId,
      make: 'Furuno',
      model: 'DRS4D-NXT',
      characteristics: { /* ... */ },
      controls: [ /* ... */ ],

      // Declare what this provider implements
      supportedFeatures: ['arpa', 'guardZones', 'trails']
    }),

    // Only implement if declared in supportedFeatures
    getTargets: async (radarId) => { /* ... */ },
    acquireTarget: async (radarId, bearing, distance) => { /* ... */ },
    // ...
  }
}
```

### Rust (Mayara)

```rust
// mayara-core/src/capabilities/mod.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportedFeature {
    Arpa,
    GuardZones,
    Trails,
    DualRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityManifest {
    pub id: String,
    pub make: String,
    pub model: String,
    pub characteristics: Characteristics,
    pub controls: Vec<ControlDefinition>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_features: Option<Vec<SupportedFeature>>,
}
```

---

## Backward Compatibility

- `supportedFeatures` is optional - older providers won't have it
- Clients should use defensive checks: `caps.supportedFeatures?.includes('arpa') ?? false`
- If field is absent, clients can fall back to trying endpoints

---

## Relationship to `characteristics`

Some features overlap with `characteristics`:

| `characteristics` field | `supportedFeatures` equivalent |
|------------------------|-------------------------------|
| `hasDoppler: true` | - (hardware capability, always reported) |
| `hasDualRange: true` | `dualRange` (optional, provider may not implement API) |

**Distinction:**
- `characteristics` = hardware capabilities (what the radar CAN do)
- `supportedFeatures` = API capabilities (what the provider IMPLEMENTS)

Example: A radar might have `hasDualRange: true` in characteristics, but the provider might not implement dual-range API endpoints yet. In that case, `dualRange` would NOT be in `supportedFeatures`.

---

## Future Features

The `supportedFeatures` array is extensible. Potential additions:

| Feature | Description |
|---------|-------------|
| `noTransmitZones` | Sector blanking configuration |
| `targetAssociation` | Associate radar targets with AIS |
| `recording` | Record/playback spoke data |
| `colorPalettes` | Custom display palettes |

---

## SignalK Server Changes Required

To implement this in SignalK server:

1. Add `supportedFeatures` to `CapabilityManifest` type in `radarapi.ts`
2. No routing changes needed - it's just a field in the capabilities response
3. Document in radar_api.md

---

## Summary

| Question | Answer |
|----------|--------|
| Is ARPA optional? | Yes - provider methods are optional (`getTargets?:`) |
| How does client know? | Check `capabilities.supportedFeatures.includes('arpa')` |
| Backward compatible? | Yes - field is optional, clients use defensive checks |
| Extensible? | Yes - just add new feature strings as API grows |

# Project Freedom: ANY-to-ANY Radar Interoperability

> **Status**: IDEA - Not implemented. Needs review and discussion before development.

## Vision

Enable **any chart plotter** to work with **any radar**, regardless of manufacturer.
- Furuno MFD → Navico HALO
- Simrad NSX → Garmin Fantom
- Raymarine Axiom → Furuno DRS-NXT
- **True hardware freedom for marine electronics.**

---

## Feasibility: YES (with caveats)

The mayara architecture is uniquely positioned for this:

1. **All 4 protocols decoded** - Furuno, Navico, Raymarine, Garmin in `/docs/radar-protocols/`
2. **Unified abstraction exists** - `mayara-core` already normalizes radar data
3. **Good compatibility** - Most radars use 2048 spokes, 4-bit pixels

---

## Architecture: Universal Translation Layer

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           PROJECT FREEDOM                                 │
│                                                                          │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐            │
│  │  Furuno    │ │  Navico    │ │ Raymarine  │ │  Garmin    │            │
│  │  Emulator  │ │  Emulator  │ │  Emulator  │ │  Emulator  │  ← MFD     │
│  │  (server)  │ │  (server)  │ │  (server)  │ │  (server)  │    side    │
│  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └─────┬──────┘            │
│        │              │              │              │                    │
│        └──────────────┴──────────────┴──────────────┘                    │
│                              │                                           │
│                              ▼                                           │
│        ┌─────────────────────────────────────────────┐                  │
│        │         UNIFIED RADAR STATE                  │                  │
│        │  • 2048 spokes (normalized)                  │                  │
│        │  • 8-bit pixels (highest fidelity)           │                  │
│        │  • Abstract controls (gain, sea, rain...)    │                  │
│        │  • Feature flags (doppler, dual-range...)    │                  │
│        └─────────────────────────────────────────────┘                  │
│                              │                                           │
│        ┌──────────────┬──────┴───────┬──────────────┐                   │
│        │              │              │              │                    │
│  ┌─────┴──────┐ ┌─────┴──────┐ ┌─────┴──────┐ ┌─────┴──────┐            │
│  │  Furuno    │ │  Navico    │ │ Raymarine  │ │  Garmin    │  ← Radar   │
│  │  Driver    │ │  Driver    │ │  Driver    │ │  Driver    │    side    │
│  │  (client)  │ │  (client)  │ │  (client)  │ │  (client)  │            │
│  └────────────┘ └────────────┘ └────────────┘ └────────────┘            │
│                    (existing mayara-core code)                           │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## Protocol Compatibility Matrix

### Spoke Characteristics

| Brand | Spokes/Rev | Pixel Depth | Encoding | Resampling Needed? |
|-------|-----------|-------------|----------|-------------------|
| Furuno | 2048 | 4-bit | raw | No (baseline) |
| Navico | 2048 | 4-bit | nibble-packed | No |
| Raymarine | 2048 | 4-7 bit | RLE compressed | Decompress only |
| Garmin | **1440** | **8-bit** | unpacked | **Yes: 1440↔2048** |

### Translation Difficulty

| From ↓ To → | Furuno MFD | Navico MFD | Raymarine MFD | Garmin MFD |
|-------------|------------|------------|---------------|------------|
| **Furuno Radar** | N/A | ⚠️ Moderate | ⚠️ Moderate | ⚠️ Moderate |
| **Navico Radar** | ⚠️ Moderate | N/A | ✅ Easy | ⚠️ Angular resample |
| **Raymarine Radar** | ⚠️ Moderate | ✅ Easy | N/A | ⚠️ Angular resample |
| **Garmin Radar** | ⚠️ Quantize+resample | ⚠️ Quantize+resample | ⚠️ Quantize+resample | N/A |

### Legend
- ✅ **Easy**: Same spokes, same pixel depth, just reformat
- ⚠️ **Moderate**: Protocol translation + possible minor conversion

---

## What Translates Well

| Feature | Translatable? | Notes |
|---------|--------------|-------|
| Transmit/Standby | ✅ Yes | Universal |
| Range selection | ✅ Yes | Map range tables |
| Gain (manual/auto) | ✅ Yes | Scale 0-100 ↔ 0-255 |
| Sea clutter | ✅ Yes | Scale values |
| Rain clutter | ✅ Yes | Scale values |
| Interference rejection | ⚠️ Partial | Different level counts |
| Bearing alignment | ✅ Yes | Both use degrees |
| No-transmit zones | ⚠️ Partial | Different zone counts |
| Scan speed | ⚠️ Partial | Not all support it |

## What CANNOT Translate

| Feature | Why | Affected |
|---------|-----|----------|
| **Doppler** | Only HALO & Quantum Q24D have it | Color-coded moving targets lost |
| **RezBoost** | Furuno-specific signal processing | No equivalent |
| **Bird Mode** | Furuno-specific | No equivalent |
| **Target Analyzer** | Furuno-specific | Maps to Doppler on HALO only |
| **Dual-range** | Different implementations | Partial: Furuno vs Navico differ |
| **FAR commercial features** | 27 undocumented features | Cannot translate what we don't know |

---

## Protocol Gaps Needing More Research

| Protocol | Gap | Impact |
|----------|-----|--------|
| **Furuno** | ~50% of command IDs undocumented | FAR series features unknown |
| **Furuno** | Magnetron warmup curve | Cannot simulate FAR startup |
| **Navico** | B&G specific variations? | May need B&G MFD captures |
| **Raymarine** | Beacon retry/timeout logic | Discovery robustness |
| **Garmin** | HD vs xHD protocol differences | Legacy support |

**To improve**: Capture network traffic from B&G MFDs, Simrad NSS/NSX, more Garmin models.

---

## Implementation Plan

### Crate Structure (ANY-to-ANY)

```
mayara-server/
├── mayara-freedom/                    # NEW standalone binary
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                    # CLI entry, tokio runtime
│       ├── config.rs                  # TOML configuration
│       │
│       ├── state.rs                   # UnifiedRadarState (normalized)
│       │   • 2048 spokes, 8-bit pixels
│       │   • Abstract controls
│       │   • Feature capability flags
│       │
│       ├── emulator/                  # MFD-facing (server-side)
│       │   ├── mod.rs                 # EmulatorTrait
│       │   ├── furuno.rs              # TCP login, ASCII commands, spoke multicast
│       │   ├── navico.rs              # UDP beacon, binary commands
│       │   ├── raymarine.rs           # Two-phase beacon, binary commands
│       │   └── garmin.rs              # Implicit discovery, uniform packets
│       │
│       ├── translator/                # Protocol translation
│       │   ├── mod.rs
│       │   ├── spoke.rs               # Angular resampling, pixel quantization
│       │   └── control.rs             # Abstract control → brand-specific
│       │
│       └── bridge.rs                  # Coordinator: emulator ↔ state ↔ driver
│
├── mayara-core/                       # Existing - radar drivers (client-side)
│   └── src/controllers/               # Reuse: FurunoController, NavicoController, etc.
│
└── Cargo.toml                         # Add mayara-freedom to workspace
```

### Unified State Model

```rust
/// Normalized radar state - highest common denominator
pub struct UnifiedRadarState {
    // Spoke data (normalized)
    pub spokes_per_rev: u16,           // Always 2048 internally
    pub pixel_depth: u8,               // Always 8-bit internally
    pub current_range_m: u32,          // Meters

    // Controls (abstract)
    pub power: PowerState,             // Off/Standby/Transmit/Warming
    pub gain: ControlValue,            // 0-100 + auto flag
    pub sea: ControlValue,             // 0-100 + auto flag
    pub rain: ControlValue,            // 0-100
    pub interference_rejection: u8,    // 0-3 (normalized)
    pub bearing_alignment: i16,        // Degrees × 10
    pub blind_sectors: Vec<BlindSector>,

    // Capabilities (what the actual radar supports)
    pub has_doppler: bool,
    pub has_dual_range: bool,
    pub max_range_m: u32,
    pub supported_features: FeatureSet,
}
```

### Implementation Phases

**Phase 1: Foundation**
- Create `mayara-freedom` crate
- Implement `UnifiedRadarState`
- Implement `EmulatorTrait` interface
- Basic config file parsing

**Phase 2: First Pair (Furuno MFD ↔ Navico Radar)**
- `FurunoEmulator`: beacon, TCP login, command parsing
- Wire to existing `NavicoController`
- Spoke translation (header only - pixels compatible)
- Control mapping table

**Phase 3: Add More Emulators**
- `NavicoEmulator` (for Simrad/Lowrance/B&G MFDs)
- `RaymarineEmulator` (for Axiom MFDs)
- `GarminEmulator` (for GPSMAP MFDs)

**Phase 4: Angular Resampling**
- 1440 ↔ 2048 spoke interpolation (for Garmin)
- Pixel depth conversion (8-bit ↔ 4-bit)

**Phase 5: Advanced Features**
- Dual-range pass-through
- Doppler preservation (where possible)
- ARPA target forwarding
- Guard zone translation

### Network Configuration

```toml
# mayara-freedom.toml

[mfd]
brand = "furuno"           # What MFD expects
interface = "eth0"         # MFD-facing NIC
advertise_model = "DRS4D-NXT"

[radar]
brand = "navico"           # Actual radar brand
interface = "eth1"         # Radar-facing NIC
# model auto-detected from beacon

[features]
doppler_passthrough = true   # If both support it
dual_range = false           # Complex, disable initially
log_unsupported = true       # Log when features can't translate
```

---

## Critical Files

| File | Purpose |
|------|---------|
| `mayara-core/src/protocol/furuno/command.rs` | Furuno command format functions |
| `mayara-core/src/protocol/furuno/dispatch.rs` | Command ID → wire format routing |
| `mayara-core/src/controllers/navico.rs` | Navico control methods |
| `mayara-core/src/protocol/navico.rs` | Navico spoke/report parsing |
| `docs/radar-protocols/furuno/protocol.md` | Complete Furuno protocol spec |
| `docs/radar-protocols/navico/protocol.md` | Complete Navico protocol spec |

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Firmware updates change protocol | Version detection, fallback modes |
| Control semantics don't map 1:1 | Document gaps, best-effort translation |
| Timing sensitivity | Benchmark, optimize hot paths |
| Network complexity | Clear docs, auto-configuration |

---

## Conclusion

**ANY-to-ANY is technically feasible.** Here's the reality:

### What Works Great (12 of 16 combinations)
- Furuno ↔ Navico ↔ Raymarine (all 2048 spokes, 4-bit)
- Just header reformatting + control translation

### What Needs Extra Work (4 combinations involving Garmin)
- Garmin uses 1440 spokes + 8-bit pixels
- Requires angular resampling + pixel quantization
- Some fidelity loss but functional

### What Cannot Translate
- Doppler (only HALO, Quantum Q24D have it)
- Brand-specific signal processing (RezBoost, Bird Mode)
- Undocumented FAR commercial features

### Recommended Starting Point
**Furuno MFD ↔ Navico Radar** - Perfect compatibility, well-documented protocols

### Expand To
1. NavicoEmulator (Simrad/Lowrance/B&G MFDs)
2. RaymarineEmulator (Axiom MFDs)
3. GarminEmulator (with resampling)

### Protocol Gaps to Fill
- B&G MFD captures (verify Navico compatibility)
- More Garmin model testing
- Furuno FAR commercial features

---

## Discussion Points

Before starting implementation, consider:

1. **Legal/ethical**: Any concerns with protocol translation for personal use?
2. **Safety**: How to handle translation errors in safety-critical features?
3. **Priority**: Which MFD↔Radar combinations are most valuable to users?
4. **Testing**: How to test without owning every brand combination?
5. **Community**: Would others contribute protocol captures?

---

*This document was generated during a brainstorming session exploring the feasibility of cross-brand radar interoperability using the mayara architecture.*

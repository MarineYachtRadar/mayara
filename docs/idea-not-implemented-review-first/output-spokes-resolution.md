# Output Resolution Profile for Radar Spokes

## Problem
Furuno hardware sends 8192 spokes per revolution, but web clients (GUI) allocate textures based on the reported `spokesPerRevolution`. This causes:
- 8MB+ texture allocations (8192 × 1024 bytes)
- The WASM plugin already downsamples to 2048 spokes, but standalone server doesn't
- Model database currently has 8192 for Furuno (hardware truth), but this shouldn't be exposed to clients

## Current State
- **WASM plugin**: Already downsamples Furuno 8192→2048 via `FURUNO_SPOKE_REDUCTION = 4` in `spoke_receiver.rs:17-21`
- **Standalone server**: Passes hardware spokes directly to clients (no reduction)
- **Capabilities builder**: Has `build_capabilities_from_model_with_spokes()` that accepts custom values
- **GUI**: Dynamically allocates based on received `spokesPerRevolution`

## Proposed Solution

Add an `output_spokes` field to `ModelInfo` that represents the **recommended output resolution** for clients, separate from the native hardware resolution.

### Key Principle
- `spokes_per_revolution`: Native hardware capability (truth)
- `output_spokes`: Recommended output resolution for clients (what servers should report)

## Implementation

### 1. Add `output_spokes` to ModelInfo
**File:** `mayara-core/src/models/mod.rs`

```rust
pub struct ModelInfo {
    // ... existing fields ...
    pub spokes_per_revolution: u16,  // Native hardware (keep as 8192 for Furuno)
    pub output_spokes: u16,          // NEW: Recommended output (2048 for Furuno)
    pub max_spoke_length: u16,
    // ...
}
```

### 2. Update Furuno Model Definitions
**File:** `mayara-core/src/models/furuno.rs`

For all Furuno models:
```rust
spokes_per_revolution: 8192,  // Native hardware
output_spokes: 2048,          // Reduced for clients
```

### 3. Update Other Brand Models
**Files:** `mayara-core/src/models/{navico,raymarine,garmin}.rs`

For brands with 2048 native:
```rust
spokes_per_revolution: 2048,
output_spokes: 2048,  // Same as native
```

### 4. Update UNKNOWN_MODEL
**File:** `mayara-core/src/models/mod.rs`

```rust
pub static UNKNOWN_MODEL: ModelInfo = ModelInfo {
    // ...
    spokes_per_revolution: 2048,
    output_spokes: 2048,
    // ...
};
```

### 5. Update Capabilities Builder
**File:** `mayara-core/src/capabilities/builder.rs`

Update `build_capabilities()` and `build_capabilities_from_model()` to use `output_spokes`:

```rust
characteristics: Characteristics {
    spokes_per_revolution: model_info.output_spokes,  // Use output, not native
    // ...
}
```

### 6. Update Standalone Server
**File:** `mayara-server/src/web.rs`

In `get_radar_capabilities()`, use `model_info.output_spokes`:

```rust
let capabilities = build_capabilities_from_model_with_spokes(
    model_info,
    &params.radar_id,
    supported_features,
    model_info.output_spokes,  // Use output spokes from model
    info.max_spoke_len,
);
```

### 7. Verify WASM Plugin
**File:** `mayara-signalk-wasm/src/spoke_receiver.rs`

Already correct - uses `FURUNO_OUTPUT_SPOKES = 2048`. The `build_capabilities()` in radar_provider.rs uses discovery which comes from the protocol (8192), so we need to update that to use model's output_spokes too.

**File:** `mayara-signalk-wasm/src/radar_provider.rs`

Update `get_capabilities_v5()` to use model's `output_spokes` instead of discovery's `spokes_per_revolution`.

## Files to Modify
1. `mayara-core/src/models/mod.rs` - Add `output_spokes` field to ModelInfo and UNKNOWN_MODEL
2. `mayara-core/src/models/furuno.rs` - Add `output_spokes: 2048` to all models
3. `mayara-core/src/models/navico.rs` - Add `output_spokes: 2048` to all models
4. `mayara-core/src/models/raymarine.rs` - Add `output_spokes: 2048` to all models
5. `mayara-core/src/models/garmin.rs` - Add `output_spokes: 2048` to all models
6. `mayara-core/src/capabilities/builder.rs` - Use `output_spokes` in builders
7. `mayara-server/src/web.rs` - Use `model_info.output_spokes` in capabilities endpoint
8. `mayara-signalk-wasm/src/radar_provider.rs` - Use model's `output_spokes` in capabilities

## Benefits
- Clean separation of hardware truth vs client-facing resolution
- Model database becomes the single source of truth for output resolution
- No changes needed to GUI - it adapts automatically
- Future flexibility: could add different output profiles (e.g., `output_spokes_hd: 4096`)

## Status
**NOT IMPLEMENTED** - User wants to test with 8192 first in the new design before implementing this reduction.

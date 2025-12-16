//! Radar Provider
//!
//! Implements the SignalK Radar Provider interface.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};

use mayara_core::arpa::{ArpaEvent, ArpaProcessor, ArpaSettings, ArpaTarget};
use mayara_core::capabilities::{
    builder::build_capabilities, CapabilityManifest, ControlError, RadarStateV5, SupportedFeature,
};
use mayara_core::controllers::{
    FurunoController, GarminController, NavicoController, NavicoModel, RaymarineController,
    RaymarineVariant,
};
use mayara_core::dual_range::{DualRangeConfig, DualRangeController, DualRangeState};
use mayara_core::guard_zones::{GuardZone, GuardZoneProcessor, GuardZoneStatus};
use mayara_core::radar::RadarDiscovery;
use mayara_core::trails::{TrailData, TrailSettings, TrailStore};
use mayara_core::Brand;
use crate::locator::{LocatorEvent, RadarLocator};
use crate::signalk_ffi::{debug, emit_json, read_config, save_config};
use crate::spoke_receiver::{SpokeReceiver, FURUNO_OUTPUT_SPOKES};
use crate::wasm_io::WasmIoProvider;

// =============================================================================
// Unified Radar Controller
// =============================================================================

/// Unified controller enum for all radar brands.
///
/// This consolidates the 4 separate controller maps into a single dispatch pattern,
/// reducing code duplication in control methods.
pub enum RadarController {
    Furuno(FurunoController),
    Navico(NavicoController),
    Raymarine(RaymarineController),
    Garmin(GarminController),
}

impl RadarController {
    /// Poll the controller for events.
    /// Returns true if the controller is connected/active.
    pub fn poll(&mut self, io: &mut WasmIoProvider) -> bool {
        match self {
            RadarController::Furuno(c) => {
                let _events = c.poll(io);
                // Furuno returns events, consider connected if state is not Disconnected
                c.is_connected()
            }
            RadarController::Navico(c) => c.poll(io),
            RadarController::Raymarine(c) => c.poll(io),
            RadarController::Garmin(c) => c.poll(io),
        }
    }

    /// Get the brand of this controller.
    #[allow(dead_code)]
    pub fn brand(&self) -> Brand {
        match self {
            RadarController::Furuno(_) => Brand::Furuno,
            RadarController::Navico(_) => Brand::Navico,
            RadarController::Raymarine(_) => Brand::Raymarine,
            RadarController::Garmin(_) => Brand::Garmin,
        }
    }

    /// Set power/transmit state.
    pub fn set_power(&mut self, io: &mut WasmIoProvider, transmit: bool) {
        match self {
            RadarController::Furuno(c) => c.set_transmit(io, transmit),
            RadarController::Navico(c) => c.set_power(io, transmit),
            RadarController::Raymarine(c) => c.set_power(io, transmit),
            RadarController::Garmin(c) => c.set_power(io, transmit),
        }
    }

    /// Set range in meters.
    pub fn set_range(&mut self, io: &mut WasmIoProvider, range: u32) {
        match self {
            RadarController::Furuno(c) => c.set_range(io, range),
            RadarController::Navico(c) => c.set_range(io, (range * 10) as i32), // decimeters
            RadarController::Raymarine(c) => c.set_range(io, (range / 100).min(255) as u8), // index
            RadarController::Garmin(c) => c.set_range(io, range),
        }
    }

    /// Set gain.
    pub fn set_gain(&mut self, io: &mut WasmIoProvider, value: u8, auto: bool) {
        match self {
            RadarController::Furuno(c) => c.set_gain(io, value as i32, auto),
            RadarController::Navico(c) => c.set_gain(io, value, auto),
            RadarController::Raymarine(c) => c.set_gain(io, value, auto),
            RadarController::Garmin(c) => c.set_gain(io, value as u32, auto),
        }
    }

    /// Set sea clutter.
    pub fn set_sea(&mut self, io: &mut WasmIoProvider, value: u8, auto: bool) {
        match self {
            RadarController::Furuno(c) => c.set_sea(io, value as i32, auto),
            RadarController::Navico(c) => c.set_sea(io, value, auto),
            RadarController::Raymarine(c) => c.set_sea(io, value, auto),
            RadarController::Garmin(c) => c.set_sea(io, value as u32, auto),
        }
    }

    /// Set rain clutter.
    pub fn set_rain(&mut self, io: &mut WasmIoProvider, value: u8, auto: bool) {
        match self {
            RadarController::Furuno(c) => c.set_rain(io, value as i32, auto),
            RadarController::Navico(c) => c.set_rain(io, value), // Navico rain has no auto
            RadarController::Raymarine(c) => c.set_rain(io, value, auto),
            RadarController::Garmin(c) => c.set_rain(io, value as u32, auto),
        }
    }

    /// Get as Furuno controller (for extended controls).
    pub fn as_furuno(&self) -> Option<&FurunoController> {
        match self {
            RadarController::Furuno(c) => Some(c),
            _ => None,
        }
    }

    /// Get as mutable Furuno controller (for extended controls).
    #[allow(dead_code)]
    pub fn as_furuno_mut(&mut self) -> Option<&mut FurunoController> {
        match self {
            RadarController::Furuno(c) => Some(c),
            _ => None,
        }
    }

    /// Get the radar's detected model name (if available).
    pub fn model(&self) -> Option<&str> {
        match self {
            RadarController::Furuno(c) => c.model(),
            RadarController::Navico(_) => None, // TODO: implement
            RadarController::Raymarine(_) => None, // TODO: implement
            RadarController::Garmin(_) => None, // TODO: implement
        }
    }

    /// Get radar state (currently only Furuno).
    pub fn radar_state(&self) -> Option<&mayara_core::state::RadarState> {
        match self {
            RadarController::Furuno(c) => Some(c.radar_state()),
            _ => None,
        }
    }
}

// =============================================================================

/// Custom deserializer for antenna height that accepts both float and int
/// Handles migration from old category values (0, 1, 2) to meters (0-100)
fn deserialize_antenna_height<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    // Try to deserialize as any JSON value
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;

    match value {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => {
            // Accept both integer and float, convert to i32
            if let Some(i) = n.as_i64() {
                // Migrate old category values (0, 1, 2) to meters
                let meters = match i {
                    0 => 2,   // Old "Under 3m" -> 2m
                    1 => 5,   // Old "3-10m" -> 5m
                    2 => 15,  // Old "Over 10m" -> 15m
                    _ => i.clamp(0, 100) as i32,  // Already meters, clamp to range
                };
                Ok(Some(meters))
            } else if let Some(f) = n.as_f64() {
                // Float value - treat as meters directly
                Ok(Some((f as i32).clamp(0, 100)))
            } else {
                Err(D::Error::custom("invalid antenna height value"))
            }
        }
        Some(_) => Err(D::Error::custom("antenna height must be a number")),
    }
}

/// Installation configuration for a radar
///
/// These are configuration values stored locally, not queried from the radar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarInstallationConfig {
    /// Bearing alignment offset in degrees (-180 to 180)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearing_alignment: Option<f64>,
    /// Antenna height in meters (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_antenna_height")]
    pub antenna_height: Option<i32>,
}

/// Plugin configuration stored via SignalK
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    /// Installation configs per radar ID
    #[serde(default)]
    pub radars: HashMap<String, RadarInstallationConfig>,
}

/// Sanitize a string to be safe for JSON and SignalK paths
fn sanitize_string(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Legend entry for PPI color mapping
#[derive(Debug, Clone, Serialize)]
pub struct LegendEntry {
    pub color: String,
}

/// Radar state for SignalK API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarState {
    pub id: String,
    pub name: String,
    pub brand: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: String,
    pub spokes_per_revolution: u16,
    pub max_spoke_len: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_url: Option<String>,
    pub controls: BTreeMap<String, serde_json::Value>,
    pub legend: BTreeMap<String, LegendEntry>,
}

impl From<&RadarDiscovery> for RadarState {
    fn from(d: &RadarDiscovery) -> Self {
        let sanitized_name = sanitize_string(&d.name);
        let brand_str = d.brand.as_str();
        let id = format!("{}-{}", brand_str, sanitized_name);
        let ip = d.address.split(':').next().unwrap_or(&d.address);

        // Build default legend (256 entries)
        // Furuno radars use 6-bit values (0-63), so we scale to that range
        // Color gradient matching TimeZero Pro style (Green → Yellow → Orange → Red):
        // - Index 0: transparent (noise floor)
        // - Index 1-15: dark green (weak returns)
        // - Index 16-31: green to yellow (medium returns)
        // - Index 32-47: yellow to orange (stronger returns)
        // - Index 48-63: orange to bright red (strong returns / land)
        // - Index 64-255: max red (overflow)
        let mut legend = BTreeMap::new();
        for i in 0..256u16 {
            let (r, g, b) = if i == 0 {
                // Index 0: transparent/black (noise floor)
                (0u8, 0u8, 0u8)
            } else if i <= 15 {
                // 1-15: dark green (weak returns)
                let t = (i - 1) as f32 / 14.0;
                (0, (50.0 + t * 100.0) as u8, 0)
            } else if i <= 31 {
                // 16-31: green to yellow-green
                let t = (i - 16) as f32 / 15.0;
                ((t * 200.0) as u8, (150.0 + t * 55.0) as u8, 0)
            } else if i <= 47 {
                // 32-47: yellow to orange
                let t = (i - 32) as f32 / 15.0;
                ((200.0 + t * 55.0) as u8, (180.0 - t * 100.0) as u8, 0)
            } else if i <= 63 {
                // 48-63: orange to bright red (strong returns / land)
                let t = (i - 48) as f32 / 15.0;
                (255u8, (80.0 - t * 80.0).max(0.0) as u8, 0)
            } else {
                // 64-255: max red (overflow protection)
                (255u8, 0u8, 0u8)
            };
            let color = format!("#{:02X}{:02X}{:02X}", r, g, b);
            legend.insert(i.to_string(), LegendEntry { color });
        }

        // Build basic controls
        let mut controls = BTreeMap::new();

        // Control 0: Status (read-only, required by webapp)
        controls.insert(
            "0".to_string(),
            serde_json::json!({
                "name": "Status",
                "isReadOnly": true
            }),
        );

        // Control 1: Power transmit/standby
        controls.insert(
            "1".to_string(),
            serde_json::json!({
                "name": "Power",
                "validValues": ["transmit", "standby"],
                "descriptions": {
                    "transmit": "Transmit",
                    "standby": "Standby"
                }
            }),
        );

        // Note: control_url is for mayara-server if running separately
        // stream_url is omitted so clients use SignalK's built-in /radars/{id}/stream
        let _ = ip; // Suppress unused warning

        // For Furuno radars, we reduce 8192 spokes to 2048 for WebSocket efficiency
        // This reduction happens in spoke_receiver.rs using max-of-4 combining
        let spokes_per_revolution = if d.brand == Brand::Furuno {
            FURUNO_OUTPUT_SPOKES
        } else {
            d.spokes_per_revolution
        };

        Self {
            id: id.clone(),
            name: sanitized_name.clone(),
            brand: brand_str.to_string(),
            model: d.model.clone().map(|m| sanitize_string(&m)),
            status: "standby".to_string(),
            spokes_per_revolution,
            max_spoke_len: d.max_spoke_len,
            // No external streamUrl - clients use SignalK's built-in /radars/{id}/stream
            // Spokes are emitted via sk_radar_emit_spokes FFI
            stream_url: None,
            // No external controlUrl - use SignalK REST API for controls
            control_url: None,
            controls,
            legend,
        }
    }
}

/// Radar Provider implementation
pub struct RadarProvider {
    /// I/O provider for platform-independent socket operations
    io: WasmIoProvider,
    locator: RadarLocator,
    spoke_receiver: SpokeReceiver,
    /// Unified controllers for all radar brands (keyed by radar ID)
    controllers: BTreeMap<String, RadarController>,
    /// ARPA processors for each radar (keyed by radar ID)
    arpa_processors: BTreeMap<String, ArpaProcessor>,
    /// Guard zone processors for each radar (keyed by radar ID)
    /// Note: Methods exist but FFI exports not yet implemented
    #[allow(dead_code)]
    guard_zone_processors: BTreeMap<String, GuardZoneProcessor>,
    /// Trail stores for each radar (keyed by radar ID)
    /// Note: Methods exist but FFI exports not yet implemented
    #[allow(dead_code)]
    trail_stores: BTreeMap<String, TrailStore>,
    /// Dual-range controllers for each radar (keyed by radar ID)
    /// Note: Methods exist but FFI exports not yet implemented
    #[allow(dead_code)]
    dual_range_controllers: BTreeMap<String, DualRangeController>,
    poll_count: u64,
    /// Plugin configuration (installation settings per radar)
    config: PluginConfig,
}

impl RadarProvider {
    /// Create a new radar provider
    pub fn new() -> Self {
        let mut io = WasmIoProvider::new();
        let mut locator = RadarLocator::new();
        locator.start(&mut io);

        // Load saved configuration
        let config = Self::load_config();
        debug(&format!("Loaded config: {} radars configured", config.radars.len()));

        Self {
            io,
            locator,
            spoke_receiver: SpokeReceiver::new(),
            controllers: BTreeMap::new(),
            arpa_processors: BTreeMap::new(),
            guard_zone_processors: BTreeMap::new(),
            trail_stores: BTreeMap::new(),
            dual_range_controllers: BTreeMap::new(),
            poll_count: 0,
            config,
        }
    }

    /// Load configuration from SignalK
    fn load_config() -> PluginConfig {
        if let Some(json) = read_config() {
            match serde_json::from_str::<PluginConfig>(&json) {
                Ok(config) => {
                    debug(&format!("Loaded config from SignalK: {:?}", config));
                    return config;
                }
                Err(e) => {
                    debug(&format!("Failed to parse config, using defaults: {}", e));
                }
            }
        }
        PluginConfig::default()
    }

    /// Save configuration to SignalK
    fn save_config(&self) {
        match serde_json::to_string(&self.config) {
            Ok(json) => {
                if save_config(&json) {
                    debug(&format!("Saved config to SignalK: {} radars", self.config.radars.len()));
                } else {
                    debug("Failed to save config to SignalK");
                }
            }
            Err(e) => {
                debug(&format!("Failed to serialize config: {}", e));
            }
        }
    }

    /// Get installation config for a radar
    #[allow(dead_code)]
    pub fn get_installation_config(&self, radar_id: &str) -> Option<&RadarInstallationConfig> {
        self.config.radars.get(radar_id)
    }

    /// Set installation config for a radar and save
    pub fn set_installation_config(&mut self, radar_id: &str, config: RadarInstallationConfig) {
        self.config.radars.insert(radar_id.to_string(), config);
        self.save_config();
    }

    /// Poll for radar events
    pub fn poll(&mut self) -> i32 {
        self.poll_count += 1;

        // Update timestamp (in a real implementation, get from host)
        self.io.set_time(self.poll_count * 100);

        // Poll for new radars
        let new_radars = self.locator.poll(&mut self.io);

        // Emit delta for each new radar
        for event in &new_radars {
            if let LocatorEvent::RadarDiscovered(discovery) = event {
                self.emit_radar_discovered(discovery);
            }
        }

        // Register ALL Furuno radars for spoke tracking and create controllers
        // This ensures radars discovered before spoke_receiver was ready are also tracked
        let radar_count = self.locator.radars.len();
        if self.poll_count % 100 == 1 {
            debug(&format!("Checking {} radars for spoke tracking", radar_count));
        }

        // Collect radar info to create controllers (avoid borrow issues)
        struct RadarInfo {
            id: String,
            ip: String,
            brand: Brand,
            cmd_port: u16,
            data_port: u16,
            model: Option<String>,
        }
        let radar_infos: Vec<RadarInfo> = self.locator.radars.values()
            .map(|r| {
                let state = RadarState::from(&r.discovery);
                RadarInfo {
                    id: state.id,
                    ip: r.discovery.address.split(':').next().unwrap_or(&r.discovery.address).to_string(),
                    brand: r.discovery.brand,
                    cmd_port: r.discovery.command_port,
                    data_port: r.discovery.data_port,
                    model: r.discovery.model.clone(),
                }
            })
            .collect();

        // Create controllers for each radar
        for info in radar_infos {
            if self.controllers.contains_key(&info.id) {
                continue;
            }

            debug(&format!("Creating controller for {} ({:?}) at {}", info.id, info.brand, info.ip));

            let controller = match info.brand {
                Brand::Furuno => {
                    // Register for spoke tracking
                    self.spoke_receiver.add_furuno_radar(&info.id, &info.ip, &mut self.io);
                    RadarController::Furuno(FurunoController::new(&info.id, &info.ip))
                }
                Brand::Navico => {
                    // Navico uses default multicast addresses
                    let cmd_addr = "236.6.7.8";
                    let report_addr = "236.6.7.9";
                    let report_port = 6680u16;
                    let navico_model = match info.model.as_deref() {
                        Some(m) if m.contains("HALO") => NavicoModel::Halo,
                        Some(m) if m.contains("4G") => NavicoModel::Gen4,
                        Some(m) if m.contains("3G") => NavicoModel::Gen3,
                        Some(m) if m.contains("BR24") => NavicoModel::BR24,
                        _ => NavicoModel::Gen4,
                    };
                    RadarController::Navico(NavicoController::new(
                        &info.id, cmd_addr, info.cmd_port, report_addr, report_port, navico_model,
                    ))
                }
                Brand::Raymarine => {
                    let (variant, has_doppler) = match info.model.as_deref() {
                        Some(m) if m.contains("Quantum 2") => (RaymarineVariant::Quantum, true),
                        Some(m) if m.contains("Quantum") => (RaymarineVariant::Quantum, false),
                        _ => (RaymarineVariant::RD, false),
                    };
                    RadarController::Raymarine(RaymarineController::new(
                        &info.id, &info.ip, info.cmd_port, &info.ip, info.data_port, variant, has_doppler,
                    ))
                }
                Brand::Garmin => {
                    RadarController::Garmin(GarminController::new(&info.id, &info.ip))
                }
            };
            self.controllers.insert(info.id, controller);
        }

        // Poll all controllers and update model info
        for (radar_id, controller) in self.controllers.iter_mut() {
            controller.poll(&mut self.io);

            // Update radar discovery with model from controller (if available)
            if let Some(model) = controller.model() {
                for radar_info in self.locator.radars.values_mut() {
                    let state = RadarState::from(&radar_info.discovery);
                    if state.id == *radar_id && radar_info.discovery.model.as_deref() != Some(model) {
                        debug(&format!(
                            "Updating radar {} model from controller: {:?} -> {}",
                            radar_id, radar_info.discovery.model, model
                        ));
                        radar_info.discovery.model = Some(model.to_string());
                    }
                }
            }
        }

        // Poll for spoke data and emit to SignalK stream
        let spokes_emitted = self.spoke_receiver.poll(&mut self.io);

        // Log spoke activity periodically (every 100 polls or when spokes emitted)
        if self.poll_count % 100 == 0 {
            debug(&format!(
                "RadarProvider poll #{}: {} radars, {} spokes emitted",
                self.poll_count,
                self.locator.radars.len(),
                spokes_emitted
            ));
        }

        // Periodically emit radar list
        if self.poll_count % 100 == 0 {
            self.emit_radar_list();
        }

        0
    }

    /// Emit a radar discovery delta
    fn emit_radar_discovered(&self, discovery: &RadarDiscovery) {
        let state = RadarState::from(discovery);
        let path = format!("radars.{}", state.id);

        // Debug: show what we're sending
        if let Ok(json) = serde_json::to_string(&state) {
            debug(&format!("Radar JSON ({}): {}", json.len(), &json[..json.len().min(200)]));
        }

        emit_json(&path, &state);
        debug(&format!("Emitted radar discovery: {} at path {}", state.id, path));
    }

    /// Emit the full radar list
    fn emit_radar_list(&self) {
        let count = self.locator.radars.len();
        if count == 0 {
            return;
        }

        // Emit each radar individually (SignalK expects individual path updates)
        for radar_info in self.locator.radars.values() {
            let state = RadarState::from(&radar_info.discovery);
            let path = format!("radars.{}", state.id);
            emit_json(&path, &state);
        }

        debug(&format!("Emitted {} radar(s)", count));
    }

    /// Shutdown the provider
    pub fn shutdown(&mut self) {
        self.locator.shutdown(&mut self.io);
        self.spoke_receiver.shutdown(&mut self.io);
    }

    /// Get list of radar IDs for the Radar Provider API
    pub fn get_radar_ids(&self) -> Vec<&str> {
        self.locator
            .radars
            .values()
            .map(|r| {
                // Generate the same ID format as RadarState
                // We need to return &str, so we'll store the IDs differently
                // For now, leak the string (acceptable in WASM single-use context)
                let state = RadarState::from(&r.discovery);
                let id: &'static str = Box::leak(state.id.into_boxed_str());
                id
            })
            .collect()
    }

    /// Get radar info for the Radar Provider API
    pub fn get_radar_info(&self, radar_id: &str) -> Option<RadarState> {
        // Find the radar by ID
        for radar_info in self.locator.radars.values() {
            let state = RadarState::from(&radar_info.discovery);
            if state.id == radar_id {
                return Some(state);
            }
        }
        None
    }

    /// Find radar discovery by ID
    fn find_radar(&self, radar_id: &str) -> Option<&crate::locator::DiscoveredRadar> {
        for radar_info in self.locator.radars.values() {
            let state = RadarState::from(&radar_info.discovery);
            if state.id == radar_id {
                return Some(radar_info);
            }
        }
        None
    }

    /// Set radar power state
    pub fn set_power(&mut self, radar_id: &str, state: &str) -> bool {
        debug(&format!("set_power({}, {})", radar_id, state));
        let transmit = state == "transmit";

        // Send Furuno announce if this is a Furuno radar
        if let Some(controller) = self.controllers.get(radar_id) {
            if matches!(controller, RadarController::Furuno(_)) {
                self.locator.send_furuno_announce(&mut self.io);
            }
        }

        if let Some(controller) = self.controllers.get_mut(radar_id) {
            controller.set_power(&mut self.io, transmit);
            return true;
        }

        debug(&format!("No controller found for '{}'", radar_id));
        false
    }

    /// Set radar range in meters
    pub fn set_range(&mut self, radar_id: &str, range: u32) -> bool {
        debug(&format!("set_range({}, {}m)", radar_id, range));

        // Send Furuno announce if this is a Furuno radar
        if let Some(controller) = self.controllers.get(radar_id) {
            if matches!(controller, RadarController::Furuno(_)) {
                self.locator.send_furuno_announce(&mut self.io);
            }
        }

        if let Some(controller) = self.controllers.get_mut(radar_id) {
            controller.set_range(&mut self.io, range);
            return true;
        }

        debug(&format!("No controller found for '{}'", radar_id));
        false
    }

    /// Set radar gain
    pub fn set_gain(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_gain({}, auto={}, value={:?})", radar_id, auto, value));
        let val = value.unwrap_or(50);

        if let Some(controller) = self.controllers.get_mut(radar_id) {
            controller.set_gain(&mut self.io, val, auto);
            return true;
        }

        debug(&format!("No controller found for '{}'", radar_id));
        false
    }

    /// Set radar sea clutter
    pub fn set_sea(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_sea({}, auto={}, value={:?})", radar_id, auto, value));
        let val = value.unwrap_or(50);

        if let Some(controller) = self.controllers.get_mut(radar_id) {
            controller.set_sea(&mut self.io, val, auto);
            return true;
        }

        debug(&format!("No controller found for '{}'", radar_id));
        false
    }

    /// Set radar rain clutter
    pub fn set_rain(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_rain({}, auto={}, value={:?})", radar_id, auto, value));
        let val = value.unwrap_or(50);

        if let Some(controller) = self.controllers.get_mut(radar_id) {
            controller.set_rain(&mut self.io, val, auto);
            return true;
        }

        debug(&format!("No controller found for '{}'", radar_id));
        false
    }

    /// Set multiple radar controls at once
    pub fn set_controls(&mut self, radar_id: &str, controls: &serde_json::Value) -> bool {
        debug(&format!("set_controls({}, {:?})", radar_id, controls));

        let controls_obj = match controls.as_object() {
            Some(obj) => obj,
            None => {
                debug("set_controls: controls must be an object");
                return false;
            }
        };

        let mut success = true;
        for (control_id, value) in controls_obj {
            let ok = match control_id.as_str() {
                "power" => {
                    if let Some(state) = value.as_str() {
                        self.set_power(radar_id, state)
                    } else {
                        false
                    }
                }
                "range" => {
                    if let Some(range) = value.as_u64() {
                        self.set_range(radar_id, range as u32)
                    } else {
                        false
                    }
                }
                "gain" => {
                    let auto = value.get("mode").and_then(|m| m.as_str()) == Some("auto");
                    let val = value.get("value").and_then(|v| v.as_u64()).map(|v| v as u8);
                    self.set_gain(radar_id, auto, val)
                }
                "sea" => {
                    let auto = value.get("mode").and_then(|m| m.as_str()) == Some("auto");
                    let val = value.get("value").and_then(|v| v.as_u64()).map(|v| v as u8);
                    self.set_sea(radar_id, auto, val)
                }
                "rain" => {
                    let auto = value.get("mode").and_then(|m| m.as_str()) == Some("auto");
                    let val = value.get("value").and_then(|v| v.as_u64()).map(|v| v as u8);
                    self.set_rain(radar_id, auto, val)
                }
                _ => {
                    // Try extended control (v5 API)
                    self.set_control_v5(radar_id, control_id, &value).is_ok()
                }
            };
            if !ok {
                success = false;
            }
        }
        success
    }

    // =========================================================================
    // v5 API Methods
    // =========================================================================

    /// Get capability manifest for a radar (v5 API)
    pub fn get_capabilities(&self, radar_id: &str) -> Option<CapabilityManifest> {
        let radar = self.find_radar(radar_id)?;

        // Check if controller has model info (more up-to-date than discovery)
        let mut discovery = radar.discovery.clone();
        if let Some(controller) = self.controllers.get(radar_id) {
            if let Some(model) = controller.model() {
                discovery.model = Some(model.to_string());
            }
        }

        // WASM plugin implements ARPA, Guard Zones, Trails, and conditionally DualRange
        let mut supported_features = vec![
            SupportedFeature::Arpa,
            SupportedFeature::GuardZones,
            SupportedFeature::Trails,
        ];

        // Check if radar supports dual-range based on model
        if let Some(model_name) = &discovery.model {
            if let Some(model_info) = mayara_core::models::get_model(discovery.brand, model_name) {
                if model_info.has_dual_range {
                    supported_features.push(SupportedFeature::DualRange);
                }
            }
        }

        Some(build_capabilities(&discovery, radar_id, supported_features))
    }

    /// Get current state in v5 format
    pub fn get_state_v5(&self, radar_id: &str) -> Option<RadarStateV5> {
        let radar = self.find_radar(radar_id)?;
        let state = RadarState::from(&radar.discovery);

        // Get live state from controller and use core's to_controls_map()
        let mut controls: BTreeMap<String, serde_json::Value> = self.controllers
            .get(radar_id)
            .and_then(|c| c.radar_state())
            .map(|live_state| {
                // Use core's to_controls_map() - converts HashMap to BTreeMap
                live_state.to_controls_map().into_iter().collect()
            })
            .unwrap_or_else(|| {
                // Fallback to defaults if no controller
                let mut defaults = BTreeMap::new();
                defaults.insert("power".to_string(), serde_json::json!(state.status));
                defaults.insert("range".to_string(), serde_json::json!(1852));
                defaults.insert("gain".to_string(), serde_json::json!({"mode": "auto", "value": 50}));
                defaults.insert("sea".to_string(), serde_json::json!({"mode": "auto", "value": 50}));
                defaults.insert("rain".to_string(), serde_json::json!({"mode": "manual", "value": 0}));
                defaults.insert("noiseReduction".to_string(), serde_json::json!(false));
                defaults.insert("interferenceRejection".to_string(), serde_json::json!(false));
                defaults
            });

        // Add firmware version and operating hours (from Furuno controller)
        if let Some(fc) = self.controllers.get(radar_id).and_then(|c| c.as_furuno()) {
            if let Some(firmware) = fc.firmware_version() {
                controls.insert("firmwareVersion".to_string(), serde_json::json!(firmware));
            }
            if let Some(hours) = fc.operating_hours() {
                controls.insert("operatingHours".to_string(), serde_json::json!(hours));
            }
        }

        // Serial number from discovery (UDP model report)
        if let Some(serial) = &radar.discovery.serial_number {
            controls.insert("serialNumber".to_string(), serde_json::json!(serial));
        }

        // Installation config values from stored config
        if let Some(install_config) = self.config.radars.get(radar_id) {
            if let Some(bearing) = install_config.bearing_alignment {
                controls.insert("bearingAlignment".to_string(), serde_json::json!(bearing));
            }
            if let Some(height) = install_config.antenna_height {
                controls.insert("antennaHeight".to_string(), serde_json::json!(height));
            }
        }

        // Get ISO timestamp (placeholder - WASM doesn't have system time)
        let timestamp = "2025-01-01T00:00:00Z".to_string();

        // Use live power state for status field
        let status = controls.get("power")
            .and_then(|v| v.as_str())
            .unwrap_or(&state.status)
            .to_string();

        Some(RadarStateV5 {
            id: state.id,
            timestamp,
            status,
            controls,
            disabled_controls: vec![],
        })
    }

    /// Get a single control value (v5 API)
    pub fn get_control(&self, radar_id: &str, control_id: &str) -> Option<serde_json::Value> {
        // Reuse get_state_v5() which already uses core's to_controls_map()
        let state = self.get_state_v5(radar_id)?;
        state.controls.get(control_id).cloned()
    }

    /// Set a single control value (v5 generic interface)
    pub fn set_control_v5(
        &mut self,
        radar_id: &str,
        control_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), ControlError> {
        debug(&format!(
            "set_control_v5({}, {}, {:?})",
            radar_id, control_id, value
        ));

        // Check if radar exists
        if self.find_radar(radar_id).is_none() {
            return Err(ControlError::RadarNotFound);
        }

        // Dispatch based on control ID
        match control_id {
            "power" => {
                let state = value.as_str().ok_or_else(|| {
                    ControlError::InvalidValue("power must be a string".to_string())
                })?;
                if self.set_power(radar_id, state) {
                    Ok(())
                } else {
                    Err(ControlError::ControllerNotAvailable)
                }
            }
            "range" => {
                let range = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue("range must be a number".to_string())
                })? as u32;
                if self.set_range(radar_id, range) {
                    Ok(())
                } else {
                    Err(ControlError::ControllerNotAvailable)
                }
            }
            "gain" => {
                let (auto, val) = parse_compound_control(value)?;
                if self.set_gain(radar_id, auto, val) {
                    Ok(())
                } else {
                    Err(ControlError::ControllerNotAvailable)
                }
            }
            "sea" => {
                let (auto, val) = parse_compound_control(value)?;
                if self.set_sea(radar_id, auto, val) {
                    Ok(())
                } else {
                    Err(ControlError::ControllerNotAvailable)
                }
            }
            "rain" => {
                let (auto, val) = parse_compound_control(value)?;
                if self.set_rain(radar_id, auto, val) {
                    Ok(())
                } else {
                    Err(ControlError::ControllerNotAvailable)
                }
            }
            _ => {
                // Extended controls - dispatch by brand
                self.set_extended_control(radar_id, control_id, value)
            }
        }
    }

    /// Set an extended control (brand-specific)
    fn set_extended_control(
        &mut self,
        radar_id: &str,
        control_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), ControlError> {
        // Get radar brand
        let radar = self
            .find_radar(radar_id)
            .ok_or(ControlError::RadarNotFound)?;
        let brand = radar.discovery.brand;

        match brand {
            Brand::Furuno => self.furuno_set_extended_control(radar_id, control_id, value),
            Brand::Navico => {
                debug(&format!(
                    "Navico extended control {} not yet implemented",
                    control_id
                ));
                Err(ControlError::ControlNotFound(control_id.to_string()))
            }
            Brand::Raymarine => {
                debug(&format!(
                    "Raymarine extended control {} not yet implemented",
                    control_id
                ));
                Err(ControlError::ControlNotFound(control_id.to_string()))
            }
            Brand::Garmin => {
                debug(&format!(
                    "Garmin extended control {} not yet implemented",
                    control_id
                ));
                Err(ControlError::ControlNotFound(control_id.to_string()))
            }
        }
    }

    /// Furuno extended control dispatch
    fn furuno_set_extended_control(
        &mut self,
        radar_id: &str,
        control_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), ControlError> {
        // Send announce packets before control attempt
        self.locator.send_furuno_announce(&mut self.io);

        // Take controller out to avoid borrow conflict with io
        let mut controller_enum = match self.controllers.remove(radar_id) {
            Some(c) => c,
            None => {
                debug(&format!(
                    "No controller for {} to set {}",
                    radar_id, control_id
                ));
                return Err(ControlError::ControllerNotAvailable);
            }
        };

        // Extract Furuno controller from enum
        let controller = match &mut controller_enum {
            RadarController::Furuno(c) => c,
            _ => {
                // Put controller back and return error
                self.controllers.insert(radar_id.to_string(), controller_enum);
                return Err(ControlError::ControllerNotAvailable);
            }
        };

        let result = match control_id {
            "beamSharpening" => {
                let level = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue("beamSharpening must be a number".to_string())
                })? as i32;
                controller.set_rezboost(&mut self.io, level);
                Ok(())
            }
            "interferenceRejection" => {
                // Furuno IR is boolean in schema, convert to protocol value
                let enabled = if let Some(b) = value.as_bool() {
                    b
                } else if let Some(n) = value.as_u64() {
                    n != 0
                } else {
                    self.controllers.insert(radar_id.to_string(), controller_enum);
                    return Err(ControlError::InvalidValue(
                        "interferenceRejection must be a boolean".to_string(),
                    ));
                };
                controller.set_interference_rejection(&mut self.io, enabled);
                Ok(())
            }
            "scanSpeed" => {
                let speed = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue("scanSpeed must be a number".to_string())
                })? as i32;
                controller.set_scan_speed(&mut self.io, speed);
                Ok(())
            }
            "birdMode" => {
                let level = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue("birdMode must be a number (0-3)".to_string())
                })? as i32;
                controller.set_bird_mode(&mut self.io, level);
                Ok(())
            }
            "dopplerMode" => {
                // Doppler mode is a compound control: {enabled: bool, mode: "target"|"rain"}
                let enabled = value
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mode_str = value
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("target");
                // Convert mode string to numeric: 0=target, 1=rain
                let mode: i32 = if mode_str == "rain" { 1 } else { 0 };
                controller.set_target_analyzer(&mut self.io, enabled, mode);
                Ok(())
            }
            "bearingAlignment" => {
                let degrees = value.as_f64().ok_or_else(|| {
                    ControlError::InvalidValue("bearingAlignment must be a number".to_string())
                })?;
                controller.set_bearing_alignment(&mut self.io, degrees);
                // Also persist to local config
                let mut install_config = self.config.radars.get(radar_id).cloned().unwrap_or_default();
                install_config.bearing_alignment = Some(degrees);
                // Put controller back before saving config (which mutates self)
                self.controllers.insert(radar_id.to_string(), controller_enum);
                self.set_installation_config(radar_id, install_config);
                return Ok(());
            }
            "noiseReduction" => {
                let enabled = value.as_bool().ok_or_else(|| {
                    ControlError::InvalidValue("noiseReduction must be a boolean".to_string())
                })?;
                controller.set_noise_reduction(&mut self.io, enabled);
                Ok(())
            }
            "mainBangSuppression" => {
                let percent = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue(
                        "mainBangSuppression must be a number".to_string(),
                    )
                })? as i32;
                controller.set_main_bang_suppression(&mut self.io, percent);
                Ok(())
            }
            "txChannel" => {
                let channel = value.as_u64().ok_or_else(|| {
                    ControlError::InvalidValue("txChannel must be a number".to_string())
                })? as i32;
                controller.set_tx_channel(&mut self.io, channel);
                Ok(())
            }
            "autoAcquire" => {
                let enabled = value.as_bool().ok_or_else(|| {
                    ControlError::InvalidValue("autoAcquire must be a boolean".to_string())
                })?;
                controller.set_auto_acquire(&mut self.io, enabled);
                Ok(())
            }
            "dopplerSpeed" => {
                // dopplerSpeed is the threshold for target analyzer
                let speed = value.as_f64().ok_or_else(|| {
                    ControlError::InvalidValue("dopplerSpeed must be a number".to_string())
                })? as i32;
                controller.set_target_analyzer(&mut self.io, true, speed);
                Ok(())
            }
            "antennaHeight" => {
                let meters = value.as_i64().ok_or_else(|| {
                    ControlError::InvalidValue("antennaHeight must be a number (meters)".to_string())
                })? as i32;
                if !(0..=100).contains(&meters) {
                    self.controllers.insert(radar_id.to_string(), controller_enum);
                    return Err(ControlError::InvalidValue(
                        "antennaHeight must be 0-100 meters".to_string()
                    ));
                }
                controller.set_antenna_height(&mut self.io, meters);
                // Persist to local config
                let mut install_config = self.config.radars.get(radar_id).cloned().unwrap_or_default();
                install_config.antenna_height = Some(meters);
                // Put controller back before saving config (which mutates self)
                self.controllers.insert(radar_id.to_string(), controller_enum);
                self.set_installation_config(radar_id, install_config);
                return Ok(());
            }
            "noTransmitZones" => {
                let zones = value
                    .get("zones")
                    .and_then(|z| z.as_array())
                    .ok_or_else(|| {
                        ControlError::InvalidValue(
                            "noTransmitZones must have a 'zones' array".to_string(),
                        )
                    })?;

                // Parse zone 1
                let (z1_enabled, z1_start, z1_end) = if let Some(z1) = zones.first() {
                    (
                        z1.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                        z1.get("start").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        z1.get("end").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    )
                } else {
                    (false, 0, 0)
                };

                // Parse zone 2
                let (z2_enabled, z2_start, z2_end) = if let Some(z2) = zones.get(1) {
                    (
                        z2.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                        z2.get("start").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        z2.get("end").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    )
                } else {
                    (false, 0, 0)
                };

                controller.set_blind_sector(
                    &mut self.io,
                    z1_enabled, z1_start, z1_end,
                    z2_enabled, z2_start, z2_end,
                );
                Ok(())
            }
            _ => {
                debug(&format!(
                    "Unknown Furuno extended control: {}",
                    control_id
                ));
                Err(ControlError::ControlNotFound(control_id.to_string()))
            }
        };

        // Put controller back
        self.controllers.insert(radar_id.to_string(), controller_enum);
        result
    }

    // =========================================================================
    // v6 ARPA Target Methods
    // =========================================================================

    /// Get or create ARPA processor for a radar
    fn get_or_create_arpa(&mut self, radar_id: &str) -> &mut ArpaProcessor {
        if !self.arpa_processors.contains_key(radar_id) {
            debug(&format!("Creating ARPA processor for {}", radar_id));
            let settings = ArpaSettings::default();
            self.arpa_processors.insert(radar_id.to_string(), ArpaProcessor::new(settings));
        }
        self.arpa_processors.get_mut(radar_id).unwrap()
    }

    /// Get all tracked ARPA targets for a radar
    pub fn get_targets(&self, radar_id: &str) -> Option<Vec<ArpaTarget>> {
        self.arpa_processors.get(radar_id).map(|p| p.get_targets())
    }

    /// Manually acquire a target at the specified position
    pub fn acquire_target(&mut self, radar_id: &str, bearing: f64, distance: f64) -> Result<u32, String> {
        // Validate inputs
        if bearing < 0.0 || bearing >= 360.0 {
            return Err("bearing must be 0-360".to_string());
        }
        if distance <= 0.0 {
            return Err("distance must be positive".to_string());
        }

        let timestamp = self.poll_count * 100;  // Approximate timestamp
        let arpa = self.get_or_create_arpa(radar_id);

        match arpa.acquire_target(bearing, distance, timestamp) {
            Some(id) => {
                debug(&format!("Acquired target {} at bearing={}, distance={}", id, bearing, distance));
                Ok(id)
            }
            None => Err("max targets reached".to_string()),
        }
    }

    /// Cancel tracking of a target
    pub fn cancel_target(&mut self, radar_id: &str, target_id: u32) -> bool {
        if let Some(arpa) = self.arpa_processors.get_mut(radar_id) {
            let result = arpa.cancel_target(target_id);
            if result {
                debug(&format!("Cancelled target {} on radar {}", target_id, radar_id));
            }
            result
        } else {
            false
        }
    }

    /// Get ARPA settings for a radar
    pub fn get_arpa_settings(&self, radar_id: &str) -> Option<ArpaSettings> {
        self.arpa_processors.get(radar_id).map(|p| p.settings().clone())
    }

    /// Update ARPA settings for a radar
    pub fn set_arpa_settings(&mut self, radar_id: &str, settings: ArpaSettings) -> Result<(), String> {
        let arpa = self.get_or_create_arpa(radar_id);
        arpa.update_settings(settings);
        debug(&format!("Updated ARPA settings for {}", radar_id));
        Ok(())
    }

    /// Process ARPA events and emit collision notifications
    #[allow(dead_code)]
    pub fn process_arpa_events(&self, radar_id: &str, events: &[ArpaEvent]) {
        use crate::signalk_ffi::publish_collision_warning;

        for event in events {
            match event {
                ArpaEvent::CollisionWarning { target_id, state, cpa, tcpa } => {
                    let state_str = state.as_signalk_state();
                    publish_collision_warning(radar_id, *target_id, state_str, *cpa, *tcpa);
                    debug(&format!(
                        "Published collision warning: radar={}, target={}, state={}, cpa={:.0}m, tcpa={:.0}s",
                        radar_id, target_id, state_str, cpa, tcpa
                    ));
                }
                ArpaEvent::TargetAcquired { target } => {
                    debug(&format!("Target acquired: {} on radar {}", target.id, radar_id));
                }
                ArpaEvent::TargetLost { target_id, .. } => {
                    // Clear the notification when target is lost
                    publish_collision_warning(radar_id, *target_id, "normal", 0.0, 0.0);
                    debug(&format!("Target lost: {} on radar {}", target_id, radar_id));
                }
                ArpaEvent::TargetUpdate { .. } => {
                    // Regular updates don't need notifications
                }
            }
        }
    }

    // =========================================================================
    // Guard Zone Methods (FFI exports not yet implemented)
    // =========================================================================

    /// Get or create guard zone processor for a radar
    #[allow(dead_code)]
    fn get_or_create_guard_zone_processor(&mut self, radar_id: &str) -> &mut GuardZoneProcessor {
        if !self.guard_zone_processors.contains_key(radar_id) {
            debug(&format!("Creating guard zone processor for {}", radar_id));
            self.guard_zone_processors.insert(radar_id.to_string(), GuardZoneProcessor::new());
        }
        self.guard_zone_processors.get_mut(radar_id).unwrap()
    }

    /// Get all guard zones for a radar
    #[allow(dead_code)]
    pub fn get_guard_zones(&self, radar_id: &str) -> Vec<GuardZoneStatus> {
        self.guard_zone_processors
            .get(radar_id)
            .map(|p| p.get_all_zone_status())
            .unwrap_or_default()
    }

    /// Get a specific guard zone
    #[allow(dead_code)]
    pub fn get_guard_zone(&self, radar_id: &str, zone_id: u32) -> Option<GuardZoneStatus> {
        self.guard_zone_processors
            .get(radar_id)
            .and_then(|p| p.get_zone_status(zone_id))
    }

    /// Create or update a guard zone
    #[allow(dead_code)]
    pub fn set_guard_zone(&mut self, radar_id: &str, zone: GuardZone) {
        let processor = self.get_or_create_guard_zone_processor(radar_id);
        processor.add_zone(zone.clone());
        debug(&format!("Set guard zone {} on radar {}", zone.id, radar_id));
    }

    /// Delete a guard zone
    #[allow(dead_code)]
    pub fn delete_guard_zone(&mut self, radar_id: &str, zone_id: u32) -> bool {
        if let Some(processor) = self.guard_zone_processors.get_mut(radar_id) {
            let result = processor.remove_zone(zone_id);
            if result {
                debug(&format!("Deleted guard zone {} on radar {}", zone_id, radar_id));
            }
            result
        } else {
            false
        }
    }

    // =========================================================================
    // Trail Methods (FFI exports not yet implemented)
    // =========================================================================

    /// Get or create trail store for a radar
    #[allow(dead_code)]
    fn get_or_create_trail_store(&mut self, radar_id: &str) -> &mut TrailStore {
        if !self.trail_stores.contains_key(radar_id) {
            debug(&format!("Creating trail store for {}", radar_id));
            self.trail_stores.insert(radar_id.to_string(), TrailStore::new(TrailSettings::default()));
        }
        self.trail_stores.get_mut(radar_id).unwrap()
    }

    /// Get all trails for a radar
    #[allow(dead_code)]
    pub fn get_all_trails(&self, radar_id: &str) -> Vec<TrailData> {
        self.trail_stores
            .get(radar_id)
            .map(|s| s.get_all_trail_data())
            .unwrap_or_default()
    }

    /// Get trail for a specific target
    #[allow(dead_code)]
    pub fn get_trail(&self, radar_id: &str, target_id: u32) -> Option<TrailData> {
        self.trail_stores
            .get(radar_id)
            .and_then(|s| s.get_trail_data(target_id))
    }

    /// Clear all trails for a radar
    #[allow(dead_code)]
    pub fn clear_all_trails(&mut self, radar_id: &str) {
        if let Some(store) = self.trail_stores.get_mut(radar_id) {
            store.clear_all();
            debug(&format!("Cleared all trails on radar {}", radar_id));
        }
    }

    /// Clear trail for a specific target
    #[allow(dead_code)]
    pub fn clear_trail(&mut self, radar_id: &str, target_id: u32) {
        if let Some(store) = self.trail_stores.get_mut(radar_id) {
            store.clear_trail(target_id);
            debug(&format!("Cleared trail for target {} on radar {}", target_id, radar_id));
        }
    }

    /// Get trail settings for a radar
    #[allow(dead_code)]
    pub fn get_trail_settings(&self, radar_id: &str) -> TrailSettings {
        self.trail_stores
            .get(radar_id)
            .map(|s| s.settings().clone())
            .unwrap_or_default()
    }

    /// Update trail settings for a radar
    #[allow(dead_code)]
    pub fn set_trail_settings(&mut self, radar_id: &str, settings: TrailSettings) {
        let store = self.get_or_create_trail_store(radar_id);
        store.update_settings(settings);
        debug(&format!("Updated trail settings for {}", radar_id));
    }

    // =========================================================================
    // Dual-Range Methods (FFI exports not yet implemented)
    // =========================================================================

    /// Get or create dual-range controller for a radar
    #[allow(dead_code)]
    fn get_or_create_dual_range_controller(&mut self, radar_id: &str) -> Option<&mut DualRangeController> {
        // Get model info to determine max secondary range
        let radar = self.find_radar(radar_id)?;
        let model_name = radar.discovery.model.as_ref()?;
        let model_info = mayara_core::models::get_model(radar.discovery.brand, model_name)?;

        if !model_info.has_dual_range {
            return None;
        }

        if !self.dual_range_controllers.contains_key(radar_id) {
            debug(&format!("Creating dual-range controller for {}", radar_id));
            let controller = DualRangeController::new(
                model_info.max_dual_range,
                model_info.range_table.to_vec(),
            );
            self.dual_range_controllers.insert(radar_id.to_string(), controller);
        }
        self.dual_range_controllers.get_mut(radar_id)
    }

    /// Check if a radar supports dual-range
    #[allow(dead_code)]
    pub fn supports_dual_range(&self, radar_id: &str) -> bool {
        if let Some(radar) = self.find_radar(radar_id) {
            if let Some(model_name) = &radar.discovery.model {
                if let Some(model_info) = mayara_core::models::get_model(radar.discovery.brand, model_name) {
                    return model_info.has_dual_range;
                }
            }
        }
        false
    }

    /// Get dual-range state for a radar
    #[allow(dead_code)]
    pub fn get_dual_range_state(&self, radar_id: &str) -> Option<DualRangeState> {
        self.dual_range_controllers
            .get(radar_id)
            .map(|c| c.state().clone())
    }

    /// Get available secondary ranges for dual-range mode
    pub fn get_dual_range_available_ranges(&self, radar_id: &str) -> Vec<u32> {
        self.dual_range_controllers
            .get(radar_id)
            .map(|c| c.available_ranges().to_vec())
            .unwrap_or_default()
    }

    /// Set dual-range configuration
    pub fn set_dual_range_config(&mut self, radar_id: &str, config: DualRangeConfig) -> Result<(), String> {
        // First check if radar supports dual-range
        if !self.supports_dual_range(radar_id) {
            return Err("Radar does not support dual-range".to_string());
        }

        // Get or create controller
        if let Some(controller) = self.get_or_create_dual_range_controller(radar_id) {
            if !controller.apply_config(&config) {
                return Err(format!(
                    "Secondary range {} exceeds maximum",
                    config.secondary_range
                ));
            }
            debug(&format!("Set dual-range config for {}: enabled={}", radar_id, config.enabled));
            Ok(())
        } else {
            Err("Failed to create dual-range controller".to_string())
        }
    }

    /// Enable or disable dual-range mode
    pub fn set_dual_range_enabled(&mut self, radar_id: &str, enabled: bool) -> Result<(), String> {
        if !self.supports_dual_range(radar_id) {
            return Err("Radar does not support dual-range".to_string());
        }

        if let Some(controller) = self.get_or_create_dual_range_controller(radar_id) {
            controller.set_enabled(enabled);
            debug(&format!("Set dual-range enabled={} for {}", enabled, radar_id));
            Ok(())
        } else {
            Err("Failed to create dual-range controller".to_string())
        }
    }

    /// Set secondary range in meters
    pub fn set_secondary_range(&mut self, radar_id: &str, range: u32) -> Result<(), String> {
        if !self.supports_dual_range(radar_id) {
            return Err("Radar does not support dual-range".to_string());
        }

        if let Some(controller) = self.get_or_create_dual_range_controller(radar_id) {
            if !controller.set_secondary_range(range) {
                return Err("Range exceeds maximum for dual-range mode".to_string());
            }
            debug(&format!("Set secondary range to {} for {}", range, radar_id));
            Ok(())
        } else {
            Err("Failed to create dual-range controller".to_string())
        }
    }
}

/// Parse a compound control value (mode + value)
fn parse_compound_control(value: &serde_json::Value) -> Result<(bool, Option<u8>), ControlError> {
    // Can be either a simple number or {mode: "auto"|"manual", value: N}
    if let Some(n) = value.as_u64() {
        // Simple number = manual mode
        return Ok((false, Some(n as u8)));
    }

    if let Some(obj) = value.as_object() {
        let mode = obj.get("mode").and_then(|v| v.as_str()).unwrap_or("manual");
        let auto = mode == "auto";
        let val = obj.get("value").and_then(|v| v.as_u64()).map(|v| v as u8);
        return Ok((auto, val));
    }

    Err(ControlError::InvalidValue(
        "Expected number or {mode, value} object".to_string(),
    ))
}

impl Default for RadarProvider {
    fn default() -> Self {
        Self::new()
    }
}

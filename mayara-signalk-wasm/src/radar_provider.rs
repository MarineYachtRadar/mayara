//! Radar Provider
//!
//! Implements the SignalK Radar Provider interface.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};

use mayara_core::capabilities::{
    builder::build_capabilities, CapabilityManifest, ControlError, RadarStateV5,
};
use mayara_core::radar::RadarDiscovery;
use mayara_core::Brand;

use crate::furuno_controller::FurunoController;
use crate::locator::RadarLocator;
use crate::signalk_ffi::{debug, emit_json, read_config, save_config};
use crate::spoke_receiver::{SpokeReceiver, FURUNO_OUTPUT_SPOKES};

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
    locator: RadarLocator,
    spoke_receiver: SpokeReceiver,
    /// TCP controllers for Furuno radars (keyed by radar ID)
    furuno_controllers: BTreeMap<String, FurunoController>,
    poll_count: u64,
    /// Plugin configuration (installation settings per radar)
    config: PluginConfig,
}

impl RadarProvider {
    /// Create a new radar provider
    pub fn new() -> Self {
        let mut locator = RadarLocator::new();
        locator.start();

        // Load saved configuration
        let config = Self::load_config();
        debug(&format!("Loaded config: {} radars configured", config.radars.len()));

        Self {
            locator,
            spoke_receiver: SpokeReceiver::new(),
            furuno_controllers: BTreeMap::new(),
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
        self.locator.current_time_ms = self.poll_count * 100;

        // Poll for new radars
        let new_radars = self.locator.poll();

        // Emit delta for each new radar
        for discovery in &new_radars {
            self.emit_radar_discovered(discovery);
        }

        // Register ALL Furuno radars for spoke tracking and create controllers
        // This ensures radars discovered before spoke_receiver was ready are also tracked
        let radar_count = self.locator.radars.len();
        if self.poll_count % 100 == 1 {
            debug(&format!("Checking {} radars for spoke tracking", radar_count));
        }

        // Collect radar info first to avoid borrow issues
        let furuno_radars: Vec<(String, String)> = self.locator.radars.values()
            .filter(|r| r.discovery.brand == mayara_core::Brand::Furuno)
            .map(|r| {
                let state = RadarState::from(&r.discovery);
                let ip = r.discovery.address.split(':').next().unwrap_or(&r.discovery.address).to_string();
                (state.id, ip)
            })
            .collect();

        for (radar_id, ip) in furuno_radars {
            if self.poll_count % 100 == 1 {
                debug(&format!("Furuno radar {} at {} for spokes", radar_id, ip));
            }
            // Register for spoke tracking
            self.spoke_receiver.add_furuno_radar(&radar_id, &ip);

            // Create controller if not exists
            if !self.furuno_controllers.contains_key(&radar_id) {
                debug(&format!("Creating FurunoController for {}", radar_id));
                let controller = FurunoController::new(&radar_id, &ip);
                self.furuno_controllers.insert(radar_id.clone(), controller);
            }
        }

        // Poll all Furuno controllers and update model info
        for (radar_id, controller) in self.furuno_controllers.iter_mut() {
            controller.poll();

            // Update radar discovery with model from controller (if available)
            if let Some(model) = controller.model() {
                // Find the radar in locator and update its model
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
        let spokes_emitted = self.spoke_receiver.poll();

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
        self.locator.shutdown();
        self.spoke_receiver.shutdown();
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
        debug(&format!("set_power({}, {}) - {} controllers registered",
            radar_id, state, self.furuno_controllers.len()));

        // Debug: list all controller IDs
        for id in self.furuno_controllers.keys() {
            debug(&format!("  Registered controller: '{}'", id));
        }

        // For Furuno radars, use the direct TCP controller
        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            let transmit = state == "transmit";
            debug(&format!("Using FurunoController for {} (transmit={})", radar_id, transmit));

            // Send announce packets immediately before TCP connection attempt
            // The radar only accepts TCP from clients that have recently announced
            self.locator.send_furuno_announce();

            controller.set_transmit(transmit);
            return true;
        }

        debug(&format!("No FurunoController found for '{}', falling back to UDP", radar_id));

        // Fallback to UDP for other radar types (requires mayara-server)
        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_power",
                "radarId": radar_id,
                "state": state
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_power: radar {} not found", radar_id));
            false
        }
    }

    /// Set radar range in meters
    pub fn set_range(&mut self, radar_id: &str, range: u32) -> bool {
        debug(&format!("set_range({}, {}) - {} controllers registered",
            radar_id, range, self.furuno_controllers.len()));

        // For Furuno radars, use the direct TCP controller
        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            debug(&format!("Using FurunoController for {} (range={}m)", radar_id, range));

            // Send announce packets immediately before TCP connection attempt
            self.locator.send_furuno_announce();

            controller.set_range(range);
            return true;
        }

        debug(&format!("No FurunoController found for '{}', falling back to UDP", radar_id));

        // Fallback to UDP for other radar types (requires mayara-server)
        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_range",
                "radarId": radar_id,
                "range": range
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_range: radar {} not found", radar_id));
            false
        }
    }

    /// Set radar gain
    pub fn set_gain(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_gain({}, auto={}, value={:?})", radar_id, auto, value));

        // For Furuno radars, use the direct TCP controller
        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            let val = value.unwrap_or(50) as i32;
            debug(&format!("Using FurunoController for {} (gain={}, auto={})", radar_id, val, auto));

            // Send announce packets immediately before TCP connection attempt
            self.locator.send_furuno_announce();

            controller.set_gain(val, auto);
            return true;
        }

        debug(&format!("No FurunoController found for '{}', falling back to UDP", radar_id));

        // Fallback to UDP for other radar types (requires mayara-server)
        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_gain",
                "radarId": radar_id,
                "auto": auto,
                "value": value
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_gain: radar {} not found", radar_id));
            false
        }
    }

    /// Set radar sea clutter
    pub fn set_sea(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_sea({}, auto={}, value={:?})", radar_id, auto, value));

        // For Furuno radars, use the direct TCP controller
        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            let val = value.unwrap_or(50) as i32;
            debug(&format!("Using FurunoController for {} (sea={}, auto={})", radar_id, val, auto));

            // Send announce packets immediately before TCP connection attempt
            self.locator.send_furuno_announce();

            controller.set_sea(val, auto);
            return true;
        }

        debug(&format!("No FurunoController found for '{}', falling back to UDP", radar_id));

        // Fallback to UDP for other radar types (requires mayara-server)
        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_sea",
                "radarId": radar_id,
                "auto": auto,
                "value": value
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_sea: radar {} not found", radar_id));
            false
        }
    }

    /// Set radar rain clutter
    pub fn set_rain(&mut self, radar_id: &str, auto: bool, value: Option<u8>) -> bool {
        debug(&format!("set_rain({}, auto={}, value={:?})", radar_id, auto, value));

        // For Furuno radars, use the direct TCP controller
        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            let val = value.unwrap_or(50) as i32;
            debug(&format!("Using FurunoController for {} (rain={}, auto={})", radar_id, val, auto));

            // Send announce packets immediately before TCP connection attempt
            self.locator.send_furuno_announce();

            controller.set_rain(val, auto);
            return true;
        }

        debug(&format!("No FurunoController found for '{}', falling back to UDP", radar_id));

        // Fallback to UDP for other radar types (requires mayara-server)
        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_rain",
                "radarId": radar_id,
                "auto": auto,
                "value": value
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_rain: radar {} not found", radar_id));
            false
        }
    }

    /// Set multiple radar controls at once
    pub fn set_controls(&mut self, radar_id: &str, controls: &serde_json::Value) -> bool {
        debug(&format!("set_controls({}, {:?})", radar_id, controls));

        if let Some(radar) = self.find_radar(radar_id) {
            let ip = radar.discovery.address.split(':').next().unwrap_or("127.0.0.1");
            let cmd = serde_json::json!({
                "type": "set_controls",
                "radarId": radar_id,
                "controls": controls
            });
            self.send_control_command(ip, &cmd)
        } else {
            debug(&format!("set_controls: radar {} not found", radar_id));
            false
        }
    }

    /// Send control command to mayara-server via UDP
    fn send_control_command(&self, ip: &str, cmd: &serde_json::Value) -> bool {
        use crate::signalk_ffi::UdpSocket;

        // mayara-server control port (convention: 3002 for control commands)
        const CONTROL_PORT: u16 = 3002;

        let json = match serde_json::to_string(cmd) {
            Ok(j) => j,
            Err(e) => {
                debug(&format!("Failed to serialize control command: {}", e));
                return false;
            }
        };

        debug(&format!("Sending control to {}:{}: {}", ip, CONTROL_PORT, json));

        // Create UDP socket and send command
        match UdpSocket::bind("0.0.0.0:0") {
            Ok(socket) => {
                match socket.send_to(json.as_bytes(), ip, CONTROL_PORT) {
                    Ok(_) => {
                        debug("Control command sent successfully");
                        true
                    }
                    Err(e) => {
                        debug(&format!("Failed to send control command: {:?}", e));
                        false
                    }
                }
            }
            Err(e) => {
                debug(&format!("Failed to create control socket: {:?}", e));
                false
            }
        }
    }

    // =========================================================================
    // v5 API Methods
    // =========================================================================

    /// Get capability manifest for a radar (v5 API)
    pub fn get_capabilities(&self, radar_id: &str) -> Option<CapabilityManifest> {
        let radar = self.find_radar(radar_id)?;

        // Check if controller has model info (more up-to-date than discovery)
        let mut discovery = radar.discovery.clone();
        if let Some(controller) = self.furuno_controllers.get(radar_id) {
            if let Some(model) = controller.model() {
                discovery.model = Some(model.to_string());
            }
        }

        Some(build_capabilities(&discovery, radar_id))
    }

    /// Get current state in v5 format
    pub fn get_state_v5(&self, radar_id: &str) -> Option<RadarStateV5> {
        let radar = self.find_radar(radar_id)?;
        let state = RadarState::from(&radar.discovery);

        // Build controls map with current values from the controller
        let mut controls = HashMap::new();

        // Get live state from controller if available
        if let Some(controller) = self.furuno_controllers.get(radar_id) {
            let live_state = controller.radar_state();

            // Power state from live radar state
            let power_str = match live_state.power {
                mayara_core::state::PowerState::Off => "off",
                mayara_core::state::PowerState::Standby => "standby",
                mayara_core::state::PowerState::Transmit => "transmit",
                mayara_core::state::PowerState::Warming => "warming",
            };
            controls.insert("power".to_string(), serde_json::json!(power_str));

            // Range from live state
            controls.insert("range".to_string(), serde_json::json!(live_state.range));

            // Gain, sea, rain from live state
            controls.insert(
                "gain".to_string(),
                serde_json::json!({"mode": live_state.gain.mode, "value": live_state.gain.value}),
            );
            controls.insert(
                "sea".to_string(),
                serde_json::json!({"mode": live_state.sea.mode, "value": live_state.sea.value}),
            );
            controls.insert(
                "rain".to_string(),
                serde_json::json!({"mode": live_state.rain.mode, "value": live_state.rain.value}),
            );

            // Signal processing controls from live state
            controls.insert(
                "noiseReduction".to_string(),
                serde_json::json!(live_state.noise_reduction),
            );
            controls.insert(
                "interferenceRejection".to_string(),
                serde_json::json!(live_state.interference_rejection),
            );

            // Extended controls from live state
            controls.insert(
                "beamSharpening".to_string(),
                serde_json::json!(live_state.beam_sharpening),
            );
            controls.insert(
                "birdMode".to_string(),
                serde_json::json!(live_state.bird_mode),
            );
            controls.insert(
                "dopplerMode".to_string(),
                serde_json::json!({
                    "enabled": live_state.doppler_mode.enabled,
                    "mode": live_state.doppler_mode.mode
                }),
            );
            controls.insert(
                "scanSpeed".to_string(),
                serde_json::json!(live_state.scan_speed),
            );
            controls.insert(
                "mainBangSuppression".to_string(),
                serde_json::json!(live_state.main_bang_suppression),
            );
            controls.insert(
                "txChannel".to_string(),
                serde_json::json!(live_state.tx_channel),
            );
            controls.insert(
                "noTransmitZones".to_string(),
                serde_json::json!({
                    "zones": live_state.no_transmit_zones.zones.iter().map(|z| {
                        serde_json::json!({
                            "enabled": z.enabled,
                            "start": z.start,
                            "end": z.end
                        })
                    }).collect::<Vec<_>>()
                }),
            );

            // Firmware version and operating hours
            if let Some(firmware) = controller.firmware_version() {
                controls.insert("firmwareVersion".to_string(), serde_json::json!(firmware));
            }
            if let Some(hours) = controller.operating_hours() {
                controls.insert("operatingHours".to_string(), serde_json::json!(hours));
            }
        } else {
            // Fallback to defaults if no controller
            controls.insert("power".to_string(), serde_json::json!(state.status));
            controls.insert("range".to_string(), serde_json::json!(1852));
            controls.insert(
                "gain".to_string(),
                serde_json::json!({"mode": "auto", "value": 50}),
            );
            controls.insert(
                "sea".to_string(),
                serde_json::json!({"mode": "auto", "value": 50}),
            );
            controls.insert(
                "rain".to_string(),
                serde_json::json!({"mode": "manual", "value": 0}),
            );
            controls.insert("noiseReduction".to_string(), serde_json::json!(false));
            controls.insert("interferenceRejection".to_string(), serde_json::json!(false));
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
        let radar = self.find_radar(radar_id)?;

        // Try to get live state from controller
        let controller = self.furuno_controllers.get(radar_id);
        let live_state = controller.map(|c| c.radar_state());

        match control_id {
            "power" => {
                if let Some(state) = live_state {
                    let power_str = match state.power {
                        mayara_core::state::PowerState::Off => "off",
                        mayara_core::state::PowerState::Standby => "standby",
                        mayara_core::state::PowerState::Transmit => "transmit",
                        mayara_core::state::PowerState::Warming => "warming",
                    };
                    Some(serde_json::json!(power_str))
                } else {
                    Some(serde_json::json!("standby"))
                }
            }
            "range" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.range))
                } else {
                    Some(serde_json::json!(1852))
                }
            }
            "gain" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!({"mode": state.gain.mode, "value": state.gain.value}))
                } else {
                    Some(serde_json::json!({"mode": "auto", "value": 50}))
                }
            }
            "sea" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!({"mode": state.sea.mode, "value": state.sea.value}))
                } else {
                    Some(serde_json::json!({"mode": "auto", "value": 50}))
                }
            }
            "rain" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!({"mode": state.rain.mode, "value": state.rain.value}))
                } else {
                    Some(serde_json::json!({"mode": "manual", "value": 0}))
                }
            }
            // Info controls (read-only)
            "serialNumber" => radar.discovery.serial_number.as_ref().map(|s| serde_json::json!(s)),
            "firmwareVersion" => controller
                .and_then(|c| c.firmware_version())
                .map(|v| serde_json::json!(v)),
            "operatingHours" => controller
                .and_then(|c| c.operating_hours())
                .map(|h| serde_json::json!(h)),
            // Signal processing controls (from radar state)
            "noiseReduction" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.noise_reduction))
                } else {
                    Some(serde_json::json!(false))
                }
            }
            "interferenceRejection" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.interference_rejection))
                } else {
                    Some(serde_json::json!(false))
                }
            }
            // Installation config values (stored locally)
            "bearingAlignment" => self.config.radars.get(radar_id)
                .and_then(|c| c.bearing_alignment)
                .map(|v| serde_json::json!(v)),
            "antennaHeight" => self.config.radars.get(radar_id)
                .and_then(|c| c.antenna_height)
                .map(|v| serde_json::json!(v)),
            // Extended controls from live state
            "beamSharpening" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.beam_sharpening))
                } else {
                    Some(serde_json::json!(0))
                }
            }
            "birdMode" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.bird_mode))
                } else {
                    Some(serde_json::json!(0))
                }
            }
            "dopplerMode" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!({
                        "enabled": state.doppler_mode.enabled,
                        "mode": state.doppler_mode.mode
                    }))
                } else {
                    Some(serde_json::json!({"enabled": false, "mode": "target"}))
                }
            }
            "scanSpeed" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.scan_speed))
                } else {
                    Some(serde_json::json!(0))
                }
            }
            "mainBangSuppression" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.main_bang_suppression))
                } else {
                    Some(serde_json::json!(0))
                }
            }
            "txChannel" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!(state.tx_channel))
                } else {
                    Some(serde_json::json!(0))
                }
            }
            "noTransmitZones" => {
                if let Some(state) = live_state {
                    Some(serde_json::json!({
                        "zones": state.no_transmit_zones.zones.iter().map(|z| {
                            serde_json::json!({
                                "enabled": z.enabled,
                                "start": z.start,
                                "end": z.end
                            })
                        }).collect::<Vec<_>>()
                    }))
                } else {
                    Some(serde_json::json!({"zones": [
                        {"enabled": false, "start": 0, "end": 0},
                        {"enabled": false, "start": 0, "end": 0}
                    ]}))
                }
            }
            _ => {
                debug(&format!("Unknown control: {}", control_id));
                None
            }
        }
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
        self.locator.send_furuno_announce();

        if let Some(controller) = self.furuno_controllers.get_mut(radar_id) {
            match control_id {
                "beamSharpening" => {
                    let level = value.as_u64().ok_or_else(|| {
                        ControlError::InvalidValue("beamSharpening must be a number".to_string())
                    })? as u8;
                    controller.set_rezboost(level);
                    Ok(())
                }
                "interferenceRejection" => {
                    // Furuno IR is boolean in schema, convert to protocol value (0=off, 2=on)
                    let level: u8 = if let Some(b) = value.as_bool() {
                        if b { 2 } else { 0 }
                    } else if let Some(n) = value.as_u64() {
                        // Also accept numeric for backwards compatibility
                        n as u8
                    } else {
                        return Err(ControlError::InvalidValue(
                            "interferenceRejection must be a boolean".to_string(),
                        ));
                    };
                    controller.set_interference_rejection(level);
                    Ok(())
                }
                "scanSpeed" => {
                    let speed = value.as_u64().ok_or_else(|| {
                        ControlError::InvalidValue("scanSpeed must be a number".to_string())
                    })? as u8;
                    controller.set_scan_speed(speed);
                    Ok(())
                }
                "birdMode" => {
                    let level = value.as_u64().ok_or_else(|| {
                        ControlError::InvalidValue("birdMode must be a number (0-3)".to_string())
                    })? as u8;
                    controller.set_bird_mode(level);
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
                    let mode: u8 = if mode_str == "rain" { 1 } else { 0 };
                    controller.set_target_analyzer(enabled, mode);
                    Ok(())
                }
                "bearingAlignment" => {
                    let degrees = value.as_f64().ok_or_else(|| {
                        ControlError::InvalidValue("bearingAlignment must be a number".to_string())
                    })?;
                    // Send to radar hardware
                    controller.set_bearing_alignment(degrees);
                    // Also persist to local config
                    let mut install_config = self.config.radars.get(radar_id).cloned().unwrap_or_default();
                    install_config.bearing_alignment = Some(degrees);
                    self.set_installation_config(radar_id, install_config);
                    Ok(())
                }
                "noiseReduction" => {
                    let enabled = value.as_bool().ok_or_else(|| {
                        ControlError::InvalidValue("noiseReduction must be a boolean".to_string())
                    })?;
                    controller.set_noise_reduction(enabled);
                    Ok(())
                }
                "mainBangSuppression" => {
                    let percent = value.as_u64().ok_or_else(|| {
                        ControlError::InvalidValue(
                            "mainBangSuppression must be a number".to_string(),
                        )
                    })? as u8;
                    controller.set_main_bang_suppression(percent);
                    Ok(())
                }
                "txChannel" => {
                    let channel = value.as_u64().ok_or_else(|| {
                        ControlError::InvalidValue("txChannel must be a number".to_string())
                    })? as u8;
                    controller.set_tx_channel(channel);
                    Ok(())
                }
                "autoAcquire" => {
                    let enabled = value.as_bool().ok_or_else(|| {
                        ControlError::InvalidValue("autoAcquire must be a boolean".to_string())
                    })?;
                    controller.set_auto_acquire(enabled);
                    Ok(())
                }
                "dopplerSpeed" => {
                    // dopplerSpeed is the threshold for target analyzer
                    // It's part of the dopplerMode compound control but can be set separately
                    let speed = value.as_f64().ok_or_else(|| {
                        ControlError::InvalidValue("dopplerSpeed must be a number".to_string())
                    })? as u8;
                    // Need to enable target analyzer with the new speed
                    controller.set_target_analyzer(true, speed);
                    Ok(())
                }
                "antennaHeight" => {
                    // antennaHeight in meters (0-100)
                    let meters = value.as_i64().ok_or_else(|| {
                        ControlError::InvalidValue("antennaHeight must be a number (meters)".to_string())
                    })? as i32;
                    if !(0..=100).contains(&meters) {
                        return Err(ControlError::InvalidValue(
                            "antennaHeight must be 0-100 meters".to_string()
                        ));
                    }
                    // Send to radar first (while we have the mutable borrow)
                    controller.set_antenna_height(meters);
                    // Then persist to local config
                    let mut install_config = self.config.radars.get(radar_id).cloned().unwrap_or_default();
                    install_config.antenna_height = Some(meters);
                    self.set_installation_config(radar_id, install_config);
                    Ok(())
                }
                "noTransmitZones" => {
                    // noTransmitZones: { zones: [{ enabled, start, end }, { enabled, start, end }] }
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
            }
        } else {
            debug(&format!(
                "No FurunoController for {} to set {}",
                radar_id, control_id
            ));
            Err(ControlError::ControllerNotAvailable)
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

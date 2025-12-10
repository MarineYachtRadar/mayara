//! Radar Provider
//!
//! Implements the SignalK Radar Provider interface.

use serde::Serialize;
use std::collections::BTreeMap;

use mayara_core::radar::RadarDiscovery;

use crate::furuno_controller::FurunoController;
use crate::locator::RadarLocator;
use crate::signalk_ffi::{debug, emit_json};
use crate::spoke_receiver::SpokeReceiver;

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
        // Color gradient like TimeZero Pro: blue (weak) -> red (strong)
        // Index 0 = transparent, indices 1-9 also transparent (noise floor)
        // Real returns start at index 10+
        let mut legend = BTreeMap::new();
        for i in 0..256u16 {
            let (r, g, b) = if i < 10 {
                // Index 0-9: transparent/black (noise floor)
                (0u8, 0u8, 0u8)
            } else if i < 20 {
                // 10-19: dark blue (very weak returns)
                let t = ((i - 10) * 15) as u8;
                (0, 0, 80 + t)
            } else if i < 40 {
                // 20-39: blue to purple transition
                let t = ((i - 20) * 10) as u8;
                (t * 2, 0, 180 - t)
            } else if i < 60 {
                // 40-59: purple to red transition
                let t = ((i - 40) * 8) as u8;
                (180 + t / 2, 0, 80 - t)
            } else {
                // 60-255: red to bright red (strong returns)
                let t = ((i - 60) as u16 * 55 / 196).min(55) as u8;
                (220 + t / 2, t / 2, t / 3)
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

        Self {
            id: id.clone(),
            name: sanitized_name.clone(),
            brand: brand_str.to_string(),
            model: d.model.clone().map(|m| sanitize_string(&m)),
            status: "standby".to_string(),
            spokes_per_revolution: d.spokes_per_revolution,
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
}

impl RadarProvider {
    /// Create a new radar provider
    pub fn new() -> Self {
        let mut locator = RadarLocator::new();
        locator.start();

        Self {
            locator,
            spoke_receiver: SpokeReceiver::new(),
            furuno_controllers: BTreeMap::new(),
            poll_count: 0,
        }
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

        // Poll all Furuno controllers
        for controller in self.furuno_controllers.values_mut() {
            controller.poll();
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
        debug(&format!("set_range({}, {})", radar_id, range));

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
}

impl Default for RadarProvider {
    fn default() -> Self {
        Self::new()
    }
}

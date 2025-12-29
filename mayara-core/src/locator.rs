//! Generic Radar Locator using IoProvider
//!
//! Discovers radars by listening on multicast addresses for beacon packets.
//! Works on both native (tokio) and WASM (FFI) platforms via the IoProvider trait.

use std::collections::BTreeMap;

use crate::io::{IoProvider, UdpSocketHandle};
use crate::protocol::{furuno, garmin, navico, raymarine};
use crate::radar::RadarDiscovery;
use crate::Brand;

/// Event from the radar locator
#[derive(Debug, Clone)]
pub enum LocatorEvent {
    /// A new radar was discovered
    RadarDiscovered(RadarDiscovery),
    /// An existing radar's info was updated (e.g., model report received)
    RadarUpdated(RadarDiscovery),
}

/// A discovered radar with its metadata
#[derive(Debug, Clone)]
pub struct DiscoveredRadar {
    pub discovery: RadarDiscovery,
    pub last_seen_ms: u64,
}

/// Status of a single brand's listener
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandStatus {
    /// Which brand this is for
    pub brand: Brand,
    /// Human-readable status ("Listening", "Failed to bind", etc.)
    pub status: String,
    /// Port being listened on (if active)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Multicast address (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multicast: Option<String>,

    /// Socket being listened on (if active)
    #[serde(skip_serializing)]
    pub(crate) socket: Option<UdpSocketHandle>,
    /// Network interface being used
    #[serde(skip_serializing)]
    pub(crate) interface: Option<String>,

    #[serde(skip_serializing)]
    poll: Option<
        fn(
            &Self,
            u64,
            &mut dyn IoProvider,
            &mut [u8],
            &mut Vec<RadarDiscovery>,
            &mut Vec<(String, Option<String>, Option<String>)>,
        ),
    >,
}

impl BrandStatus {
    fn error(brand: Brand, message: &str) -> Self {
        Self {
            brand,
            status: message.to_string(),
            port: None,
            multicast: None,
            socket: None,
            interface: None,
            poll: None,
        }
    }
}

/// Overall locator status showing which brands are being listened for
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocatorStatus {
    /// Current locator state (active, quiesced, idle)
    pub state: LocatorState,
    /// Status of each brand's listener
    pub brands: Vec<BrandStatus>,
}

/// Startup phase for staggered brand initialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupPhase {
    /// Not started yet
    NotStarted,
    /// Starting Furuno listener
    Furuno,
    /// Starting Navico BR24 listener
    NavicoBr24,
    /// Starting Navico Gen3+ listener
    NavicoGen3,
    /// Starting Raymarine listener
    Raymarine,
    /// Starting Garmin listener
    Garmin,
    /// All brands initialized
    Complete,
}

/// Locator state for quiescing support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocatorState {
    /// Locator is idle, not started
    #[default]
    Idle,
    /// Locator is actively listening for beacons
    Active,
    /// Locator is quiesced - sockets closed but can resume
    Quiesced,
}

/// Generic radar locator that discovers radars on the network
///
/// Uses the `IoProvider` trait for I/O operations, allowing the same code
/// to work on both native and WASM platforms.
pub struct RadarLocator {
    /// Discovered radars by ID (BTreeMap avoids WASI random_get requirement)
    pub radars: BTreeMap<String, DiscoveredRadar>,

    /// Poll counter for periodic announce
    poll_count: u64,

    /// Current status of each brand's listener
    status: LocatorStatus,

    /// Optional brand limitation (if set, only this brand is initialized)  
    brand_limitation: Option<Brand>,

    /// Optional interface IP for Furuno broadcasts (to prevent cross-NIC traffic)
    furuno_interface: Option<String>,

    /// Current startup phase for staggered initialization
    startup_phase: StartupPhase,

    /// List of interface IP addresses to join multicast on
    /// If empty, uses UNSPECIFIED (OS default). For multi-NIC setups,
    /// populate this with all NIC IPs to ensure multicast works on all interfaces.
    multicast_interfaces: Vec<String>,

    /// Current locator state (active, quiesced, idle)
    state: LocatorState,
}

impl RadarLocator {
    /// Create a new radar locator
    pub fn new(brand_limitation: Option<Brand>) -> Self {
        Self {
            radars: BTreeMap::new(),
            poll_count: 0,
            status: LocatorStatus::default(),
            furuno_interface: None,
            startup_phase: StartupPhase::NotStarted,
            brand_limitation,
            multicast_interfaces: Vec::new(),
            state: LocatorState::Idle,
        }
    }

    /// Set the interface IP to use for Furuno broadcasts.
    ///
    /// This is critical for multi-NIC setups to prevent broadcast packets
    /// from going out on the wrong interface (e.g., 192.168.0.x instead of 172.31.x.x).
    pub fn set_furuno_interface(&mut self, interface: &str) {
        self.furuno_interface = Some(interface.to_string());
    }

    /// Set the list of interface IPs to join multicast groups on.
    ///
    /// In multi-NIC setups, you MUST call this with all non-loopback IPv4 addresses
    /// to ensure multicast beacons are received on all interfaces. Without this,
    /// multicast joins default to a single OS-chosen interface.
    ///
    /// # Example
    /// ```rust,ignore
    /// locator.set_multicast_interfaces(&["192.168.0.106", "172.31.3.119"]);
    /// ```
    pub fn set_multicast_interfaces(&mut self, interfaces: &[&str]) {
        self.multicast_interfaces = interfaces.iter().map(|s| s.to_string()).collect();
    }

    /// Add a single interface to the multicast interface list.
    pub fn add_multicast_interface(&mut self, interface: &str) {
        if !self.multicast_interfaces.contains(&interface.to_string()) {
            self.multicast_interfaces.push(interface.to_string());
        }
    }

    /// Start listening for beacons
    ///
    /// This begins staggered initialization - one brand is initialized per poll cycle
    /// to spread out network activity (IGMP joins, etc.) and avoid flooding the network.
    pub fn start<I: IoProvider>(&mut self, io: &mut I) {
        self.status.brands.clear();
        self.startup_phase = match self.brand_limitation {
            Some(Brand::Furuno) => StartupPhase::Furuno,
            Some(Brand::Navico) => StartupPhase::NavicoBr24,
            Some(Brand::Raymarine) => StartupPhase::Raymarine,
            Some(Brand::Garmin) => StartupPhase::Garmin,
            None => StartupPhase::Furuno,
        };
        self.state = LocatorState::Active;
        io.info("Starting staggered brand initialization...");
        // First brand is initialized immediately
        self.advance_startup(io);
    }

    /// Advance startup phase - initializes one brand per call
    fn advance_startup<I: IoProvider>(&mut self, io: &mut I) {
        io.debug(&format!(
            "Advancing to startup phase: {:?}",
            self.startup_phase
        ));
        match self.startup_phase {
            StartupPhase::NotStarted => {
                // start() should be called first
            }
            StartupPhase::Furuno => {
                self.start_furuno(io);
                self.startup_phase = if self.brand_limitation.is_some() {
                    StartupPhase::Complete
                } else {
                    StartupPhase::NavicoBr24
                };
                io.debug("Startup: Furuno initialized, next: Navico BR24");
            }
            StartupPhase::NavicoBr24 => {
                self.start_navico_br24(io);
                self.startup_phase = StartupPhase::NavicoGen3;
            }
            StartupPhase::NavicoGen3 => {
                self.start_navico_gen3(io);
                self.startup_phase = if self.brand_limitation.is_some() {
                    StartupPhase::Complete
                } else {
                    StartupPhase::Raymarine
                };
            }
            StartupPhase::Raymarine => {
                self.start_raymarine(io);
                self.startup_phase = if self.brand_limitation.is_some() {
                    StartupPhase::Complete
                } else {
                    StartupPhase::Garmin
                };
            }
            StartupPhase::Garmin => {
                self.start_garmin(io);
                self.startup_phase = StartupPhase::Complete;
            }
            StartupPhase::Complete => {
                // Nothing to do
            }
        }
    }

    /// Check if startup is still in progress
    pub fn is_starting(&self) -> bool {
        self.startup_phase != StartupPhase::Complete
            && self.startup_phase != StartupPhase::NotStarted
    }

    /// Get the current status of all brand listeners
    pub fn status(&self) -> LocatorStatus {
        LocatorStatus {
            state: self.state,
            brands: self.status.brands.clone(),
        }
    }

    fn start_furuno<I: IoProvider>(&mut self, io: &mut I) {
        let status = match io.udp_create() {
            Ok(socket) => {
                // Enable broadcast mode BEFORE binding (required for sending to 172.31.255.255)
                if let Err(e) = io.udp_set_broadcast(&socket, true) {
                    io.debug(&format!("Warning: Failed to enable broadcast: {}", e));
                } else {
                    io.debug("Enabled broadcast on Furuno socket");
                }

                if io.udp_bind(&socket, furuno::BEACON_PORT).is_ok() {
                    // CRITICAL: Bind to specific interface if configured
                    // This prevents broadcast packets from going out on wrong NIC in multi-NIC setups
                    if let Some(ref interface) = self.furuno_interface {
                        if let Err(e) = io.udp_bind_interface(&socket, interface) {
                            io.debug(&format!(
                                "Warning: Failed to bind Furuno socket to interface {}: {}",
                                interface, e
                            ));
                        } else {
                            io.info(&format!(
                                "Furuno socket bound to interface {} (prevents cross-NIC traffic)",
                                interface
                            ));
                        }
                    }

                    io.debug(&format!(
                        "Listening for Furuno beacons on port {} (also used for announces)",
                        furuno::BEACON_PORT
                    ));
                    BrandStatus {
                        brand: Brand::Furuno,
                        status: "Listening".to_string(),
                        port: Some(furuno::BEACON_PORT),
                        multicast: None, // Furuno uses broadcast, not multicast
                        poll: Some(furuno::poll_beacon_packets),
                        socket: Some(socket),
                        interface: self.furuno_interface.clone(),
                    }
                } else {
                    io.debug("Failed to bind Furuno beacon socket");
                    io.udp_close(socket);
                    BrandStatus::error(Brand::Furuno, "Failed to bind")
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Furuno socket: {}", e));
                BrandStatus::error(Brand::Furuno, &format!("Failed: {e}"))
            }
        };
        self.status.brands.push(status);
    }

    /// Join a multicast group on all configured interfaces.
    /// Returns true if at least one join succeeded.
    fn join_multicast_all<I: IoProvider>(
        &self,
        io: &mut I,
        socket: &UdpSocketHandle,
        group: &str,
    ) -> bool {
        if self.multicast_interfaces.is_empty() {
            // No specific interfaces configured - use OS default
            io.udp_join_multicast(socket, group, "").is_ok()
        } else {
            // Join on each configured interface
            let mut any_success = false;
            for interface in &self.multicast_interfaces {
                match io.udp_join_multicast(socket, group, interface) {
                    Ok(()) => {
                        io.debug(&format!(
                            "Joined multicast {} on interface {}",
                            group, interface
                        ));
                        any_success = true;
                    }
                    Err(e) => {
                        io.debug(&format!(
                            "Failed to join multicast {} on {}: {}",
                            group, interface, e
                        ));
                    }
                }
            }
            any_success
        }
    }

    fn start_navico_br24<I: IoProvider>(&mut self, io: &mut I) {
        let status = match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, navico::BR24_BEACON_PORT).is_ok() {
                    if self.join_multicast_all(io, &socket, navico::BR24_BEACON_ADDR) {
                        io.debug(&format!(
                            "Listening for Navico BR24 beacons on {}:{}",
                            navico::BR24_BEACON_ADDR,
                            navico::BR24_BEACON_PORT
                        ));
                        BrandStatus {
                            brand: Brand::Navico,
                            status: "Listening (BR24)".to_string(),
                            port: Some(navico::BR24_BEACON_PORT),
                            multicast: Some(navico::BR24_BEACON_ADDR.to_string()),
                            poll: Some(navico::poll_beacon_packets),
                            socket: Some(socket),
                            interface: None,
                        }
                    } else {
                        io.debug("Failed to join Navico BR24 multicast group");
                        io.udp_close(socket);
                        BrandStatus::error(Brand::Navico, "Failed to join BR24 multicast")
                    }
                } else {
                    io.debug("Failed to bind Navico BR24 beacon socket");
                    io.udp_close(socket);
                    BrandStatus::error(Brand::Navico, "Failed to bind BR24")
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Navico BR24 socket: {}", e));
                BrandStatus::error(Brand::Navico, &format!("BR24 failed: {}", e))
            }
        };
        self.status.brands.push(status);
    }

    fn start_navico_gen3<I: IoProvider>(&mut self, io: &mut I) {
        let status = match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, navico::GEN3_BEACON_PORT).is_ok() {
                    if self.join_multicast_all(io, &socket, navico::GEN3_BEACON_ADDR) {
                        io.debug(&format!(
                            "Listening for Navico 3G/4G/HALO beacons on {}:{}",
                            navico::GEN3_BEACON_ADDR,
                            navico::GEN3_BEACON_PORT
                        ));
                        BrandStatus {
                            brand: Brand::Navico,
                            status: "Listening (G3/4/HALO)".to_string(),
                            port: Some(navico::GEN3_BEACON_PORT),
                            multicast: Some(navico::GEN3_BEACON_ADDR.to_string()),
                            poll: Some(navico::poll_beacon_packets),
                            socket: Some(socket),
                            interface: None,
                        }
                    } else {
                        io.debug("Failed to join Navico Gen3 multicast group");
                        io.udp_close(socket);
                        BrandStatus::error(Brand::Navico, "Failed to join Gen3 multicast")
                    }
                } else {
                    io.debug("Failed to bind Navico Gen3 beacon socket");
                    io.udp_close(socket);
                    BrandStatus::error(Brand::Navico, "Failed to bind Gen3")
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Navico Gen3 socket: {}", e));
                BrandStatus::error(Brand::Navico, &format!("Gen3 failed: {}", e))
            }
        };
        self.status.brands.push(status);
    }

    fn start_raymarine<I: IoProvider>(&mut self, io: &mut I) {
        let status = match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, raymarine::BEACON_PORT).is_ok() {
                    if self.join_multicast_all(io, &socket, raymarine::BEACON_ADDR) {
                        io.debug(&format!(
                            "Listening for Raymarine beacons on {}:{}",
                            raymarine::BEACON_ADDR,
                            raymarine::BEACON_PORT
                        ));
                        BrandStatus {
                            brand: Brand::Raymarine,
                            status: "Listening".to_string(),
                            port: Some(raymarine::BEACON_PORT),
                            multicast: Some(raymarine::BEACON_ADDR.to_string()),
                            poll: None,
                            socket: Some(socket),
                            interface: None,
                        }
                    } else {
                        io.debug("Failed to join Raymarine multicast group");
                        io.udp_close(socket);
                        BrandStatus::error(Brand::Raymarine, "Failed to join multicast")
                    }
                } else {
                    io.debug("Failed to bind Raymarine beacon socket");
                    io.udp_close(socket);
                    BrandStatus::error(Brand::Raymarine, "Failed to bind")
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Raymarine socket: {}", e));
                BrandStatus::error(Brand::Raymarine, &format!("Failed: {}", e))
            }
        };
        self.status.brands.push(status);
    }

    fn start_garmin<I: IoProvider>(&mut self, io: &mut I) {
        let status = match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, garmin::REPORT_PORT).is_ok() {
                    if self.join_multicast_all(io, &socket, garmin::REPORT_ADDR) {
                        io.debug(&format!(
                            "Listening for Garmin on {}:{}",
                            garmin::REPORT_ADDR,
                            garmin::REPORT_PORT
                        ));
                        BrandStatus {
                            brand: Brand::Garmin,
                            status: "Listening".to_string(),
                            port: Some(garmin::REPORT_PORT),
                            multicast: Some(garmin::REPORT_ADDR.to_string()),
                            poll: None,
                            socket: Some(socket),
                            interface: None,
                        }
                    } else {
                        io.debug("Failed to join Garmin multicast group");
                        io.udp_close(socket);
                        BrandStatus::error(Brand::Garmin, "Failed to join multicast")
                    }
                } else {
                    io.debug("Failed to bind Garmin report socket");
                    io.udp_close(socket);
                    BrandStatus::error(Brand::Garmin, "Failed to bind")
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Garmin socket: {}", e));
                BrandStatus::error(Brand::Garmin, &format!("Failed: {}", e))
            }
        };
        self.status.brands.push(status);
    }

    /// Poll for incoming beacon packets
    ///
    /// Returns list of locator events (new discoveries and updates).
    pub fn poll<I: IoProvider>(&mut self, io: &mut I) -> Vec<LocatorEvent> {
        let current_time_ms = io.current_time_ms();

        // Advance staggered startup - one brand per poll cycle
        // This spreads out IGMP joins and socket creation to avoid network flood
        if self.is_starting() {
            self.advance_startup(io);
        }

        let mut events = Vec::new();
        let mut discoveries = Vec::new();
        let mut buf = [0u8; 2048];

        // Model reports: (source_addr, model, serial)
        let mut model_reports: Vec<(String, Option<String>, Option<String>)> = Vec::new();

        for brand in &self.status.brands {
            if let Some(poll_fn) = brand.poll {
                poll_fn(
                    brand,
                    self.poll_count,
                    io,
                    &mut buf,
                    &mut discoveries,
                    &mut model_reports,
                );
            }
        }
        self.poll_count += 1;

        // Add all discoveries to the radar list
        for discovery in discoveries {
            if self.add_radar(io, &discovery, current_time_ms) {
                events.push(LocatorEvent::RadarDiscovered(discovery));
            }
        }

        // Apply model reports to existing radars (after discoveries are added)
        // This ensures the radar exists before we try to update its model info
        for (addr, model, serial) in model_reports {
            if let Some(updated) =
                self.update_radar_model_info(io, &addr, model.as_deref(), serial.as_deref())
            {
                events.push(LocatorEvent::RadarUpdated(updated));
            }
        }

        events
    }

    /// Update model/serial info for an existing radar.
    /// Returns the updated discovery if anything changed.
    fn update_radar_model_info<I: IoProvider>(
        &mut self,
        io: &I,
        source_addr: &str,
        model: Option<&str>,
        serial: Option<&str>,
    ) -> Option<RadarDiscovery> {
        let source_ip = source_addr.split(':').next().unwrap_or(source_addr);

        for (_id, radar) in self.radars.iter_mut() {
            let radar_ip = radar
                .discovery
                .address
                .split(':')
                .next()
                .unwrap_or(&radar.discovery.address);

            if radar_ip == source_ip {
                let mut changed = false;

                if let Some(m) = model {
                    if radar.discovery.model.is_none()
                        || radar.discovery.model.as_deref() != Some(m)
                    {
                        io.info(&format!(
                            "Updating radar {} model: {:?} -> {}",
                            radar.discovery.name, radar.discovery.model, m
                        ));
                        radar.discovery.model = Some(m.to_string());
                        changed = true;
                    }
                }
                if let Some(s) = serial {
                    if radar.discovery.serial_number.is_none()
                        || radar.discovery.serial_number.as_deref() != Some(s)
                    {
                        io.debug(&format!(
                            "Updating radar {} serial: {:?} -> {}",
                            radar.discovery.name, radar.discovery.serial_number, s
                        ));
                        radar.discovery.serial_number = Some(s.to_string());
                        changed = true;
                    }
                }

                if changed {
                    return Some(radar.discovery.clone());
                }
                return None;
            }
        }

        io.debug(&format!(
            "Model report for unknown radar at {}: model={:?}, serial={:?}",
            source_addr, model, serial
        ));
        None
    }

    fn add_radar<I: IoProvider>(
        &mut self,
        io: &I,
        discovery: &RadarDiscovery,
        current_time_ms: u64,
    ) -> bool {
        let id = self.make_radar_id(discovery);

        if self.radars.contains_key(&id) {
            if let Some(radar) = self.radars.get_mut(&id) {
                radar.last_seen_ms = current_time_ms;
            }
            false
        } else {
            io.debug(&format!(
                "Discovered {} radar: {} at {}",
                discovery.brand, discovery.name, discovery.address
            ));
            self.radars.insert(
                id,
                DiscoveredRadar {
                    discovery: discovery.clone(),
                    last_seen_ms: current_time_ms,
                },
            );
            true
        }
    }

    fn make_radar_id(&self, discovery: &RadarDiscovery) -> String {
        if let Some(suffix) = &discovery.suffix {
            format!("{}-{}-{}", discovery.brand, discovery.name, suffix)
        } else {
            format!("{}-{}", discovery.brand, discovery.name)
        }
    }

    /// Stop all locator sockets and clean up
    pub fn shutdown<I: IoProvider>(&mut self, io: &mut I) {
        self.close_all_sockets(io);
        self.state = LocatorState::Idle;
        self.radars.clear();
        io.info("Locator shutdown complete");
    }

    /// Quiesce the locator - close sockets but retain discovered radars
    ///
    /// Call this when all radars are connected to save CPU and network resources.
    /// The locator stops listening for beacons but remembers discovered radars.
    /// Use `resume()` to start listening again (e.g., when a radar disconnects).
    ///
    /// Unlike `shutdown()`, quiesce preserves the radar list and can be resumed.
    pub fn quiesce<I: IoProvider>(&mut self, io: &mut I) {
        if self.state != LocatorState::Active {
            io.debug("Locator not active, nothing to quiesce");
            return;
        }

        self.close_all_sockets(io);
        self.state = LocatorState::Quiesced;
        self.status.brands.iter_mut().for_each(|b| {
            b.status = "Quiesced".to_string();
            b.port = None;
            b.multicast = None;
        });
        io.info(&format!(
            "Locator quiesced - {} radars retained, sockets closed",
            self.radars.len()
        ));
    }

    /// Resume the locator after quiescing
    ///
    /// Reopens sockets and restarts listening for beacons.
    /// Call this when a radar disconnects and you want to rediscover radars.
    pub fn resume<I: IoProvider>(&mut self, io: &mut I) {
        if self.state != LocatorState::Quiesced {
            io.debug("Locator not quiesced, nothing to resume");
            return;
        }

        io.info(&format!(
            "Resuming locator - {} radars already known",
            self.radars.len()
        ));
        // Restart uses staggered initialization like start()
        self.start(io);
    }

    /// Get the current locator state
    pub fn state(&self) -> LocatorState {
        self.state
    }

    /// Check if the locator is quiesced
    pub fn is_quiesced(&self) -> bool {
        self.state == LocatorState::Quiesced
    }

    /// Check if the locator is actively listening
    pub fn is_active(&self) -> bool {
        self.state == LocatorState::Active
    }

    /// Close all sockets (helper for shutdown/quiesce)
    fn close_all_sockets<I: IoProvider>(&mut self, io: &mut I) {
        for brand in &mut self.status.brands {
            if let Some(socket) = brand.socket.take() {
                io.udp_close(socket);
            }
        }
    }
}

impl Default for RadarLocator {
    fn default() -> Self {
        Self::new(None)
    }
}

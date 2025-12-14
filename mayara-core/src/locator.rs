//! Generic Radar Locator using IoProvider
//!
//! Discovers radars by listening on multicast addresses for beacon packets.
//! Works on both native (tokio) and WASM (FFI) platforms via the IoProvider trait.

use std::collections::BTreeMap;

use crate::io::{IoProvider, UdpSocketHandle};
use crate::protocol::{furuno, garmin, navico, raymarine};
use crate::radar::RadarDiscovery;

/// Furuno beacon/announce broadcast address
const FURUNO_BEACON_BROADCAST: &str = "172.31.255.255";

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

/// Generic radar locator that discovers radars on the network
///
/// Uses the `IoProvider` trait for I/O operations, allowing the same code
/// to work on both native and WASM platforms.
pub struct RadarLocator {
    /// Furuno beacon socket (for receiving beacons AND sending announces)
    furuno_socket: Option<UdpSocketHandle>,
    /// Navico BR24 beacon socket
    navico_br24_socket: Option<UdpSocketHandle>,
    /// Navico Gen3+ beacon socket
    navico_gen3_socket: Option<UdpSocketHandle>,
    /// Raymarine beacon socket
    raymarine_socket: Option<UdpSocketHandle>,
    /// Garmin report socket
    garmin_socket: Option<UdpSocketHandle>,

    /// Discovered radars by ID (BTreeMap avoids WASI random_get requirement)
    pub radars: BTreeMap<String, DiscoveredRadar>,

    /// Poll counter for periodic announce
    poll_count: u64,
}

impl RadarLocator {
    /// Create a new radar locator
    pub fn new() -> Self {
        Self {
            furuno_socket: None,
            navico_br24_socket: None,
            navico_gen3_socket: None,
            raymarine_socket: None,
            garmin_socket: None,
            radars: BTreeMap::new(),
            poll_count: 0,
        }
    }

    /// Start listening for beacons
    pub fn start<I: IoProvider>(&mut self, io: &mut I) {
        self.start_furuno(io);
        self.start_navico_br24(io);
        self.start_navico_gen3(io);
        self.start_raymarine(io);
        self.start_garmin(io);
    }

    fn start_furuno<I: IoProvider>(&mut self, io: &mut I) {
        match io.udp_create() {
            Ok(socket) => {
                // Enable broadcast mode BEFORE binding (required for sending to 172.31.255.255)
                if let Err(e) = io.udp_set_broadcast(&socket, true) {
                    io.debug(&format!("Warning: Failed to enable broadcast: {}", e));
                } else {
                    io.debug("Enabled broadcast on Furuno socket");
                }

                if io.udp_bind(&socket, furuno::BEACON_PORT).is_ok() {
                    io.debug(&format!(
                        "Listening for Furuno beacons on port {} (also used for announces)",
                        furuno::BEACON_PORT
                    ));
                    self.furuno_socket = Some(socket);
                    // Send initial announce from the same socket (port 10010)
                    self.send_furuno_announce(io);
                } else {
                    io.debug("Failed to bind Furuno beacon socket");
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Furuno socket: {}", e));
            }
        }
    }

    /// Send Furuno announce and beacon request packets
    ///
    /// This should be called before attempting TCP connections to Furuno radars,
    /// as the radar only accepts TCP from clients that have recently announced.
    pub fn send_furuno_announce<I: IoProvider>(&self, io: &mut I) {
        if let Some(socket) = &self.furuno_socket {
            let addr = FURUNO_BEACON_BROADCAST;
            let port = furuno::BEACON_PORT;

            // Send beacon request
            if let Err(e) = io.udp_send_to(socket, &furuno::REQUEST_BEACON_PACKET, addr, port) {
                io.debug(&format!("Failed to send Furuno beacon request: {}", e));
            }

            // Send model request
            if let Err(e) = io.udp_send_to(socket, &furuno::REQUEST_MODEL_PACKET, addr, port) {
                io.debug(&format!("Failed to send Furuno model request: {}", e));
            }

            // Send announce packet - this tells the radar we exist
            if let Err(e) = io.udp_send_to(socket, &furuno::ANNOUNCE_PACKET, addr, port) {
                io.debug(&format!("Failed to send Furuno announce: {}", e));
            } else {
                io.debug(&format!("Sent Furuno announce to {}:{}", addr, port));
            }
        }
    }

    fn start_navico_br24<I: IoProvider>(&mut self, io: &mut I) {
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, navico::BR24_BEACON_PORT).is_ok() {
                    if io.udp_join_multicast(&socket, navico::BR24_BEACON_ADDR, "").is_ok() {
                        io.debug(&format!(
                            "Listening for Navico BR24 beacons on {}:{}",
                            navico::BR24_BEACON_ADDR,
                            navico::BR24_BEACON_PORT
                        ));
                        self.navico_br24_socket = Some(socket);
                    } else {
                        io.debug("Failed to join Navico BR24 multicast group");
                        io.udp_close(socket);
                    }
                } else {
                    io.debug("Failed to bind Navico BR24 beacon socket");
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Navico BR24 socket: {}", e));
            }
        }
    }

    fn start_navico_gen3<I: IoProvider>(&mut self, io: &mut I) {
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, navico::GEN3_BEACON_PORT).is_ok() {
                    if io.udp_join_multicast(&socket, navico::GEN3_BEACON_ADDR, "").is_ok() {
                        io.debug(&format!(
                            "Listening for Navico 3G/4G/HALO beacons on {}:{}",
                            navico::GEN3_BEACON_ADDR,
                            navico::GEN3_BEACON_PORT
                        ));
                        self.navico_gen3_socket = Some(socket);
                    } else {
                        io.debug("Failed to join Navico Gen3 multicast group");
                        io.udp_close(socket);
                    }
                } else {
                    io.debug("Failed to bind Navico Gen3 beacon socket");
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Navico Gen3 socket: {}", e));
            }
        }
    }

    fn start_raymarine<I: IoProvider>(&mut self, io: &mut I) {
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, raymarine::BEACON_PORT).is_ok() {
                    if io.udp_join_multicast(&socket, raymarine::BEACON_ADDR, "").is_ok() {
                        io.debug(&format!(
                            "Listening for Raymarine beacons on {}:{}",
                            raymarine::BEACON_ADDR,
                            raymarine::BEACON_PORT
                        ));
                        self.raymarine_socket = Some(socket);
                    } else {
                        io.debug("Failed to join Raymarine multicast group");
                        io.udp_close(socket);
                    }
                } else {
                    io.debug("Failed to bind Raymarine beacon socket");
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Raymarine socket: {}", e));
            }
        }
    }

    fn start_garmin<I: IoProvider>(&mut self, io: &mut I) {
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, garmin::REPORT_PORT).is_ok() {
                    if io.udp_join_multicast(&socket, garmin::REPORT_ADDR, "").is_ok() {
                        io.debug(&format!(
                            "Listening for Garmin on {}:{}",
                            garmin::REPORT_ADDR,
                            garmin::REPORT_PORT
                        ));
                        self.garmin_socket = Some(socket);
                    } else {
                        io.debug("Failed to join Garmin multicast group");
                        io.udp_close(socket);
                    }
                } else {
                    io.debug("Failed to bind Garmin report socket");
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("Failed to create Garmin socket: {}", e));
            }
        }
    }

    /// Poll for incoming beacon packets
    ///
    /// Returns list of locator events (new discoveries and updates).
    pub fn poll<I: IoProvider>(&mut self, io: &mut I) -> Vec<LocatorEvent> {
        self.poll_count += 1;
        let current_time_ms = io.current_time_ms();

        // Send Furuno announce periodically (every ~2 seconds at 10 polls/sec)
        const ANNOUNCE_INTERVAL: u64 = 20;
        if self.poll_count % ANNOUNCE_INTERVAL == 0 {
            self.send_furuno_announce(io);
        }

        let mut events = Vec::new();
        let mut discoveries = Vec::new();
        let mut buf = [0u8; 2048];

        // Model reports: (source_addr, model, serial)
        let mut model_reports: Vec<(String, Option<String>, Option<String>)> = Vec::new();

        // Poll Furuno
        self.poll_furuno(io, &mut buf, &mut discoveries, &mut model_reports);

        // Apply model reports to existing radars
        for (addr, model, serial) in model_reports {
            if let Some(updated) = self.update_radar_model_info(io, &addr, model.as_deref(), serial.as_deref()) {
                events.push(LocatorEvent::RadarUpdated(updated));
            }
        }

        // Poll Navico BR24
        if let Some(socket) = self.navico_br24_socket {
            while let Some((len, addr, _port)) = io.udp_recv_from(&socket, &mut buf) {
                let data = &buf[..len];
                if !navico::is_beacon_response(data) {
                    continue;
                }
                match navico::parse_beacon_response(data, &addr) {
                    Ok(discovery) => {
                        io.debug(&format!("Navico BR24 beacon from {}: {:?}", addr, discovery.model));
                        discoveries.push(discovery);
                    }
                    Err(e) => {
                        io.debug(&format!("Navico BR24 parse error: {}", e));
                    }
                }
            }
        }

        // Poll Navico Gen3+
        if let Some(socket) = self.navico_gen3_socket {
            while let Some((len, addr, _port)) = io.udp_recv_from(&socket, &mut buf) {
                let data = &buf[..len];
                if !navico::is_beacon_response(data) {
                    continue;
                }
                match navico::parse_beacon_response(data, &addr) {
                    Ok(discovery) => {
                        io.debug(&format!("Navico Gen3 beacon from {}: {:?}", addr, discovery.model));
                        discoveries.push(discovery);
                    }
                    Err(e) => {
                        io.debug(&format!("Navico Gen3 parse error: {}", e));
                    }
                }
            }
        }

        // Poll Raymarine
        if let Some(socket) = self.raymarine_socket {
            while let Some((len, addr, _port)) = io.udp_recv_from(&socket, &mut buf) {
                let data = &buf[..len];
                if !raymarine::is_beacon_36(data) && !raymarine::is_beacon_56(data) {
                    continue;
                }
                match raymarine::parse_beacon_response(data, &addr) {
                    Ok(discovery) => {
                        io.debug(&format!("Raymarine beacon from {}: {:?}", addr, discovery.model));
                        discoveries.push(discovery);
                    }
                    Err(e) => {
                        io.debug(&format!("Raymarine parse error: {}", e));
                    }
                }
            }
        }

        // Poll Garmin
        if let Some(socket) = self.garmin_socket {
            while let Some((len, addr, _port)) = io.udp_recv_from(&socket, &mut buf) {
                let data = &buf[..len];
                if !garmin::is_report_packet(data) {
                    continue;
                }
                let discovery = garmin::create_discovery(&addr);
                discoveries.push(discovery);
            }
        }

        // Add all discoveries to the radar list
        for discovery in discoveries {
            if self.add_radar(io, &discovery, current_time_ms) {
                events.push(LocatorEvent::RadarDiscovered(discovery));
            }
        }

        events
    }

    fn poll_furuno<I: IoProvider>(
        &self,
        io: &mut I,
        buf: &mut [u8],
        discoveries: &mut Vec<RadarDiscovery>,
        model_reports: &mut Vec<(String, Option<String>, Option<String>)>,
    ) {
        if let Some(socket) = self.furuno_socket {
            while let Some((len, addr, _port)) = io.udp_recv_from(&socket, buf) {
                let data = &buf[..len];

                if furuno::is_beacon_response(data) {
                    match furuno::parse_beacon_response(data, &addr) {
                        Ok(discovery) => {
                            io.debug(&format!("Furuno beacon from {}: {:?}", addr, discovery.model));
                            discoveries.push(discovery);
                        }
                        Err(e) => {
                            io.debug(&format!("Furuno beacon parse error: {}", e));
                        }
                    }
                } else if furuno::is_model_report(data) {
                    match furuno::parse_model_report(data) {
                        Ok((model, serial)) => {
                            io.debug(&format!(
                                "Furuno model report from {}: model={:?}, serial={:?}",
                                addr, model, serial
                            ));
                            if model.is_some() || serial.is_some() {
                                model_reports.push((addr.clone(), model, serial));
                            }
                        }
                        Err(e) => {
                            io.debug(&format!("Furuno model report parse error: {}", e));
                        }
                    }
                }
            }
        }
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
            let radar_ip = radar.discovery.address.split(':').next().unwrap_or(&radar.discovery.address);

            if radar_ip == source_ip {
                let mut changed = false;

                if let Some(m) = model {
                    if radar.discovery.model.is_none() || radar.discovery.model.as_deref() != Some(m) {
                        io.debug(&format!(
                            "Updating radar {} model: {:?} -> {}",
                            radar.discovery.name, radar.discovery.model, m
                        ));
                        radar.discovery.model = Some(m.to_string());
                        changed = true;
                    }
                }
                if let Some(s) = serial {
                    if radar.discovery.serial_number.is_none() || radar.discovery.serial_number.as_deref() != Some(s) {
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

    fn add_radar<I: IoProvider>(&mut self, io: &I, discovery: &RadarDiscovery, current_time_ms: u64) -> bool {
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
        format!("{}-{}", discovery.brand, discovery.name)
    }

    /// Stop all locator sockets and clean up
    pub fn shutdown<I: IoProvider>(&mut self, io: &mut I) {
        if let Some(socket) = self.furuno_socket.take() {
            io.udp_close(socket);
        }
        if let Some(socket) = self.navico_br24_socket.take() {
            io.udp_close(socket);
        }
        if let Some(socket) = self.navico_gen3_socket.take() {
            io.udp_close(socket);
        }
        if let Some(socket) = self.raymarine_socket.take() {
            io.udp_close(socket);
        }
        if let Some(socket) = self.garmin_socket.take() {
            io.udp_close(socket);
        }
    }
}

impl Default for RadarLocator {
    fn default() -> Self {
        Self::new()
    }
}

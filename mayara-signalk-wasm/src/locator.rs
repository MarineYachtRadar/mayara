//! Radar Locator
//!
//! Discovers radars by listening on multicast addresses for beacon packets.
//! Uses mayara-core for protocol parsing.

use std::collections::BTreeMap;

use mayara_core::protocol::{furuno, garmin, navico, raymarine};
use mayara_core::radar::RadarDiscovery;

use crate::signalk_ffi::{debug, set_error, UdpSocket};

/// Furuno beacon/announce broadcast address
const FURUNO_BEACON_BROADCAST: &str = "172.31.255.255";

/// A discovered radar with its metadata
#[derive(Debug, Clone)]
pub struct DiscoveredRadar {
    pub discovery: RadarDiscovery,
    pub last_seen_ms: u64,
}

/// Radar locator that discovers radars on the network
pub struct RadarLocator {
    /// Furuno beacon socket (for receiving beacons AND sending announces)
    /// Both use port 10010 - the radar requires announces from the beacon port
    furuno_socket: Option<UdpSocket>,
    /// Navico BR24 beacon socket
    navico_br24_socket: Option<UdpSocket>,
    /// Navico Gen3+ beacon socket
    navico_gen3_socket: Option<UdpSocket>,
    /// Raymarine beacon socket
    raymarine_socket: Option<UdpSocket>,
    /// Garmin report socket
    garmin_socket: Option<UdpSocket>,

    /// Discovered radars by ID (BTreeMap avoids WASI random_get requirement)
    pub radars: BTreeMap<String, DiscoveredRadar>,

    /// Current timestamp (updated externally)
    pub current_time_ms: u64,

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
            current_time_ms: 0,
            poll_count: 0,
        }
    }

    /// Start listening for beacons
    pub fn start(&mut self) {
        self.start_furuno();
        self.start_navico_br24();
        self.start_navico_gen3();
        self.start_raymarine();
        self.start_garmin();
    }

    fn start_furuno(&mut self) {
        // Create socket for both receiving beacons AND sending announces
        // IMPORTANT: We must send announces from port 10010 (same as we listen on)
        // The Furuno radar only accepts TCP connections from clients that announce
        // from the beacon port (10010), not from ephemeral ports.
        match UdpSocket::new() {
            Ok(socket) => {
                // Enable broadcast mode BEFORE binding (required for sending to 172.31.255.255)
                if let Err(e) = socket.set_broadcast(true) {
                    debug(&format!("Warning: Failed to enable broadcast: {}", e));
                } else {
                    debug("Enabled broadcast on Furuno socket");
                }

                if socket.bind_port(furuno::BEACON_PORT).is_ok() {
                    debug(&format!(
                        "Listening for Furuno beacons on port {} (also used for announces)",
                        furuno::BEACON_PORT
                    ));
                    self.furuno_socket = Some(socket);
                    // Send initial announce from the same socket (port 10010)
                    self.send_furuno_announce();
                } else {
                    set_error("Failed to bind Furuno beacon socket");
                }
            }
            Err(e) => {
                set_error(&format!("Failed to create Furuno socket: {}", e));
            }
        }

        // No separate announce socket needed - we use furuno_socket for both
        // receiving beacons and sending announces (from port 10010)
    }

    /// Send Furuno announce and beacon request packets
    ///
    /// This should be called before attempting TCP connections to Furuno radars,
    /// as the radar only accepts TCP from clients that have recently announced.
    /// Announces are sent from port 10010 (the beacon port) - this is required
    /// for the radar to accept our TCP connections.
    pub fn send_furuno_announce(&self) {
        if let Some(socket) = &self.furuno_socket {
            // Send to broadcast address on the Furuno subnet
            let addr = FURUNO_BEACON_BROADCAST;
            let port = furuno::BEACON_PORT;

            // Send beacon request
            if let Err(e) = socket.send_to(&furuno::REQUEST_BEACON_PACKET, addr, port) {
                debug(&format!("Failed to send Furuno beacon request: {}", e));
            }

            // Send model request
            if let Err(e) = socket.send_to(&furuno::REQUEST_MODEL_PACKET, addr, port) {
                debug(&format!("Failed to send Furuno model request: {}", e));
            }

            // Send announce packet - this tells the radar we exist
            if let Err(e) = socket.send_to(&furuno::ANNOUNCE_PACKET, addr, port) {
                debug(&format!("Failed to send Furuno announce: {}", e));
            } else {
                debug(&format!("Sent Furuno announce to {}:{}", addr, port));
            }
        }
    }

    fn start_navico_br24(&mut self) {
        match UdpSocket::new() {
            Ok(socket) => {
                if socket.bind_port(navico::BR24_BEACON_PORT).is_ok() {
                    if socket.join_multicast(navico::BR24_BEACON_ADDR).is_ok() {
                        debug(&format!(
                            "Listening for Navico BR24 beacons on {}:{}",
                            navico::BR24_BEACON_ADDR,
                            navico::BR24_BEACON_PORT
                        ));
                        self.navico_br24_socket = Some(socket);
                    } else {
                        set_error("Failed to join Navico BR24 multicast group");
                    }
                } else {
                    set_error("Failed to bind Navico BR24 beacon socket");
                }
            }
            Err(e) => {
                set_error(&format!("Failed to create Navico BR24 socket: {}", e));
            }
        }
    }

    fn start_navico_gen3(&mut self) {
        match UdpSocket::new() {
            Ok(socket) => {
                if socket.bind_port(navico::GEN3_BEACON_PORT).is_ok() {
                    if socket.join_multicast(navico::GEN3_BEACON_ADDR).is_ok() {
                        debug(&format!(
                            "Listening for Navico 3G/4G/HALO beacons on {}:{}",
                            navico::GEN3_BEACON_ADDR,
                            navico::GEN3_BEACON_PORT
                        ));
                        self.navico_gen3_socket = Some(socket);
                    } else {
                        set_error("Failed to join Navico Gen3 multicast group");
                    }
                } else {
                    set_error("Failed to bind Navico Gen3 beacon socket");
                }
            }
            Err(e) => {
                set_error(&format!("Failed to create Navico Gen3 socket: {}", e));
            }
        }
    }

    fn start_raymarine(&mut self) {
        match UdpSocket::new() {
            Ok(socket) => {
                if socket.bind_port(raymarine::BEACON_PORT).is_ok() {
                    if socket.join_multicast(raymarine::BEACON_ADDR).is_ok() {
                        debug(&format!(
                            "Listening for Raymarine beacons on {}:{}",
                            raymarine::BEACON_ADDR,
                            raymarine::BEACON_PORT
                        ));
                        self.raymarine_socket = Some(socket);
                    } else {
                        set_error("Failed to join Raymarine multicast group");
                    }
                } else {
                    set_error("Failed to bind Raymarine beacon socket");
                }
            }
            Err(e) => {
                set_error(&format!("Failed to create Raymarine socket: {}", e));
            }
        }
    }

    fn start_garmin(&mut self) {
        match UdpSocket::new() {
            Ok(socket) => {
                if socket.bind_port(garmin::REPORT_PORT).is_ok() {
                    if socket.join_multicast(garmin::REPORT_ADDR).is_ok() {
                        debug(&format!(
                            "Listening for Garmin on {}:{}",
                            garmin::REPORT_ADDR,
                            garmin::REPORT_PORT
                        ));
                        self.garmin_socket = Some(socket);
                    } else {
                        set_error("Failed to join Garmin multicast group");
                    }
                } else {
                    set_error("Failed to bind Garmin report socket");
                }
            }
            Err(e) => {
                set_error(&format!("Failed to create Garmin socket: {}", e));
            }
        }
    }

    /// Poll for incoming beacon packets
    ///
    /// Returns list of newly discovered radars.
    pub fn poll(&mut self) -> Vec<RadarDiscovery> {
        self.poll_count += 1;

        // Send Furuno announce periodically (every ~2 seconds at 10 polls/sec)
        // This is needed for the radar to accept TCP connections from us
        // Native mayara-lib sends every 2 seconds
        const ANNOUNCE_INTERVAL: u64 = 20;
        if self.poll_count % ANNOUNCE_INTERVAL == 0 {
            self.send_furuno_announce();
        }

        let mut discoveries = Vec::new();
        let mut buf = [0u8; 2048];

        // Poll Furuno - collect discoveries and model reports
        // Model reports are 170 bytes and come separately from beacons
        // Tuple: (source_addr, model, serial)
        let mut model_reports: Vec<(String, Option<String>, Option<String>)> = Vec::new();

        self.poll_socket_furuno(&mut buf, &mut discoveries, &mut model_reports);

        // Apply model reports to existing radars
        for (addr, model, serial) in model_reports {
            self.update_radar_model_info(&addr, model.as_deref(), serial.as_deref());
        }

        // Poll Navico BR24
        self.poll_socket(&self.navico_br24_socket.as_ref(), &mut buf, &mut discoveries, |data, addr| {
            if !navico::is_beacon_response(data) {
                return None;
            }
            match navico::parse_beacon_response(data, addr) {
                Ok(discovery) => {
                    debug(&format!("Navico BR24 beacon from {}: {:?}", addr, discovery.model));
                    Some(discovery)
                }
                Err(e) => {
                    debug(&format!("Navico BR24 parse error: {}", e));
                    None
                }
            }
        });

        // Poll Navico Gen3+
        self.poll_socket(&self.navico_gen3_socket.as_ref(), &mut buf, &mut discoveries, |data, addr| {
            if !navico::is_beacon_response(data) {
                return None;
            }
            match navico::parse_beacon_response(data, addr) {
                Ok(discovery) => {
                    debug(&format!("Navico Gen3 beacon from {}: {:?}", addr, discovery.model));
                    Some(discovery)
                }
                Err(e) => {
                    debug(&format!("Navico Gen3 parse error: {}", e));
                    None
                }
            }
        });

        // Poll Raymarine
        self.poll_socket(&self.raymarine_socket.as_ref(), &mut buf, &mut discoveries, |data, addr| {
            if !raymarine::is_beacon_36(data) && !raymarine::is_beacon_56(data) {
                return None;
            }
            match raymarine::parse_beacon_response(data, addr) {
                Ok(discovery) => {
                    debug(&format!("Raymarine beacon from {}: {:?}", addr, discovery.model));
                    Some(discovery)
                }
                Err(e) => {
                    debug(&format!("Raymarine parse error: {}", e));
                    None
                }
            }
        });

        // Poll Garmin (uses reports, not beacons)
        self.poll_socket(&self.garmin_socket.as_ref(), &mut buf, &mut discoveries, |data, addr| {
            if !garmin::is_report_packet(data) {
                return None;
            }
            Some(garmin::create_discovery(addr))
        });

        // Now add all discoveries to the radar list (mutable borrow)
        let mut new_radars = Vec::new();
        for discovery in discoveries {
            if self.add_radar(&discovery) {
                new_radars.push(discovery);
            }
        }

        new_radars
    }

    fn poll_socket<F>(
        &self,
        socket: &Option<&UdpSocket>,
        buf: &mut [u8],
        discoveries: &mut Vec<RadarDiscovery>,
        parser: F,
    ) where
        F: Fn(&[u8], &str) -> Option<RadarDiscovery>,
    {
        if let Some(socket) = socket {
            while let Some((len, addr, _port)) = socket.recv_from(buf) {
                if let Some(discovery) = parser(&buf[..len], &addr) {
                    discoveries.push(discovery);
                }
            }
        }
    }

    /// Poll Furuno socket for beacons AND model reports
    fn poll_socket_furuno(
        &self,
        buf: &mut [u8],
        discoveries: &mut Vec<RadarDiscovery>,
        model_reports: &mut Vec<(String, Option<String>, Option<String>)>,
    ) {
        if let Some(socket) = &self.furuno_socket {
            while let Some((len, addr, _port)) = socket.recv_from(buf) {
                let data = &buf[..len];

                // Check for beacon response first
                if furuno::is_beacon_response(data) {
                    match furuno::parse_beacon_response(data, &addr) {
                        Ok(discovery) => {
                            debug(&format!("Furuno beacon from {}: {:?}", addr, discovery.model));
                            discoveries.push(discovery);
                        }
                        Err(e) => {
                            debug(&format!("Furuno beacon parse error: {}", e));
                        }
                    }
                }
                // Check for model report (170 bytes)
                else if furuno::is_model_report(data) {
                    match furuno::parse_model_report(data) {
                        Ok((model, serial)) => {
                            debug(&format!(
                                "Furuno model report from {}: model={:?}, serial={:?}",
                                addr, model, serial
                            ));
                            // Store both model and serial
                            if model.is_some() || serial.is_some() {
                                model_reports.push((addr.clone(), model, serial));
                            }
                        }
                        Err(e) => {
                            debug(&format!("Furuno model report parse error: {}", e));
                        }
                    }
                }
            }
        }
    }

    /// Update a radar's model and serial number based on source address
    fn update_radar_model_info(&mut self, source_addr: &str, model: Option<&str>, serial: Option<&str>) {
        // Find radar by address (IP part only, without port)
        let source_ip = source_addr.split(':').next().unwrap_or(source_addr);

        for (_id, radar) in self.radars.iter_mut() {
            let radar_ip = radar.discovery.address.split(':').next().unwrap_or(&radar.discovery.address);

            if radar_ip == source_ip {
                // Update model if provided and different
                if let Some(m) = model {
                    if radar.discovery.model.is_none() || radar.discovery.model.as_deref() != Some(m) {
                        debug(&format!(
                            "Updating radar {} model: {:?} -> {}",
                            radar.discovery.name, radar.discovery.model, m
                        ));
                        radar.discovery.model = Some(m.to_string());
                    }
                }
                // Update serial number if provided and different
                if let Some(s) = serial {
                    if radar.discovery.serial_number.is_none() || radar.discovery.serial_number.as_deref() != Some(s) {
                        debug(&format!(
                            "Updating radar {} serial: {:?} -> {}",
                            radar.discovery.name, radar.discovery.serial_number, s
                        ));
                        radar.discovery.serial_number = Some(s.to_string());
                    }
                }
                return;
            }
        }

        // No radar found for this address - that's ok, model report may arrive before beacon
        debug(&format!("Model report for unknown radar at {}: model={:?}, serial={:?}", source_addr, model, serial));
    }

    /// Add a radar to the discovered list
    ///
    /// Returns true if this is a new radar.
    fn add_radar(&mut self, discovery: &RadarDiscovery) -> bool {
        let id = self.make_radar_id(discovery);

        if self.radars.contains_key(&id) {
            // Update last seen time
            if let Some(radar) = self.radars.get_mut(&id) {
                radar.last_seen_ms = self.current_time_ms;
            }
            false
        } else {
            debug(&format!(
                "Discovered {} radar: {} at {}",
                discovery.brand, discovery.name, discovery.address
            ));
            self.radars.insert(
                id,
                DiscoveredRadar {
                    discovery: discovery.clone(),
                    last_seen_ms: self.current_time_ms,
                },
            );
            true
        }
    }

    /// Generate a unique ID for a radar
    fn make_radar_id(&self, discovery: &RadarDiscovery) -> String {
        format!("{}-{}", discovery.brand, discovery.name)
    }

    /// Stop all locator sockets
    pub fn shutdown(&mut self) {
        self.furuno_socket = None;
        self.navico_br24_socket = None;
        self.navico_gen3_socket = None;
        self.raymarine_socket = None;
        self.garmin_socket = None;
    }
}

impl Default for RadarLocator {
    fn default() -> Self {
        Self::new()
    }
}

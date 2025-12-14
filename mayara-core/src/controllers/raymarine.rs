//! Raymarine Radar UDP Controller
//!
//! Platform-independent controller for Raymarine radars (Quantum and RD series)
//! using the [`IoProvider`] trait. All communication is via UDP.
//!
//! # Protocol
//!
//! Raymarine radars use UDP for all communication:
//! - Commands: Sent to radar's command address (from beacon)
//! - Reports: Received on report multicast address (from beacon)
//!
//! # Models
//!
//! | Series | Models | Spokes | Doppler |
//! |--------|--------|--------|---------|
//! | Quantum | Q24, Q24C, Q24D, Cyclone | 250 | Q24D, Cyclone |
//! | RD | RD418/424 HD, Magnum | 2048 | No |

use crate::io::{IoProvider, UdpSocketHandle};

/// Raymarine radar variant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaymarineVariant {
    /// Quantum series (solid-state CHIRP)
    Quantum,
    /// RD series (magnetron)
    RD,
}

impl Default for RaymarineVariant {
    fn default() -> Self {
        RaymarineVariant::Quantum
    }
}

/// Controller state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaymarineControllerState {
    /// Not initialized
    Disconnected,
    /// Sockets created, waiting for reports
    Listening,
    /// Receiving reports, ready for commands
    Connected,
}

/// Raymarine radar UDP controller
///
/// Manages UDP communication for Raymarine radars.
/// Handles both Quantum and RD variants with their different command formats.
pub struct RaymarineController {
    /// Radar ID (for logging)
    radar_id: String,
    /// Command address (from beacon)
    command_addr: String,
    command_port: u16,
    /// Report multicast address (from beacon)
    report_addr: String,
    report_port: u16,
    /// Command socket
    command_socket: Option<UdpSocketHandle>,
    /// Report socket
    report_socket: Option<UdpSocketHandle>,
    /// Current state
    state: RaymarineControllerState,
    /// Radar variant
    variant: RaymarineVariant,
    /// Poll count
    poll_count: u64,
    /// Has doppler capability
    has_doppler: bool,
}

impl RaymarineController {
    /// Create a new Raymarine controller
    pub fn new(
        radar_id: &str,
        command_addr: &str,
        command_port: u16,
        report_addr: &str,
        report_port: u16,
        variant: RaymarineVariant,
        has_doppler: bool,
    ) -> Self {
        Self {
            radar_id: radar_id.to_string(),
            command_addr: command_addr.to_string(),
            command_port,
            report_addr: report_addr.to_string(),
            report_port,
            command_socket: None,
            report_socket: None,
            state: RaymarineControllerState::Disconnected,
            variant,
            poll_count: 0,
            has_doppler,
        }
    }

    /// Get current state
    pub fn state(&self) -> RaymarineControllerState {
        self.state
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.state == RaymarineControllerState::Connected
    }

    /// Get radar variant
    pub fn variant(&self) -> RaymarineVariant {
        self.variant
    }

    /// Check if radar has doppler
    pub fn has_doppler(&self) -> bool {
        self.has_doppler
    }

    /// Poll the controller
    pub fn poll<I: IoProvider>(&mut self, io: &mut I) -> bool {
        self.poll_count += 1;

        match self.state {
            RaymarineControllerState::Disconnected => {
                self.start_sockets(io);
                true
            }
            RaymarineControllerState::Listening | RaymarineControllerState::Connected => {
                self.poll_connected(io)
            }
        }
    }

    fn start_sockets<I: IoProvider>(&mut self, io: &mut I) {
        // Create command socket
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, 0).is_ok() {
                    self.command_socket = Some(socket);
                    io.debug(&format!(
                        "[{}] Command socket created for {}:{}",
                        self.radar_id, self.command_addr, self.command_port
                    ));
                } else {
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("[{}] Failed to create command socket: {}", self.radar_id, e));
            }
        }

        // Create report socket
        match io.udp_create() {
            Ok(socket) => {
                if io.udp_bind(&socket, self.report_port).is_ok() {
                    if io.udp_join_multicast(&socket, &self.report_addr, "").is_ok() {
                        self.report_socket = Some(socket);
                        io.debug(&format!(
                            "[{}] Joined report multicast {}:{}",
                            self.radar_id, self.report_addr, self.report_port
                        ));
                        self.state = RaymarineControllerState::Listening;
                    } else {
                        io.debug(&format!("[{}] Failed to join report multicast", self.radar_id));
                        io.udp_close(socket);
                    }
                } else {
                    io.debug(&format!("[{}] Failed to bind report socket", self.radar_id));
                    io.udp_close(socket);
                }
            }
            Err(e) => {
                io.debug(&format!("[{}] Failed to create report socket: {}", self.radar_id, e));
            }
        }
    }

    fn poll_connected<I: IoProvider>(&mut self, io: &mut I) -> bool {
        let mut activity = false;

        // Process incoming reports
        if let Some(socket) = self.report_socket {
            let mut buf = [0u8; 2048];
            while let Some((len, _addr, _port)) = io.udp_recv_from(&socket, &mut buf) {
                self.process_report(io, &buf[..len]);
                activity = true;
                if self.state == RaymarineControllerState::Listening {
                    self.state = RaymarineControllerState::Connected;
                }
            }
        }

        activity
    }

    fn process_report<I: IoProvider>(&mut self, io: &I, data: &[u8]) {
        if data.len() < 4 {
            return;
        }

        // Report ID is first 4 bytes (little-endian)
        let report_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        io.debug(&format!(
            "[{}] Report ID: 0x{:08X}, len: {}",
            self.radar_id, report_id, data.len()
        ));

        // Quantum reports: 0x2800xx
        // RD reports: 0x0100xx or 0x0188xx
    }

    fn send_command<I: IoProvider>(&self, io: &mut I, data: &[u8]) {
        if let Some(socket) = self.command_socket {
            if let Err(e) = io.udp_send_to(&socket, data, &self.command_addr, self.command_port) {
                io.debug(&format!("[{}] Failed to send command: {}", self.radar_id, e));
            }
        }
    }

    // Quantum command builders
    fn quantum_command(&self, opcode: u16, data: &[u8]) -> Vec<u8> {
        let mut cmd = Vec::with_capacity(4 + data.len());
        cmd.extend_from_slice(&opcode.to_le_bytes());
        cmd.push(0x28);
        cmd.push(0x00);
        cmd.extend_from_slice(data);
        cmd
    }

    // RD command builders
    fn rd_command(&self, opcode: u16, data: &[u8]) -> Vec<u8> {
        let mut cmd = Vec::with_capacity(4 + data.len());
        cmd.extend_from_slice(&opcode.to_le_bytes());
        cmd.push(0x01);
        cmd.push(0x00);
        cmd.extend_from_slice(data);
        cmd
    }

    // Control methods

    /// Set power state (transmit/standby)
    pub fn set_power<I: IoProvider>(&mut self, io: &mut I, transmit: bool) {
        let value = if transmit { 0x01u8 } else { 0x00u8 };
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                self.quantum_command(0x0100, &[value, 0x00, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                self.rd_command(0x8101, &[0x01, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set power: {}", self.radar_id, transmit));
    }

    /// Set range index
    pub fn set_range<I: IoProvider>(&mut self, io: &mut I, range_index: u8) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                self.quantum_command(0x0101, &[0x00, range_index, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                self.rd_command(0x8101, &[0x01, 0x00, 0x00, 0x00, range_index, 0x00, 0x00, 0x00])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set range index: {}", self.radar_id, range_index));
    }

    /// Set gain (0-255)
    pub fn set_gain<I: IoProvider>(&mut self, io: &mut I, value: u8, auto: bool) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                let auto_byte = if auto { 0x01 } else { 0x00 };
                self.quantum_command(0x0106, &[auto_byte, value, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                // RD uses two commands for auto and value
                let auto_byte = if auto { 0x01 } else { 0x00 };
                self.rd_command(0x8301, &[
                    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    auto_byte, 0x00, 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);

        // For RD, send value separately if manual
        if self.variant == RaymarineVariant::RD && !auto {
            let value_cmd = self.rd_command(0x8301, &[
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                value, 0x00, 0x00, 0x00,
            ]);
            self.send_command(io, &value_cmd);
        }

        io.debug(&format!("[{}] Set gain: {} auto={}", self.radar_id, value, auto));
    }

    /// Set sea clutter (0-255)
    pub fn set_sea<I: IoProvider>(&mut self, io: &mut I, value: u8, auto: bool) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                let auto_byte = if auto { 0x01 } else { 0x00 };
                self.quantum_command(0x0107, &[auto_byte, value, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                let auto_byte = if auto { 0x01 } else { 0x00 };
                self.rd_command(0x8401, &[
                    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    auto_byte, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set sea: {} auto={}", self.radar_id, value, auto));
    }

    /// Set rain clutter (0-255)
    pub fn set_rain<I: IoProvider>(&mut self, io: &mut I, value: u8, enabled: bool) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                let enabled_byte = if enabled { 0x01 } else { 0x00 };
                self.quantum_command(0x0108, &[enabled_byte, value, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                let enabled_byte = if enabled { 0x01 } else { 0x00 };
                self.rd_command(0x8501, &[
                    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    enabled_byte, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set rain: {} enabled={}", self.radar_id, value, enabled));
    }

    /// Set interference rejection (0-3)
    pub fn set_interference_rejection<I: IoProvider>(&mut self, io: &mut I, level: u8) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                self.quantum_command(0x0109, &[level, 0x00, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                self.rd_command(0x8A01, &[
                    0x01, 0x00, 0x00, 0x00, level, 0x00, 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set IR: {}", self.radar_id, level));
    }

    /// Set target expansion (0-2)
    pub fn set_target_expansion<I: IoProvider>(&mut self, io: &mut I, level: u8) {
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                self.quantum_command(0x010A, &[level, 0x00, 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                self.rd_command(0x8901, &[
                    0x01, 0x00, 0x00, 0x00, level, 0x00, 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set target expansion: {}", self.radar_id, level));
    }

    /// Set bearing alignment in degrees (-180 to 180)
    pub fn set_bearing_alignment<I: IoProvider>(&mut self, io: &mut I, degrees: f32) {
        // Convert to wire format (varies by model)
        let wire_value = (degrees * 10.0) as i16;
        let cmd = match self.variant {
            RaymarineVariant::Quantum => {
                let bytes = wire_value.to_le_bytes();
                self.quantum_command(0x010B, &[bytes[0], bytes[1], 0x00, 0x00])
            }
            RaymarineVariant::RD => {
                let bytes = wire_value.to_le_bytes();
                self.rd_command(0x8B01, &[
                    0x01, 0x00, 0x00, 0x00, bytes[0], bytes[1], 0x00, 0x00,
                ])
            }
        };
        self.send_command(io, &cmd);
        io.debug(&format!("[{}] Set bearing alignment: {}", self.radar_id, degrees));
    }

    /// Set FTC (RD only, 0-255)
    pub fn set_ftc<I: IoProvider>(&mut self, io: &mut I, value: u8, enabled: bool) {
        if self.variant == RaymarineVariant::RD {
            let enabled_byte = if enabled { 0x01 } else { 0x00 };
            let cmd = self.rd_command(0x8601, &[
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                enabled_byte, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00,
            ]);
            self.send_command(io, &cmd);
            io.debug(&format!("[{}] Set FTC: {} enabled={}", self.radar_id, value, enabled));
        }
    }

    /// Set tune (RD only, 0-255)
    pub fn set_tune<I: IoProvider>(&mut self, io: &mut I, value: u8, auto: bool) {
        if self.variant == RaymarineVariant::RD {
            let auto_byte = if auto { 0x01 } else { 0x00 };
            let cmd = self.rd_command(0x8701, &[
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                auto_byte, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00,
            ]);
            self.send_command(io, &cmd);
            io.debug(&format!("[{}] Set tune: {} auto={}", self.radar_id, value, auto));
        }
    }

    /// Set mode (Quantum only, 0=Harbor, 1=Coastal, 2=Offshore, 3=Weather)
    pub fn set_mode<I: IoProvider>(&mut self, io: &mut I, mode: u8) {
        if self.variant == RaymarineVariant::Quantum {
            let cmd = self.quantum_command(0x010C, &[mode, 0x00, 0x00, 0x00]);
            self.send_command(io, &cmd);
            io.debug(&format!("[{}] Set mode: {}", self.radar_id, mode));
        }
    }

    /// Set color gain (Quantum only, 0-255)
    pub fn set_color_gain<I: IoProvider>(&mut self, io: &mut I, value: u8, auto: bool) {
        if self.variant == RaymarineVariant::Quantum {
            let auto_byte = if auto { 0x01 } else { 0x00 };
            let cmd = self.quantum_command(0x010D, &[auto_byte, value, 0x00, 0x00]);
            self.send_command(io, &cmd);
            io.debug(&format!("[{}] Set color gain: {} auto={}", self.radar_id, value, auto));
        }
    }

    /// Shutdown the controller
    pub fn shutdown<I: IoProvider>(&mut self, io: &mut I) {
        io.debug(&format!("[{}] Shutting down", self.radar_id));
        if let Some(socket) = self.command_socket.take() {
            io.udp_close(socket);
        }
        if let Some(socket) = self.report_socket.take() {
            io.udp_close(socket);
        }
        self.state = RaymarineControllerState::Disconnected;
    }
}

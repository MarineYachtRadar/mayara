//! Furuno Radar TCP Controller
//!
//! Handles the TCP connection to Furuno radars for control commands.
//! Implements the login sequence and command protocol.

use crate::signalk_ffi::{debug, TcpSocket};
use mayara_core::protocol::furuno::command::{
    format_antenna_height_command, format_auto_acquire_command, format_bird_mode_command,
    format_blind_sector_command, format_gain_command, format_heading_align_command,
    format_interference_rejection_command, format_keepalive, format_main_bang_command,
    format_noise_reduction_command, format_rain_command, format_range_command,
    format_request_modules, format_request_ontime, format_rezboost_command,
    format_scan_speed_command, format_sea_command, format_status_command,
    format_target_analyzer_command, format_tx_channel_command, meters_to_range_index,
    parse_login_response, parse_signal_processing_response, LOGIN_MESSAGE,
    // State request functions
    format_request_bird_mode, format_request_blind_sector, format_request_gain,
    format_request_interference_rejection, format_request_main_bang,
    format_request_noise_reduction, format_request_rain, format_request_range,
    format_request_rezboost, format_request_scan_speed, format_request_sea,
    format_request_status, format_request_target_analyzer, format_request_tx_channel,
};
use mayara_core::protocol::furuno::{BASE_PORT, BEACON_PORT};
use mayara_core::state::RadarState;

/// Controller state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    /// Not connected, needs login
    Disconnected,
    /// Sent login message, waiting for response
    LoggingIn,
    /// Got command port, connecting to it
    Connecting,
    /// Connected and ready for commands
    Connected,
    /// Trying fallback direct connection to command port
    TryingFallback,
}

/// Furuno radar TCP controller
///
/// Manages the TCP connection for sending commands to the radar.
/// Handles login, keep-alive, and command sending.
pub struct FurunoController {
    /// Radar ID (for logging)
    radar_id: String,
    /// Radar IP address
    radar_addr: String,
    /// Login socket (port 10000)
    login_socket: Option<TcpSocket>,
    /// Command socket (dynamic port)
    command_socket: Option<TcpSocket>,
    /// Current state
    state: ControllerState,
    /// Command port received from login
    command_port: u16,
    /// Last keep-alive time (poll count)
    last_keepalive: u32,
    /// Current poll count
    poll_count: u32,
    /// Pending command to send once connected
    pending_command: Option<String>,
    /// Retry count for connection attempts
    retry_count: u32,
    /// Poll count when last retry started (for backoff)
    last_retry_poll: u32,
    /// Index into login ports to try
    login_port_idx: usize,
    /// Index into fallback command ports to try
    fallback_port_idx: usize,
    /// Firmware version from $N96 response
    firmware_version: Option<String>,
    /// Radar model from $N96 response (e.g., "DRS4D-NXT")
    model: Option<String>,
    /// Operating hours from $N8E response
    operating_hours: Option<f64>,
    /// Whether info requests have been sent after connection
    info_requested: bool,
    /// Whether state requests have been sent after connection
    state_requested: bool,
    /// Current radar control state (gain, sea, rain, range, power)
    radar_state: RadarState,
}

impl FurunoController {
    /// Maximum number of connection retries
    const MAX_RETRIES: u32 = 5;
    /// Base delay between retries (in poll counts, ~100ms per poll)
    const RETRY_DELAY_BASE: u32 = 10;
    /// Login ports to try (some radars use 10000, others use 10010)
    const LOGIN_PORTS: [u16; 2] = [BEACON_PORT, BASE_PORT]; // Try 10010 first, then 10000
    /// Fallback command ports to try when login port is refused
    /// These are common command port offsets observed in the wild
    const FALLBACK_PORTS: [u16; 3] = [10100, 10001, 10002];

    /// Create a new controller for a Furuno radar
    ///
    /// The controller will automatically attempt to connect to get model info.
    pub fn new(radar_id: &str, radar_addr: &str) -> Self {
        debug(&format!(
            "FurunoController::new({}, {})",
            radar_id, radar_addr
        ));
        let mut controller = Self {
            radar_id: radar_id.to_string(),
            radar_addr: radar_addr.to_string(),
            login_socket: None,
            command_socket: None,
            state: ControllerState::Disconnected,
            command_port: 0,
            last_keepalive: 0,
            poll_count: 0,
            pending_command: None,
            retry_count: 0,
            last_retry_poll: 0,
            login_port_idx: 0,
            fallback_port_idx: 0,
            firmware_version: None,
            model: None,
            operating_hours: None,
            info_requested: false,
            state_requested: false,
            radar_state: RadarState::default(),
        };
        // Start connection immediately to get model/firmware info
        // Use keepalive as the "command" to trigger connection without changing radar state
        controller.request_info();
        controller
    }

    /// Request radar info by initiating a connection
    ///
    /// This queues a keepalive command to trigger the login sequence,
    /// which will then send info requests ($R96, $R8E) once connected.
    pub fn request_info(&mut self) {
        if self.state == ControllerState::Disconnected && self.pending_command.is_none() {
            debug(&format!("[{}] Initiating connection to get radar info", self.radar_id));
            // Queue a keepalive as the trigger command - harmless but establishes connection
            let cmd = format_keepalive();
            let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
            self.pending_command = Some(cmd.to_string());
        }
    }

    /// Get current state
    pub fn state(&self) -> ControllerState {
        self.state
    }

    /// Check if connected and ready for commands
    pub fn is_connected(&self) -> bool {
        self.state == ControllerState::Connected
    }

    /// Set radar to transmit
    pub fn set_transmit(&mut self, transmit: bool) {
        let cmd = format_status_command(transmit);
        // Remove trailing \r\n since send_line adds it
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing command: {}",
            self.radar_id,
            cmd
        ));

        if self.is_connected() {
            self.send_command(cmd);
        } else {
            // Queue the command and start connection
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Set radar range in meters
    pub fn set_range(&mut self, range_meters: u32) {
        let range_index = meters_to_range_index(range_meters as i32);
        let cmd = format_range_command(range_index);
        // Remove trailing \r\n since send_line adds it
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing range command: {} (index {} for {}m)",
            self.radar_id, cmd, range_index, range_meters
        ));

        if self.is_connected() {
            self.send_command(cmd);
        } else {
            // Queue the command and start connection
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Set radar gain
    ///
    /// # Arguments
    /// * `value` - Gain value (0-100)
    /// * `auto` - true for automatic gain control
    pub fn set_gain(&mut self, value: i32, auto: bool) {
        let cmd = format_gain_command(value, auto);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing gain command: {} (value={}, auto={})",
            self.radar_id, cmd, value, auto
        ));

        if self.is_connected() {
            self.send_command(cmd);
        } else {
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Set radar sea clutter
    ///
    /// # Arguments
    /// * `value` - Sea clutter value (0-100)
    /// * `auto` - true for automatic sea clutter control
    pub fn set_sea(&mut self, value: i32, auto: bool) {
        let cmd = format_sea_command(value, auto);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing sea command: {} (value={}, auto={})",
            self.radar_id, cmd, value, auto
        ));

        if self.is_connected() {
            self.send_command(cmd);
        } else {
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Set radar rain clutter
    ///
    /// # Arguments
    /// * `value` - Rain clutter value (0-100)
    /// * `auto` - true for automatic rain clutter control
    pub fn set_rain(&mut self, value: i32, auto: bool) {
        let cmd = format_rain_command(value, auto);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing rain command: {} (value={}, auto={})",
            self.radar_id, cmd, value, auto
        ));

        if self.is_connected() {
            self.send_command(cmd);
        } else {
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Set RezBoost (beam sharpening) level
    ///
    /// # Arguments
    /// * `level` - 0=Off, 1=On, 2=High (model dependent)
    pub fn set_rezboost(&mut self, level: u8) {
        let cmd = format_rezboost_command(level as i32, 0);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing rezboost command: {} (level={})",
            self.radar_id, cmd, level
        ));
        self.queue_command(cmd);
    }

    /// Set interference rejection
    ///
    /// # Arguments
    /// * `level` - 0=Off, 1=On
    pub fn set_interference_rejection(&mut self, level: u8) {
        let cmd = format_interference_rejection_command(level > 0);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing IR command: {} (level={})",
            self.radar_id, cmd, level
        ));
        self.queue_command(cmd);
    }

    /// Set scan speed
    ///
    /// # Arguments
    /// * `speed` - 0=Normal, 1=Fast
    pub fn set_scan_speed(&mut self, speed: u8) {
        let cmd = format_scan_speed_command(speed as i32);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing scan speed command: {} (speed={})",
            self.radar_id, cmd, speed
        ));
        self.queue_command(cmd);
    }

    /// Set bird mode
    ///
    /// # Arguments
    /// * `level` - 0=OFF, 1=Low, 2=Medium, 3=High
    pub fn set_bird_mode(&mut self, level: u8) {
        let cmd = format_bird_mode_command(level as i32, 0);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing bird mode command: {} (level={})",
            self.radar_id, cmd, level
        ));
        self.queue_command(cmd);
    }

    /// Set target analyzer (Doppler mode)
    ///
    /// # Arguments
    /// * `enabled` - true to enable target analyzer
    /// * `mode` - 0=Target mode (highlights collision threats), 1=Rain mode (identifies precipitation)
    pub fn set_target_analyzer(&mut self, enabled: bool, mode: u8) {
        let cmd = format_target_analyzer_command(enabled, mode as i32, 0);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing target analyzer command: {} (enabled={}, mode={})",
            self.radar_id, cmd, enabled, mode
        ));
        self.queue_command(cmd);
    }

    /// Set bearing alignment (heading offset)
    ///
    /// # Arguments
    /// * `degrees` - Offset in degrees (-180 to 180)
    pub fn set_bearing_alignment(&mut self, degrees: f64) {
        // Convert to tenths of a degree for the protocol
        let degrees_x10 = (degrees * 10.0) as i32;
        let cmd = format_heading_align_command(degrees_x10);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing bearing alignment command: {} (degrees={})",
            self.radar_id, cmd, degrees
        ));
        self.queue_command(cmd);
    }

    /// Set noise reduction
    ///
    /// # Arguments
    /// * `enabled` - true to enable noise reduction
    pub fn set_noise_reduction(&mut self, enabled: bool) {
        let cmd = format_noise_reduction_command(enabled);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing noise reduction command: {} (enabled={})",
            self.radar_id, cmd, enabled
        ));
        self.queue_command(cmd);
    }

    /// Set main bang suppression
    ///
    /// # Arguments
    /// * `percent` - Suppression level (0-100%)
    pub fn set_main_bang_suppression(&mut self, percent: u8) {
        let cmd = format_main_bang_command(percent as i32);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing main bang suppression command: {} (percent={})",
            self.radar_id, cmd, percent
        ));
        self.queue_command(cmd);
    }

    /// Set TX channel
    ///
    /// # Arguments
    /// * `channel` - 0=Auto, 1-3=Channel 1-3
    pub fn set_tx_channel(&mut self, channel: u8) {
        let cmd = format_tx_channel_command(channel as i32);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing TX channel command: {} (channel={})",
            self.radar_id, cmd, channel
        ));
        self.queue_command(cmd);
    }

    /// Set auto acquire (ARPA by Doppler)
    ///
    /// # Arguments
    /// * `enabled` - true to enable auto acquire
    pub fn set_auto_acquire(&mut self, enabled: bool) {
        let cmd = format_auto_acquire_command(enabled);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing auto acquire command: {} (enabled={})",
            self.radar_id, cmd, enabled
        ));
        self.queue_command(cmd);
    }

    /// Set antenna height
    ///
    /// # Arguments
    /// * `meters` - Antenna height in meters (0-100)
    pub fn set_antenna_height(&mut self, meters: i32) {
        let cmd = format_antenna_height_command(meters);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing antenna height command: {} (meters={})",
            self.radar_id, cmd, meters
        ));
        self.queue_command(cmd);
    }

    /// Set blind sector (no-transmit zones)
    ///
    /// # Arguments
    /// * `zone1_enabled` - Enable zone 1
    /// * `zone1_start` - Zone 1 start angle (0-359)
    /// * `zone1_end` - Zone 1 end angle (0-359)
    /// * `zone2_enabled` - Enable zone 2
    /// * `zone2_start` - Zone 2 start angle (0-359)
    /// * `zone2_end` - Zone 2 end angle (0-359)
    pub fn set_blind_sector(
        &mut self,
        zone1_enabled: bool,
        zone1_start: i32,
        zone1_end: i32,
        zone2_enabled: bool,
        zone2_start: i32,
        zone2_end: i32,
    ) {
        // Convert start/end to start/width for the protocol
        let z1_width = if zone1_enabled {
            ((zone1_end - zone1_start + 360) % 360).max(1)
        } else {
            0
        };
        let z2_width = if zone2_enabled {
            ((zone2_end - zone2_start + 360) % 360).max(1)
        } else {
            0
        };
        let cmd = format_blind_sector_command(zone2_enabled, zone1_start, z1_width, zone2_start, z2_width);
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        debug(&format!(
            "[{}] Queueing blind sector command: {} (z1: {}-{} enabled={}, z2: {}-{} enabled={})",
            self.radar_id, cmd, zone1_start, zone1_end, zone1_enabled, zone2_start, zone2_end, zone2_enabled
        ));
        self.queue_command(cmd);
    }

    /// Helper to queue a command and start connection if needed
    fn queue_command(&mut self, cmd: &str) {
        if self.is_connected() {
            self.send_command(cmd);
        } else {
            self.pending_command = Some(cmd.to_string());
            if self.state == ControllerState::Disconnected {
                self.start_login();
            }
        }
    }

    /// Poll the controller - call this regularly from the main poll loop
    ///
    /// Returns true if there's activity, false if idle.
    pub fn poll(&mut self) -> bool {
        self.poll_count += 1;

        match self.state {
            ControllerState::Disconnected => {
                // If we have a pending command, start the login process
                if self.pending_command.is_some() {
                    // Check if we should retry (with exponential backoff)
                    if self.retry_count > 0 {
                        let delay = Self::RETRY_DELAY_BASE * (1 << self.retry_count.min(4));
                        let elapsed = self.poll_count - self.last_retry_poll;
                        if elapsed < delay {
                            return false; // Wait for backoff
                        }
                        if self.retry_count >= Self::MAX_RETRIES {
                            debug(&format!(
                                "[{}] Max retries ({}) reached, giving up",
                                self.radar_id, Self::MAX_RETRIES
                            ));
                            self.pending_command = None;
                            self.retry_count = 0;
                            return false;
                        }
                        debug(&format!(
                            "[{}] Retry {} of {} (after {} polls)",
                            self.radar_id, self.retry_count + 1, Self::MAX_RETRIES, elapsed
                        ));
                    }
                    self.start_login();
                    true
                } else {
                    false
                }
            }
            ControllerState::LoggingIn => {
                self.poll_login()
            }
            ControllerState::Connecting => {
                self.poll_connecting()
            }
            ControllerState::Connected => {
                self.poll_connected()
            }
            ControllerState::TryingFallback => {
                self.poll_fallback()
            }
        }
    }

    /// Start the login process - try each login port in sequence
    fn start_login(&mut self) {
        if self.login_port_idx >= Self::LOGIN_PORTS.len() {
            // All login ports exhausted, try fallback command ports
            debug(&format!(
                "[{}] All login ports exhausted, trying fallback ports",
                self.radar_id
            ));
            self.login_port_idx = 0;
            self.start_fallback_connection();
            return;
        }

        let login_port = Self::LOGIN_PORTS[self.login_port_idx];
        debug(&format!(
            "[{}] Starting login to {}:{} (port idx {})",
            self.radar_id, self.radar_addr, login_port, self.login_port_idx
        ));

        // Create login socket in raw mode (binary response)
        match TcpSocket::new() {
            Ok(socket) => {
                // Set to raw mode for binary login response
                let _ = socket.set_line_buffering(false);

                if socket.connect(&self.radar_addr, login_port).is_ok() {
                    self.login_socket = Some(socket);
                    self.state = ControllerState::LoggingIn;
                    debug(&format!("[{}] Login connection initiated to port {}", self.radar_id, login_port));
                } else {
                    debug(&format!("[{}] Failed to initiate login connection to port {}", self.radar_id, login_port));
                    // Try next login port
                    self.login_port_idx += 1;
                    self.start_login();
                }
            }
            Err(e) => {
                debug(&format!("[{}] Failed to create login socket: {}", self.radar_id, e));
                // Try next login port
                self.login_port_idx += 1;
                self.start_login();
            }
        }
    }

    /// Poll during login state
    fn poll_login(&mut self) -> bool {
        let socket = match &self.login_socket {
            Some(s) => s,
            None => {
                debug(&format!("[{}] poll_login: no socket, going disconnected", self.radar_id));
                self.state = ControllerState::Disconnected;
                return false;
            }
        };

        // Check if socket was closed/errored
        let valid = socket.is_valid();
        let connected = socket.is_connected();
        debug(&format!("[{}] poll_login: valid={}, connected={}", self.radar_id, valid, connected));

        if !valid {
            debug(&format!("[{}] Login socket closed/errored on login port idx {}", self.radar_id, self.login_port_idx));
            self.login_socket = None;
            // Try next login port
            self.login_port_idx += 1;
            self.start_login();
            return true;
        }

        // Wait for connection
        if !socket.is_connected() {
            return true; // Still connecting
        }

        // Send login message
        debug(&format!("[{}] Sending login message", self.radar_id));
        if socket.send(&LOGIN_MESSAGE).is_err() {
            debug(&format!("[{}] Failed to send login message", self.radar_id));
            self.disconnect();
            return false;
        }

        // Check for response
        let mut buf = [0u8; 64];
        if let Some(len) = socket.recv_raw(&mut buf) {
            debug(&format!(
                "[{}] Login response: {} bytes",
                self.radar_id, len
            ));

            if let Some(port) = parse_login_response(&buf[..len]) {
                debug(&format!(
                    "[{}] Got command port: {}",
                    self.radar_id, port
                ));
                self.command_port = port;
                self.login_socket = None; // Close login socket
                self.start_command_connection();
            } else {
                debug(&format!("[{}] Invalid login response", self.radar_id));
                self.disconnect();
            }
        }

        true
    }

    /// Start connection to command port
    fn start_command_connection(&mut self) {
        debug(&format!(
            "[{}] Connecting to command port {}",
            self.radar_id, self.command_port
        ));

        match TcpSocket::new() {
            Ok(socket) => {
                // Command socket uses line buffering (text protocol)
                let _ = socket.set_line_buffering(true);

                if socket.connect(&self.radar_addr, self.command_port).is_ok() {
                    self.command_socket = Some(socket);
                    self.state = ControllerState::Connecting;
                } else {
                    debug(&format!("[{}] Failed to connect to command port", self.radar_id));
                    self.state = ControllerState::Disconnected;
                }
            }
            Err(e) => {
                debug(&format!("[{}] Failed to create command socket: {}", self.radar_id, e));
                self.state = ControllerState::Disconnected;
            }
        }
    }

    /// Poll during connecting state
    fn poll_connecting(&mut self) -> bool {
        let socket = match &self.command_socket {
            Some(s) => s,
            None => {
                self.state = ControllerState::Disconnected;
                return false;
            }
        };

        // Check if socket was closed/errored
        if !socket.is_valid() {
            debug(&format!("[{}] Command socket closed/errored", self.radar_id));
            self.command_socket = None;
            self.state = ControllerState::Disconnected;
            self.retry_count += 1;
            self.last_retry_poll = self.poll_count;
            return false;
        }

        if socket.is_connected() {
            debug(&format!("[{}] Command connection established", self.radar_id));
            self.state = ControllerState::Connected;
            self.last_keepalive = self.poll_count;
            self.retry_count = 0; // Reset retry count on successful connection
            self.login_port_idx = 0; // Reset login port index for next time

            // Send any pending command
            if let Some(cmd) = self.pending_command.take() {
                self.send_command(&cmd);
            }
        }

        true
    }

    /// Poll while connected
    fn poll_connected(&mut self) -> bool {
        let socket = match &self.command_socket {
            Some(s) => s,
            None => {
                self.state = ControllerState::Disconnected;
                return false;
            }
        };

        // Check connection is still alive
        if !socket.is_connected() {
            debug(&format!("[{}] Command connection lost", self.radar_id));
            self.disconnect();
            return false;
        }

        // Send info requests once after connecting
        if !self.info_requested {
            self.info_requested = true;
            self.send_info_requests();
        }

        // Send state requests once after connecting
        if !self.state_requested {
            self.state_requested = true;
            self.send_state_requests();
        }

        // Collect responses first to avoid borrow conflicts
        let mut responses = Vec::new();
        while let Some(line) = socket.recv_line_string() {
            debug(&format!("[{}] Response: {}", self.radar_id, line));
            responses.push(line);
        }

        // Process collected responses
        for line in responses {
            self.parse_response(&line);
        }

        // Send keep-alive every ~5 seconds (assuming 10 polls/sec = 50 polls)
        // Adjust based on actual poll rate
        const KEEPALIVE_INTERVAL: u32 = 50;
        if self.poll_count - self.last_keepalive > KEEPALIVE_INTERVAL {
            self.send_keepalive();
            self.last_keepalive = self.poll_count;
        }

        true
    }

    /// Send a command to the radar
    fn send_command(&self, cmd: &str) {
        if let Some(socket) = &self.command_socket {
            debug(&format!("[{}] Sending: {}", self.radar_id, cmd));
            if socket.send_line(cmd).is_err() {
                debug(&format!("[{}] Failed to send command", self.radar_id));
            }
        }
    }

    /// Send keep-alive message
    fn send_keepalive(&self) {
        let cmd = format_keepalive();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);
    }

    /// Send info requests (firmware version, operating hours)
    fn send_info_requests(&self) {
        // Request module/firmware info ($R96)
        let cmd = format_request_modules();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request operating hours ($R8E,0,0)
        let cmd = format_request_ontime();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        debug(&format!("[{}] Sent info requests", self.radar_id));
    }

    /// Send state requests (all controls that support querying)
    fn send_state_requests(&self) {
        // Base controls
        // Request status ($R69)
        let cmd = format_request_status();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request range ($R62)
        let cmd = format_request_range();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request gain ($R63)
        let cmd = format_request_gain();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request sea ($R64)
        let cmd = format_request_sea();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request rain ($R65)
        let cmd = format_request_rain();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request signal processing - query each feature separately
        // Noise reduction ($R67,0,3)
        let cmd = format_request_noise_reduction();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Interference rejection ($R67,0,0)
        let cmd = format_request_interference_rejection();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Extended controls
        // Request RezBoost ($REE)
        let cmd = format_request_rezboost();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request Bird Mode ($RED)
        let cmd = format_request_bird_mode();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request Target Analyzer ($REF)
        let cmd = format_request_target_analyzer();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request Scan Speed ($R89)
        let cmd = format_request_scan_speed();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request Main Bang Suppression ($R83)
        let cmd = format_request_main_bang();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request TX Channel ($REC)
        let cmd = format_request_tx_channel();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        // Request Blind Sector / No-Transmit Zones ($R77)
        let cmd = format_request_blind_sector();
        let cmd = cmd.trim_end_matches('\n').trim_end_matches('\r');
        self.send_command(cmd);

        debug(&format!("[{}] Sent state requests (base + extended)", self.radar_id));
    }

    /// Parse a response line from the radar
    fn parse_response(&mut self, line: &str) {
        // Debug: Log $N67 responses specifically with full parsing details
        if line.starts_with("$N67") {
            let parse_result = parse_signal_processing_response(line);
            debug(&format!(
                "[{}] $N67 response: '{}' (len={}, bytes={:?}) -> parse_result={:?}",
                self.radar_id,
                line,
                line.len(),
                line.as_bytes(),
                parse_result
            ));
        }

        // Try to update radar state from control responses ($N62-$N69, $N67, $NEE, $NED, $NEF, $N89, $N83, $NEC)
        if self.radar_state.update_from_response(line) {
            debug(&format!(
                "[{}] State updated: power={:?}, range={}, gain={}/{}, sea={}/{}, rain={}/{}, nr={}, ir={}, rezboost={}, bird={}, doppler={}/{}, scan={}, mbs={}, txch={}",
                self.radar_id,
                self.radar_state.power,
                self.radar_state.range,
                self.radar_state.gain.mode,
                self.radar_state.gain.value,
                self.radar_state.sea.mode,
                self.radar_state.sea.value,
                self.radar_state.rain.mode,
                self.radar_state.rain.value,
                self.radar_state.noise_reduction,
                self.radar_state.interference_rejection,
                self.radar_state.beam_sharpening,
                self.radar_state.bird_mode,
                self.radar_state.doppler_mode.enabled,
                self.radar_state.doppler_mode.mode,
                self.radar_state.scan_speed,
                self.radar_state.main_bang_suppression,
                self.radar_state.tx_channel
            ));
            return;
        } else if line.starts_with("$N67") {
            debug(&format!(
                "[{}] $N67 NOT parsed - update_from_response returned false",
                self.radar_id
            ));
        }

        // Parse $N96 - Module/firmware response
        // Format: $N96,part-version,part-version,...
        // Example: $N96,0359360-01.05,0330920-02.01,...
        // The first part number (0359360) identifies the radar model
        if line.starts_with("$N96,") {
            // Extract first part-version pair
            let parts: Vec<&str> = line[5..].split(',').collect();
            if let Some(first) = parts.first() {
                // Extract part code and version
                if let Some(idx) = first.find('-') {
                    let part_code = &first[..idx];
                    let version = &first[idx + 1..];
                    debug(&format!("[{}] Firmware version: {}", self.radar_id, version));
                    self.firmware_version = Some(version.to_string());

                    // Map part code to model name
                    // Based on TimeZero Fec.Wrapper.SensorProperty.GetRadarSensorType
                    let model = match part_code {
                        "0359235" => Some("DRS"),
                        "0359255" => Some("FAR-1417"),
                        "0359204" => Some("FAR-2117"),
                        "0359321" => Some("FAR-1417"),
                        "0359338" => Some("DRS4D"),
                        "0359367" => Some("DRS4D"),
                        "0359281" => Some("FAR-3000"),
                        "0359286" => Some("FAR-3000"),
                        "0359477" => Some("FAR-3000"),
                        "0359360" => Some("DRS4D-NXT"),
                        "0359421" => Some("DRS6A-NXT"),
                        "0359355" => Some("DRS6A-X"),
                        "0359344" => Some("FAR-1513"),
                        "0359397" => Some("FAR-1416"),
                        _ => None,
                    };

                    if let Some(m) = model {
                        debug(&format!("[{}] Model identified: {} (part code {})", self.radar_id, m, part_code));
                        self.model = Some(m.to_string());
                    } else {
                        debug(&format!("[{}] Unknown part code: {}", self.radar_id, part_code));
                    }
                }
            }
        }
        // Parse $N8E - Operating hours response
        // Format: $N8E,seconds
        // Example: $N8E,4442123
        else if line.starts_with("$N8E,") {
            if let Ok(seconds) = line[5..].trim().parse::<u64>() {
                let hours = seconds as f64 / 3600.0;
                debug(&format!(
                    "[{}] Operating hours: {:.1} ({} seconds)",
                    self.radar_id, hours, seconds
                ));
                self.operating_hours = Some(hours);
            }
        }
    }

    /// Get firmware version (if available)
    pub fn firmware_version(&self) -> Option<&str> {
        self.firmware_version.as_deref()
    }

    /// Get radar model (if available)
    /// Returns the model string (e.g., "DRS4D-NXT") identified from the TCP $N96 response
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Get operating hours (if available)
    pub fn operating_hours(&self) -> Option<f64> {
        self.operating_hours
    }

    /// Get current radar state (gain, sea, rain, range, power)
    ///
    /// Returns the cached state that was populated from $N responses.
    /// State is automatically queried when the controller connects.
    pub fn radar_state(&self) -> &RadarState {
        &self.radar_state
    }

    /// Start trying fallback command ports directly (skip login)
    fn start_fallback_connection(&mut self) {
        if self.fallback_port_idx >= Self::FALLBACK_PORTS.len() {
            debug(&format!(
                "[{}] All fallback ports exhausted, giving up",
                self.radar_id
            ));
            self.fallback_port_idx = 0;
            self.pending_command = None;
            self.state = ControllerState::Disconnected;
            return;
        }

        let port = Self::FALLBACK_PORTS[self.fallback_port_idx];
        debug(&format!(
            "[{}] Trying fallback port {} (idx {})",
            self.radar_id, port, self.fallback_port_idx
        ));

        match TcpSocket::new() {
            Ok(socket) => {
                // Command socket uses line buffering (text protocol)
                let _ = socket.set_line_buffering(true);

                if socket.connect(&self.radar_addr, port).is_ok() {
                    self.command_port = port;
                    // Try to send command immediately on connection
                    // before the radar has a chance to close it
                    if let Some(cmd) = &self.pending_command {
                        debug(&format!(
                            "[{}] Sending command immediately on fallback connect: {}",
                            self.radar_id, cmd
                        ));
                        let _ = socket.send_line(cmd);
                    }
                    self.command_socket = Some(socket);
                    self.state = ControllerState::TryingFallback;
                } else {
                    debug(&format!("[{}] Failed to initiate fallback connection", self.radar_id));
                    self.fallback_port_idx += 1;
                    self.start_fallback_connection(); // Try next port
                }
            }
            Err(e) => {
                debug(&format!("[{}] Failed to create fallback socket: {}", self.radar_id, e));
                self.fallback_port_idx += 1;
                self.start_fallback_connection(); // Try next port
            }
        }
    }

    /// Poll during fallback connection attempt
    fn poll_fallback(&mut self) -> bool {
        let socket = match &self.command_socket {
            Some(s) => s,
            None => {
                self.fallback_port_idx += 1;
                self.start_fallback_connection();
                return true;
            }
        };

        // Check if socket was closed/errored
        if !socket.is_valid() {
            debug(&format!(
                "[{}] Fallback port {} failed, trying next",
                self.radar_id, self.command_port
            ));
            self.command_socket = None;
            self.fallback_port_idx += 1;
            self.start_fallback_connection();
            return true;
        }

        if socket.is_connected() {
            debug(&format!(
                "[{}] Fallback connection to port {} succeeded!",
                self.radar_id, self.command_port
            ));
            self.state = ControllerState::Connected;
            self.last_keepalive = self.poll_count;
            self.retry_count = 0;
            self.login_port_idx = 0;
            self.fallback_port_idx = 0;

            // Send any pending command
            if let Some(cmd) = self.pending_command.take() {
                self.send_command(&cmd);
            }
        }

        true
    }

    /// Disconnect and reset state
    pub fn disconnect(&mut self) {
        debug(&format!("[{}] Disconnecting", self.radar_id));
        self.login_socket = None;
        self.command_socket = None;
        self.state = ControllerState::Disconnected;
        self.command_port = 0;
        self.info_requested = false; // Re-request info on next connection
        self.state_requested = false; // Re-request state on next connection
    }
}

impl Drop for FurunoController {
    fn drop(&mut self) {
        self.disconnect();
    }
}

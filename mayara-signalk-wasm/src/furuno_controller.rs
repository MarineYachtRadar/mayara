//! Furuno Radar TCP Controller
//!
//! Handles the TCP connection to Furuno radars for control commands.
//! Implements the login sequence and command protocol.

use crate::signalk_ffi::{debug, TcpSocket};
use mayara_core::protocol::furuno::command::{
    format_keepalive, format_status_command, parse_login_response, LOGIN_MESSAGE,
};
use mayara_core::protocol::furuno::{BASE_PORT, BEACON_PORT};

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
    pub fn new(radar_id: &str, radar_addr: &str) -> Self {
        debug(&format!(
            "FurunoController::new({}, {})",
            radar_id, radar_addr
        ));
        Self {
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

        // Process any responses
        while let Some(line) = socket.recv_line_string() {
            debug(&format!("[{}] Response: {}", self.radar_id, line));
            // We could parse responses here if needed
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
    }
}

impl Drop for FurunoController {
    fn drop(&mut self) {
        self.disconnect();
    }
}

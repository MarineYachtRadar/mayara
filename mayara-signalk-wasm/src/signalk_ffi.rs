//! SignalK FFI bindings
//!
//! These are the host functions provided by SignalK for WASM plugins.
//! They provide socket I/O, logging, and delta emission.
//!
//! Based on SignalK WASM Plugin Developer Guide.

#![allow(dead_code)] // Some FFI functions are not used yet but will be needed for commands

// =============================================================================
// External host functions (provided by SignalK runtime)
// =============================================================================

#[link(wasm_import_module = "env")]
extern "C" {
    // Logging and status functions
    fn sk_debug(ptr: *const u8, len: usize);
    fn sk_set_status(ptr: *const u8, len: usize);
    fn sk_set_error(ptr: *const u8, len: usize);

    // SignalK delta emission
    fn sk_handle_message(ptr: *const u8, len: usize);

    // Radar provider registration
    fn sk_register_radar_provider(name_ptr: *const u8, name_len: usize) -> i32;

    // Radar spoke streaming (streams binary data to connected WebSocket clients)
    fn sk_radar_emit_spokes(
        radar_id_ptr: *const u8,
        radar_id_len: usize,
        spoke_data_ptr: *const u8,
        spoke_data_len: usize,
    ) -> i32;

    // UDP Socket functions (rawSockets capability)
    fn sk_udp_create(socket_type: i32) -> i32;
    fn sk_udp_bind(socket_id: i32, port: u16) -> i32;
    fn sk_udp_join_multicast(
        socket_id: i32,
        group_ptr: *const u8,
        group_len: usize,
        iface_ptr: *const u8,
        iface_len: usize,
    ) -> i32;
    fn sk_udp_recv(
        socket_id: i32,
        buf_ptr: *mut u8,
        buf_max_len: usize,
        addr_out_ptr: *mut u8,
        port_out_ptr: *mut u16,
    ) -> i32;
    fn sk_udp_send(
        socket_id: i32,
        addr_ptr: *const u8,
        addr_len: usize,
        port: u16,
        data_ptr: *const u8,
        data_len: usize,
    ) -> i32;
    fn sk_udp_close(socket_id: i32);
    fn sk_udp_pending(socket_id: i32) -> i32;
    fn sk_udp_set_broadcast(socket_id: i32, enabled: i32) -> i32;

    // Plugin configuration
    fn sk_read_config(buf_ptr: *mut u8, buf_max_len: usize) -> i32;
    fn sk_save_config(config_ptr: *const u8, config_len: usize) -> i32;

    // TCP Socket functions (rawSockets capability)
    fn sk_tcp_create() -> i32;
    fn sk_tcp_connect(
        socket_id: i32,
        addr_ptr: *const u8,
        addr_len: usize,
        port: u16,
    ) -> i32;
    fn sk_tcp_connected(socket_id: i32) -> i32;
    fn sk_tcp_set_line_buffering(socket_id: i32, line_buffering: i32) -> i32;
    fn sk_tcp_send(socket_id: i32, data_ptr: *const u8, data_len: usize) -> i32;
    fn sk_tcp_recv_line(
        socket_id: i32,
        buf_ptr: *mut u8,
        buf_max_len: usize,
    ) -> i32;
    fn sk_tcp_recv_raw(
        socket_id: i32,
        buf_ptr: *mut u8,
        buf_max_len: usize,
    ) -> i32;
    fn sk_tcp_pending(socket_id: i32) -> i32;
    fn sk_tcp_close(socket_id: i32);
}

// =============================================================================
// Safe Rust wrappers for logging
// =============================================================================

/// Log a debug message
pub fn debug(msg: &str) {
    unsafe {
        sk_debug(msg.as_ptr(), msg.len());
    }
}

/// Set plugin status message
pub fn set_status(msg: &str) {
    unsafe {
        sk_set_status(msg.as_ptr(), msg.len());
    }
}

/// Set plugin error message
pub fn set_error(msg: &str) {
    unsafe {
        sk_set_error(msg.as_ptr(), msg.len());
    }
}

// Convenience aliases for compatibility
pub fn sk_info(msg: &str) {
    debug(msg);
}

pub fn sk_debug_log(msg: &str) {
    debug(msg);
}

pub fn sk_warn(msg: &str) {
    set_error(msg);
}

// =============================================================================
// UDP Socket wrapper
// =============================================================================

/// A UDP socket wrapper that uses SignalK's host socket implementation
pub struct UdpSocket {
    id: i32,
}

impl UdpSocket {
    /// Create a new UDP socket (IPv4)
    pub fn new() -> Result<Self, i32> {
        let id = unsafe { sk_udp_create(0) }; // 0 = udp4
        if id < 0 {
            Err(id)
        } else {
            Ok(Self { id })
        }
    }

    /// Create and bind a UDP socket to an address:port string
    ///
    /// Address format: "ip:port" (e.g., "0.0.0.0:0")
    pub fn bind(addr: &str) -> Result<Self, i32> {
        let socket = Self::new()?;
        let port = addr.rsplit(':').next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        socket.bind_port(port)?;
        Ok(socket)
    }

    /// Bind the socket to a port
    ///
    /// Use 0 for any available port.
    pub fn bind_port(&self, port: u16) -> Result<(), i32> {
        let result = unsafe { sk_udp_bind(self.id, port) };
        if result < 0 {
            Err(result)
        } else {
            Ok(())
        }
    }

    /// Enable or disable broadcast mode
    ///
    /// Must be enabled before sending to broadcast addresses.
    pub fn set_broadcast(&self, enabled: bool) -> Result<(), i32> {
        let result = unsafe { sk_udp_set_broadcast(self.id, if enabled { 1 } else { 0 }) };
        if result < 0 {
            Err(result)
        } else {
            Ok(())
        }
    }

    /// Join a multicast group on a specific interface
    ///
    /// Use empty string for interface to use default.
    pub fn join_multicast(&self, group: &str) -> Result<(), i32> {
        self.join_multicast_on_interface(group, "")
    }

    /// Join a multicast group on a specific interface
    pub fn join_multicast_on_interface(&self, group: &str, interface: &str) -> Result<(), i32> {
        let result = unsafe {
            sk_udp_join_multicast(
                self.id,
                group.as_ptr(),
                group.len(),
                interface.as_ptr(),
                interface.len(),
            )
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(())
        }
    }

    /// Send data to a specific address
    pub fn send_to(&self, data: &[u8], addr: &str, port: u16) -> Result<usize, i32> {
        let result = unsafe {
            sk_udp_send(
                self.id,
                addr.as_ptr(),
                addr.len(),
                port,
                data.as_ptr(),
                data.len(),
            )
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(result as usize)
        }
    }

    /// Check if there's data available to receive
    pub fn pending(&self) -> i32 {
        unsafe { sk_udp_pending(self.id) }
    }

    /// Receive data (non-blocking)
    ///
    /// Returns None if no data is available, or Some((len, addr, port)) on success.
    pub fn recv_from(&self, buf: &mut [u8]) -> Option<(usize, String, u16)> {
        let mut addr_buf = [0u8; 64];
        let mut port: u16 = 0;

        let result = unsafe {
            sk_udp_recv(
                self.id,
                buf.as_mut_ptr(),
                buf.len(),
                addr_buf.as_mut_ptr(),
                &mut port,
            )
        };

        if result <= 0 {
            None
        } else {
            // Find null terminator or use full length
            let addr_len = addr_buf.iter().position(|&b| b == 0).unwrap_or(addr_buf.len());
            let addr = String::from_utf8_lossy(&addr_buf[..addr_len]).to_string();
            Some((result as usize, addr, port))
        }
    }

    /// Close the socket
    pub fn close(&mut self) {
        if self.id >= 0 {
            unsafe { sk_udp_close(self.id) };
            self.id = -1;
        }
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        self.close();
    }
}

// =============================================================================
// TCP Socket wrapper
// =============================================================================

/// A TCP socket wrapper that uses SignalK's host socket implementation
///
/// Supports both line-buffered (text protocol) and raw (binary) modes.
/// Default is line-buffered mode, suitable for protocols like Furuno that
/// use \r\n terminated commands.
pub struct TcpSocket {
    id: i32,
}

impl TcpSocket {
    /// Create a new TCP socket
    pub fn new() -> Result<Self, i32> {
        let id = unsafe { sk_tcp_create() };
        if id < 0 {
            Err(id)
        } else {
            Ok(Self { id })
        }
    }

    /// Connect to a remote host
    ///
    /// This initiates a connection asynchronously. Use `is_connected()` to check status.
    pub fn connect(&self, addr: &str, port: u16) -> Result<(), i32> {
        let result = unsafe {
            sk_tcp_connect(self.id, addr.as_ptr(), addr.len(), port)
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(())
        }
    }

    /// Check if the socket is connected
    pub fn is_connected(&self) -> bool {
        unsafe { sk_tcp_connected(self.id) == 1 }
    }

    /// Check if the socket exists (not closed/errored)
    /// Returns false if the socket was closed due to error
    pub fn is_valid(&self) -> bool {
        unsafe { sk_tcp_connected(self.id) >= 0 }
    }

    /// Set buffering mode
    ///
    /// * `true` - Line-buffered mode (default): receives complete \r\n or \n terminated lines
    /// * `false` - Raw mode: receives binary data chunks as they arrive
    pub fn set_line_buffering(&self, enabled: bool) -> Result<(), i32> {
        let result = unsafe {
            sk_tcp_set_line_buffering(self.id, if enabled { 1 } else { 0 })
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(())
        }
    }

    /// Send data over the connection
    pub fn send(&self, data: &[u8]) -> Result<usize, i32> {
        if !self.is_connected() {
            return Err(-1);
        }
        let result = unsafe {
            sk_tcp_send(self.id, data.as_ptr(), data.len())
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(result as usize)
        }
    }

    /// Send a string with \r\n terminator
    pub fn send_line(&self, line: &str) -> Result<usize, i32> {
        let data = format!("{}\r\n", line);
        self.send(data.as_bytes())
    }

    /// Receive a complete line (non-blocking)
    ///
    /// Only works in line-buffered mode. Returns None if no complete line is available.
    pub fn recv_line(&self, buf: &mut [u8]) -> Option<usize> {
        let result = unsafe {
            sk_tcp_recv_line(self.id, buf.as_mut_ptr(), buf.len())
        };
        if result <= 0 {
            None
        } else {
            Some(result as usize)
        }
    }

    /// Receive a line as a String (non-blocking)
    ///
    /// Convenience method that allocates a buffer and returns a String.
    pub fn recv_line_string(&self) -> Option<String> {
        let mut buf = [0u8; 1024];
        self.recv_line(&mut buf).map(|len| {
            String::from_utf8_lossy(&buf[..len]).to_string()
        })
    }

    /// Receive raw data (non-blocking)
    ///
    /// Only works in raw mode. Returns None if no data is available.
    pub fn recv_raw(&self, buf: &mut [u8]) -> Option<usize> {
        let result = unsafe {
            sk_tcp_recv_raw(self.id, buf.as_mut_ptr(), buf.len())
        };
        if result <= 0 {
            None
        } else {
            Some(result as usize)
        }
    }

    /// Get number of buffered items waiting to be received
    pub fn pending(&self) -> i32 {
        unsafe { sk_tcp_pending(self.id) }
    }

    /// Close the socket
    pub fn close(&mut self) {
        if self.id >= 0 {
            unsafe { sk_tcp_close(self.id) };
            self.id = -1;
        }
    }
}

impl Drop for TcpSocket {
    fn drop(&mut self) {
        self.close();
    }
}

// =============================================================================
// SignalK Delta Emission
// =============================================================================

/// Emit a SignalK delta message
///
/// The message should be a complete SignalK delta JSON object.
pub fn handle_message(msg: &str) {
    unsafe {
        sk_handle_message(msg.as_ptr(), msg.len());
    }
}

/// Register as a radar provider
///
/// Returns true on success, false on failure.
pub fn register_radar_provider(name: &str) -> bool {
    unsafe { sk_register_radar_provider(name.as_ptr(), name.len()) != 0 }
}

/// Emit radar spoke data to connected WebSocket clients
///
/// The spoke_data should be binary protobuf RadarMessage format.
/// Returns true if at least one client received the data.
pub fn emit_radar_spokes(radar_id: &str, spoke_data: &[u8]) -> bool {
    unsafe {
        sk_radar_emit_spokes(
            radar_id.as_ptr(),
            radar_id.len(),
            spoke_data.as_ptr(),
            spoke_data.len(),
        ) == 1
    }
}

/// Emit a SignalK delta update for a specific path
///
/// Creates a properly formatted delta message and sends it.
pub fn emit_delta(path: &str, value: &str) {
    // Create a proper SignalK delta message
    let delta = format!(
        r#"{{"updates":[{{"values":[{{"path":"{}","value":{}}}]}}]}}"#,
        path, value
    );
    handle_message(&delta);
}

/// Emit a JSON value to a SignalK path
pub fn emit_json<T: serde::Serialize>(path: &str, value: &T) {
    match serde_json::to_string(value) {
        Ok(json) => {
            // Sanitize: replace any control characters that could break JSON parsing
            let sanitized: String = json
                .chars()
                .map(|c| if c.is_control() && c != '\n' && c != '\r' && c != '\t' { ' ' } else { c })
                .collect();
            debug(&format!("emit_json path={} len={}", path, sanitized.len()));
            emit_delta(path, &sanitized);
        }
        Err(e) => {
            set_error(&format!("Failed to serialize JSON for path {}: {}", path, e));
        }
    }
}

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Read the plugin configuration from SignalK
///
/// Returns the configuration as a JSON string, or None if not available.
pub fn read_config() -> Option<String> {
    const BUF_SIZE: usize = 32768; // 32KB should be enough for config
    let mut buf = vec![0u8; BUF_SIZE];

    let len = unsafe { sk_read_config(buf.as_mut_ptr(), buf.len()) };

    if len <= 0 {
        None
    } else {
        buf.truncate(len as usize);
        String::from_utf8(buf).ok()
    }
}

/// Save the plugin configuration to SignalK
///
/// Returns true on success.
pub fn save_config(config_json: &str) -> bool {
    let result = unsafe {
        sk_save_config(config_json.as_ptr(), config_json.len())
    };
    result >= 0
}

//! WASM implementation of IoProvider using SignalK FFI calls.
//!
//! This module provides `WasmIoProvider` which implements `mayara_core::IoProvider`
//! using the SignalK host's socket FFI functions.

use mayara_core::io::{IoError, IoProvider, TcpSocketHandle, UdpSocketHandle};

use crate::signalk_ffi;

/// WASM implementation of IoProvider using SignalK FFI.
///
/// This wraps the raw SignalK FFI socket calls in the IoProvider interface,
/// allowing shared locator/controller code to run on WASM.
pub struct WasmIoProvider {
    /// Current timestamp in milliseconds (must be updated externally via `set_time`)
    current_time_ms: u64,
}

impl WasmIoProvider {
    /// Create a new WASM I/O provider.
    pub fn new() -> Self {
        Self { current_time_ms: 0 }
    }

    /// Update the current timestamp.
    ///
    /// Call this from `plugin_poll()` with the timestamp provided by SignalK.
    pub fn set_time(&mut self, time_ms: u64) {
        self.current_time_ms = time_ms;
    }
}

impl Default for WasmIoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl IoProvider for WasmIoProvider {
    // -------------------------------------------------------------------------
    // UDP Operations
    // -------------------------------------------------------------------------

    fn udp_create(&mut self) -> Result<UdpSocketHandle, IoError> {
        let id = unsafe { signalk_ffi::raw::sk_udp_create(0) }; // 0 = udp4
        if id < 0 {
            Err(IoError::from_code(id))
        } else {
            Ok(UdpSocketHandle(id))
        }
    }

    fn udp_bind(&mut self, socket: &UdpSocketHandle, port: u16) -> Result<(), IoError> {
        let result = unsafe { signalk_ffi::raw::sk_udp_bind(socket.0, port) };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(())
        }
    }

    fn udp_set_broadcast(&mut self, socket: &UdpSocketHandle, enabled: bool) -> Result<(), IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_udp_set_broadcast(socket.0, if enabled { 1 } else { 0 })
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(())
        }
    }

    fn udp_join_multicast(
        &mut self,
        socket: &UdpSocketHandle,
        group: &str,
        interface: &str,
    ) -> Result<(), IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_udp_join_multicast(
                socket.0,
                group.as_ptr(),
                group.len(),
                interface.as_ptr(),
                interface.len(),
            )
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(())
        }
    }

    fn udp_send_to(
        &mut self,
        socket: &UdpSocketHandle,
        data: &[u8],
        addr: &str,
        port: u16,
    ) -> Result<usize, IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_udp_send(
                socket.0,
                addr.as_ptr(),
                addr.len(),
                port,
                data.as_ptr(),
                data.len(),
            )
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(result as usize)
        }
    }

    fn udp_recv_from(
        &mut self,
        socket: &UdpSocketHandle,
        buf: &mut [u8],
    ) -> Option<(usize, String, u16)> {
        let mut addr_buf = [0u8; 64];
        let mut port: u16 = 0;

        let result = unsafe {
            signalk_ffi::raw::sk_udp_recv(
                socket.0,
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

    fn udp_pending(&self, socket: &UdpSocketHandle) -> i32 {
        unsafe { signalk_ffi::raw::sk_udp_pending(socket.0) }
    }

    fn udp_close(&mut self, socket: UdpSocketHandle) {
        unsafe { signalk_ffi::raw::sk_udp_close(socket.0) };
    }

    // -------------------------------------------------------------------------
    // TCP Operations
    // -------------------------------------------------------------------------

    fn tcp_create(&mut self) -> Result<TcpSocketHandle, IoError> {
        let id = unsafe { signalk_ffi::raw::sk_tcp_create() };
        if id < 0 {
            Err(IoError::from_code(id))
        } else {
            Ok(TcpSocketHandle(id))
        }
    }

    fn tcp_connect(
        &mut self,
        socket: &TcpSocketHandle,
        addr: &str,
        port: u16,
    ) -> Result<(), IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_tcp_connect(socket.0, addr.as_ptr(), addr.len(), port)
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(())
        }
    }

    fn tcp_is_connected(&self, socket: &TcpSocketHandle) -> bool {
        unsafe { signalk_ffi::raw::sk_tcp_connected(socket.0) == 1 }
    }

    fn tcp_is_valid(&self, socket: &TcpSocketHandle) -> bool {
        unsafe { signalk_ffi::raw::sk_tcp_connected(socket.0) >= 0 }
    }

    fn tcp_set_line_buffering(
        &mut self,
        socket: &TcpSocketHandle,
        enabled: bool,
    ) -> Result<(), IoError> {
        let result = unsafe {
            signalk_ffi::raw::sk_tcp_set_line_buffering(socket.0, if enabled { 1 } else { 0 })
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(())
        }
    }

    fn tcp_send(&mut self, socket: &TcpSocketHandle, data: &[u8]) -> Result<usize, IoError> {
        if !self.tcp_is_connected(socket) {
            return Err(IoError::not_connected());
        }
        let result = unsafe {
            signalk_ffi::raw::sk_tcp_send(socket.0, data.as_ptr(), data.len())
        };
        if result < 0 {
            Err(IoError::from_code(result))
        } else {
            Ok(result as usize)
        }
    }

    fn tcp_recv_line(&mut self, socket: &TcpSocketHandle, buf: &mut [u8]) -> Option<usize> {
        let result = unsafe {
            signalk_ffi::raw::sk_tcp_recv_line(socket.0, buf.as_mut_ptr(), buf.len())
        };
        if result <= 0 {
            None
        } else {
            Some(result as usize)
        }
    }

    fn tcp_recv_raw(&mut self, socket: &TcpSocketHandle, buf: &mut [u8]) -> Option<usize> {
        let result = unsafe {
            signalk_ffi::raw::sk_tcp_recv_raw(socket.0, buf.as_mut_ptr(), buf.len())
        };
        if result <= 0 {
            None
        } else {
            Some(result as usize)
        }
    }

    fn tcp_pending(&self, socket: &TcpSocketHandle) -> i32 {
        unsafe { signalk_ffi::raw::sk_tcp_pending(socket.0) }
    }

    fn tcp_close(&mut self, socket: TcpSocketHandle) {
        unsafe { signalk_ffi::raw::sk_tcp_close(socket.0) };
    }

    // -------------------------------------------------------------------------
    // Utility
    // -------------------------------------------------------------------------

    fn current_time_ms(&self) -> u64 {
        self.current_time_ms
    }

    fn debug(&self, msg: &str) {
        signalk_ffi::debug(msg);
    }
}

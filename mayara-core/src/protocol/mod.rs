//! Radar protocol implementations.
//!
//! This module contains wire protocol parsing and formatting for each radar brand.
//! All functions are pure (no I/O) and suitable for WASM compilation.
//!
//! # Structure
//!
//! Each brand module provides:
//! - **Beacon parsing** - Discovery packet parsing
//! - **Command formatting** - Control command generation
//! - **Response parsing** - Status response parsing
//! - **Dispatch functions** - Control ID → wire command routing
//!
//! # Example
//!
//! ```rust,no_run
//! use mayara_core::protocol::furuno;
//!
//! // Parse discovery beacon
//! let packet: &[u8] = &[/* beacon data */];
//! if furuno::is_beacon_response(packet) {
//!     let discovery = furuno::parse_beacon_response(packet, "172.31.6.1").unwrap();
//!     println!("Found: {}", discovery.name);
//! }
//!
//! // Format a control command
//! use mayara_core::protocol::furuno::dispatch;
//! if let Some(cmd) = dispatch::format_control_command("gain", 50, false) {
//!     // Send cmd over TCP
//! }
//! ```

#[cfg(feature = "furuno")]
pub mod furuno;

#[cfg(feature = "navico")]
pub mod navico;

#[cfg(feature = "raymarine")]
pub mod raymarine;

#[cfg(feature = "garmin")]
pub mod garmin;

/// Helper function to extract a null-terminated C string from bytes
pub fn c_string(bytes: &[u8]) -> Option<String> {
    let null_pos = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..null_pos])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c_string() {
        assert_eq!(c_string(b"hello\0world"), Some("hello".to_string()));
        assert_eq!(c_string(b"hello"), Some("hello".to_string()));
        assert_eq!(c_string(b"\0"), None);
        assert_eq!(c_string(b"  test  \0"), Some("test".to_string()));
    }
}

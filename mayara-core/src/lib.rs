//! Mayara Core - Platform-independent radar protocol library
//!
//! This crate contains pure parsing logic for marine radar protocols.
//! It has no I/O dependencies and can be compiled for any target including WASM.
//!
//! # Supported Radars
//!
//! - **Furuno**: DRS series, FAR series
//! - **Navico**: BR24, 3G, 4G, HALO series
//! - **Raymarine**: Quantum, RD series
//! - **Garmin**: xHD series
//!
//! # Example
//!
//! ```rust,no_run
//! use mayara_core::protocol::furuno;
//!
//! // Parse a beacon response
//! let packet: &[u8] = &[0u8; 32]; // Real packet would come from network
//! match furuno::parse_beacon_response(packet, "172.31.6.1") {
//!     Ok(discovery) => println!("Found radar: {}", discovery.name),
//!     Err(e) => println!("Parse error: {}", e),
//! }
//! ```

pub mod brand;
pub mod capabilities;
pub mod error;
pub mod models;
pub mod protocol;
pub mod radar;
pub mod state;

// Re-export commonly used types
pub use brand::Brand;
pub use error::ParseError;
pub use state::{ControlValueState, PowerState, RadarState};

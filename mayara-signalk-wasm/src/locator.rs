//! Radar Locator (re-export from mayara-core)
//!
//! This module re-exports the generic RadarLocator from mayara-core.
//! The actual implementation uses IoProvider for platform-independent I/O.

pub use mayara_core::locator::{DiscoveredRadar, RadarLocator};

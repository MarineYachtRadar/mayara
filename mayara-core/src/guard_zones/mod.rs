//! Guard Zone Alerting
//!
//! This module provides guard zone detection for radar returns.
//! Guard zones are configurable areas that trigger alerts when
//! radar returns are detected within them.
//!
//! # Features
//!
//! - Arc-shaped guard zones (defined by bearing/distance range)
//! - Multiple zones per radar
//! - Configurable sensitivity and alert states
//!
//! # Example
//!
//! ```rust,ignore
//! use mayara_core::guard_zones::{GuardZoneProcessor, GuardZone, ZoneShape};
//!
//! let mut processor = GuardZoneProcessor::new();
//!
//! // Add a guard zone
//! processor.add_zone(GuardZone {
//!     id: 1,
//!     enabled: true,
//!     shape: ZoneShape::Arc {
//!         start_bearing: 0.0,
//!         end_bearing: 90.0,
//!         inner_radius: 500.0,
//!         outer_radius: 1000.0,
//!     },
//!     sensitivity: 128,
//! });
//!
//! // Check spoke for zone intrusions
//! let alerts = processor.check_spoke(&spoke_data, 45.0, 1852.0, timestamp);
//! ```

mod zone;

pub use zone::*;

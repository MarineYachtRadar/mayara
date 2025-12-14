//! Target Trail History
//!
//! This module stores position history for tracked targets, enabling
//! trail visualization and motion analysis.
//!
//! # Features
//!
//! - Configurable trail length (time-based or point-based)
//! - Efficient circular buffer storage
//! - Support for multiple trail modes (relative/true motion)
//!
//! # Example
//!
//! ```rust,ignore
//! use mayara_core::trails::{TrailStore, TrailSettings, TrailPoint};
//!
//! let settings = TrailSettings::default();
//! let mut store = TrailStore::new(settings);
//!
//! // Add a trail point
//! store.add_point(1, TrailPoint {
//!     timestamp: 1000,
//!     bearing: 45.0,
//!     distance: 1000.0,
//!     latitude: Some(51.5),
//!     longitude: Some(-0.1),
//! });
//!
//! // Get trail for target
//! let trail = store.get_trail(1);
//! ```

mod history;

pub use history::*;

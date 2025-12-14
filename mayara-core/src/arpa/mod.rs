//! ARPA (Automatic Radar Plotting Aid) Target Tracking
//!
//! This module provides automatic target detection, tracking, and collision
//! avoidance calculations. It is designed to be platform-independent and
//! can be used in both native and WASM environments.
//!
//! # Architecture
//!
//! The ARPA module is split into several submodules:
//!
//! - **polar**: Polar coordinate types and conversions
//! - **doppler**: Doppler state machine for approaching/receding targets
//! - **contour**: Contour detection and representation
//! - **history**: History buffer for storing radar spoke data
//! - **kalman**: Extended Kalman filter for target tracking
//! - **target**: Target state and refresh algorithm
//! - **cpa**: CPA/TCPA calculations
//! - **detector**: Simple target detection for auto-acquisition
//! - **tracker**: High-level processor (simple API)
//! - **types**: Legacy API types (ArpaTarget, ArpaSettings, etc.)
//!
//! # Usage
//!
//! For full-featured ARPA with contour detection and Doppler:
//!
//! ```rust,ignore
//! use mayara_core::arpa::{
//!     HistoryBuffer, TargetState, RefreshConfig, refresh_target, Pass,
//!     Legend, ExtendedPosition, TargetStatus,
//! };
//!
//! // Create history buffer
//! let mut history = HistoryBuffer::new(2048);
//!
//! // Update spoke data
//! history.update_spoke(angle, &data, timestamp, lat, lon, &Legend::default());
//!
//! // Create target
//! let pos = ExtendedPosition::new(lat, lon, 0.0, 0.0, timestamp, 0.0, 0.0);
//! let mut target = TargetState::new(1, pos, own_lat, own_lon, 2048, TargetStatus::Acquire0, false);
//!
//! // Refresh target
//! let config = RefreshConfig { ... };
//! refresh_target(&mut target, &mut history, own_lat, own_lon, &config, search_radius, Pass::First);
//! ```
//!
//! For simple detection-based ARPA (SignalK API style):
//!
//! ```rust,ignore
//! use mayara_core::arpa::{ArpaProcessor, ArpaSettings, OwnShip};
//!
//! let settings = ArpaSettings::default();
//! let mut processor = ArpaProcessor::new(settings);
//! processor.update_own_ship(OwnShip { ... });
//! let events = processor.process_spoke(&spoke_data, bearing, timestamp);
//! ```

// New modular ARPA implementation
mod polar;
mod doppler;
mod contour;
mod history;
mod kalman;
mod target;

// Legacy/simple implementation
mod types;
mod tracker;
mod cpa;
mod detector;

// Re-export new modular types
pub use polar::{
    Polar, LocalPosition, PolarConverter, FOUR_DIRECTIONS,
    METERS_PER_DEGREE_LATITUDE, NAUTICAL_MILE, KN_TO_MS, MS_TO_KN,
    meters_per_degree_longitude,
};
pub use doppler::DopplerState;
pub use contour::{Contour, ContourError, MIN_CONTOUR_LENGTH, MAX_CONTOUR_LENGTH};
pub use history::{HistoryPixel, HistorySpoke, HistoryBuffer, Legend};
pub use kalman::KalmanFilter;
pub use target::{
    TargetState, TargetStatus, RefreshState, Pass, ExtendedPosition,
    RefreshConfig, refresh_target,
    MAX_LOST_COUNT, MAX_DETECTION_SPEED_KN,
};

// Re-export legacy types (for backward compatibility)
pub use types::*;
pub use tracker::ArpaProcessor;
pub use cpa::CpaResult;
pub use detector::TargetDetector;

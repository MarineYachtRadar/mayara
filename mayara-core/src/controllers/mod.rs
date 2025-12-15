//! Brand-specific radar controllers using IoProvider
//!
//! This module provides platform-independent radar controllers that work on both
//! native (tokio) and WASM (FFI) platforms via the [`IoProvider`](crate::IoProvider) trait.
//!
//! # Architecture
//!
//! Each controller handles:
//! - TCP/UDP connection management
//! - Login sequences (brand-specific)
//! - Command sending
//! - Response parsing and state updates
//!
//! The controllers use a poll-based design that works with any I/O backend.
//! Controllers emit [`ControllerEvent`]s that the shell (server/WASM) handles
//! to update its state management (e.g., SharedRadars, SignalK paths).
//!
//! ```rust,ignore
//! use mayara_core::controllers::{FurunoController, ControllerEvent};
//! use mayara_core::IoProvider;
//!
//! fn main_loop<I: IoProvider>(io: &mut I, controller: &mut FurunoController) {
//!     loop {
//!         // Poll returns events for the shell to handle
//!         for event in controller.poll(io) {
//!             match event {
//!                 ControllerEvent::ModelDetected { model, version } => {
//!                     // Shell-specific: update shared state, set ranges, etc.
//!                 }
//!                 ControllerEvent::Connected => {
//!                     // Controller is now connected and ready for commands
//!                 }
//!                 _ => {}
//!             }
//!         }
//!
//!         // Set controls as needed
//!         controller.set_gain(io, 50, false);
//!     }
//! }
//! ```
//!
//! # Supported Brands
//!
//! | Brand | Controller | Protocol | Features |
//! |-------|------------|----------|----------|
//! | Furuno | [`FurunoController`] | TCP login + command | NXT Doppler |
//! | Navico | [`NavicoController`] | UDP multicast | HALO Doppler |
//! | Raymarine | [`RaymarineController`] | UDP | Quantum/RD variants |
//! | Garmin | [`GarminController`] | UDP | xHD series |
//!
//! # Example: Multi-brand support
//!
//! ```rust,ignore
//! use mayara_core::controllers::*;
//! use mayara_core::{Brand, IoProvider};
//!
//! enum RadarController {
//!     Furuno(FurunoController),
//!     Navico(NavicoController),
//!     Raymarine(RaymarineController),
//!     Garmin(GarminController),
//! }
//!
//! impl RadarController {
//!     fn poll<I: IoProvider>(&mut self, io: &mut I) -> bool {
//!         match self {
//!             RadarController::Furuno(c) => c.poll(io),
//!             RadarController::Navico(c) => c.poll(io),
//!             RadarController::Raymarine(c) => c.poll(io),
//!             RadarController::Garmin(c) => c.poll(io),
//!         }
//!     }
//! }
//! ```

pub mod furuno;
pub mod garmin;
pub mod navico;
pub mod raymarine;

// Re-export main types
pub use furuno::{ControllerState, FurunoController};
pub use garmin::{GarminController, GarminControllerState};
pub use navico::{NavicoController, NavicoControllerState, NavicoModel};
pub use raymarine::{RaymarineController, RaymarineControllerState, RaymarineVariant};

/// Events emitted by controllers for the shell to handle.
///
/// Controllers are platform-independent and don't know about server's `SharedRadars`
/// or WASM's SignalK paths. Instead, they emit events that the shell handles
/// according to its own state management.
///
/// This follows the architecture principle: **mayara-core is the database,
/// shells are thin I/O adapters**.
#[derive(Debug, Clone)]
pub enum ControllerEvent {
    /// Controller has established connection to the radar.
    /// Shell may want to log this or update UI state.
    Connected,

    /// Controller connection was lost.
    /// Shell may want to update UI state or trigger reconnection logic.
    Disconnected,

    /// Radar model and firmware version detected.
    /// Shell should:
    /// - Look up model in mayara-core's model database
    /// - Set ranges from the model's range_table
    /// - Add model-specific controls
    /// - Update shared state so radar appears in API
    ModelDetected {
        /// Model name (e.g., "DRS4D-NXT")
        model: String,
        /// Firmware version (e.g., "01.05")
        version: String,
    },

    /// Operating hours retrieved from radar.
    /// Shell may want to display this or store for diagnostics.
    OperatingHoursUpdated {
        /// Total operating hours
        hours: f64,
    },
}

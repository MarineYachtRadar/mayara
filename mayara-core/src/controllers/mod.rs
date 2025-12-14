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
//! The controllers use a poll-based design that works with any I/O backend:
//!
//! ```rust,ignore
//! use mayara_core::controllers::FurunoController;
//! use mayara_core::IoProvider;
//!
//! fn main_loop<I: IoProvider>(io: &mut I, controller: &mut FurunoController) {
//!     loop {
//!         // Poll returns any state updates
//!         controller.poll(io);
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

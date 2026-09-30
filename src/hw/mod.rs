//! Board support for the Music Thing Workshop System Computer.
//!
//! No BSP crate exists for this module, so this is ours. It is kept behind a
//! clean seam from the musical logic for two reasons: the sequencer code stays
//! testable on the host, and several facts in here are unverified against real
//! hardware, so we want one obvious place to correct them.
//!
//! The submodules are deliberately data-and-arithmetic rather than
//! driver-shaped where possible — command word construction, calibration
//! fitting and control scaling are all pure functions with tests, so the parts
//! that can be wrong in a *musical* way are checkable without a module
//! plugged in.

// Driver glue, ARM only: the Cortex-M crates cannot build for the host.
#[cfg(target_arch = "arm")]
pub mod board;

pub mod calibration;
pub mod controls;
pub mod cv;
pub mod dac;
pub mod mux;
pub mod pins;

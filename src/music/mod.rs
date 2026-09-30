//! Sequence generation: rhythm, voltage, and the four modes.
//!
//! Pitch here is raw voltage, never a note number — see [`voltage`] for the
//! reasoning. All of this is pure logic with host tests, so the musical
//! behaviour is checkable without hardware.

pub mod euclid;
pub mod modes;
pub mod rng;
pub mod voltage;

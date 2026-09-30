//! gengengen — a generative sequencer for the Music Thing Workshop System
//! Computer.
//!
//! The crate is split so the musical logic can be tested on the host: `hw` is
//! board support (pin map, mux, DAC framing, calibration), `music` is scales
//! and the mode generators, `seq` is clocking and step state. Only `hw`'s
//! driver glue needs real hardware; everything else is pure functions with
//! tests.
#![cfg_attr(not(test), no_std)]

pub mod hw;

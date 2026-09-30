//! GPIO pin map for the Music Thing Workshop System Computer (Rev 1, RP2040).
//!
//! Source: the "RP2040 to Computer Pinout" table in `Computer_ Rev 1
//! Documentation.pdf` in the Workshop_Computer repo, cross-checked against
//! ComputerCard.h and Brian Dorsey's Rust cards. All three agree on the GPIO
//! assignments below.
//!
//! Several signals are **inverted** in hardware. See [`invert`] for the ones
//! that matter and why.

/// Pulse (gate/trigger) inputs. Inverted: a low input reads high.
///
/// These need the RP2040 internal pull-up enabled — it biases the input
/// transistor, so without it the input does not work at all.
pub const PULSE_IN_1: u8 = 2;
pub const PULSE_IN_2: u8 = 3;

/// Normalisation probe. Driven as an output and toggled to detect which
/// sockets have jacks inserted.
pub const NORMALISATION_PROBE: u8 = 4;

/// Board ID bits. `000` = Proto 1.2 (floating), `100` = Proto 2.0/2.0.1/Rev 1.
pub const BOARD_ID_0: u8 = 5;
pub const BOARD_ID_1: u8 = 6;
pub const BOARD_ID_2: u8 = 7;

/// Pulse outputs. Inverted: writing 1 drives the output low, so the idle
/// state is to drive these **high**.
pub const PULSE_OUT_1: u8 = 8;
pub const PULSE_OUT_2: u8 = 9;

/// The six LEDs, in panel order (2 wide x 3 tall, top-left to bottom-right).
/// Driven through an NPN array off the +12 V rail — deliberately, to keep
/// switching noise off the RP2040's supply. 1 = illuminated.
pub const LED: [u8; 6] = [10, 11, 12, 13, 14, 15];

/// I2C to the calibration EEPROM (Zetta ZD24C08A or equivalent 24C0x).
/// 2.2k pull-ups are on the board. Note this EEPROM is on the *module*, not
/// the program card, so calibration is per-unit and shared across cards.
pub const EEPROM_SDA: u8 = 16;
pub const EEPROM_SCL: u8 = 17;

/// SPI to the MCP4822 dual 12-bit DAC, which drives the two "audio" outputs.
/// These are the precise outputs — use them for pitch.
pub const DAC_SCK: u8 = 18;
pub const DAC_MOSI: u8 = 19;
pub const DAC_CS: u8 = 21;

/// CV outputs, as inverted PWM through a two-pole active filter.
/// 11-bit at 60 kHz: 2047 = -6 V, 1024 = 0 V, 0 = +6 V.
pub const CV_OUT_1_PWM: u8 = 23;
pub const CV_OUT_2_PWM: u8 = 22;

/// 4052 analogue mux address lines. See [`super::mux`].
pub const MUX_SELECT_A: u8 = 24;
pub const MUX_SELECT_B: u8 = 25;

/// Direct bipolar analogue inputs, straight into the RP2040 ADC (ch 0 and 1).
/// Inverted: +6 V reads 0, 0 V reads 2048, -6 V reads 4095. DC-coupled.
///
/// Nominally 12-bit, but the hardware doc is candid that it is "really more
/// like 8-10 bits". Still more precise than the muxed CV inputs.
///
/// The L/R naming is **unverified**: ComputerCard's `#define`s swap L and R
/// relative to the hardware doc. The GPIO-to-ADC-channel mapping is not in
/// dispute, only which one the panel calls "left".
pub const AUDIO_IN_1: u8 = 26;
pub const AUDIO_IN_2: u8 = 27;

/// Mux outputs into ADC ch 2 and 3. Channel 2 carries the knobs and switch,
/// channel 3 carries the two CV inputs.
pub const MUX_IO_1: u8 = 28;
pub const MUX_IO_2: u8 = 29;

/// Hardware signals that are inverted between the RP2040 and the panel.
///
/// Collected here because forgetting one of these produces behaviour that
/// looks like a logic bug rather than a wiring bug: gates that fire on the
/// off-beat, or CV that sweeps the wrong way.
pub mod invert {
    /// Pulse inputs read inverted (low in = high reading).
    pub const PULSE_IN: bool = true;
    /// Pulse outputs drive inverted (write 1 = output low; idle is high).
    pub const PULSE_OUT: bool = true;
    /// CV PWM outputs are inverted (higher duty = lower voltage).
    pub const CV_OUT: bool = true;
    /// Analogue inputs read inverted (+6 V = 0, -6 V = 4095).
    pub const ANALOGUE_IN: bool = true;
}

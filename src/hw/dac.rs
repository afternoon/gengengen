//! MCP4822 dual 12-bit SPI DAC — the two "audio" outputs.
//!
//! These are the precise outputs on the module: a real DAC updated
//! synchronously, as opposed to the CV outputs, which are filtered inverted
//! PWM. Pitch goes here.
//!
//! SPI framing, per ComputerCard: 16-bit frames, MSB first, CPOL=0 CPHA=0, up
//! to 15.625 MHz.

/// Which of the MCP4822's two channels.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DacChannel {
    A,
    B,
}

impl DacChannel {
    /// Channel select bit in the command word.
    fn select_bit(self) -> u16 {
        match self {
            DacChannel::A => 0x0000,
            DacChannel::B => 0x8000,
        }
    }
}

/// Gain 1x and output enable. Both are wanted on every write, so they are
/// folded into one constant rather than exposed as options.
const GAIN_AND_ENABLE: u16 = 0x3000;

/// Build the 16-bit MCP4822 command word for a signed 12-bit value.
///
/// `value` is -2048..=2047, matching ComputerCard's signed convention, where 0
/// is the midpoint of the bipolar output range. Values outside that range are
/// clamped rather than allowed to wrap — a wrapped pitch CV would jump an
/// octave, which is far worse musically than flattening at the top of the
/// range.
pub fn command_word(channel: DacChannel, value: i16) -> u16 {
    let clamped = value.clamp(-2048, 2047);
    let offset_binary = ((clamped as i32) + 0x800) as u16 & 0x0FFF;
    channel.select_bit() | GAIN_AND_ENABLE | offset_binary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_is_half_scale() {
        let w = command_word(DacChannel::A, 0);
        assert_eq!(w & 0x0FFF, 0x800);
    }

    #[test]
    fn channel_select_differs_and_does_not_disturb_value() {
        let a = command_word(DacChannel::A, 100);
        let b = command_word(DacChannel::B, 100);
        assert_eq!(a & 0x8000, 0x0000);
        assert_eq!(b & 0x8000, 0x8000);
        assert_eq!(a & 0x0FFF, b & 0x0FFF);
    }

    #[test]
    fn gain_and_enable_always_set() {
        for v in [-2048i16, -1, 0, 1, 2047] {
            assert_eq!(command_word(DacChannel::A, v) & GAIN_AND_ENABLE, GAIN_AND_ENABLE);
        }
    }

    #[test]
    fn extremes_map_to_full_scale() {
        assert_eq!(command_word(DacChannel::A, -2048) & 0x0FFF, 0x000);
        assert_eq!(command_word(DacChannel::A, 2047) & 0x0FFF, 0xFFF);
    }

    #[test]
    fn out_of_range_clamps_rather_than_wrapping() {
        // A wrapped value would be an octave jump on a pitch output.
        assert_eq!(command_word(DacChannel::A, 3000) & 0x0FFF, 0xFFF);
        assert_eq!(command_word(DacChannel::A, -3000) & 0x0FFF, 0x000);
    }
}

/// Half-span of the DAC outputs in millivolts. Mirrors `board::DAC_RANGE_MV`;
/// kept here so the mapping can be tested on the host, where `board` does not
/// compile.
pub const DAC_RANGE_MV: i32 = 6000;

/// Map a voltage in millivolts onto a signed 12-bit DAC code.
///
/// The MCP4822 is a plain non-inverted DAC, so this is linear and rising: more
/// millivolts means a higher code. Deliberately *not* routed through the EEPROM
/// calibration, which describes the inverted PWM CV outputs instead — applying
/// it here would invert pitch and waste most of the DAC's range.
pub fn millivolts_to_code(mv: i32) -> i16 {
    let clamped = mv.clamp(-DAC_RANGE_MV, DAC_RANGE_MV);
    (clamped * 2048 / DAC_RANGE_MV).clamp(-2048, 2047) as i16
}

#[cfg(test)]
mod voltage_mapping_tests {
    use super::*;

    #[test]
    fn zero_volts_is_the_midpoint() {
        assert_eq!(millivolts_to_code(0), 0);
    }

    #[test]
    fn rising_voltage_gives_a_rising_code() {
        // The bug this guards: routing pitch through the PWM calibration made
        // this fall, so every sequence played upside down.
        let mut prev = i16::MIN;
        for mv in (-6000..=6000).step_by(100) {
            let code = millivolts_to_code(mv);
            assert!(code >= prev, "{mv} mV went backwards: {prev} -> {code}");
            prev = code;
        }
    }

    #[test]
    fn two_octaves_uses_most_of_the_available_range() {
        // The other half of the same bug: a two-octave span was compressed into
        // about a third of the DAC's codes, so the switch's range setting lied.
        // A 2 V span out of a 12 V range should be about a sixth of 4096.
        let span = millivolts_to_code(1000) as i32 - millivolts_to_code(-1000) as i32;
        let expected = 4096 * 2000 / 12000;
        let ratio = span as f64 / expected as f64;
        assert!(
            (0.95..1.05).contains(&ratio),
            "2V span used {span} codes, expected about {expected}"
        );
    }

    #[test]
    fn octave_spans_are_proportional() {
        // Two octaves must be twice one octave, or the switch positions are
        // not the intervals they claim.
        let one = millivolts_to_code(500) as i32 - millivolts_to_code(-500) as i32;
        let two = millivolts_to_code(1000) as i32 - millivolts_to_code(-1000) as i32;
        let ratio = two as f64 / one as f64;
        assert!((1.9..2.1).contains(&ratio), "ratio was {ratio:.2}, expected 2");
    }

    #[test]
    fn extremes_reach_the_rails_without_wrapping() {
        assert_eq!(millivolts_to_code(-DAC_RANGE_MV), -2048);
        assert_eq!(millivolts_to_code(DAC_RANGE_MV), 2047);
        // Beyond the range, clamp rather than wrap: a wrapped pitch would jump
        // an octave, which is far worse than flattening out.
        assert_eq!(millivolts_to_code(100_000), 2047);
        assert_eq!(millivolts_to_code(-100_000), -2048);
    }

    #[test]
    fn fine_differences_survive() {
        // Unquantised pitch needs sub-semitone resolution to reach the DAC.
        // A semitone is ~83 mV; the DAC's step is ~2.9 mV.
        assert_ne!(millivolts_to_code(0), millivolts_to_code(10));
    }

    #[test]
    fn command_word_round_trips_a_voltage() {
        // End to end: a positive voltage must land above the DAC's midpoint.
        let hi = command_word(DacChannel::A, millivolts_to_code(2000)) & 0x0FFF;
        let mid = command_word(DacChannel::A, millivolts_to_code(0)) & 0x0FFF;
        let lo = command_word(DacChannel::A, millivolts_to_code(-2000)) & 0x0FFF;
        assert!(lo < mid && mid < hi, "{lo} < {mid} < {hi} failed");
    }
}

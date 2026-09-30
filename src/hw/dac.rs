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

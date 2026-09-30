//! Voltage output through the calibration.
//!
//! Deliberately thin: pitch is generated as raw millivolts in `music::voltage`
//! and this module's only job is turning millivolts into a calibrated DAC
//! setting. There is no note number, no semitone, and no quantisation anywhere
//! in the signal path — see `music::voltage` for why.
//!
//! Calibration still matters even unquantised. Without it the *range* is wrong:
//! ask for two octaves of span and you get some other span, so the sequence
//! covers a different interval than the knob says.

use crate::hw::calibration::CalLine;

/// DAC setting for a voltage in millivolts, via the channel's calibration.
pub fn millivolts_to_dac(mv: i32, cal: &CalLine) -> u32 {
    cal.dac_for_millivolts(mv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::calibration::{least_squares, DEFAULT_POINTS};

    fn default_cal() -> CalLine {
        least_squares(&DEFAULT_POINTS)
    }

    #[test]
    fn higher_voltage_gives_lower_dac_setting() {
        // The outputs are inverted. If this ever flips, every sequence sweeps
        // backwards.
        let cal = default_cal();
        let low = millivolts_to_dac(0, &cal);
        let high = millivolts_to_dac(1000, &cal);
        assert!(high < low, "expected inverted mapping: {low} -> {high}");
    }

    #[test]
    fn monotonic_across_the_useful_range() {
        let cal = default_cal();
        let mut prev = u32::MAX;
        for mv in (-2000..=2000).step_by(50) {
            let dac = millivolts_to_dac(mv, &cal);
            assert!(dac <= prev, "{mv} mV broke monotonicity");
            prev = dac;
        }
    }

    #[test]
    fn range_span_is_proportional() {
        // The reason calibration still matters without quantisation: a 2000 mV
        // request must actually produce twice the DAC travel of 1000 mV, or the
        // switch's octave ranges are wrong.
        let cal = default_cal();
        let one = millivolts_to_dac(-500, &cal) as i64 - millivolts_to_dac(500, &cal) as i64;
        let two = millivolts_to_dac(-1000, &cal) as i64 - millivolts_to_dac(1000, &cal) as i64;
        let ratio = two as f64 / one as f64;
        assert!(
            (1.9..2.1).contains(&ratio),
            "two octaves was {ratio:.2}x one octave, expected 2x"
        );
    }

    #[test]
    fn stays_within_nineteen_bits() {
        let cal = default_cal();
        for mv in [-100_000, -6000, 0, 6000, 100_000] {
            assert!(millivolts_to_dac(mv, &cal) <= 524_287);
        }
    }

    #[test]
    fn fine_voltage_differences_survive() {
        // Unquantised means sub-semitone differences must reach the DAC. A
        // semitone is ~83 mV; 10 mV steps must produce distinct settings.
        let cal = default_cal();
        let a = millivolts_to_dac(0, &cal);
        let b = millivolts_to_dac(10, &cal);
        assert_ne!(a, b, "10 mV difference was lost");
    }
}

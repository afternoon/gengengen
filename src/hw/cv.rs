//! The **PWM CV outputs**, through the EEPROM calibration.
//!
//! This module was originally called `pitch`, which was a mistake worth
//! recording: the EEPROM calibration block describes the PWM CV outputs, not the
//! SPI DAC, and the name invited routing pitch through it. Doing so inverted
//! every sequence and compressed a two-octave span into about a third of the
//! available range.
//!
//! Pitch goes to the DAC via `hw::dac::millivolts_to_code`, which is linear and
//! non-inverted. This module is for the aux CV outputs, where the calibration
//! genuinely applies and where its inverted slope is correct.

use crate::hw::calibration::CalLine;

/// 19-bit CV setting for a voltage in millivolts, via the channel's calibration.
///
/// The returned value is on the calibration's own inverted scale: a higher
/// voltage gives a *lower* number, matching the hardware.
pub fn millivolts_to_cv(mv: i32, cal: &CalLine) -> u32 {
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
        let low = millivolts_to_cv(0, &cal);
        let high = millivolts_to_cv(1000, &cal);
        assert!(high < low, "expected inverted mapping: {low} -> {high}");
    }

    #[test]
    fn monotonic_across_the_useful_range() {
        let cal = default_cal();
        let mut prev = u32::MAX;
        for mv in (-2000..=2000).step_by(50) {
            let dac = millivolts_to_cv(mv, &cal);
            assert!(dac <= prev, "{mv} mV broke monotonicity");
            prev = dac;
        }
    }

    #[test]
    fn range_span_is_proportional() {
        // A 2000 mV request must produce twice the travel of 1000 mV, or the
        // aux CV's range does not match what was asked for.
        let cal = default_cal();
        let one = millivolts_to_cv(-500, &cal) as i64 - millivolts_to_cv(500, &cal) as i64;
        let two = millivolts_to_cv(-1000, &cal) as i64 - millivolts_to_cv(1000, &cal) as i64;
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
            assert!(millivolts_to_cv(mv, &cal) <= 524_287);
        }
    }

    #[test]
    fn fine_voltage_differences_survive() {
        // Unquantised means sub-semitone differences must reach the DAC. A
        // semitone is ~83 mV; 10 mV steps must produce distinct settings.
        let cal = default_cal();
        let a = millivolts_to_cv(0, &cal);
        let b = millivolts_to_cv(10, &cal);
        assert_ne!(a, b, "10 mV difference was lost");
    }
}

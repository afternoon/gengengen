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

/// PWM top for the CV outputs. Mirrors `board::CV_PWM_TOP`, kept here so the
/// scaling can be tested on the host where `board` does not compile.
pub const CV_PWM_TOP: u16 = 2047;

/// The PWM duty for a voltage, through the calibration.
///
/// Inversion is the thing to be careful about. The calibration line is already
/// inverted (higher volts, lower value) and the PWM hardware inverts again, so
/// exactly one more flip is needed overall — not two, and not none. An earlier
/// version flipped twice and ran the outputs backwards, which is the bug the
/// tests below exist to catch.
pub fn millivolts_to_pwm_duty(mv: i32, cal: &CalLine) -> u16 {
    let wide = cal.dac_for_millivolts(mv);
    let scaled = ((wide >> 8) as u16).min(CV_PWM_TOP);
    CV_PWM_TOP - scaled
}

#[cfg(test)]
mod pwm_scaling_tests {
    use super::*;
    use crate::hw::calibration::{least_squares, DEFAULT_POINTS};

    fn cal() -> CalLine {
        least_squares(&DEFAULT_POINTS)
    }

    #[test]
    fn duty_rises_with_voltage() {
        // The bug this guards: two cancelling inversions left the CV outputs
        // running backwards. Once pitch moved onto these outputs that would
        // have played every sequence upside down.
        let c = cal();
        let mut prev = 0u16;
        for mv in (-2000..=2000).step_by(100) {
            let duty = millivolts_to_pwm_duty(mv, &c);
            assert!(
                duty >= prev,
                "{mv} mV gave duty {duty}, below the previous {prev}"
            );
            prev = duty;
        }
    }

    #[test]
    fn zero_volts_is_near_the_middle() {
        let duty = millivolts_to_pwm_duty(0, &cal());
        let mid = CV_PWM_TOP / 2;
        let off = (duty as i32 - mid as i32).abs();
        assert!(off < 120, "0 V gave duty {duty}, expected near {mid}");
    }

    #[test]
    fn octave_spans_are_proportional() {
        // Two octaves must be twice one octave, or the switch's range settings
        // are not the intervals they claim.
        let c = cal();
        let one = millivolts_to_pwm_duty(500, &c) as i32
            - millivolts_to_pwm_duty(-500, &c) as i32;
        let two = millivolts_to_pwm_duty(1000, &c) as i32
            - millivolts_to_pwm_duty(-1000, &c) as i32;
        let ratio = two as f64 / one as f64;
        assert!((1.9..2.1).contains(&ratio), "ratio was {ratio:.2}, expected 2");
    }

    #[test]
    fn stays_within_the_pwm_range() {
        let c = cal();
        for mv in [-100_000, -6000, 0, 6000, 100_000] {
            assert!(millivolts_to_pwm_duty(mv, &c) <= CV_PWM_TOP);
        }
    }

    #[test]
    fn fine_differences_survive() {
        // 11 bits over ~12 V is about 6 mV per step, so a 50 mV difference must
        // still move the output - unquantised pitch depends on it.
        let c = cal();
        assert_ne!(
            millivolts_to_pwm_duty(0, &c),
            millivolts_to_pwm_duty(50, &c)
        );
    }
}

/// Sigma-delta modulator for the CV outputs.
///
/// The CV jacks are the Workshop System's designated *precision* pitch outputs —
/// Music Thing's own description of the panel calls that pair "precision control
/// voltages for pitch", and both the Turing Machine and Simple MIDI cards put
/// 1V/oct there. ComputerCard reaches ~19-bit effective resolution on an 11-bit
/// PWM by dithering the duty cycle between adjacent values, so the two-pole
/// filter on the output averages them into the value in between.
///
/// Without this, an 11-bit duty gives ~5.9 mV steps, which is about **7 cents**
/// — an audible grid under a sequencer whose entire premise is unquantised
/// pitch. With it the step is ~0.02 cents, i.e. gone.
///
/// The modulator carries an error accumulator between updates: each update adds
/// the fractional part it could not represent to the next one, so the *average*
/// duty converges on the exact requested value.
#[derive(Copy, Clone, Debug, Default)]
pub struct SigmaDelta {
    /// Accumulated fractional error, in the 19-bit scale's sub-PWM bits.
    error: u32,
}

/// How many bits the 19-bit calibration scale sits above the 11-bit PWM.
const SUB_BITS: u32 = 8;
#[cfg(test)]
const SUB_SCALE: u32 = 1 << SUB_BITS;

impl SigmaDelta {
    pub fn new() -> Self {
        Self { error: 0 }
    }

    /// Next PWM duty for a 19-bit target, dithering to preserve the low bits.
    ///
    /// Call this at a steady rate — the averaging is what creates the extra
    /// resolution, so an irregular update rate spreads the dither unevenly.
    pub fn next_duty(&mut self, target_19bit: u32) -> u16 {
        let target = target_19bit.min(u32::from(CV_PWM_TOP) << SUB_BITS);

        // Add what we owe from previous updates, then take the whole part.
        let adjusted = target + self.error;
        let whole = adjusted >> SUB_BITS;
        // Carry the remainder forward rather than discarding it; that residue
        // is precisely the resolution the plain PWM throws away.
        self.error = adjusted - (whole << SUB_BITS);

        (whole as u16).min(CV_PWM_TOP)
    }
}

/// The 19-bit CV target for a voltage, through the calibration.
///
/// Like [`millivolts_to_pwm_duty`] but keeping the calibration's full precision
/// for the sigma-delta modulator, rather than truncating to 11 bits.
pub fn millivolts_to_cv_19bit(mv: i32, cal: &CalLine) -> u32 {
    let wide = cal.dac_for_millivolts(mv);
    // The calibration line is inverted; flip once so higher volts means a higher
    // target, matching what the duty should do before the hardware inverts it.
    let full = u32::from(CV_PWM_TOP) << SUB_BITS;
    full.saturating_sub(wide.min(full))
}

#[cfg(test)]
mod sigma_delta_tests {
    use super::*;
    use crate::hw::calibration::{least_squares, DEFAULT_POINTS};

    fn cal() -> CalLine {
        least_squares(&DEFAULT_POINTS)
    }

    #[test]
    fn average_duty_converges_on_the_target() {
        // The whole point: a target between two PWM steps must average out to
        // that in-between value rather than sitting on one side of it.
        let mut sd = SigmaDelta::new();
        // Half a PWM step above an exact value.
        let target = (1000u32 << SUB_BITS) + SUB_SCALE / 2;

        let n = 1000;
        let sum: u32 = (0..n).map(|_| u32::from(sd.next_duty(target))).sum();
        let mean = sum as f64 / n as f64;
        assert!(
            (mean - 1000.5).abs() < 0.05,
            "mean duty was {mean}, expected about 1000.5"
        );
    }

    #[test]
    fn exact_values_do_not_dither() {
        // A target that lands exactly on a PWM step should hold steady, not
        // jitter either side of it.
        let mut sd = SigmaDelta::new();
        let target = 1234u32 << SUB_BITS;
        for _ in 0..100 {
            assert_eq!(sd.next_duty(target), 1234);
        }
    }

    #[test]
    fn resolves_below_one_pwm_step() {
        // Two targets a quarter of a PWM step apart must produce different
        // averages - this is the resolution plain PWM cannot reach, and it is
        // what keeps a ~7 cent grid out of the pitch.
        let base = 1000u32 << SUB_BITS;
        let mean_of = |t: u32| {
            let mut sd = SigmaDelta::new();
            let n = 2000;
            let sum: u32 = (0..n).map(|_| u32::from(sd.next_duty(t))).sum();
            sum as f64 / n as f64
        };
        let a = mean_of(base);
        let b = mean_of(base + SUB_SCALE / 4);
        assert!(
            (b - a - 0.25).abs() < 0.05,
            "quarter-step difference came out as {}",
            b - a
        );
    }

    #[test]
    fn duty_stays_in_range() {
        let mut sd = SigmaDelta::new();
        for target in [0u32, 1, 500 << SUB_BITS, u32::MAX] {
            for _ in 0..50 {
                assert!(sd.next_duty(target) <= CV_PWM_TOP);
            }
        }
    }

    #[test]
    fn extremes_are_stable() {
        // At the rails there is nothing to dither toward, so the output must
        // sit still rather than flickering off the end.
        let mut sd = SigmaDelta::new();
        for _ in 0..100 {
            assert_eq!(sd.next_duty(0), 0);
        }
        let mut sd = SigmaDelta::new();
        let max = u32::from(CV_PWM_TOP) << SUB_BITS;
        for _ in 0..100 {
            assert_eq!(sd.next_duty(max), CV_PWM_TOP);
        }
    }

    #[test]
    fn nineteen_bit_target_rises_with_voltage() {
        let c = cal();
        let mut prev = 0u32;
        for mv in (-2000..=2000).step_by(100) {
            let t = millivolts_to_cv_19bit(mv, &c);
            assert!(t >= prev, "{mv} mV went backwards: {prev} -> {t}");
            prev = t;
        }
    }

    #[test]
    fn nineteen_bit_agrees_with_the_coarse_path() {
        // The dithered path must land in the same place as the plain one, just
        // with the low bits preserved - otherwise pitch would shift when the
        // modulator was introduced.
        let c = cal();
        for mv in [-1500, -500, 0, 500, 1500] {
            let coarse = millivolts_to_pwm_duty(mv, &c);
            let fine = (millivolts_to_cv_19bit(mv, &c) >> SUB_BITS) as u16;
            let diff = (coarse as i32 - fine as i32).abs();
            assert!(diff <= 1, "{mv} mV: coarse {coarse} vs fine {fine}");
        }
    }
}

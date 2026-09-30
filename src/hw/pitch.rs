//! 1V/oct pitch output.
//!
//! Where calibration meets music. A semitone is 1000/12 mV ~= 83.333 mV, so
//! rounding errors accumulate audibly if you work in integer millivolts per
//! semitone — 83 mV per semitone is 4 cents flat, which over an octave is
//! nearly half a semitone. So we compute in a finer unit and only round at the
//! point of output.

use crate::hw::calibration::CalLine;

/// Millivolts per octave, by definition of the standard.
pub const MV_PER_OCTAVE: i32 = 1000;

/// Semitones per octave.
pub const SEMITONES_PER_OCTAVE: i32 = 12;

/// Which note number sits at 0 V.
///
/// The outputs are bipolar (roughly -6 V to +6 V), so putting the reference in
/// the middle gives usable range in both directions. MIDI note 60 (middle C)
/// at 0 V means note 0 would be -5 V and note 120 would be +5 V, both inside
/// the output range.
pub const ZERO_VOLT_NOTE: i32 = 60;

/// Millivolts for a MIDI-style note number under 1V/oct.
///
/// Computed in tenths of a millivolt internally and rounded once, so that
/// twelve semitones land on exactly 1000 mV rather than 996.
pub fn note_to_millivolts(note: i32) -> i32 {
    let semitones_from_ref = note - ZERO_VOLT_NOTE;
    // tenths of a mV: semitones * 10000 / 12
    let tenths = semitones_from_ref * MV_PER_OCTAVE * 10 / SEMITONES_PER_OCTAVE;
    // Round to nearest mV rather than truncating toward zero.
    if tenths >= 0 {
        (tenths + 5) / 10
    } else {
        (tenths - 5) / 10
    }
}

/// The calibrated 19-bit DAC setting for a note.
pub fn note_to_dac(note: i32, cal: &CalLine) -> u32 {
    cal.dac_for_millivolts(note_to_millivolts(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_note_is_zero_volts() {
        assert_eq!(note_to_millivolts(ZERO_VOLT_NOTE), 0);
    }

    #[test]
    fn an_octave_is_exactly_one_volt() {
        // The whole point of 1V/oct. If this drifts, everything is out of tune.
        for octave in -4..=4 {
            let note = ZERO_VOLT_NOTE + octave * SEMITONES_PER_OCTAVE;
            assert_eq!(
                note_to_millivolts(note),
                octave * MV_PER_OCTAVE,
                "octave {octave} is not exactly {} mV",
                octave * MV_PER_OCTAVE
            );
        }
    }

    #[test]
    fn semitones_are_within_a_cent_of_ideal() {
        // 83 mV/semitone (naive integer division) would be 4 cents flat and
        // would compound. Check every semitone of a two-octave span, which is
        // the widest range the switch selects.
        for semi in -24..=24 {
            let note = ZERO_VOLT_NOTE + semi;
            let got = note_to_millivolts(note) as f64;
            let ideal = semi as f64 * 1000.0 / 12.0;
            let cents_off = (got - ideal) / (1000.0 / 12.0) * 100.0;
            assert!(
                cents_off.abs() < 1.0,
                "semitone {semi}: {got} mV vs ideal {ideal:.2} mV = {cents_off:.2} cents"
            );
        }
    }

    #[test]
    fn monotonic_across_the_range() {
        let mut prev = i32::MIN;
        for note in 0..=127 {
            let mv = note_to_millivolts(note);
            assert!(mv > prev, "note {note} did not rise");
            prev = mv;
        }
    }

    #[test]
    fn two_octave_range_fits_in_output_range() {
        // The switch selects 1 or 2 octaves. Both must stay inside the roughly
        // +/-6 V the hardware can produce, or high notes would clip flat.
        let low = note_to_millivolts(ZERO_VOLT_NOTE - 24);
        let high = note_to_millivolts(ZERO_VOLT_NOTE + 24);
        assert!(low > -6000 && high < 6000, "{low}..{high} mV exceeds output range");
    }

    #[test]
    fn dac_conversion_is_inverted_like_the_hardware() {
        // Higher note => lower DAC setting, because the outputs are inverted.
        let cal = crate::hw::calibration::least_squares(
            &crate::hw::calibration::DEFAULT_POINTS,
        );
        let low = note_to_dac(ZERO_VOLT_NOTE, &cal);
        let high = note_to_dac(ZERO_VOLT_NOTE + 12, &cal);
        assert!(high < low, "expected inverted mapping: {low} -> {high}");
    }
}

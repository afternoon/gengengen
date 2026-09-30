//! Pitch as raw voltage — unquantised, on purpose.
//!
//! No scales, no note numbers, no equal temperament. A random voltage goes to
//! the VCO and it plays whatever that is. This is the Turing Machine model and
//! it is the reason an unquantised random voltage sounds like *a modular* while
//! a scale-locked arpeggio sounds like a plugin.
//!
//! Everything here is in millivolts, and 1V/oct is still the reference for
//! *range* — a "two octave range" means 2000 mV of span — but nothing snaps to
//! a semitone anywhere in this module.

/// Millivolts per octave. Only used to size ranges, never to quantise.
pub const MV_PER_OCTAVE: i32 = 1000;

/// Pitch range, selected by the Z switch.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PitchRange {
    /// Switch middle: one octave of span.
    OneOctave,
    /// Switch up (latching): two octaves of span.
    TwoOctaves,
}

impl PitchRange {
    /// Span in millivolts.
    pub fn span_mv(self) -> i32 {
        match self {
            PitchRange::OneOctave => MV_PER_OCTAVE,
            PitchRange::TwoOctaves => 2 * MV_PER_OCTAVE,
        }
    }
}

/// The voltage the range is centred on.
///
/// The outputs are bipolar (roughly ±6 V), so sitting at 0 V leaves headroom
/// in both directions and lets the patched VCO's own tuning decide the register.
/// Transposition is the oscillator's job, not ours.
pub const CENTRE_MV: i32 = 0;

/// Lowest and highest millivolts for a range, centred on [`CENTRE_MV`].
pub fn bounds_mv(range: PitchRange) -> (i32, i32) {
    let half = range.span_mv() / 2;
    (CENTRE_MV - half, CENTRE_MV + half)
}

/// Clamp a voltage into a range's bounds.
///
/// Clamping rather than wrapping: a wrapped pitch would leap an octave, which
/// is far more noticeable than a line flattening out at the top of its travel.
pub fn clamp_to_range(mv: i32, range: PitchRange) -> i32 {
    let (lo, hi) = bounds_mv(range);
    mv.clamp(lo, hi)
}

/// Fold a voltage back into range by reflecting at the boundaries.
///
/// Used by Arp-run: a run that walks off the top should turn around and come
/// back rather than stick to the ceiling, which would waste the rest of the
/// run on a repeated note.
pub fn fold_into_range(mv: i32, range: PitchRange) -> i32 {
    let (lo, hi) = bounds_mv(range);
    let span = hi - lo;
    if span <= 0 {
        return lo;
    }
    let mut v = mv;
    // Reflect repeatedly, in case the value is more than one span outside.
    let mut guard = 0;
    while (v < lo || v > hi) && guard < 8 {
        if v < lo {
            v = lo + (lo - v);
        }
        if v > hi {
            v = hi - (v - hi);
        }
        guard += 1;
    }
    v.clamp(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_octave_spans_a_volt() {
        assert_eq!(PitchRange::OneOctave.span_mv(), 1000);
        assert_eq!(PitchRange::TwoOctaves.span_mv(), 2000);
    }

    #[test]
    fn ranges_are_centred() {
        for r in [PitchRange::OneOctave, PitchRange::TwoOctaves] {
            let (lo, hi) = bounds_mv(r);
            assert_eq!(lo + hi, 2 * CENTRE_MV, "{r:?} not centred");
            assert_eq!(hi - lo, r.span_mv());
        }
    }

    #[test]
    fn both_ranges_fit_the_output() {
        // Outputs are roughly +/-6 V; both ranges must sit well inside.
        for r in [PitchRange::OneOctave, PitchRange::TwoOctaves] {
            let (lo, hi) = bounds_mv(r);
            assert!(lo > -6000 && hi < 6000, "{r:?} = {lo}..{hi} mV");
        }
    }

    #[test]
    fn clamp_holds_the_boundaries() {
        let r = PitchRange::OneOctave;
        let (lo, hi) = bounds_mv(r);
        assert_eq!(clamp_to_range(lo - 5000, r), lo);
        assert_eq!(clamp_to_range(hi + 5000, r), hi);
        assert_eq!(clamp_to_range(0, r), 0);
    }

    #[test]
    fn fold_reflects_rather_than_sticking() {
        let r = PitchRange::OneOctave;
        let (lo, hi) = bounds_mv(r);
        // 100 mV over the top should come back 100 mV under it, so a run
        // reverses instead of flattening.
        assert_eq!(fold_into_range(hi + 100, r), hi - 100);
        assert_eq!(fold_into_range(lo - 100, r), lo + 100);
    }

    #[test]
    fn fold_leaves_in_range_values_alone() {
        let r = PitchRange::TwoOctaves;
        for mv in [-999, -1, 0, 1, 999] {
            assert_eq!(fold_into_range(mv, r), mv);
        }
    }

    #[test]
    fn fold_always_lands_in_range() {
        // Including values several spans out, which the guard loop must handle.
        let r = PitchRange::OneOctave;
        let (lo, hi) = bounds_mv(r);
        for mv in [-20_000, -3000, -501, 501, 3000, 20_000] {
            let folded = fold_into_range(mv, r);
            assert!(
                (lo..=hi).contains(&folded),
                "{mv} folded to {folded}, outside {lo}..{hi}"
            );
        }
    }

    #[test]
    fn nothing_here_quantises() {
        // The point of the module. A semitone is 83.33 mV; values between
        // semitones must survive untouched.
        let r = PitchRange::TwoOctaves;
        for mv in [1, 7, 42, 137, 499] {
            assert_eq!(clamp_to_range(mv, r), mv, "{mv} mV was altered");
            assert_eq!(fold_into_range(mv, r), mv, "{mv} mV was altered");
        }
    }
}

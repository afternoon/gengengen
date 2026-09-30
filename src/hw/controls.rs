//! Turning raw ADC readings into usable control values.
//!
//! Two problems to solve here. The knobs do not reach the rails — raw ADC is
//! typically 14..4095 rather than 0..4095 — and an untouched knob jitters
//! between adjacent values. For a continuous parameter that jitter is
//! inaudible, but for selecting a *mode* or a *step count* it means the value
//! flickers between two neighbours, so quantised controls need hysteresis.

/// Raw ADC reading, 0..=4095.
pub type Raw = u16;

/// The ADC's nominal full scale.
pub const ADC_MAX: Raw = 4095;

/// Lowest raw reading a knob actually produces at the bottom of its travel.
///
/// Measured values in the wild are around 14. If your unit reads higher, raise
/// this or the bottom of the knob's travel will be a dead zone.
pub const KNOB_RAW_MIN: Raw = 14;

/// Stretch a raw knob reading to the full 0..=4095 range.
///
/// The knobs read roughly [`KNOB_RAW_MIN`]..4095 in practice, so without this
/// the extremes are unreachable — you could not get 16 steps out of the X knob,
/// or select the last mode with Y.
///
/// ComputerCard does the same job with `(raw * 130 - 60000) >> 11`, but that
/// expression maps into its own internal ~0..230 range, not 0..4095, so it is
/// not reusable here as-is.
pub fn stretch_knob(raw: Raw) -> Raw {
    if raw <= KNOB_RAW_MIN {
        return 0;
    }
    let span = (ADC_MAX - KNOB_RAW_MIN) as u32;
    let numerator = (raw - KNOB_RAW_MIN) as u32 * ADC_MAX as u32;
    // Round to nearest rather than truncating, so the top of travel reaches
    // exactly ADC_MAX.
    (((numerator + span / 2) / span) as Raw).min(ADC_MAX)
}

/// Position of the three-position Z switch.
///
/// The switch is `(ON)-OFF-ON`: momentary when pushed **down**, latching when
/// pulled **up**. It is read as an analogue value through the mux rather than
/// as a GPIO, hence the thresholds.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SwitchPosition {
    /// Latching. Two-octave pitch range.
    Up,
    /// Resting. One-octave pitch range.
    Middle,
    /// Momentary — springs back. Regenerates the sequences.
    Down,
}

impl SwitchPosition {
    /// Classify a raw mux reading. Thresholds match ComputerCard and Dorsey's
    /// card, which agree here.
    pub fn from_raw(raw: Raw) -> Self {
        if raw > 3000 {
            SwitchPosition::Up
        } else if raw > 1000 {
            SwitchPosition::Middle
        } else {
            SwitchPosition::Down
        }
    }
}

/// A knob quantised to `N` discrete positions, with hysteresis so that jitter
/// at a boundary does not flip the value back and forth.
///
/// Used for mode selection (Y, 4 positions) and sequence length (X, 16
/// positions). Without the hysteresis, a knob resting exactly on a boundary
/// would switch mode every scan — and since mode changes are quantised to
/// pattern boundaries, that would mean a different mode every bar.
pub struct Quantised<const N: usize> {
    current: usize,
    /// Extra travel, in raw ADC counts, required to leave the current position.
    hysteresis: Raw,
}

impl<const N: usize> Quantised<N> {
    /// `hysteresis` is in raw ADC counts. A sensible default is a few percent
    /// of a position's width: for 4 positions that is ~1024 counts wide, so 64
    /// is comfortably stable without feeling sticky.
    pub fn new(hysteresis: Raw) -> Self {
        Self {
            current: 0,
            hysteresis,
        }
    }

    /// Current position, 0..N.
    pub fn position(&self) -> usize {
        self.current
    }

    /// Feed a stretched knob reading; returns the (possibly unchanged)
    /// position.
    pub fn update(&mut self, stretched: Raw) -> usize {
        let width = (ADC_MAX as u32 + 1) / N as u32;

        // Where the reading sits if we ignore where we already are.
        let naive = ((stretched as u32) / width).min(N as u32 - 1) as usize;
        if naive == self.current {
            return self.current;
        }

        // Require the reading to be past the boundary by the hysteresis amount
        // before we accept the move, so a value hovering on a boundary stays
        // put rather than oscillating.
        let boundary = if naive > self.current {
            // Moving up: the boundary is the top of the current position.
            (self.current as u32 + 1) * width
        } else {
            // Moving down: the boundary is the bottom of the current position.
            self.current as u32 * width
        };

        let past_boundary = if naive > self.current {
            (stretched as u32) >= boundary + self.hysteresis as u32
        } else {
            (stretched as u32) + self.hysteresis as u32 <= boundary
        };

        if past_boundary {
            self.current = naive;
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_reaches_both_rails() {
        // The real complaint: a knob that physically bottoms out at
        // KNOB_RAW_MIN and tops out at 4095 must still be able to select
        // position 0 and position N-1.
        assert_eq!(stretch_knob(KNOB_RAW_MIN), 0);
        assert_eq!(stretch_knob(ADC_MAX), ADC_MAX);
    }

    #[test]
    fn stretch_covers_the_range_not_just_the_endpoints() {
        // Guards against a formula that happens to hit both rails while
        // compressing everything in between (the mistake of lifting
        // ComputerCard's differently-scaled expression).
        let mid = stretch_knob(ADC_MAX / 2);
        assert!(
            (1900..=2200).contains(&mid),
            "midpoint of travel mapped to {mid}, expected near 2048"
        );
    }

    #[test]
    fn stretch_uses_most_of_the_output_range() {
        let distinct_high = stretch_knob(4000);
        assert!(
            distinct_high > 3900,
            "near-full travel only reached {distinct_high}"
        );
    }

    #[test]
    fn stretch_clamps_below_the_floor() {
        assert_eq!(stretch_knob(0), 0);
        assert_eq!(stretch_knob(KNOB_RAW_MIN - 1), 0);
    }

    #[test]
    fn stretch_is_monotonic() {
        let mut prev = 0;
        for raw in 0..=ADC_MAX {
            let s = stretch_knob(raw);
            assert!(s >= prev, "stretch went backwards at raw={raw}");
            prev = s;
        }
    }

    #[test]
    fn switch_thresholds() {
        assert_eq!(SwitchPosition::from_raw(4095), SwitchPosition::Up);
        assert_eq!(SwitchPosition::from_raw(2000), SwitchPosition::Middle);
        assert_eq!(SwitchPosition::from_raw(0), SwitchPosition::Down);
    }

    #[test]
    fn quantised_spans_full_range() {
        let mut q = Quantised::<4>::new(0);
        assert_eq!(q.update(0), 0);
        assert_eq!(q.update(ADC_MAX), 3);
    }

    #[test]
    fn quantised_length_gives_all_sixteen() {
        // X knob must be able to select every length from 1 to 16 steps.
        let mut q = Quantised::<16>::new(0);
        let mut seen = [false; 16];
        for raw in 0..=ADC_MAX {
            seen[q.update(raw)] = true;
        }
        assert!(seen.iter().all(|&s| s), "not every step count reachable");
    }

    #[test]
    fn hysteresis_holds_position_against_jitter() {
        // A knob parked just below the 0/1 boundary, dithering by a few counts,
        // must not flip mode on every scan.
        let width = (ADC_MAX as u32 + 1) / 4;
        let boundary = width as Raw;
        let mut q = Quantised::<4>::new(64);

        q.update(boundary - 100); // settle in position 0
        assert_eq!(q.position(), 0);

        for delta in [-2i32, 1, -1, 2, 0, 3] {
            let jittered = (boundary as i32 + delta) as Raw;
            assert_eq!(
                q.update(jittered),
                0,
                "jitter at the boundary changed position"
            );
        }
    }

    #[test]
    fn hysteresis_still_allows_deliberate_moves() {
        let mut q = Quantised::<4>::new(64);
        q.update(0);
        assert_eq!(q.position(), 0);
        // A real turn well past the boundary must be honoured. Position 2 of 4
        // spans 2048..3071, so aim at its middle rather than at ADC_MAX/2,
        // which falls just inside position 1.
        assert_eq!(q.update(2560), 2);
    }

    #[test]
    fn hysteresis_allows_moving_back_down() {
        let mut q = Quantised::<4>::new(64);
        q.update(ADC_MAX);
        assert_eq!(q.position(), 3);
        assert_eq!(q.update(0), 0);
    }
}

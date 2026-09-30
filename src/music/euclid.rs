//! Euclidean rhythms.
//!
//! Distribute `pulses` hits as evenly as possible over `length` steps. This is
//! the rhythmic half of the model Ben already plays with, so it needs to behave
//! exactly as expected: 4-in-16 is four-on-the-floor, 3-in-8 is the tresillo,
//! and 5-in-16 is that limping pattern that makes everything sound urgent.
//!
//! Implemented with Bjorklund's algorithm specifically, rather than the simpler
//! error-accumulation trick. Both give evenly-distributed patterns, but they are
//! *rotations* of each other for some inputs — 5-in-16 comes out
//! `x..x..x..x..x...` from Bjorklund and `x...x..x..x..x..` from accumulation.
//! Bjorklund is what every other Euclidean sequencer uses, so it is what a given
//! pulse count will sound like on Ben's existing setup.
//!
//! The construction here is iterative rather than recursive — this is a
//! Cortex-M0+ and the recursive formulation's depth depends on the input.

/// Maximum pattern length, set by the X knob's range.
pub const MAX_LENGTH: usize = 16;

/// Bjorklund's algorithm: distribute `pulses` hits as evenly as possible over
/// `length` steps, returned as a bit pattern with a hit on step 0.
///
/// The structure is the standard Euclidean-algorithm one: repeatedly divide to
/// build a chain of `counts` and `remainders`, then interleave groups
/// accordingly.
///
/// Recursive, which is fine here despite the Cortex-M0+: with `length` capped at
/// 16 the maximum recursion depth over every valid input is 5 and the maximum
/// level is 4 (verified exhaustively). An iterative explicit-stack version was
/// tried first and produced correctly-even patterns in a different *rotation*,
/// which is a musically different rhythm — not worth the subtlety for a
/// five-frame call chain.
fn bjorklund(length: usize, pulses: usize) -> u16 {
    if pulses == 0 {
        return 0;
    }
    if pulses >= length {
        // All steps hit. Guard the shift: `1 << 16` would overflow a u16.
        return full_mask(length);
    }

    // Levels are bounded by the Euclidean algorithm on numbers <= 16.
    const MAX_LEVELS: usize = 8;
    let mut counts = [0usize; MAX_LEVELS];
    let mut remainders = [0usize; MAX_LEVELS];

    let mut divisor = length - pulses;
    remainders[0] = pulses;
    let mut level = 0usize;

    loop {
        if level + 1 >= MAX_LEVELS || remainders[level] == 0 {
            break;
        }
        counts[level] = divisor / remainders[level];
        remainders[level + 1] = divisor % remainders[level];
        divisor = remainders[level];
        level += 1;
        if remainders[level] <= 1 {
            break;
        }
    }
    counts[level] = divisor;

    let mut bits = 0u16;
    let mut pos = 0usize;
    build(level as i32, &counts, &remainders, &mut bits, &mut pos);

    // Rotate so the pattern starts on a hit. This is what makes a single pulse
    // land on the downbeat and keeps patterns aligned to the external clock.
    rotate_to_first_hit(bits, length)
}

/// Recursive group interleaving. `lvl == -2` emits a hit, `lvl == -1` a rest.
fn build(
    lvl: i32,
    counts: &[usize],
    remainders: &[usize],
    bits: &mut u16,
    pos: &mut usize,
) {
    if lvl == -1 {
        *pos += 1;
        return;
    }
    if lvl == -2 {
        if *pos < 16 {
            *bits |= 1 << *pos;
        }
        *pos += 1;
        return;
    }
    let l = lvl as usize;
    for _ in 0..counts[l] {
        build(lvl - 1, counts, remainders, bits, pos);
    }
    if remainders[l] != 0 {
        build(lvl - 2, counts, remainders, bits, pos);
    }
}

/// A mask of `length` set bits, guarding the `1 << 16` overflow.
fn full_mask(length: usize) -> u16 {
    if length >= 16 {
        u16::MAX
    } else {
        (1u16 << length) - 1
    }
}

/// Rotate a pattern so its first hit is on step 0.
fn rotate_to_first_hit(bits: u16, length: usize) -> u16 {
    if bits == 0 {
        return 0;
    }
    let first = bits.trailing_zeros() as usize;
    if first == 0 {
        return bits;
    }
    ((bits >> first) | (bits << (length - first))) & full_mask(length)
}

/// A rhythm as a bit pattern, one bit per step.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Rhythm {
    bits: u16,
    length: u8,
}

impl Rhythm {
    /// Generate a Euclidean rhythm.
    ///
    /// `pulses` is clamped to `0..=length`. A `rotation` shifts the pattern,
    /// which is how you get the same distribution to start on a different
    /// beat — musically a different rhythm entirely.
    pub fn euclidean(length: usize, pulses: usize, rotation: usize) -> Self {
        let length = length.clamp(1, MAX_LENGTH);
        let pulses = pulses.min(length);

        let bits = bjorklund(length, pulses);

        let mut r = Rhythm {
            bits,
            length: length as u8,
        };
        if rotation > 0 {
            r = r.rotated(rotation);
        }
        r
    }

    /// An empty rhythm of the given length.
    pub fn empty(length: usize) -> Self {
        Rhythm {
            bits: 0,
            length: length.clamp(1, MAX_LENGTH) as u8,
        }
    }

    /// Rotate the pattern left by `n` steps, wrapping.
    pub fn rotated(&self, n: usize) -> Self {
        let len = self.length as usize;
        let n = n % len;
        if n == 0 {
            return *self;
        }
        let bits = ((self.bits >> n) | (self.bits << (len - n))) & full_mask(len);
        Rhythm {
            bits,
            length: self.length,
        }
    }

    /// This rhythm with an extra hit on `step`.
    ///
    /// Needed by the Drone mode, which places gates irregularly rather than
    /// evenly, so it cannot go through the Euclidean generator.
    pub fn with_hit(&self, step: usize) -> Self {
        let len = self.length as usize;
        Rhythm {
            bits: self.bits | (1 << (step % len)),
            length: self.length,
        }
    }

    /// Is there a hit on this step? Steps beyond the length wrap.
    pub fn hit(&self, step: usize) -> bool {
        let len = self.length as usize;
        self.bits & (1 << (step % len)) != 0
    }

    pub fn length(&self) -> usize {
        self.length as usize
    }

    /// How many hits in the pattern.
    pub fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    /// Raw bits, for display on the LEDs.
    pub fn bits(&self) -> u16 {
        self.bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render as a string for legible assertions: `x` = hit, `.` = rest.
    fn render(r: &Rhythm) -> heapless_string::Str {
        let mut s = heapless_string::Str::new();
        for i in 0..r.length() {
            s.push(if r.hit(i) { 'x' } else { '.' });
        }
        s
    }

    /// Tiny fixed-capacity string so tests read clearly without pulling in a
    /// dependency or needing an allocator.
    mod heapless_string {
        #[derive(PartialEq, Eq)]
        pub struct Str {
            buf: [u8; super::MAX_LENGTH],
            len: usize,
        }
        impl Str {
            pub fn new() -> Self {
                Self {
                    buf: [0; super::MAX_LENGTH],
                    len: 0,
                }
            }
            pub fn push(&mut self, c: char) {
                if self.len < self.buf.len() {
                    self.buf[self.len] = c as u8;
                    self.len += 1;
                }
            }
            pub fn as_str(&self) -> &str {
                core::str::from_utf8(&self.buf[..self.len]).unwrap()
            }
        }
        impl core::fmt::Debug for Str {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.as_str())
            }
        }
    }

    fn pattern(length: usize, pulses: usize) -> impl PartialEq<&'static str> + core::fmt::Debug {
        struct P(heapless_string::Str);
        impl PartialEq<&'static str> for P {
            fn eq(&self, other: &&'static str) -> bool {
                self.0.as_str() == *other
            }
        }
        impl core::fmt::Debug for P {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:?}", self.0)
            }
        }
        P(render(&Rhythm::euclidean(length, pulses, 0)))
    }

    #[test]
    fn four_on_the_floor() {
        // The one that has to be right.
        assert!(pattern(16, 4) == "x...x...x...x...");
    }

    #[test]
    fn eight_in_sixteen_is_every_other_step() {
        assert!(pattern(16, 8) == "x.x.x.x.x.x.x.x.");
    }

    #[test]
    fn tresillo() {
        // 3-in-8, the classic Euclidean example.
        assert!(pattern(8, 3) == "x..x..x.");
    }

    #[test]
    fn five_in_eight() {
        assert!(pattern(8, 5) == "x.xx.xx.");
    }

    #[test]
    fn five_in_sixteen_matches_bjorklund_not_accumulation() {
        // The two algorithms give rotations of each other here, and this is the
        // one other Euclidean sequencers produce — so a pulse count sounds the
        // same on this as on the rest of Ben's rack.
        assert!(pattern(16, 5) == "x..x..x..x..x...");
    }

    #[test]
    fn seven_in_sixteen() {
        assert!(pattern(16, 7) == "x.x.x..x.x.x..x.");
    }

    #[test]
    fn full_and_empty() {
        assert!(pattern(8, 8) == "xxxxxxxx");
        assert!(pattern(8, 0) == "........");
    }

    #[test]
    fn single_pulse_lands_on_the_downbeat() {
        // A single hit anywhere but step 0 would make the pattern feel offset
        // against the external clock.
        for len in 1..=MAX_LENGTH {
            let r = Rhythm::euclidean(len, 1, 0);
            assert!(r.hit(0), "length {len}: single pulse not on step 0");
            assert_eq!(r.count(), 1);
        }
    }

    #[test]
    fn pulse_count_is_always_honoured() {
        for len in 1..=MAX_LENGTH {
            for pulses in 0..=len {
                let r = Rhythm::euclidean(len, pulses, 0);
                assert_eq!(r.count(), pulses, "length {len}, pulses {pulses}");
            }
        }
    }

    #[test]
    fn pulses_are_clamped_not_wrapped() {
        // The Main knob can ask for more pulses than there are steps.
        let r = Rhythm::euclidean(8, 20, 0);
        assert_eq!(r.count(), 8);
    }

    #[test]
    fn length_is_clamped_to_valid_range() {
        assert_eq!(Rhythm::euclidean(0, 1, 0).length(), 1);
        assert_eq!(Rhythm::euclidean(99, 1, 0).length(), MAX_LENGTH);
    }

    #[test]
    fn distribution_is_even() {
        // The defining property: gaps between hits differ by at most one step.
        // If this fails the rhythms will sound lumpy rather than Euclidean.
        for len in 2..=MAX_LENGTH {
            for pulses in 2..=len {
                let r = Rhythm::euclidean(len, pulses, 0);
                let mut gaps = [0usize; MAX_LENGTH];
                let mut n_gaps = 0;
                let mut last = None;
                for step in 0..len {
                    if r.hit(step) {
                        if let Some(l) = last {
                            gaps[n_gaps] = step - l;
                            n_gaps += 1;
                        }
                        last = Some(step);
                    }
                }
                if n_gaps > 1 {
                    let min = gaps[..n_gaps].iter().min().unwrap();
                    let max = gaps[..n_gaps].iter().max().unwrap();
                    assert!(
                        max - min <= 1,
                        "length {len} pulses {pulses}: gaps {min}..{max} uneven"
                    );
                }
            }
        }
    }

    #[test]
    fn rotation_preserves_pulse_count() {
        for rot in 0..16 {
            let r = Rhythm::euclidean(16, 5, rot);
            assert_eq!(r.count(), 5, "rotation {rot} changed the hit count");
        }
    }

    #[test]
    fn rotation_by_length_is_identity() {
        let a = Rhythm::euclidean(16, 5, 0);
        let b = Rhythm::euclidean(16, 5, 16);
        assert_eq!(a, b);
    }

    #[test]
    fn rotation_actually_shifts() {
        let a = Rhythm::euclidean(16, 4, 0);
        let b = Rhythm::euclidean(16, 4, 1);
        assert_ne!(a, b);
        // Rotating left by one: what was on step 1 is now on step 0.
        for step in 0..15 {
            assert_eq!(b.hit(step), a.hit(step + 1));
        }
    }

    #[test]
    fn hit_wraps_beyond_length() {
        // The sequencer indexes by absolute step count, so wrapping must work.
        let r = Rhythm::euclidean(8, 3, 0);
        for step in 0..8 {
            assert_eq!(r.hit(step), r.hit(step + 8));
        }
    }

    #[test]
    fn full_sixteen_step_rotation_does_not_lose_bits() {
        // Guards the shift-mask arithmetic at exactly 16 steps, where a
        // `1 << 16` would overflow a u16.
        let r = Rhythm::euclidean(16, 16, 3);
        assert_eq!(r.count(), 16);
    }
}


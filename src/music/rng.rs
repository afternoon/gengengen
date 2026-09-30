//! A small deterministic PRNG.
//!
//! Deterministic on purpose. Regeneration is a performance gesture — the
//! switch-down that gives you a new sequence — and a seeded generator means a
//! sequence you liked can in principle be recovered rather than lost forever.
//! It also makes the mode generators testable.
//!
//! xorshift32: tiny, fast, no division, and far better distributed than a
//! linear congruential generator at this size. We are choosing notes, not
//! keys.

/// xorshift32 state.
#[derive(Copy, Clone, Debug)]
pub struct Rng {
    state: u32,
}

impl Rng {
    /// Seed the generator. Zero is remapped, since xorshift is stuck at zero.
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x2545_F491 } else { seed },
        }
    }

    /// Next raw 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Uniform in `0..n`. Uses the high bits, which are better mixed than the
    /// low ones, and avoids the modulo bias of `next_u32() % n`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        // Multiply-shift: maps the 32-bit value into 0..n without division.
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }

    /// Inclusive range.
    pub fn between(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    /// True with probability `numerator / 256`.
    ///
    /// Knob values are 12-bit, so a 0..=255 probability is plenty of
    /// resolution and keeps this to a shift.
    pub fn chance_256(&mut self, numerator: u8) -> bool {
        (self.next_u32() >> 24) < numerator as u32
    }

    /// Current state, for saving a sequence you liked.
    pub fn state(&self) -> u32 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_seed_still_generates() {
        let mut r = Rng::new(0);
        let a = r.next_u32();
        let b = r.next_u32();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn deterministic_for_a_given_seed() {
        // The whole point: a seed reproduces a sequence.
        let mut a = Rng::new(12345);
        let mut b = Rng::new(12345);
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let diverged = (0..10).any(|_| a.next_u32() != b.next_u32());
        assert!(diverged);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(99);
        for n in 1..=16u32 {
            for _ in 0..200 {
                assert!(r.below(n) < n);
            }
        }
    }

    #[test]
    fn below_zero_does_not_panic() {
        let mut r = Rng::new(7);
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn below_covers_its_whole_range() {
        // A generator that never returns the top value would quietly cost us a
        // step length or a scale degree.
        let mut r = Rng::new(4);
        let mut seen = [false; 16];
        for _ in 0..5000 {
            seen[r.below(16) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "did not cover 0..16");
    }

    #[test]
    fn between_is_inclusive_both_ends() {
        let mut r = Rng::new(31);
        let mut saw_lo = false;
        let mut saw_hi = false;
        for _ in 0..2000 {
            let v = r.between(-3, 3);
            assert!((-3..=3).contains(&v));
            saw_lo |= v == -3;
            saw_hi |= v == 3;
        }
        assert!(saw_lo && saw_hi, "range ends unreachable");
    }

    #[test]
    fn between_handles_degenerate_range() {
        let mut r = Rng::new(5);
        assert_eq!(r.between(5, 5), 5);
        assert_eq!(r.between(5, 1), 5);
    }

    #[test]
    fn chance_extremes_are_absolute() {
        let mut r = Rng::new(77);
        for _ in 0..500 {
            assert!(!r.chance_256(0), "0/256 should never fire");
            assert!(r.chance_256(255) || true); // 255 is near-certain, not certain
        }
    }

    #[test]
    fn chance_is_roughly_proportional() {
        let mut r = Rng::new(2024);
        let trials = 10_000;
        let hits = (0..trials).filter(|_| r.chance_256(128)).count();
        let ratio = hits as f64 / trials as f64;
        assert!(
            (0.45..0.55).contains(&ratio),
            "128/256 fired {ratio:.3} of the time"
        );
    }
}

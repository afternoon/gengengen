//! Voltage slewing, for the Drone mode's glides.
//!
//! The danceable modes step: the pitch CV jumps to its new value on the clock
//! edge and stays there. The Drone mode glides, which is most of what makes it
//! read as texture rather than as a slow sequence.
//!
//! Linear rather than exponential. A real slew limiter is exponential and that
//! is arguably more musical, but it needs either a division or a multiply per
//! update at control rate, and linear interpolation over a known tick count is
//! exact, terminates precisely on target, and is trivially testable. On a glide
//! lasting several seconds the difference is not the interesting part.

/// A voltage heading toward a target over a number of ticks.
#[derive(Copy, Clone, Debug)]
pub struct Slew {
    current_mv: i32,
    target_mv: i32,
    /// Ticks remaining to reach the target.
    remaining: u16,
    /// Total ticks the current glide was given, for interpolation.
    total: u16,
    /// Where the glide started.
    start_mv: i32,
}

impl Slew {
    /// A slew already settled at `mv`.
    pub fn at(mv: i32) -> Self {
        Self {
            current_mv: mv,
            target_mv: mv,
            remaining: 0,
            total: 0,
            start_mv: mv,
        }
    }

    /// Head for `target_mv` over `ticks`.
    ///
    /// `ticks == 0` jumps immediately, which is what the danceable modes use —
    /// so stepping and gliding are the same code path with a different argument
    /// rather than two mechanisms.
    pub fn glide_to(&mut self, target_mv: i32, ticks: u16) {
        self.start_mv = self.current_mv;
        self.target_mv = target_mv;
        self.total = ticks;
        self.remaining = ticks;
        if ticks == 0 {
            self.current_mv = target_mv;
        }
    }

    /// Advance one tick. Returns the current voltage.
    pub fn tick(&mut self) -> i32 {
        if self.remaining == 0 {
            self.current_mv = self.target_mv;
            return self.current_mv;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            // Land exactly on target rather than accumulating rounding error.
            self.current_mv = self.target_mv;
        } else {
            let elapsed = (self.total - self.remaining) as i64;
            let span = (self.target_mv - self.start_mv) as i64;
            self.current_mv =
                self.start_mv + ((span * elapsed) / self.total as i64) as i32;
        }
        self.current_mv
    }

    pub fn current_mv(&self) -> i32 {
        self.current_mv
    }

    /// Has the glide finished?
    pub fn settled(&self) -> bool {
        self.remaining == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_ticks_jumps_immediately() {
        // The danceable modes rely on this: stepping is a glide of length 0.
        let mut s = Slew::at(0);
        s.glide_to(1000, 0);
        assert_eq!(s.current_mv(), 1000);
        assert!(s.settled());
    }

    #[test]
    fn reaches_target_exactly() {
        // Rounding must not leave a glide slightly short, or held drone notes
        // would sit a few millivolts flat.
        for ticks in 1..=64u16 {
            let mut s = Slew::at(-500);
            s.glide_to(700, ticks);
            for _ in 0..ticks {
                s.tick();
            }
            assert_eq!(s.current_mv(), 700, "ticks={ticks} did not land on target");
            assert!(s.settled());
        }
    }

    #[test]
    fn moves_monotonically_toward_target() {
        let mut s = Slew::at(0);
        s.glide_to(1000, 16);
        let mut prev = s.current_mv();
        for _ in 0..16 {
            let now = s.tick();
            assert!(now >= prev, "slew went backwards: {prev} -> {now}");
            prev = now;
        }
    }

    #[test]
    fn descending_glide_works() {
        let mut s = Slew::at(1000);
        s.glide_to(-1000, 8);
        let mut prev = s.current_mv();
        for _ in 0..8 {
            let now = s.tick();
            assert!(now <= prev);
            prev = now;
        }
        assert_eq!(s.current_mv(), -1000);
    }

    #[test]
    fn stays_put_after_settling() {
        let mut s = Slew::at(0);
        s.glide_to(500, 4);
        for _ in 0..10 {
            s.tick();
        }
        assert_eq!(s.current_mv(), 500);
    }

    #[test]
    fn midpoint_is_roughly_halfway() {
        let mut s = Slew::at(0);
        s.glide_to(1000, 10);
        for _ in 0..5 {
            s.tick();
        }
        let mid = s.current_mv();
        assert!((400..=600).contains(&mid), "midpoint was {mid}");
    }

    #[test]
    fn retargeting_mid_glide_starts_from_current() {
        // Turning the knob mid-glide must not snap back to the old start.
        let mut s = Slew::at(0);
        s.glide_to(1000, 10);
        for _ in 0..5 {
            s.tick();
        }
        let interrupted = s.current_mv();
        s.glide_to(-1000, 10);
        // First tick of the new glide should move away from `interrupted`
        // toward the new target, not jump elsewhere.
        let next = s.tick();
        assert!(next < interrupted, "{interrupted} -> {next} should descend");
    }
}

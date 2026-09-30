//! The sequencer engine: what happens on each clock tick.
//!
//! Externally clocked via Pulse In 1, so there is no internal tempo — the engine
//! is purely reactive. One clock pulse is one step.
//!
//! The important behaviour here is *when changes take effect*. Turning the mode
//! or length knob mid-bar must not drop a gate in the wrong place, so pending
//! changes are held and applied at the next pattern boundary. The switch is the
//! override for when you want it now.

use crate::music::modes::{generate, GenParams, Mode, Pattern, PatternPair, StepEvent};
use crate::music::rng::Rng;
use crate::music::voltage::PitchRange;
use crate::seq::slew::Slew;

/// Front-panel state, sampled from the knobs and switch.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Controls {
    pub mode: Mode,
    /// 1..=16 from the X knob.
    pub length: usize,
    /// 0..=4095 from the Main knob.
    pub main: u16,
    pub range: PitchRange,
}

/// One voice's live output state.
#[derive(Copy, Clone, Debug)]
pub struct VoiceOut {
    /// Is the gate high right now?
    pub gate: bool,
    pub pitch_mv: i32,
    pub aux_mv: i32,
}

/// Both voices' outputs.
#[derive(Copy, Clone, Debug)]
pub struct Outputs {
    pub a: VoiceOut,
    pub b: VoiceOut,
}

/// Per-voice runtime state: where the gate is in its life, and the pitch glide.
#[derive(Copy, Clone, Debug)]
struct VoiceState {
    /// Ticks of gate remaining. 0 = gate low.
    gate_remaining: u16,
    pitch: Slew,
    aux: Slew,
}

impl VoiceState {
    fn new() -> Self {
        Self {
            gate_remaining: 0,
            pitch: Slew::at(0),
            aux: Slew::at(0),
        }
    }

    /// Apply a step's event.
    fn begin_step(&mut self, ev: &StepEvent) {
        if ev.gate {
            // Retrigger: a new gate replaces whatever was still running, so a
            // long gate never swallows the next hit.
            self.gate_remaining = ev.gate_ticks.max(1);
            self.pitch.glide_to(ev.pitch_mv, ev.slew_ticks);
            self.aux.glide_to(ev.aux_mv, ev.slew_ticks);
        }
    }

    /// Advance one tick and report the outputs.
    fn tick(&mut self) -> VoiceOut {
        let gate = self.gate_remaining > 0;
        if self.gate_remaining > 0 {
            self.gate_remaining -= 1;
        }
        VoiceOut {
            gate,
            pitch_mv: self.pitch.tick(),
            aux_mv: self.aux.tick(),
        }
    }
}

/// The sequencer.
pub struct Engine {
    /// Currently playing patterns.
    patterns: PatternPair,
    /// The controls the current patterns were generated from.
    active: Controls,
    /// Controls that will take effect at the next pattern boundary.
    pending: Option<Controls>,
    /// Absolute step count since the last regeneration.
    step: usize,
    a: VoiceState,
    b: VoiceState,
    rng: Rng,
}

impl Engine {
    pub fn new(controls: Controls, seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let params = GenParams {
            mode: controls.mode,
            length: controls.length,
            main: controls.main,
            range: controls.range,
        };
        let patterns = generate(&params, &mut rng);
        Self {
            patterns,
            active: controls,
            pending: None,
            step: 0,
            a: VoiceState::new(),
            b: VoiceState::new(),
            rng,
        }
    }

    /// Note new control positions.
    ///
    /// Mode and length changes are deferred to the next pattern boundary; the
    /// Main knob takes effect on the next regeneration, since re-running the
    /// generator every time the knob moves would make the pattern unstable
    /// under your hand.
    pub fn set_controls(&mut self, controls: Controls) {
        if controls == self.active {
            // Nothing to do, and importantly: clear any stale pending change so
            // turning a knob and turning it back does not queue a regeneration.
            self.pending = None;
            return;
        }
        self.pending = Some(controls);
    }

    /// Apply pending changes immediately, rather than waiting for the boundary.
    ///
    /// This is the switch-down override, and it also regenerates — which is what
    /// makes one gesture do the obvious thing: "give me something new, now".
    pub fn force_now(&mut self) {
        if let Some(p) = self.pending.take() {
            self.active = p;
        }
        self.regenerate();
        self.step = 0;
    }

    /// Generate new patterns from the active controls.
    pub fn regenerate(&mut self) {
        let params = GenParams {
            mode: self.active.mode,
            length: self.active.length,
            main: self.active.main,
            range: self.active.range,
        };
        self.patterns = generate(&params, &mut self.rng);
    }

    /// Advance one clock pulse. Call this on each rising edge of Pulse In 1.
    pub fn clock(&mut self) -> Outputs {
        // At a pattern boundary, adopt any pending control change. Doing this
        // before reading the step means the new pattern starts cleanly on step
        // 0 rather than mid-phrase.
        if self.at_boundary() {
            if let Some(p) = self.pending.take() {
                let length_changed = p.length != self.active.length;
                let mode_changed = p.mode != self.active.mode;
                self.active = p;
                // A new mode or length needs new material; a Main-knob-only
                // change reshapes the existing pattern on the next regenerate,
                // so we do not throw away a sequence the player likes just
                // because they nudged a knob.
                if mode_changed || length_changed {
                    self.regenerate();
                    self.step = 0;
                }
            }
        }

        let ev_a = self.patterns.a.event_at(self.step);
        let ev_b = self.patterns.b.event_at(self.step);
        self.a.begin_step(&ev_a);
        self.b.begin_step(&ev_b);

        self.step = self.step.wrapping_add(1);

        Outputs {
            a: self.a.tick(),
            b: self.b.tick(),
        }
    }

    /// Advance the gate/slew state without consuming a step.
    ///
    /// Gate lengths and slews are measured in step-clock ticks, so between
    /// clock pulses there is nothing to interpolate — this exists for the case
    /// where the output task runs faster than the clock and wants the glide to
    /// move smoothly rather than in step-sized jumps.
    pub fn sub_tick(&mut self) -> Outputs {
        Outputs {
            a: VoiceOut {
                gate: self.a.gate_remaining > 0,
                pitch_mv: self.a.pitch.tick(),
                aux_mv: self.a.aux.tick(),
            },
            b: VoiceOut {
                gate: self.b.gate_remaining > 0,
                pitch_mv: self.b.pitch.tick(),
                aux_mv: self.b.aux.tick(),
            },
        }
    }

    /// Are we at the start of a pattern?
    fn at_boundary(&self) -> bool {
        self.step.is_multiple_of(self.active.length.max(1))
    }

    pub fn active(&self) -> Controls {
        self.active
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Current step within the pattern, for the LEDs.
    pub fn step_in_pattern(&self) -> usize {
        self.step % self.active.length.max(1)
    }

    pub fn pattern_a(&self) -> &Pattern {
        &self.patterns.a
    }

    pub fn pattern_b(&self) -> &Pattern {
        &self.patterns.b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(mode: Mode, length: usize) -> Controls {
        Controls {
            mode,
            length,
            main: 2048,
            range: PitchRange::OneOctave,
        }
    }

    #[test]
    fn clocking_advances_through_the_pattern() {
        let mut e = Engine::new(controls(Mode::EuclidTuring, 8), 1);
        for expected in 1..=8 {
            e.clock();
            assert_eq!(e.step_in_pattern(), expected % 8);
        }
    }

    #[test]
    fn gates_fire_on_pattern_hits() {
        // With all 16 steps pulsing, every clock should produce a gate.
        let mut c = controls(Mode::EuclidTuring, 16);
        c.main = 4095; // maximum pulses
        let mut e = Engine::new(c, 2);
        for step in 0..16 {
            let out = e.clock();
            assert!(out.a.gate, "step {step} should gate");
        }
    }

    #[test]
    fn gate_length_holds_across_ticks() {
        // A gate_ticks of n should keep the gate high for n clocks. Drone
        // depends on this, and so do ties in the danceable modes.
        let mut e = Engine::new(controls(Mode::Drone, 16), 3);
        let mut high = 0;
        for _ in 0..16 {
            if e.clock().a.gate {
                high += 1;
            }
        }
        assert!(high > 1, "drone gate did not hold: only {high} ticks high");
    }

    #[test]
    fn mode_change_waits_for_the_pattern_boundary() {
        // The core live-performance requirement: turning the mode knob mid-bar
        // must not take effect until the bar ends.
        let mut e = Engine::new(controls(Mode::EuclidTuring, 8), 4);
        e.clock(); // step 1, mid-pattern
        e.set_controls(controls(Mode::Drone, 8));
        assert!(e.has_pending());
        assert_eq!(e.active().mode, Mode::EuclidTuring, "changed too early");

        // Clocks 2..=8 finish the current pattern; the change must not land on
        // any of them.
        for _ in 0..7 {
            e.clock();
            assert_eq!(e.active().mode, Mode::EuclidTuring, "changed mid-pattern");
        }
        // Clock 9 is the one that begins the next pattern, so it adopts.
        e.clock();
        assert_eq!(e.active().mode, Mode::Drone, "did not change at boundary");
        assert!(!e.has_pending());
    }

    #[test]
    fn length_change_also_waits() {
        let mut e = Engine::new(controls(Mode::EuclidTuring, 8), 5);
        e.clock();
        e.set_controls(controls(Mode::EuclidTuring, 16));
        assert_eq!(e.active().length, 8);
        // Same timing as a mode change: adopted by the clock that starts the
        // next pattern, which is the 9th after construction.
        for _ in 0..7 {
            e.clock();
            assert_eq!(e.active().length, 8, "changed mid-pattern");
        }
        e.clock();
        assert_eq!(e.active().length, 16);
    }

    #[test]
    fn force_now_applies_immediately() {
        // The switch-down override.
        let mut e = Engine::new(controls(Mode::EuclidTuring, 8), 6);
        e.clock();
        e.set_controls(controls(Mode::Drone, 8));
        e.force_now();
        assert_eq!(e.active().mode, Mode::Drone);
        assert!(!e.has_pending());
        assert_eq!(e.step_in_pattern(), 0, "should restart the pattern");
    }

    #[test]
    fn returning_a_knob_cancels_the_pending_change() {
        // Nudging a knob and putting it back should not queue a regeneration
        // that throws away the current sequence.
        let mut e = Engine::new(controls(Mode::EuclidTuring, 8), 7);
        e.clock();
        e.set_controls(controls(Mode::Drone, 8));
        assert!(e.has_pending());
        e.set_controls(controls(Mode::EuclidTuring, 8));
        assert!(!e.has_pending(), "pending change should have been cancelled");
    }

    #[test]
    fn regenerate_changes_the_material() {
        let mut e = Engine::new(controls(Mode::EuclidTuring, 16), 8);
        let before = e.pattern_a().pitch_mv;
        e.regenerate();
        let after = e.pattern_a().pitch_mv;
        assert_ne!(before, after, "regenerate produced identical voltages");
    }

    #[test]
    fn regenerate_keeps_the_rhythm_for_the_same_settings() {
        // Euclidean rhythm is a function of length and pulses, so regenerating
        // should give new pitches over the same groove rather than a different
        // rhythm - that is what makes switch-down usable mid-phrase.
        let mut e = Engine::new(controls(Mode::EuclidTuring, 16), 9);
        let rhythm_before = e.pattern_a().rhythm;
        e.regenerate();
        assert_eq!(rhythm_before, e.pattern_a().rhythm);
    }

    #[test]
    fn pitch_steps_immediately_in_danceable_modes() {
        let mut e = Engine::new(controls(Mode::ArpRun, 16), 10);
        let out = e.clock();
        // Arp-run has slew 0, so the first tick should already be at the
        // pattern's first pitch rather than gliding toward it.
        assert_eq!(out.a.pitch_mv, e.pattern_a().pitch_mv[0]);
    }

    #[test]
    fn pitch_glides_in_drone_mode() {
        let mut e = Engine::new(controls(Mode::Drone, 16), 11);
        let first = e.clock();
        // With a slew, the voltage should not arrive instantly.
        let target = e.pattern_a().pitch_mv[0];
        if e.pattern_a().slew_ticks[0] > 1 {
            assert_ne!(
                first.a.pitch_mv, target,
                "drone pitch arrived instantly despite slew"
            );
        }
    }

    #[test]
    fn survives_a_long_run_at_every_mode_and_length() {
        // A dead module mid-set is the worst failure, so hammer the state
        // machine across every combination.
        for mode in Mode::ALL {
            for length in 1..=16 {
                let mut e = Engine::new(controls(mode, length), 12);
                for i in 0..200 {
                    e.clock();
                    if i % 37 == 0 {
                        e.set_controls(controls(mode, (length % 16) + 1));
                    }
                    if i % 53 == 0 {
                        e.force_now();
                    }
                }
            }
        }
    }

    #[test]
    fn step_counter_does_not_overflow_in_a_long_set() {
        // wrapping_add plus modulo must stay correct far past any real set.
        let mut e = Engine::new(controls(Mode::EuclidTuring, 16), 13);
        for _ in 0..100_000 {
            e.clock();
        }
        assert!(e.step_in_pattern() < 16);
    }
}

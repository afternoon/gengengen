//! The four sequence generation modes, selected by the Y knob.
//!
//! Three danceable, one not. Each one interprets the Main knob differently;
//! X is always sequence length and the Z switch is always
//! regenerate/contrast, so those live outside the modes themselves.
//!
//! All four produce voltages rather than notes. Nothing is quantised.

use crate::music::euclid::{Rhythm, MAX_LENGTH};
use crate::music::rng::Rng;
use crate::music::voltage::{bounds_mv, fold_into_range, PitchRange};

/// Which generator is running.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Euclidean gates, random-walk voltage. The existing model.
    EuclidTuring,
    /// Runs of adjacent voltage steps. Basilisk-inspired.
    ArpRun,
    /// Two voices in dialogue across both output sets.
    CallResponse,
    /// The non-danceable one: sparse long gates, gliding voltage.
    Drone,
}

impl Mode {
    /// In Y-knob order.
    pub const ALL: [Mode; 4] = [
        Mode::EuclidTuring,
        Mode::ArpRun,
        Mode::CallResponse,
        Mode::Drone,
    ];

    /// For the display/LEDs.
    pub fn name(self) -> &'static str {
        match self {
            Mode::EuclidTuring => "euclid",
            Mode::ArpRun => "arp-run",
            Mode::CallResponse => "call-resp",
            Mode::Drone => "drone",
        }
    }

    /// Is this mode intended to be danceable? The one `false` here is what
    /// makes the mode set work as a performance tool rather than four flavours.
    pub fn is_danceable(self) -> bool {
        !matches!(self, Mode::Drone)
    }

    pub fn from_position(pos: usize) -> Mode {
        Mode::ALL[pos.min(Mode::ALL.len() - 1)]
    }
}

/// What one step tells the outputs to do.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct StepEvent {
    /// Fire a gate this step.
    pub gate: bool,
    /// How long to hold it, in ticks of the step clock. 1 = a single step.
    /// Values above 1 produce ties; the Drone mode uses long ones.
    pub gate_ticks: u16,
    /// Pitch voltage in millivolts, unquantised.
    pub pitch_mv: i32,
    /// The auxiliary CV in millivolts — accent or timbre, per patching.
    pub aux_mv: i32,
    /// Slew time to reach `pitch_mv`, in ticks. 0 = stepped, which is what the
    /// danceable modes want; the Drone mode glides.
    pub slew_ticks: u16,
}

/// One voice's generated sequence: a rhythm plus per-step voltages.
///
/// Fixed-size arrays, no allocation — this is a microcontroller and the maximum
/// length is known.
#[derive(Copy, Clone, Debug)]
pub struct Pattern {
    pub rhythm: Rhythm,
    pub pitch_mv: [i32; MAX_LENGTH],
    pub aux_mv: [i32; MAX_LENGTH],
    pub gate_ticks: [u16; MAX_LENGTH],
    pub slew_ticks: [u16; MAX_LENGTH],
    pub length: usize,
}

impl Pattern {
    pub fn empty(length: usize) -> Self {
        Self {
            rhythm: Rhythm::empty(length),
            pitch_mv: [0; MAX_LENGTH],
            aux_mv: [0; MAX_LENGTH],
            gate_ticks: [1; MAX_LENGTH],
            slew_ticks: [0; MAX_LENGTH],
            length: length.clamp(1, MAX_LENGTH),
        }
    }

    /// The event for a given absolute step, wrapping at the pattern length.
    pub fn event_at(&self, step: usize) -> StepEvent {
        let i = step % self.length;
        StepEvent {
            gate: self.rhythm.hit(i),
            gate_ticks: self.gate_ticks[i],
            pitch_mv: self.pitch_mv[i],
            aux_mv: self.aux_mv[i],
            slew_ticks: self.slew_ticks[i],
        }
    }
}

/// A pair of patterns, one per output set.
///
/// Most modes drive both sets from the same material; Call/response is the one
/// that makes them genuinely different.
#[derive(Copy, Clone, Debug)]
pub struct PatternPair {
    pub a: Pattern,
    pub b: Pattern,
}

/// Everything the generators need from the front panel.
#[derive(Copy, Clone, Debug)]
pub struct GenParams {
    pub mode: Mode,
    /// Sequence length from X, 1..=16.
    pub length: usize,
    /// Main knob, 0..=4095.
    pub main: u16,
    pub range: PitchRange,
}

/// Generate both voices' patterns for the current settings.
pub fn generate(params: &GenParams, rng: &mut Rng) -> PatternPair {
    match params.mode {
        Mode::EuclidTuring => euclid_turing(params, rng),
        Mode::ArpRun => arp_run(params, rng),
        Mode::CallResponse => call_response(params, rng),
        Mode::Drone => drone(params, rng),
    }
}

/// Scale the Main knob to `0..=max`.
fn main_scaled(main: u16, max: u32) -> u32 {
    (main as u32 * (max + 1) / 4096).min(max)
}

/// A random voltage anywhere in the range. The Turing Machine primitive.
fn random_pitch(rng: &mut Rng, range: PitchRange) -> i32 {
    let (lo, hi) = bounds_mv(range);
    rng.between(lo, hi)
}

/// Random gate length in ticks, weighted short.
///
/// Ben's existing setup varies gate length randomly. Weighting toward short
/// keeps things percussive; the occasional long gate is what makes a line
/// breathe.
fn random_gate_ticks(rng: &mut Rng) -> u16 {
    match rng.below(8) {
        0 => 3,
        1 | 2 => 2,
        _ => 1,
    }
}

/// Aux CV, used as accent or timbre depending on how the voice is patched.
///
/// Full output range rather than the pitch range — this is not pitch, so it
/// should use all the voltage available to it.
fn random_aux(rng: &mut Rng) -> i32 {
    rng.between(0, 5000)
}

/// **Mode 1 — Euclid + Turing.** Main sets the number of Euclidean pulses.
fn euclid_turing(params: &GenParams, rng: &mut Rng) -> PatternPair {
    let len = params.length.clamp(1, MAX_LENGTH);
    // Main spans 1..=len pulses: at the bottom of the knob you still get a
    // pulse, because a silent voice is not a useful knob position live.
    let pulses = 1 + main_scaled(params.main, (len - 1) as u32) as usize;

    let mut a = Pattern::empty(len);
    a.rhythm = Rhythm::euclidean(len, pulses, 0);
    for i in 0..len {
        a.pitch_mv[i] = random_pitch(rng, params.range);
        a.aux_mv[i] = random_aux(rng);
        a.gate_ticks[i] = random_gate_ticks(rng);
    }

    // Voice B: same rhythm, rotated, with its own voltages. Rotation rather
    // than a second random rhythm keeps the two voices locked to the same
    // pulse grid, so they interlock instead of merely coexisting.
    let mut b = Pattern::empty(len);
    b.rhythm = a.rhythm.rotated(len / 2);
    for i in 0..len {
        b.pitch_mv[i] = random_pitch(rng, params.range);
        b.aux_mv[i] = random_aux(rng);
        b.gate_ticks[i] = random_gate_ticks(rng);
    }

    PatternPair { a, b }
}

/// Largest voltage step an Arp-run can take, in millivolts.
///
/// Sized so that the widest increment still gives a few steps before folding:
/// a run of four at 400 mV covers more than a full octave.
const ARP_MAX_INCREMENT_MV: i32 = 400;
/// Smallest, so that even the tightest runs move audibly.
const ARP_MIN_INCREMENT_MV: i32 = 25;

/// **Mode 2 — Arp-run.** Main sets the mean run length.
///
/// A run is a sequence of steps each a fixed increment from the last. The
/// increment is randomised per run, so some runs crawl microtonally and others
/// leap. This is what makes the line sound *played* rather than sampled from a
/// distribution — the thing plain random pitch cannot do.
fn arp_run(params: &GenParams, rng: &mut Rng) -> PatternPair {
    let len = params.length.clamp(1, MAX_LENGTH);
    // Main: mean run length 1..=8. At 1 this degenerates to random pitch,
    // which is deliberate — it means the knob sweeps continuously from the
    // Turing character into rolling runs.
    let mean_run = 1 + main_scaled(params.main, 7) as usize;

    let mut a = Pattern::empty(len);
    // All steps gate: Arp-run is about the line, so let the rhythm be even and
    // put the interest in the pitch contour.
    a.rhythm = Rhythm::euclidean(len, len, 0);
    fill_runs(&mut a, len, mean_run, params.range, rng);

    let mut b = Pattern::empty(len);
    b.rhythm = a.rhythm;
    fill_runs(&mut b, len, mean_run, params.range, rng);

    PatternPair { a, b }
}

/// Fill a pattern's voltages with runs of a fixed increment.
fn fill_runs(p: &mut Pattern, len: usize, mean_run: usize, range: PitchRange, rng: &mut Rng) {
    let mut i = 0;
    let mut current = random_pitch(rng, range);

    while i < len {
        // Run length jitters around the mean so the pattern does not become
        // mechanically periodic at the run boundary.
        let jitter = rng.between(-1, 1);
        let run_len = ((mean_run as i32 + jitter).max(1) as usize).min(len - i);

        // A fresh increment and direction per run.
        let magnitude = rng.between(ARP_MIN_INCREMENT_MV, ARP_MAX_INCREMENT_MV);
        let increment = if rng.chance_256(128) { magnitude } else { -magnitude };

        // Each run starts from a fresh position when it is a single step
        // (so mean_run == 1 really is random pitch), otherwise it continues
        // from where the last run left off, which is what makes long runs
        // feel like one gesture.
        if run_len == 1 {
            current = random_pitch(rng, range);
        }

        for k in 0..run_len {
            if k > 0 {
                current = fold_into_range(current + increment, range);
            }
            p.pitch_mv[i + k] = current;
            p.aux_mv[i + k] = random_aux(rng);
            p.gate_ticks[i + k] = 1;
        }
        i += run_len;
    }
}

/// **Mode 3 — Call/response.** Main crossfades alternation into overlap.
///
/// Voice A takes the first half of the pattern, voice B answers on the second,
/// with B's voltages derived from A's by inversion around the range centre —
/// so the answer is recognisably *related* to the call rather than merely
/// adjacent to it.
fn call_response(params: &GenParams, rng: &mut Rng) -> PatternPair {
    let len = params.length.clamp(1, MAX_LENGTH);
    let half = (len / 2).max(1);

    // Main: 0 = strict alternation, 4095 = full overlap. Expressed as how many
    // steps of each half the other voice is also allowed to play on.
    let overlap = main_scaled(params.main, half as u32) as usize;

    let mut a = Pattern::empty(len);
    let mut b = Pattern::empty(len);

    // Generate the call across the whole pattern; B mirrors it.
    let (lo, hi) = bounds_mv(params.range);
    let centre = (lo + hi) / 2;

    for i in 0..len {
        let pitch = random_pitch(rng, params.range);
        a.pitch_mv[i] = pitch;
        // Inversion about the centre: a rising call becomes a falling answer.
        b.pitch_mv[i] = fold_into_range(centre - (pitch - centre), params.range);
        a.aux_mv[i] = random_aux(rng);
        b.aux_mv[i] = random_aux(rng);
        a.gate_ticks[i] = random_gate_ticks(rng);
        b.gate_ticks[i] = random_gate_ticks(rng);
    }

    // Gates: A owns the first half, B the second, each extending into the
    // other's territory by `overlap` steps. Offsetting B's rotation by half the
    // pattern is what puts the answer after the call.
    let pulses = (half + overlap).min(len);
    a.rhythm = Rhythm::euclidean(len, pulses, 0);
    b.rhythm = Rhythm::euclidean(len, pulses, half);

    PatternPair { a, b }
}

/// **Mode 4 — Drone/Suspension.** Main sweeps sparse events into fully held.
///
/// The non-danceable one, and the transition tool: switch an incoming voice
/// here so it arrives as texture, bring it up, then switch it to a danceable
/// mode. Gates become long and rare; voltage glides rather than steps.
fn drone(params: &GenParams, rng: &mut Rng) -> PatternPair {
    let len = params.length.clamp(1, MAX_LENGTH);
    // Main: 0 = a couple of sparse events, 4095 = one gate held across the
    // whole pattern.
    let heldness = main_scaled(params.main, 255) as u8;

    let mut a = Pattern::empty(len);
    let mut b = Pattern::empty(len);

    for p in [&mut a, &mut b] {
        // Sparse: one or two events in the pattern, placed irregularly rather
        // than on a grid — even placement would read as a slow pulse, which is
        // the one thing this mode must not do.
        let events = if heldness > 200 { 1 } else { 1 + rng.below(2) as usize };
        let mut rhythm = Rhythm::empty(len);
        for _ in 0..events {
            rhythm = rhythm.with_hit(rng.below(len as u32) as usize);
        }
        // Both random placements can land on the same step, so guarantee a gate
        // rather than leaving the voice silent.
        if rhythm.count() == 0 {
            rhythm = rhythm.with_hit(0);
        }
        p.rhythm = rhythm;

        // Long gates, scaling with the knob toward fully held.
        let base_ticks = 2 + (heldness as u32 * len as u32 / 128).max(1);
        // Slew across a good fraction of the gate, so pitch glides rather than
        // stepping. This is the characteristic sound of the mode.
        let slew = (base_ticks / 2).max(1);

        let (lo, hi) = bounds_mv(params.range);
        for i in 0..len {
            p.pitch_mv[i] = rng.between(lo, hi);
            // Aux drifts slowly too, for filter or timbre movement.
            p.aux_mv[i] = rng.between(0, 5000);
            p.gate_ticks[i] = base_ticks.min(u16::MAX as u32) as u16;
            p.slew_ticks[i] = slew.min(u16::MAX as u32) as u16;
        }
    }

    PatternPair { a, b }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: Mode, length: usize, main: u16) -> GenParams {
        GenParams {
            mode,
            length,
            main,
            range: PitchRange::OneOctave,
        }
    }

    #[test]
    fn exactly_one_mode_is_not_danceable() {
        let n = Mode::ALL.iter().filter(|m| !m.is_danceable()).count();
        assert_eq!(n, 1, "the mode set is 3 danceable + 1 not");
        assert!(!Mode::Drone.is_danceable());
    }

    #[test]
    fn mode_selection_covers_all_four_and_clamps() {
        assert_eq!(Mode::from_position(0), Mode::EuclidTuring);
        assert_eq!(Mode::from_position(3), Mode::Drone);
        assert_eq!(Mode::from_position(99), Mode::Drone);
    }

    #[test]
    fn main_scaled_spans_its_range() {
        assert_eq!(main_scaled(0, 7), 0);
        assert_eq!(main_scaled(4095, 7), 7);
        assert_eq!(main_scaled(0, 255), 0);
        assert_eq!(main_scaled(4095, 255), 255);
    }

    #[test]
    fn every_mode_generates_at_every_length() {
        // Guards against panics from length arithmetic at the extremes, which
        // would be a dead module mid-set.
        let mut rng = Rng::new(1);
        for mode in Mode::ALL {
            for len in 1..=MAX_LENGTH {
                for main in [0u16, 2048, 4095] {
                    let p = generate(&params(mode, len, main), &mut rng);
                    assert_eq!(p.a.length, len, "{mode:?} len {len}");
                    assert_eq!(p.b.length, len);
                }
            }
        }
    }

    #[test]
    fn no_mode_produces_a_silent_voice() {
        // A knob position that silences a voice is a dead position live.
        let mut rng = Rng::new(7);
        for mode in Mode::ALL {
            for len in 1..=MAX_LENGTH {
                for main in [0u16, 1024, 2048, 3072, 4095] {
                    let p = generate(&params(mode, len, main), &mut rng);
                    assert!(
                        p.a.rhythm.count() > 0,
                        "{mode:?} len {len} main {main}: voice A silent"
                    );
                    assert!(
                        p.b.rhythm.count() > 0,
                        "{mode:?} len {len} main {main}: voice B silent"
                    );
                }
            }
        }
    }

    #[test]
    fn all_pitches_stay_in_range() {
        // Out-of-range pitch would clip flat at the DAC, collapsing the top of
        // the sequence onto one note.
        let mut rng = Rng::new(11);
        for mode in Mode::ALL {
            for range in [PitchRange::OneOctave, PitchRange::TwoOctaves] {
                let (lo, hi) = bounds_mv(range);
                let gp = GenParams {
                    mode,
                    length: 16,
                    main: 2048,
                    range,
                };
                let p = generate(&gp, &mut rng);
                for i in 0..16 {
                    for v in [p.a.pitch_mv[i], p.b.pitch_mv[i]] {
                        assert!(
                            (lo..=hi).contains(&v),
                            "{mode:?} {range:?} step {i}: {v} outside {lo}..{hi}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic_for_a_seed() {
        // Regeneration is a performance gesture; the same seed must reproduce.
        for mode in Mode::ALL {
            let mut r1 = Rng::new(42);
            let mut r2 = Rng::new(42);
            let gp = params(mode, 16, 2048);
            let a = generate(&gp, &mut r1);
            let b = generate(&gp, &mut r2);
            assert_eq!(a.a.pitch_mv, b.a.pitch_mv, "{mode:?} not reproducible");
            assert_eq!(a.a.rhythm, b.a.rhythm);
        }
    }

    #[test]
    fn euclid_main_knob_controls_pulse_count() {
        // The knob's actual job in this mode.
        let mut rng = Rng::new(3);
        let low = generate(&params(Mode::EuclidTuring, 16, 0), &mut rng);
        let high = generate(&params(Mode::EuclidTuring, 16, 4095), &mut rng);
        assert_eq!(low.a.rhythm.count(), 1, "bottom of knob should be 1 pulse");
        assert_eq!(high.a.rhythm.count(), 16, "top of knob should be all pulses");
    }

    #[test]
    fn euclid_voices_share_a_grid_but_differ() {
        // Rotation, not independent randomness: they must interlock.
        let mut rng = Rng::new(5);
        let p = generate(&params(Mode::EuclidTuring, 16, 2048), &mut rng);
        assert_eq!(
            p.a.rhythm.count(),
            p.b.rhythm.count(),
            "voices should share a pulse count"
        );
        assert_ne!(p.a.rhythm, p.b.rhythm, "voices should not be identical");
    }

    #[test]
    fn arp_run_at_minimum_is_effectively_random() {
        // The bottom of the knob should degenerate to the Turing character, so
        // the knob sweeps continuously from familiar into new.
        let mut rng = Rng::new(9);
        let p = generate(&params(Mode::ArpRun, 16, 0), &mut rng);
        // With run length 1, consecutive steps are independent, so the sign of
        // the change should vary rather than persist.
        let mut direction_changes = 0;
        for i in 2..16 {
            let d1 = p.a.pitch_mv[i - 1] - p.a.pitch_mv[i - 2];
            let d2 = p.a.pitch_mv[i] - p.a.pitch_mv[i - 1];
            if (d1 > 0) != (d2 > 0) {
                direction_changes += 1;
            }
        }
        assert!(
            direction_changes >= 4,
            "expected jittery contour, got {direction_changes} direction changes"
        );
    }

    #[test]
    fn arp_run_at_maximum_produces_sustained_runs() {
        // The point of the mode: consecutive steps moving the same direction by
        // the same increment. Without this it is just random pitch again.
        let mut rng = Rng::new(13);
        let p = generate(&params(Mode::ArpRun, 16, 4095), &mut rng);

        let mut longest = 1;
        let mut current = 1;
        for i in 2..16 {
            let d1 = p.a.pitch_mv[i - 1] - p.a.pitch_mv[i - 2];
            let d2 = p.a.pitch_mv[i] - p.a.pitch_mv[i - 1];
            if d1 == d2 && d1 != 0 {
                current += 1;
                longest = longest.max(current);
            } else {
                current = 1;
            }
        }
        assert!(
            longest >= 3,
            "longest equal-increment run was {longest}, expected a real run"
        );
    }

    #[test]
    fn arp_run_increments_are_not_semitone_quantised() {
        // The whole reason we dropped scales. If increments landed on 83 mV
        // multiples we would have reinvented equal temperament by accident.
        let mut rng = Rng::new(17);
        let p = generate(&params(Mode::ArpRun, 16, 3000), &mut rng);
        let semitone = 1000.0 / 12.0;
        let mut any_off_grid = false;
        for i in 1..16 {
            let d = (p.a.pitch_mv[i] - p.a.pitch_mv[i - 1]).unsigned_abs() as f64;
            if d > 1.0 {
                let semis = d / semitone;
                if (semis - semis.round()).abs() > 0.1 {
                    any_off_grid = true;
                }
            }
        }
        assert!(any_off_grid, "every interval landed on a semitone");
    }

    #[test]
    fn call_response_voices_are_related_not_identical() {
        let mut rng = Rng::new(19);
        let p = generate(&params(Mode::CallResponse, 16, 0), &mut rng);
        assert_ne!(p.a.pitch_mv, p.b.pitch_mv, "answer should differ from call");
        // Inversion about the centre: the two should move oppositely.
        let (lo, hi) = bounds_mv(PitchRange::OneOctave);
        let centre = (lo + hi) / 2;
        for i in 0..16 {
            let da = p.a.pitch_mv[i] - centre;
            let db = p.b.pitch_mv[i] - centre;
            assert!(
                da == 0 || db == 0 || (da > 0) != (db > 0),
                "step {i}: {da} and {db} should be opposite sides of centre"
            );
        }
    }

    #[test]
    fn call_response_alternates_at_knob_minimum() {
        // Strict alternation is the identity of the mode at main=0.
        let mut rng = Rng::new(23);
        let p = generate(&params(Mode::CallResponse, 16, 0), &mut rng);
        // The two voices should not both be dense across the whole pattern.
        let both = (0..16).filter(|&i| p.a.rhythm.hit(i) && p.b.rhythm.hit(i)).count();
        assert!(both < 12, "voices overlap on {both}/16 steps at minimum");
    }

    #[test]
    fn drone_gates_are_long() {
        // The defining feature versus the danceable modes.
        let mut rng = Rng::new(29);
        let p = generate(&params(Mode::Drone, 16, 2048), &mut rng);
        assert!(
            p.a.gate_ticks[0] > 2,
            "drone gate was only {} ticks",
            p.a.gate_ticks[0]
        );
    }

    #[test]
    fn drone_slews_where_danceable_modes_step() {
        let mut rng = Rng::new(31);
        let d = generate(&params(Mode::Drone, 16, 2048), &mut rng);
        let e = generate(&params(Mode::EuclidTuring, 16, 2048), &mut rng);
        assert!(d.a.slew_ticks[0] > 0, "drone should glide");
        assert_eq!(e.a.slew_ticks[0], 0, "euclid should step");
    }

    #[test]
    fn drone_is_sparser_than_the_danceable_modes() {
        let mut rng = Rng::new(37);
        let d = generate(&params(Mode::Drone, 16, 2048), &mut rng);
        let a = generate(&params(Mode::ArpRun, 16, 2048), &mut rng);
        assert!(
            d.a.rhythm.count() < a.a.rhythm.count(),
            "drone ({}) should be sparser than arp-run ({})",
            d.a.rhythm.count(),
            a.a.rhythm.count()
        );
    }

    #[test]
    fn event_lookup_wraps_at_pattern_length() {
        let mut rng = Rng::new(41);
        let p = generate(&params(Mode::EuclidTuring, 7, 2048), &mut rng);
        for step in 0..7 {
            assert_eq!(p.a.event_at(step), p.a.event_at(step + 7));
        }
    }
}

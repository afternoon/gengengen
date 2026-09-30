//! gengengen — generative sequencer for the Music Thing Workshop System
//! Computer.
//!
//! Externally clocked on Pulse In 1. Y selects the mode, X the sequence length,
//! Main is the per-mode parameter, and the Z switch regenerates (down) and
//! selects the pitch range (middle/up).
//!
//! Architecture: a single Embassy task polling at 5 ms, rather than a fixed
//! sample-rate ISR. The sequencer runs at step rate — tens of Hz — so there is
//! no audio path to service. See `run` for why this ended up as one task rather
//! than the several the design doc anticipated.

#![cfg_attr(target_arch = "arm", no_std, no_main)]

#[cfg(not(target_arch = "arm"))]
fn main() {
    // The firmware only builds for the Cortex-M target. This stub exists so the
    // crate still has a valid bin target on the dev machine, which keeps
    // `cargo clippy --all-targets` and `cargo test` working.
    eprintln!("gengengen is firmware; build with --target thumbv6m-none-eabi");
}

#[cfg(target_arch = "arm")]
use defmt::info;
#[cfg(target_arch = "arm")]
use defmt_rtt as _;
#[cfg(target_arch = "arm")]
use panic_probe as _;

#[cfg(target_arch = "arm")]
use embassy_executor::Spawner;
#[cfg(target_arch = "arm")]
use embassy_rp::adc;
#[cfg(target_arch = "arm")]
use embassy_rp::bind_interrupts;
#[cfg(target_arch = "arm")]
use embassy_rp::i2c;
#[cfg(target_arch = "arm")]
use embassy_rp::peripherals::I2C0;
#[cfg(target_arch = "arm")]
use embassy_time::{Duration, Instant, Timer};

#[cfg(target_arch = "arm")]
use gengengen::hw::board::{Board, Panel};
#[cfg(target_arch = "arm")]
use gengengen::hw::controls::{Quantised, SwitchPosition};
#[cfg(target_arch = "arm")]
use gengengen::music::modes::Mode;
#[cfg(target_arch = "arm")]
use gengengen::music::voltage::PitchRange;
#[cfg(target_arch = "arm")]
use gengengen::seq::engine::{Controls, Engine};

#[cfg(target_arch = "arm")]
bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

#[cfg(target_arch = "arm")]
/// How often the main loop runs: panel scan, clock poll, and one CV dither tick.
///
/// Fast enough to feel responsive under the hand, and slow enough that the mux
/// settle delays and ADC reads stay a trivial share of CPU.
///
/// It also sets the pitch resolution, which is the binding constraint. The CV
/// outputs reach beyond their 11 PWM bits by dithering, so resolution scales
/// with updates per step: at 5 ms that is ~17-25 updates per 16th note across
/// 120-174 bpm, worth about 15.5 effective bits, or ~0.3 cents. Going to 1 ms
/// would buy ~0.06 cents, which is far past anything audible - so this stays
/// where the responsiveness argument puts it.
const PANEL_SCAN_MS: u64 = 5;

#[cfg(target_arch = "arm")]
/// How long a gate output stays high, as a fraction of the observed clock
/// interval. Gate *length in steps* comes from the pattern; this is the shape
/// within a single step.
const GATE_DUTY_NUMERATOR: u32 = 1;
#[cfg(target_arch = "arm")]
const GATE_DUTY_DENOMINATOR: u32 = 2;

#[cfg(target_arch = "arm")]
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let board = Board::new(p, Irqs, Irqs).await;

    info!(
        "gengengen starting; calibration from eeprom: ch0={} ch1={}",
        board.cal[0].from_eeprom, board.cal[1].from_eeprom
    );

    spawner.must_spawn(run(board));
}

#[cfg(target_arch = "arm")]
/// Everything runs in one task.
///
/// Splitting the panel scan out into its own task was the original plan, but the
/// board owns the ADC and the DAC and the sequencer needs both on the same clock
/// edge, so sharing it would mean a mutex on the hot path for no benefit. The
/// panel scan is interleaved with waiting for the clock instead, which is where
/// the time goes anyway.
#[embassy_executor::task]
async fn run(mut board: Board) -> ! {
    let mut mode_knob = Quantised::<4>::new(64);
    // 16 positions are ~256 counts wide, so a smaller hysteresis keeps the knob
    // from feeling sticky while still not flickering between step counts.
    let mut length_knob = Quantised::<16>::new(24);

    // Seed from the boot time, so a power cycle does not replay the same set.
    let seed = Instant::now().as_micros() as u32 ^ 0x5EED_1234;

    let initial = Controls {
        mode: Mode::EuclidTuring,
        length: 16,
        main: 2048,
        range: PitchRange::OneOctave,
    };
    let mut engine = Engine::new(initial, seed);

    let mut last_clock_high = false;
    let mut last_switch = SwitchPosition::Middle;
    let mut clock_interval = Duration::from_millis(125); // 120bpm 16ths, until measured
    let mut last_edge = Instant::now();
    let mut gate_off_at: [Option<Instant>; 2] = [None, None];

    loop {
        // --- panel ---
        let panel = board.read_panel().await;
        let controls = controls_from_panel(&panel, &mut mode_knob, &mut length_knob);
        engine.set_controls(controls);

        // Switch down is momentary: act on the edge, not the level, or it would
        // regenerate continuously while held.
        let switch = panel.switch();
        if switch == SwitchPosition::Down && last_switch != SwitchPosition::Down {
            engine.force_now();
        }
        last_switch = switch;

        // --- clock ---
        let clock_high = board.pulse_in_1_high();
        if clock_high && !last_clock_high {
            let now = Instant::now();
            // Track the incoming tempo so gate length can be a sensible
            // fraction of a step rather than a fixed millisecond count.
            let measured = now.duration_since(last_edge);
            if measured > Duration::from_millis(2) && measured < Duration::from_secs(2) {
                clock_interval = measured;
            }
            last_edge = now;

            let out = engine.clock();
            apply_outputs(&mut board, &out, clock_interval, now, &mut gate_off_at);
        }
        last_clock_high = clock_high;

        // --- gate release ---
        // Gates fall partway through the step, so a step-long gate still
        // retriggers an envelope on the next step.
        let now = Instant::now();
        for (ch, slot) in gate_off_at.iter_mut().enumerate() {
            if let Some(off) = *slot {
                if now >= off {
                    board.set_pulse_out(ch, false);
                    *slot = None;
                }
            }
        }

        update_leds(&mut board, &engine, gate_off_at[0].is_some(), gate_off_at[1].is_some());

        // Keep the CV sigma-delta running between steps. The extra resolution
        // on the pitch outputs comes from the output filter averaging
        // successive duties, so this has to tick steadily whether or not the
        // sequencer advanced - without it the pitch sits on an 11-bit grid,
        // about 7 cents, which is audible under unquantised pitch.
        board.tick_cv();

        Timer::after(Duration::from_millis(PANEL_SCAN_MS)).await;
    }
}

#[cfg(target_arch = "arm")]
/// Map the panel readings onto sequencer controls.
fn controls_from_panel(
    panel: &Panel,
    mode_knob: &mut Quantised<4>,
    length_knob: &mut Quantised<16>,
) -> Controls {
    let mode_pos = mode_knob.update(panel.y_stretched());
    let length_pos = length_knob.update(panel.x_stretched());

    Controls {
        mode: Mode::from_position(mode_pos),
        // 0-indexed knob position to a 1..=16 step count.
        length: length_pos + 1,
        main: panel.main_stretched(),
        range: match panel.switch() {
            // Up latches into the wider range. Down is momentary and means
            // "regenerate", so it should not also change the range - keep
            // whatever the resting position implies.
            SwitchPosition::Up => PitchRange::TwoOctaves,
            _ => PitchRange::OneOctave,
        },
    }
}

#[cfg(target_arch = "arm")]
/// Push one step's outputs to the hardware.
fn apply_outputs(
    board: &mut Board,
    out: &gengengen::seq::engine::Outputs,
    clock_interval: Duration,
    now: Instant,
    gate_off_at: &mut [Option<Instant>; 2],
) {
    let gate_len = Duration::from_micros(
        clock_interval.as_micros() * GATE_DUTY_NUMERATOR as u64
            / GATE_DUTY_DENOMINATOR as u64,
    );

    for (ch, voice) in [(0usize, &out.a), (1usize, &out.b)] {
        if voice.gate {
            board.set_pulse_out(ch, true);
            gate_off_at[ch] = Some(now + gate_len);
        }
        // Pitch goes to the PWM CV outs and aux to the DAC "audio" outs, to
        // match the Turing Machine and Simple MIDI cards - so swapping cards
        // mid-set does not mean repatching the rack.
        //
        // This costs some pitch resolution (11-bit PWM rather than the 12-bit
        // DAC) but gains the EEPROM calibration, which describes these outputs
        // and nothing else. For unquantised pitch that is the better trade: we
        // are not trying to land on semitones, so an accurate *range* matters
        // more than fine steps.
        board.set_cv_millivolts(ch, voice.pitch_mv);
        board.set_dac_millivolts(ch, voice.aux_mv);
    }
}

#[cfg(target_arch = "arm")]
/// Show what the sequencer is doing on the six LEDs.
///
/// Top four: the current mode, as a single lit LED. Bottom two: the two voices'
/// gates. This is chosen for what is useful at arm's length in a dark room —
/// which mode am I in, and are both voices actually playing.
fn update_leds(board: &mut Board, engine: &Engine, gate_a: bool, gate_b: bool) {
    let active = engine.active();
    let mode_index = Mode::ALL
        .iter()
        .position(|m| *m == active.mode)
        .unwrap_or(0);

    for i in 0..4 {
        // Pulse the mode LED on the downbeat so there is a visible tempo
        // reference without spending an LED on it.
        let on_beat = engine.step_in_pattern() == 0;
        let brightness = if i == mode_index {
            if on_beat {
                4095
            } else {
                1200
            }
        } else {
            0
        };
        board.set_led(i, brightness);
    }

    // Bottom two LEDs: the two voices' gates, so you can see at a glance
    // whether both are actually playing. A pending knob change dims them
    // slightly, so you know the knob registered before the sound changes.
    let pending = engine.has_pending();
    for (led, playing) in [(4usize, gate_a), (5usize, gate_b)] {
        let brightness = match (playing, pending) {
            (true, _) => 4095,
            (false, true) => 500,
            (false, false) => 0,
        };
        board.set_led(led, brightness);
    }
}

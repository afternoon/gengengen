# gengengen — a generative sequencer for the Music Thing Workshop Computer

A four-mode generative sequencer program card, written in Rust with Embassy,
for playing live raw/deep warehouse techno with a modular system.

## Musical intent

Repetition with pressure, not note variety. Short, repetitive melodies that
stay put long enough to become hypnotic, with enough evolution available on a
single knob to keep a set moving. Three of the four modes are danceable; the
fourth exists to get a new voice into the mix as texture before it becomes a
rhythm.

Performance style this is built for: 3 synth voices, 1–2 playing at once,
evolving the set by fading voices out and new ones in.

## Hardware

Music Thing Modular Workshop System **Computer**, RP2040 (dual Cortex-M0+,
133 MHz stock; 144 MHz recommended to keep ADC noise down), 264 KB SRAM.
Firmware lives on a removable program card carrying 2 MB or 16 MB of SPI flash
— the card holds the module's entire memory image.

### Outputs available

| Out | Circuit | Resolution | Notes |
|---|---|---|---|
| Audio out 1, 2 | MCP4822 SPI DAC | 12-bit, signed −2048…2047 | Synchronous, accurate. **Use for pitch.** |
| CV out 1, 2 | Filtered PWM from RP2040 | 11-bit @ 60 kHz, inverted | Slower settling, asynchronous |
| Pulse out 1, 2 | Transistor-buffered digital | — | ~5–6 V, inverted at GPIO |

Both DAC and PWM outs are DC-coupled and bipolar over roughly ±6 V, so either
can carry CV — but the DAC outs are the precise ones, and pitch needs precision.

### Controls available

Pulse In 1 takes the external clock, leaving:

- **Main** (big knob) — per-mode parameter
- **X** — sequence length, 1–16 steps
- **Y** — mode select (4 positions)
- **Z** switch — 3-position `(ON)-OFF-ON`: **momentary down**, rest middle,
  **latching up**. Read as an analogue value through the mux, not as a GPIO.
  - Down (momentary) → regenerate pitch/CV sequences
  - Middle → 1 octave pitch range
  - Up (latching) → 2 octave pitch range
- 6 LEDs (2 wide × 3 tall), plain PWM-dimmed, not addressable

## Output map

Two output sets, so two independent voices. Ben has three physical voices; two
of them share a set, taking the same gate but different CV roles.

| Set | Pulse | Pitch (DAC) | CV (PWM) |
|---|---|---|---|
| **A** | Pulse out 1 | Audio out 1 | CV out 1 |
| **B** | Pulse out 2 | Audio out 2 | CV out 2 |

The CV out carries accent or timbre depending on the voice patched. Where two
voices share a set, they take the same gate; one reads the CV as accent, the
other as timbre.

> **Unverified:** which panel jacks are labelled "pitch" vs "CV" may not line up
> with which circuit is the DAC vs the PWM. Confirm on hardware before trusting
> the table above.

## The four modes

Y selects between them. All four draw from the same scale table and pitch
generator, so switching mid-set stays harmonically continuous.

### 1. Euclid + Turing (danceable)

The existing model, carried over: Euclidean rhythm on the gates, random-walk
pitch in the manner of a Turing Machine.

- **Main** → number of pulses in the Euclidean rhythm (1…length)
- **X** → pattern length
- Gate length varies randomly per step
- Switch-down regenerates the pitch and CV sequences

### 2. Arp-run (danceable)

Inspired by the Wirehead Basilisk's arp-run mode. Holds a chord shape and walks
it in *runs* — bursts of adjacent degrees — rather than picking isolated random
notes. Runs are what make a line sound played rather than sampled from a
distribution.

- **Main** → mean run length: 1 = pure random arp (≈ Turing), ~4 = rolling
  303-style lines, max = long rising/falling sweeps
- Pick a start degree and direction, play 2–5 consecutive degrees, jump, repeat

> **Open question:** whether runs should step through *diatonic scale* degrees
> or *chord* degrees. This changes the generator. The Basilisk manual's
> sequencing bullets are vector outlines that don't extract as text, so this is
> currently inferred from product descriptions rather than read from the manual.

### 3. Call/response (danceable)

Uses both output sets as two voices in dialogue. Voice A plays the first half of
the pattern, voice B answers on the second, with B's pitch derived from A's
(inverted, transposed, or retrograde).

- **Main** → crossfade from strict alternation to full overlap

### 4. Drone/Suspension (not danceable)

The breakdown and the transition tool. Gates mostly stop; what remains is long,
sparse, irregular gates — one every 2, 3, 7 bars — with slow CV glides between
held pitches instead of stepped jumps.

- **Main** → sparse events … fully held (gates high and staying high, CV
  drifting continuously)

Purpose in performance: switch an *incoming* voice to this mode so it arrives as
texture rather than as a competing rhythm, bring it up, then switch it to a
danceable mode. Mechanically it's the same scale table and pitch generator as
the others with the clock divided hard and slew applied to the CV.

## Behaviour

**Mode changes quantise to the next pattern boundary**, so turning Y past a mode
boundary mid-bar doesn't drop a gate in the wrong place. Switch-down acts as a
"do it now" override.

## Architecture

Embassy async tasks over `embassy-sync` watch channels, following the shape of
Brian Dorsey's Crafted Volts card. Explicitly **not** a fixed 48 kHz sample-rate
ISR: ComputerCard's C++ design is built around one because it targets audio, but
this card generates gates and CV at step rate — tens of Hz — so a control-rate
task architecture is the right fit, and suits sequencer state machines well.

    src/
      main.rs        task wiring
      hw/            board support: pin map, mux, DAC, EEPROM calibration, LEDs
      music/         scales, pitch generation, the four mode generators
      seq/           clock, step state, pattern boundaries, mode switching

The `hw` module is our own BSP — no Workshop Computer BSP crate exists, so this
is the first one. Keeping it behind a clean seam means the musical logic is
testable on the host and the BSP is extractable as a crate later if it earns it.

## Hardware notes that will bite

1. **Everything is inverted.** Pulse inputs, pulse outputs and the PWM CV outs
   are all inverted at the GPIO. Pulse inputs also need the RP2040 pull-up
   enabled — it biases the input transistor.
2. **The mux needs a settle delay.** Switch the 4052 address, wait, *then* read
   the ADC. Dorsey's card does this explicitly.
3. **Calibration is on the module, not the card.** An I2C EEPROM (GPIO16/17)
   holds per-unit output calibration — magic number 2001, per-channel point
   tables, big-endian, CRC-checked, least-squares fit to get slope/offset.
   Uncalibrated outs are documented as *not accurate enough for 1V/oct*, so for
   a pitch sequencer this is mandatory, not optional. The existing Rust card
   leaves it as a TODO, so we implement it ourselves.
4. **Don't hard-code ±6.000 V.** Every source hedges on the exact range. Use the
   calibration data.
5. **Knobs don't reach the rails** — raw ADC is typically 14…4095, and an
   untouched knob jitters between adjacent values. Needs stretching and
   smoothing, and mode/length selection needs hysteresis.

## Unverified, to confirm on hardware

Decision taken: code to the official hardware doc, then flip constants when
first flashed.

- **X/Y knob mux addressing.** The hardware doc's 4052 truth table and Dorsey's
  working code disagree — his code carries the comment "X and Y appear to be
  swapped compared to how I read the logic table, not sure why." We follow the
  doc; if X and Y are transposed on first flash, swap the constants.
- **Audio L/R naming.** ComputerCard's `#define`s swap L/R relative to the
  hardware doc; the GPIO↔ADC mapping itself is consistent.
- **Which panel jacks are DAC vs PWM** (see output map above).
- **Pulse input voltage threshold** — not stated in any primary doc; set by the
  transistor front end.
- **GPIO20** — doc says "not connected", ComputerCard uses it as
  `USB_HOST_STATUS` on Rev 1.1. Revision-dependent, and we don't need it.

## Deferred, not discarded

Modes considered and held back for v2, so the four shipping modes stay one per
axis (rhythm, pitch, dialogue, texture) with no overlap:

- **Ratchet/density** — per-step probability of subdividing a gate into 2/3/4
  retriggers. A build-up on one knob; arguably the most techno-specific idea of
  the lot.
- **Drift/erosion** — one step mutates per bar instead of wholesale
  regeneration, so the loop erodes rather than jumps.
- **Polymeter** — voice B runs a different length to voice A; a 5-vs-16 pair
  takes 80 steps to realign, giving long-form structure for free.
- **Accumulator/transpose ladder** — short locked pattern, transposition
  accumulating every N bars, then reset.
- **Cellular automaton** — 16 steps as bits, Rule 90 or an LFSR shift per bar.
  Clustery and lopsided where Euclid is even.
- **Chaos maps** — logistic or Lorenz driving pitch and gates, Main as the chaos
  parameter: periodic → quasi-periodic → noise.

## References

- [Workshop_Computer repo](https://github.com/TomWhitwell/Workshop_Computer) —
  hardware docs, EEPROM map, official card releases
- [ComputerCard](https://github.com/TomWhitwell/Workshop_Computer/tree/main/Demonstrations%2BHelloWorlds/PicoSDK/ComputerCard)
  (Chris Johnson) — the de-facto C++ library; the reference for DAC word format,
  calibration handling and mux sequencing even though we're not using it
- [mtmws_cards](https://codeberg.org/briandorsey/mtmws_cards) (Brian Dorsey) —
  the only known Rust/Embassy cards: Crafted Volts, Backyard Rain
- [Music Thing program cards](https://www.musicthing.co.uk/Computer_Program_Cards/)
- [Wirehead Basilisk](https://wireheadinstruments.com/basilisk) — arp-run and
  call/response inspiration

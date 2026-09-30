//! 4052 analogue mux: the three knobs, the switch, and the two CV inputs.
//!
//! Two address lines select one of four positions; each position presents one
//! signal on ADC channel 2 (knobs/switch) and another on channel 3 (CV in).
//! So a full scan is four address settings, two ADC reads each.

/// The four mux addresses, named by what they select.
///
/// # The X/Y question
///
/// The hardware doc's truth table and Brian Dorsey's working Rust card
/// **disagree** about which address selects X and which selects Y. His code
/// carries the comment: "NOTE: X and Y appear to be swapped compared to how I
/// read the logic table, not sure why." ComputerCard reads the mux positions in
/// sequence without naming them, so it does not arbitrate between the two.
///
/// We follow the hardware doc, per the project decision to code to the doc and
/// correct on hardware. If X and Y turn out transposed on the first flash, set
/// [`SWAP_XY`] to `true` — that is the only change needed.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MuxAddress {
    /// A=0 B=0 — Main knob / CV 1
    MainKnobAndCv1,
    /// A=0 B=1 — X knob / CV 2
    XKnobAndCv2,
    /// A=1 B=0 — Y knob / CV 1
    YKnobAndCv1,
    /// A=1 B=1 — Z switch / CV 2
    SwitchAndCv2,
}

/// Set to `true` if X and Y read transposed on real hardware.
///
/// See [`MuxAddress`] for why this is in doubt. Kept as a single constant so
/// the fix is one line and does not ripple through the scanning code.
pub const SWAP_XY: bool = false;

impl MuxAddress {
    /// All four addresses, in scan order.
    pub const ALL: [MuxAddress; 4] = [
        MuxAddress::MainKnobAndCv1,
        MuxAddress::XKnobAndCv2,
        MuxAddress::YKnobAndCv1,
        MuxAddress::SwitchAndCv2,
    ];

    /// The (A, B) select line states for this address.
    pub fn select_lines(self) -> (bool, bool) {
        let (a, b) = match self {
            MuxAddress::MainKnobAndCv1 => (false, false),
            MuxAddress::XKnobAndCv2 => (false, true),
            MuxAddress::YKnobAndCv1 => (true, false),
            MuxAddress::SwitchAndCv2 => (true, true),
        };
        if SWAP_XY {
            // Only the two knob addresses are in question; the Main and Switch
            // positions are not disputed.
            match self {
                MuxAddress::XKnobAndCv2 => (true, false),
                MuxAddress::YKnobAndCv1 => (false, true),
                _ => (a, b),
            }
        } else {
            (a, b)
        }
    }
}

/// How long to wait after setting the mux address before reading the ADC.
///
/// The mux output needs time to settle; reading immediately gives you a blend
/// of the previous position and this one. Dorsey's card does this explicitly
/// and it is easy to omit and then chase as a noise problem.
pub const SETTLE_MICROS: u64 = 10;

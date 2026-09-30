//! Output calibration, read from the module's I2C EEPROM.
//!
//! This matters more here than in most cards. Uncalibrated outputs are
//! documented as not accurate enough for 1V/oct, and this is a pitch
//! sequencer — out-of-tune is not a subtle defect. The existing Rust card for
//! this module leaves calibration as an explicit TODO, so this is written from
//! the EEPROM map in the hardware documentation and ComputerCard's reader.
//!
//! The EEPROM is on the **module**, not the program card, so calibration is
//! per-unit and shared across every card you plug in.
//!
//! Layout (offset 0, 88 bytes total):
//!
//! ```text
//!   0..2    magic number 2001 (absent => uncalibrated)
//!   2       version
//!   4       channel 0 block  ] 41 bytes each:
//!  45       channel 1 block  ]   1 byte  numPoints (2..=8)
//!                                then per point:
//!                                  i8  target voltage, units of 0.1 V
//!                                  u32 DAC setting, big-endian, 19-bit
//!  86..88   CRC over the preceding 86 bytes
//! ```

/// Marks a valid output calibration block.
pub const MAGIC_OUTPUT_CAL: u16 = 2001;

/// Total size of the output calibration block.
pub const CAL_BLOCK_LEN: usize = 88;

/// Per-channel block stride and base offset.
const CHANNEL_BLOCK_LEN: usize = 41;
const CHANNEL_BASE: usize = 4;

/// Maximum calibration points per channel, per the EEPROM format.
pub const MAX_POINTS: usize = 8;

/// One calibration point: a known output voltage and the DAC setting that
/// produces it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CalPoint {
    /// Target voltage in millivolts.
    pub millivolts: i32,
    /// The 19-bit DAC setting that produces it.
    pub dac_setting: u32,
}

/// A straight line fitted through the calibration points: `dac = m * mv + b`.
///
/// Note that `m` is expected to be **negative** on this hardware — the outputs
/// are inverted, so a higher DAC setting gives a lower voltage.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CalLine {
    pub m: f32,
    pub b: f32,
}

impl CalLine {
    /// The DAC setting for a given voltage, clamped to the 19-bit range.
    pub fn dac_for_millivolts(&self, mv: i32) -> u32 {
        let raw = self.m * mv as f32 + self.b;
        raw.clamp(0.0, 524_287.0) as u32
    }
}

/// The default, uncalibrated table. Deliberately the same values ComputerCard
/// falls back on, so an uncalibrated module behaves consistently between
/// firmwares rather than being differently wrong.
///
/// Note the inverted slope: -2 V maps to a *higher* DAC setting than +2 V.
pub const DEFAULT_POINTS: [CalPoint; 3] = [
    CalPoint {
        millivolts: -2000,
        dac_setting: 347_700,
    },
    CalPoint {
        millivolts: 0,
        dac_setting: 261_200,
    },
    CalPoint {
        millivolts: 2000,
        dac_setting: 174_400,
    },
];

/// Calibration for one output channel.
#[derive(Copy, Clone, Debug)]
pub struct ChannelCalibration {
    pub line: CalLine,
    /// False if we fell back to defaults, so the UI can say so.
    pub from_eeprom: bool,
}

impl ChannelCalibration {
    /// The uncalibrated fallback.
    pub fn default_uncalibrated() -> Self {
        Self {
            line: least_squares(&DEFAULT_POINTS),
            from_eeprom: false,
        }
    }
}

/// Fit `dac = m * mv + b` through the points by ordinary least squares.
///
/// Matches ComputerCard's approach. With 2 points this is just the line
/// through them; with more it averages out measurement error.
pub fn least_squares(points: &[CalPoint]) -> CalLine {
    let n = points.len() as f32;
    debug_assert!(n >= 2.0, "need at least two points to fit a line");

    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    let mut sum_xy = 0.0f32;
    let mut sum_xx = 0.0f32;

    for p in points {
        let x = p.millivolts as f32;
        let y = p.dac_setting as f32;
        sum_x += x;
        sum_y += y;
        sum_xy += x * y;
        sum_xx += x * x;
    }

    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f32::EPSILON {
        // Degenerate: every point at the same voltage. Can't fit a slope, so
        // fall back to a flat line at the mean rather than dividing by ~zero.
        return CalLine {
            m: 0.0,
            b: sum_y / n,
        };
    }

    let m = (n * sum_xy - sum_x * sum_y) / denom;
    let b = (sum_y - m * sum_x) / n;
    CalLine { m, b }
}

/// Errors from parsing the calibration block.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CalError {
    /// Buffer shorter than the 88-byte block.
    TooShort,
    /// Magic number absent — the module has never been calibrated.
    NotCalibrated,
    /// CRC mismatch — data present but untrustworthy.
    BadCrc,
    /// `numPoints` outside the valid 2..=8 range.
    BadPointCount,
}

/// Parse one channel's calibration out of the EEPROM block.
///
/// `channel` is 0 or 1. Returns the fitted line, or an error explaining why we
/// should fall back to defaults.
pub fn parse_channel(buf: &[u8], channel: usize) -> Result<CalLine, CalError> {
    if buf.len() < CAL_BLOCK_LEN {
        return Err(CalError::TooShort);
    }

    let magic = u16::from_be_bytes([buf[0], buf[1]]);
    if magic != MAGIC_OUTPUT_CAL {
        return Err(CalError::NotCalibrated);
    }

    let stored_crc = u16::from_be_bytes([buf[86], buf[87]]);
    if crc16(&buf[..86]) != stored_crc {
        return Err(CalError::BadCrc);
    }

    let base = CHANNEL_BASE + CHANNEL_BLOCK_LEN * channel;
    let num_points = buf[base] as usize;
    if !(2..=MAX_POINTS).contains(&num_points) {
        return Err(CalError::BadPointCount);
    }

    let mut points = [CalPoint {
        millivolts: 0,
        dac_setting: 0,
    }; MAX_POINTS];

    for (i, point) in points.iter_mut().enumerate().take(num_points) {
        // Each point is i8 target voltage (0.1 V units) then u32 big-endian.
        let off = base + 1 + i * 5;
        let tenths = buf[off] as i8;
        point.millivolts = tenths as i32 * 100;
        point.dac_setting = u32::from_be_bytes([
            buf[off + 1],
            buf[off + 2],
            buf[off + 3],
            buf[off + 4],
        ]);
    }

    Ok(least_squares(&points[..num_points]))
}

/// Read calibration for one channel, falling back to defaults on any problem.
///
/// Calibration failing should never stop the sequencer running — being
/// slightly out of tune beats being silent mid-set.
pub fn channel_or_default(buf: &[u8], channel: usize) -> ChannelCalibration {
    match parse_channel(buf, channel) {
        Ok(line) => ChannelCalibration {
            line,
            from_eeprom: true,
        },
        Err(_) => ChannelCalibration::default_uncalibrated(),
    }
}

/// CRC-16/CCITT-FALSE, as used by ComputerCard's calibration block.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic EEPROM block with a valid magic and CRC.
    fn synthetic_block(points: &[CalPoint], channel: usize) -> [u8; CAL_BLOCK_LEN] {
        let mut buf = [0u8; CAL_BLOCK_LEN];
        buf[0..2].copy_from_slice(&MAGIC_OUTPUT_CAL.to_be_bytes());
        buf[2] = 1; // version

        let base = CHANNEL_BASE + CHANNEL_BLOCK_LEN * channel;
        buf[base] = points.len() as u8;
        for (i, p) in points.iter().enumerate() {
            let off = base + 1 + i * 5;
            buf[off] = (p.millivolts / 100) as i8 as u8;
            buf[off + 1..off + 5].copy_from_slice(&p.dac_setting.to_be_bytes());
        }

        let crc = crc16(&buf[..86]);
        buf[86..88].copy_from_slice(&crc.to_be_bytes());
        buf
    }

    #[test]
    fn fits_a_line_through_two_points() {
        let pts = [
            CalPoint { millivolts: 0, dac_setting: 1000 },
            CalPoint { millivolts: 1000, dac_setting: 2000 },
        ];
        let line = least_squares(&pts);
        assert!((line.m - 1.0).abs() < 1e-3);
        assert!((line.b - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn default_table_has_inverted_slope() {
        // Higher DAC setting must mean lower voltage on this hardware. If this
        // ever comes out positive, the outputs will sweep backwards.
        let line = least_squares(&DEFAULT_POINTS);
        assert!(line.m < 0.0, "expected inverted slope, got m={}", line.m);
    }

    #[test]
    fn default_table_round_trips_its_own_points() {
        let line = least_squares(&DEFAULT_POINTS);
        for p in &DEFAULT_POINTS {
            let got = line.dac_for_millivolts(p.millivolts) as i64;
            let want = p.dac_setting as i64;
            assert!(
                (got - want).abs() < 2000,
                "{} mV: got {got}, want {want}",
                p.millivolts
            );
        }
    }

    #[test]
    fn parses_a_valid_block() {
        let pts = [
            CalPoint { millivolts: -2000, dac_setting: 340_000 },
            CalPoint { millivolts: 0, dac_setting: 260_000 },
            CalPoint { millivolts: 2000, dac_setting: 180_000 },
        ];
        let buf = synthetic_block(&pts, 0);
        let line = parse_channel(&buf, 0).expect("should parse");
        assert!(line.m < 0.0);
        // Should land close to the middle point it was built from.
        let mid = line.dac_for_millivolts(0) as i64;
        assert!((mid - 260_000).abs() < 3000, "midpoint off: {mid}");
    }

    #[test]
    fn both_channels_parse_independently() {
        let pts = [
            CalPoint { millivolts: 0, dac_setting: 100_000 },
            CalPoint { millivolts: 1000, dac_setting: 50_000 },
        ];
        let buf = synthetic_block(&pts, 1);
        assert!(parse_channel(&buf, 1).is_ok());
        // Channel 0 was left zeroed, so its point count is invalid.
        assert_eq!(parse_channel(&buf, 0), Err(CalError::BadPointCount));
    }

    #[test]
    fn rejects_missing_magic() {
        let mut buf = synthetic_block(&DEFAULT_POINTS, 0);
        buf[0] = 0;
        buf[1] = 0;
        assert_eq!(parse_channel(&buf, 0), Err(CalError::NotCalibrated));
    }

    #[test]
    fn rejects_bad_crc() {
        let mut buf = synthetic_block(&DEFAULT_POINTS, 0);
        // Corrupt a data byte, leaving the stored CRC stale.
        buf[10] ^= 0xFF;
        assert_eq!(parse_channel(&buf, 0), Err(CalError::BadCrc));
    }

    #[test]
    fn rejects_short_buffer() {
        assert_eq!(parse_channel(&[0u8; 10], 0), Err(CalError::TooShort));
    }

    #[test]
    fn falls_back_to_defaults_rather_than_failing() {
        // A dead EEPROM must not stop the sequencer.
        let cal = channel_or_default(&[0u8; CAL_BLOCK_LEN], 0);
        assert!(!cal.from_eeprom);
        assert!(cal.line.m < 0.0);
    }

    #[test]
    fn dac_setting_clamps_to_19_bits() {
        let line = CalLine { m: -1000.0, b: 261_200.0 };
        assert_eq!(line.dac_for_millivolts(100_000), 0);
        assert!(line.dac_for_millivolts(-100_000) <= 524_287);
    }

    #[test]
    fn degenerate_points_do_not_blow_up() {
        let pts = [
            CalPoint { millivolts: 500, dac_setting: 1000 },
            CalPoint { millivolts: 500, dac_setting: 2000 },
        ];
        let line = least_squares(&pts);
        assert_eq!(line.m, 0.0);
        assert!(line.b.is_finite());
    }
}

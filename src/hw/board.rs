//! Embassy peripheral setup and the driver glue.
//!
//! This is the one module that genuinely needs hardware, so it is kept thin and
//! everything decideable is delegated to the pure modules beside it. Structure
//! follows Brian Dorsey's Crafted Volts card, which is the only known-good Rust
//! reference for this module.
//!
//! ARM-only: the host test build does not compile this.

use embassy_rp::adc::{self, Adc, Channel as AdcChannel};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::{I2C0, SPI0};
use embassy_rp::pwm::{self, Pwm};
use embassy_rp::spi::{self, Spi};
use embassy_rp::Peripherals;
use embassy_time::{Duration, Timer};

use crate::hw::calibration::{self, ChannelCalibration, CAL_BLOCK_LEN};
use crate::hw::controls::{stretch_knob, Raw, SwitchPosition};
use crate::hw::dac::{command_word, millivolts_to_code, DacChannel};
use crate::hw::mux::{MuxAddress, SETTLE_MICROS};

/// I2C address of the calibration EEPROM's first page.
const EEPROM_ADDR: u8 = 0x50;

/// PWM top value for the CV outputs.
///
/// The CV outs are 11-bit at 60 kHz per the hardware docs. With the RP2040 at
/// 133 MHz and no clock divider, a top of 2047 gives ~65 kHz, close enough to
/// the documented figure and keeping the full 11-bit range.
const CV_PWM_TOP: u16 = 2047;

/// PWM top for the LEDs. Higher resolution than the CVs need, so gamma
/// correction has room to work at the dim end.
const LED_PWM_TOP: u16 = 4095;


/// A raw sample set from one full mux scan.
#[derive(Copy, Clone, Debug, Default)]
pub struct Panel {
    pub main: Raw,
    pub x: Raw,
    pub y: Raw,
    pub switch_raw: Raw,
    pub cv_1: Raw,
    pub cv_2: Raw,
}

impl Panel {
    /// Knob readings, stretched to the full range.
    pub fn main_stretched(&self) -> Raw {
        stretch_knob(self.main)
    }
    pub fn x_stretched(&self) -> Raw {
        stretch_knob(self.x)
    }
    pub fn y_stretched(&self) -> Raw {
        stretch_knob(self.y)
    }
    pub fn switch(&self) -> SwitchPosition {
        SwitchPosition::from_raw(self.switch_raw)
    }
}

/// Everything the firmware talks to.
pub struct Board {
    pub pulse_in_1: Input<'static>,
    pub pulse_in_2: Input<'static>,
    pub pulse_out_1: Output<'static>,
    pub pulse_out_2: Output<'static>,
    /// Three PWM slices, each driving two LEDs as its A and B channels.
    led_slices: [Pwm<'static>; 3],
    /// One slice driving both CV outputs: A = CV 2 (GPIO22), B = CV 1 (GPIO23).
    cv: Pwm<'static>,
    /// Current LED compare values, so setting one LED does not disturb its
    /// slice-mate. A PWM config write sets both channels at once.
    led_levels: [u16; 6],
    /// Same, for the two CV outputs sharing a slice.
    cv_levels: [u16; 2],
    dac: Spi<'static, SPI0, spi::Blocking>,
    dac_cs: Output<'static>,
    adc: Adc<'static, adc::Async>,
    mux_a: Output<'static>,
    mux_b: Output<'static>,
    mux_io_1: AdcChannel<'static>,
    mux_io_2: AdcChannel<'static>,
    /// Output calibration, one per DAC channel.
    pub cal: [ChannelCalibration; 2],
}

impl Board {
    /// Claim the peripherals and bring the hardware up.
    ///
    /// `irqs` is the caller's ADC interrupt binding — Embassy requires it to be
    /// declared with `bind_interrupts!` at the binary level.
    pub async fn new(
        p: Peripherals,
        adc_irqs: impl embassy_rp::interrupt::typelevel::Binding<
                embassy_rp::interrupt::typelevel::ADC_IRQ_FIFO,
                adc::InterruptHandler,
            > + 'static,
        i2c_irqs: impl embassy_rp::interrupt::typelevel::Binding<
                embassy_rp::interrupt::typelevel::I2C0_IRQ,
                i2c::InterruptHandler<I2C0>,
            > + 'static,
    ) -> Self {
        // Pulse inputs: pull-up is mandatory, it biases the input transistor.
        // Without it these do not read at all.
        let pulse_in_1 = Input::new(p.PIN_2, Pull::Up);
        let pulse_in_2 = Input::new(p.PIN_3, Pull::Up);

        // Pulse outputs are inverted: driving high is the idle (gate low)
        // state, so that is where we start.
        let pulse_out_1 = Output::new(p.PIN_8, Level::High);
        let pulse_out_2 = Output::new(p.PIN_9, Level::High);

        let led_cfg = {
            let mut c = pwm::Config::default();
            c.top = LED_PWM_TOP;
            c
        };
        // The six LEDs are GPIO10..15, which pair onto PWM slices 5, 6 and 7 as
        // (A,B). One slice drives both its pins, so each slice is claimed once
        // with `new_output_ab`. Paired LEDs therefore share `top`, but they have
        // independent compare values, so per-LED brightness still works - see
        // `set_led`, which is why LEDs are addressed as (slice, channel).
        let led_slices = [
            Pwm::new_output_ab(p.PWM_SLICE5, p.PIN_10, p.PIN_11, led_cfg.clone()),
            Pwm::new_output_ab(p.PWM_SLICE6, p.PIN_12, p.PIN_13, led_cfg.clone()),
            Pwm::new_output_ab(p.PWM_SLICE7, p.PIN_14, p.PIN_15, led_cfg),
        ];

        let cv_cfg = {
            let mut c = pwm::Config::default();
            c.top = CV_PWM_TOP;
            c
        };
        // Both CV outs are on slice 3: GPIO22 = 3A, GPIO23 = 3B. Same deal as
        // the LEDs - one slice, two independent compare values.
        let cv = Pwm::new_output_ab(p.PWM_SLICE3, p.PIN_22, p.PIN_23, cv_cfg);

        // DAC: MCP4822 over SPI0, 16-bit frames, MSB first, mode 0.
        let mut spi_cfg = spi::Config::default();
        spi_cfg.frequency = 15_625_000;
        spi_cfg.polarity = spi::Polarity::IdleLow;
        spi_cfg.phase = spi::Phase::CaptureOnFirstTransition;
        let dac = Spi::new_blocking_txonly(p.SPI0, p.PIN_18, p.PIN_19, spi_cfg);
        let dac_cs = Output::new(p.PIN_21, Level::High);

        // ADC: knobs and CV come through the mux on channels 2 and 3.
        let adc = Adc::new(p.ADC, adc_irqs, adc::Config::default());
        let mux_io_1 = AdcChannel::new_pin(p.PIN_28, Pull::None);
        let mux_io_2 = AdcChannel::new_pin(p.PIN_29, Pull::None);
        let mux_a = Output::new(p.PIN_24, Level::Low);
        let mux_b = Output::new(p.PIN_25, Level::Low);

        // Calibration lives on the module's EEPROM, so read it once at boot.
        let mut i2c_cfg = i2c::Config::default();
        i2c_cfg.frequency = 400_000;
        let mut eeprom = I2c::new_async(p.I2C0, p.PIN_17, p.PIN_16, i2c_irqs, i2c_cfg);
        let cal = read_calibration(&mut eeprom).await;

        Self {
            pulse_in_1,
            pulse_in_2,
            pulse_out_1,
            pulse_out_2,
            led_slices,
            cv,
            led_levels: [0; 6],
            cv_levels: [0; 2],
            dac,
            dac_cs,
            adc,
            mux_a,
            mux_b,
            mux_io_1,
            mux_io_2,
            cal,
        }
    }

    /// Read a pulse input, correcting for the inverted hardware.
    pub fn pulse_in_1_high(&self) -> bool {
        // Inverted: a low GPIO means a high input.
        self.pulse_in_1.is_low()
    }

    pub fn pulse_in_2_high(&self) -> bool {
        self.pulse_in_2.is_low()
    }

    /// Set a pulse output, correcting for the inverted hardware.
    pub fn set_pulse_out(&mut self, channel: usize, high: bool) {
        // Inverted: writing low drives the output high.
        let level = if high { Level::Low } else { Level::High };
        match channel {
            0 => self.pulse_out_1.set_level(level),
            _ => self.pulse_out_2.set_level(level),
        }
    }

    /// Write a voltage to a DAC channel.
    ///
    /// The MCP4822 is a plain, non-inverted 12-bit DAC over the bipolar output
    /// range, so this is a linear map from millivolts to a signed 12-bit code.
    ///
    /// Note what this deliberately does *not* do: it does not apply
    /// `self.cal`. The EEPROM calibration block describes the **PWM CV
    /// outputs** - 19-bit, inverted, a different circuit - and applying it here
    /// would invert pitch and compress a two-octave span into about a third of
    /// the DAC's codes. See `set_cv_millivolts` for the outputs it does belong
    /// to. Calibrating the DAC outs would need its own measured data, which the
    /// EEPROM format does not carry.
    pub fn set_dac_millivolts(&mut self, channel: usize, mv: i32) {
        let code = millivolts_to_code(mv);
        let word = command_word(
            if channel == 0 {
                DacChannel::A
            } else {
                DacChannel::B
            },
            code,
        );
        self.dac_cs.set_low();
        let _ = self.dac.blocking_write(&word.to_be_bytes());
        self.dac_cs.set_high();
    }

    /// Write a voltage to a CV output, through the EEPROM calibration.
    ///
    /// This is what the calibration block actually describes: the inverted,
    /// filtered-PWM CV outputs. The fitted line already accounts for the
    /// inversion, so no extra flip is needed here - `set_cv_raw` inverts the
    /// duty cycle, and the calibration maps millivolts to the 19-bit CV scale,
    /// which we shift down to the PWM's 11 bits.
    pub fn set_cv_millivolts(&mut self, channel: usize, mv: i32) {
        let cal = &self.cal[channel.min(1)];
        let wide = cal.line.dac_for_millivolts(mv);
        // 19-bit calibration scale down to the 11-bit PWM. The calibration line
        // is inverted, and set_cv_raw inverts again, so pass the value straight
        // through rather than double-correcting.
        let raw = (wide >> 8) as u16;
        self.set_cv_raw(channel, CV_PWM_TOP.saturating_sub(raw.min(CV_PWM_TOP)));
    }

    /// Write a CV output as a duty cycle, correcting for the inverted PWM.
    ///
    /// Both CV outs share PWM slice 3, and a config write sets both channels, so
    /// the other channel's current level is rewritten alongside. Without that
    /// bookkeeping, setting CV 1 would reset CV 2 to whatever was in the config.
    pub fn set_cv_raw(&mut self, channel: usize, value: u16) {
        let ch = channel.min(1);
        self.cv_levels[ch] = value.min(CV_PWM_TOP);

        let mut cfg = pwm::Config::default();
        cfg.top = CV_PWM_TOP;
        // Inverted: a higher duty gives a lower voltage.
        // Channel A is GPIO22 = CV 2; channel B is GPIO23 = CV 1.
        cfg.compare_a = CV_PWM_TOP - self.cv_levels[1];
        cfg.compare_b = CV_PWM_TOP - self.cv_levels[0];
        self.cv.set_config(&cfg);
    }

    /// Set an LED's brightness, 0..=4095, with gamma correction.
    ///
    /// LEDs pair onto shared PWM slices, and a config write sets both channels,
    /// so the slice-mate's level is rewritten alongside.
    pub fn set_led(&mut self, index: usize, brightness: u16) {
        if index >= 6 {
            return;
        }
        self.led_levels[index] = brightness.min(LED_PWM_TOP);

        let slice = index / 2;
        let a = self.led_levels[slice * 2];
        let b = self.led_levels[slice * 2 + 1];

        let mut cfg = pwm::Config::default();
        cfg.top = LED_PWM_TOP;
        cfg.compare_a = gamma_correct(a);
        cfg.compare_b = gamma_correct(b);
        self.led_slices[slice].set_config(&cfg);
    }

    /// Scan all four mux positions and return the panel state.
    pub async fn read_panel(&mut self) -> Panel {
        let mut panel = Panel::default();

        for addr in MuxAddress::ALL {
            let (a, b) = addr.select_lines();
            self.mux_a.set_level(if a { Level::High } else { Level::Low });
            self.mux_b.set_level(if b { Level::High } else { Level::Low });

            // The mux needs time to settle; reading immediately blends this
            // position with the previous one.
            Timer::after(Duration::from_micros(SETTLE_MICROS)).await;

            let knob = self.adc.read(&mut self.mux_io_1).await.unwrap_or(0);
            let cv = self.adc.read(&mut self.mux_io_2).await.unwrap_or(0);

            match addr {
                MuxAddress::MainKnobAndCv1 => {
                    panel.main = knob;
                    panel.cv_1 = cv;
                }
                MuxAddress::XKnobAndCv2 => {
                    panel.x = knob;
                    panel.cv_2 = cv;
                }
                MuxAddress::YKnobAndCv1 => {
                    panel.y = knob;
                    panel.cv_1 = cv;
                }
                MuxAddress::SwitchAndCv2 => {
                    panel.switch_raw = knob;
                    panel.cv_2 = cv;
                }
            }
        }

        panel
    }
}

/// Read the output calibration block from the module's EEPROM.
///
/// Falls back to the documented defaults on any failure — a dead or
/// uncalibrated EEPROM must not stop the sequencer, since being slightly out of
/// range beats being silent mid-set.
async fn read_calibration<'d, T: i2c::Instance>(
    eeprom: &mut I2c<'d, T, i2c::Async>,
) -> [ChannelCalibration; 2] {
    let mut buf = [0u8; CAL_BLOCK_LEN];
    // Two-byte read: write the offset, then read the block.
    match eeprom.write_read_async(EEPROM_ADDR, [0u8], &mut buf).await {
        Ok(()) => [
            calibration::channel_or_default(&buf, 0),
            calibration::channel_or_default(&buf, 1),
        ],
        Err(_) => [
            ChannelCalibration::default_uncalibrated(),
            ChannelCalibration::default_uncalibrated(),
        ],
    }
}

/// Perceptual brightness correction.
///
/// Perceived brightness is roughly the square of the duty cycle, so square the
/// input. Without this the bottom of the range is most of the visible travel and
/// the LEDs look like they only have two states.
fn gamma_correct(brightness: u16) -> u16 {
    let b = brightness.min(LED_PWM_TOP) as u32;
    ((b * b) >> 12) as u16
}

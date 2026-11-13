//! ST7789P3 240x320 display bring-up, mirroring `components/bsp/src/bsp_display.c`
//! from the C firmware.
//!
//! Wiring (from `bsp_pins.h`), 4-line SPI at mode 0:
//!
//! | Signal | GPIO | notes                                |
//! |--------|------|--------------------------------------|
//! | MOSI   | 9    | SPI2                                 |
//! | SCLK   | 8    | SPI2                                 |
//! | CS     | 1    |                                      |
//! | DC     | 20   | command / data select                |
//! | RST    | —    | not wired to the MCU, software reset |
//! | BL     | 21   | LEDC PWM backlight                   |
//!
//! The panel is factory-inverted, hence `ColorInversion::Inverted`.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    mono_font::{ascii::FONT_10X20, MonoTextStyle},
    text::{Alignment, Text},
};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{DriveMode, Level, Output, OutputConfig},
    ledc::{
        channel::{self, ChannelIFace},
        timer::{self, Timer, TimerIFace},
        Ledc, LowSpeed, LSGlobalClkSource,
    },
    peripherals::Peripherals,
    spi::{
        master::{Config as SpiConfig, Spi},
        Mode,
    },
    time::Rate,
};
use static_cell::StaticCell;

/// Native panel resolution, before any rotation.
pub const WIDTH: u16 = 240;
pub const HEIGHT: u16 = 320;

/// Scratch buffer mipidsi uses to batch pixel bytes before each SPI write.
static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();

/// The LEDC timer must outlive the channel that borrows it. Peripherals are
/// `'static`, so parking the timer in static memory satisfies that without a
/// self-referential struct.
static BACKLIGHT_TIMER: StaticCell<Timer<'static, LowSpeed>> = StaticCell::new();

// Type aliases keep the driver's signatures readable.
type SpiBus<'a> = Spi<'a, Blocking>;
type SpiDev<'a> = ExclusiveDevice<SpiBus<'a>, Output<'a>, NoDelay>;
type SpiIf<'a> = mipidsi::interface::SpiInterface<'a, SpiDev<'a>, Output<'a>>;

/// The concrete ST7789 display type the rest of the app draws into.
pub type Panel = mipidsi::Display<SpiIf<'static>, St7789P3, mipidsi::NoResetPin>;

// ---------------------------------------------------------------------------
// Vendor-specific ST7789P3 model.
//
// The C firmware ships an init table (porch / power / gamma) taken from the
// panel maker's reference example; these are NOT the ST7789 generic defaults,
// and the maker warns the panel will look wrong without them. `Builder::init`
// sends the standard sleep-out / MADCTL / INVON / COLMOD / DISPON sequence, so
// we only need to slot the vendor commands in at the right point.
// ---------------------------------------------------------------------------
pub struct St7789P3;

/// (command, params, post-command delay in ms). Mirrors `ST7789P3_CMDS`.
const VENDOR_INIT: &[(u8, &[u8], u16)] = &[
    (0xB2, &[0x05, 0x05, 0x00, 0x33, 0x33], 0),          // PORCTRL
    (0xB7, &[0x35], 0),                                   // GCTRL
    (0xBB, &[0x21], 0),                                   // VCOMS
    (0xC0, &[0x2C], 0),                                   // LCMCTRL
    (0xC2, &[0x01], 0),                                   // VDVVRHEN
    (0xC3, &[0x0B], 0),                                   // VRHS
    (0xC4, &[0x20], 0),                                   // VDVSET
    (0xC6, &[0x0F], 0),                                   // FRCTRL2
    (0xD0, &[0xA7, 0xA1], 0),                             // PWCTRL1
    (0xD0, &[0xA4, 0xA1], 0),                             // PWCTRL1 (re-sent)
    (0xD6, &[0xA1], 0),
    (0xE0, &[0xD0, 0x04, 0x08, 0x0A, 0x09, 0x05, 0x2D, 0x43,
             0x49, 0x09, 0x16, 0x15, 0x26, 0x2B], 0),       // PVGAMCTRL
    (0xE1, &[0xD0, 0x03, 0x09, 0x0A, 0x0A, 0x06, 0x2E, 0x44,
             0x40, 0x3A, 0x15, 0x15, 0x26, 0x2A], 10),      // NVGAMCTRL
];

impl mipidsi::models::Model for St7789P3 {
    type ColorFormat = Rgb565;

    const FRAMEBUFFER_SIZE: (u16, u16) = (WIDTH, HEIGHT);
    // Reset pin is not wired; the builder falls back to a soft reset.
    const RESET_DURATION: u32 = 10;

    fn init<DELAY, DI>(
        &mut self,
        di: &mut DI,
        delay: &mut DELAY,
        options: &mipidsi::options::ModelOptions,
    ) -> Result<mipidsi::dcs::SetAddressMode, mipidsi::models::ModelInitError<DI::Error>>
    where
        DELAY: embedded_hal::delay::DelayNs,
        DI: mipidsi::interface::Interface,
    {
        use mipidsi::dcs::{
            BitsPerPixel, EnterNormalMode, ExitSleepMode, InterfaceExt, PixelFormat,
            SetAddressMode, SetDisplayOn, SetInvertMode, SetPixelFormat,
        };

        // Hardware reset would normally happen here; we have none, and the
        // builder already issued a soft reset. Give the panel time to settle.
        delay.delay_us(150_000);

        let madctl = SetAddressMode::from(options);

        di.write_command(ExitSleepMode)?;
        delay.delay_us(10_000);

        di.write_command(madctl)?;

        // Vendor-specific porch / power / gamma configuration.
        for &(cmd, params, delay_ms) in VENDOR_INIT {
            di.write_raw(cmd, params)?;
            if delay_ms != 0 {
                delay.delay_ms(u32::from(delay_ms));
            }
        }

        di.write_command(SetInvertMode::new(options.invert_colors))?;

        let pf = PixelFormat::with_all(BitsPerPixel::from_rgb_color::<Self::ColorFormat>());
        di.write_command(SetPixelFormat::new(pf))?;
        delay.delay_us(10_000);

        di.write_command(EnterNormalMode)?;
        delay.delay_us(10_000);

        di.write_command(SetDisplayOn)?;
        delay.delay_us(120_000);

        Ok(madctl)
    }
}

pub struct DisplayBoard {
    pub display: Panel,
    pub backlight: Backlight<'static>,
}

pub struct Backlight<'a> {
    channel: esp_hal::ledc::channel::Channel<'a, LowSpeed>,
}

impl Backlight<'_> {
    /// Set brightness in percent (0..=100).
    pub fn set(&self, percent: u8) {
        self.channel.set_duty(percent.min(100)).ok();
    }
}

/// Initialize SPI, the ST7789 panel and the LEDC backlight.
///
/// Returns the display and backlight ready to use, with the panel already
/// powered on and the boot screen painted. Contents are otherwise undefined.
pub fn init(p: Peripherals) -> DisplayBoard {
    // --- Backlight: LEDC low-speed timer 0 / channel 0, 5 kHz, 10-bit ---
    let mut ledc = Ledc::new(p.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let timer = BACKLIGHT_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
    timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty10Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(5),
        })
        .expect("backlight LEDC timer config");

    let mut bl_channel = ledc.channel(channel::Number::Channel0, p.GPIO21);
    bl_channel
        .configure(channel::config::Config {
            timer: &*timer,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("backlight LEDC channel config");

    // --- SPI bus + CS/DC pins ---
    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(80))
            .with_mode(Mode::_0),
    )
    .expect("spi init")
    .with_sck(p.GPIO8)
    .with_mosi(p.GPIO9);

    let cs = Output::new(p.GPIO1, Level::High, OutputConfig::default());
    let dc = Output::new(p.GPIO20, Level::Low, OutputConfig::default());

    let spi_dev = ExclusiveDevice::new_no_delay(spi, cs).expect("spi device");

    let buffer: &'static mut [u8] = DISPLAY_BUFFER.init([0u8; 512]);
    let di = mipidsi::interface::SpiInterface::new(spi_dev, dc, buffer);

    let mut delay = Delay::new();

    // RST is not routed to the MCU, so no reset pin is supplied; the panel is
    // reset in software inside the driver.
    let mut display = mipidsi::Builder::new(St7789P3, di)
        .display_size(WIDTH, HEIGHT)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .color_order(mipidsi::options::ColorOrder::Rgb)
        .init(&mut delay)
        .expect("display init");

    paint_boot_screen(&mut display);

    // Only light the panel once the first frame is in the framebuffer.
    bl_channel.set_duty(100).ok();

    DisplayBoard {
        display,
        backlight: Backlight {
            channel: bl_channel,
        },
    }
}

/// Paint the "Python in the work / Rust in the board" splash.
fn paint_boot_screen(display: &mut Panel) {
    let bg = Rgb565::new(4, 18, 31); // sky-ish blue
    let fg = Rgb565::WHITE;

    display.clear(bg).ok();

    let style = MonoTextStyle::new(&FONT_10X20, fg);

    Text::with_alignment(
        "Python in the work",
        Point::new(WIDTH as i32 / 2, HEIGHT as i32 / 2 - 24),
        style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    Text::with_alignment(
        "Rust in the board",
        Point::new(WIDTH as i32 / 2, HEIGHT as i32 / 2 + 24),
        style,
        Alignment::Center,
    )
    .draw(display)
    .ok();
}

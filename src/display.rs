use std::fmt::Debug;

use anyhow::Context;
use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget};
use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{AnyInputPin, Gpio4, Gpio48, Gpio5, Gpio6, Gpio7, PinDriver};
use esp_idf_svc::hal::spi::config::{Config, MODE_0};
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriverConfig, SPI2};
use esp_idf_svc::hal::units::MegaHertz;
use log::info;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ILI9342CRgb565;
use mipidsi::options::{ColorOrder, Orientation};
use mipidsi::Builder;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;

// https://docs.espressif.com/projects/esp-idf/en/release-v5.5/esp32s3/api-reference/peripherals/gpio.html

/// Initialize the display with SPI interface
///
/// # Errors
/// Returns error if SPI device creation fails, GPIO pin configuration fails,
/// or display initialization fails
pub fn display(
    spi2: SPI2,
    sclk: Gpio7,
    sdo: Gpio6, // aka miso
    cs: Gpio5,
    dc: Gpio4,
    reset: Gpio48,
    // blacklight: Gpio47,
    buffer: &mut [u8],
) -> anyhow::Result<impl DrawTarget<Color = Rgb565, Error = impl Debug> + use<'_>> {
    let config = Config {
        baudrate: MegaHertz::from(40).into(),
        data_mode: MODE_0,
        ..Default::default()
    };

    let bus_config = SpiDriverConfig::new();

    let spi_device = SpiDeviceDriver::new_single::<SPI2>(
        spi2,
        sclk,                // sclk: serial clock
        sdo,                 // sdo: serial data output / MISO
        None::<AnyInputPin>, //Some(mosi), // sdi: serial data input / MOSI
        // None::<AnyOutputPin>, //Some(cs),   // cs: chip select
        Some(cs),
        &bus_config,
        &config,
    )
    .context("create SPI device")?;

    // Define the Data/Command select pin as a digital output (pins.gpio7)
    // LCD interface: use GPIO4 for DC.
    let dc = PinDriver::output(dc).context("data/command pin")?;

    // Display interface
    let di = SpiInterface::new(spi_device, dc, buffer);

    // Reset (GPIO48)
    let rst = PinDriver::output_od(reset).context("reset pin")?;

    // Delay
    // let mut delay = Delay::new_default();
    let mut delay = Ets;

    // Build display
    let display = Builder::new(ILI9342CRgb565, di)
        .reset_pin(rst)
        .color_order(ColorOrder::Bgr)
        .orientation(Orientation::new().flip_horizontal().flip_vertical())
        .display_size(WIDTH, HEIGHT)
        .init(&mut delay)
        .map_err(|err| anyhow::anyhow!("init display: {err:#?}"))?;

    info!("Display initialized");

    Ok(display)
}

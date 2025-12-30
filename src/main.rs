use anyhow::Context;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, hal::prelude::Peripherals, nvs::EspDefaultNvsPartition,
};

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Initialize NVS - required for WiFi calibration data and HTTP client
    let _nvs = EspDefaultNvsPartition::take().context("Failed to initialize NVS")?;

    let peripherals = Peripherals::take().context("Failed to take peripherals")?;
    let sysloop = EspSystemEventLoop::take().context("create system event loop")?;

    toboggan_esp32::run(peripherals, sysloop)
}

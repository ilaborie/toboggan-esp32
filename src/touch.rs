//! Capacitive touchscreen on the shared I2C bus (SDA GPIO8, SCL GPIO18).
//!
//! This is read-only for now: it reports where the screen was touched and does
//! nothing with it. See `docs/touch-and-imu.md` for what advancing the deck on
//! tap would additionally need.

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::i2c::I2cDriver;
use gt911::{Gt911Blocking, Point};
use log::{info, warn};

use crate::config::display::{SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::config::touch::{GT911_ADDRESSES, PROBE_TIMEOUT};

/// The touch controller, bound to the bus driver it is read through.
pub type TouchController = Gt911Blocking<I2cDriver<'static>>;

/// Names the parts the ESP32-S3-BOX-3 is known to carry, so a bus scan reads as
/// an inventory rather than a list of numbers.
fn describe(address: u8) -> &'static str {
    match address {
        0x14 | 0x5D => "GT911 touch",
        0x18 => "ES8311 codec",
        0x24 => "TT21100 touch",
        0x40 | 0x41 => "ES7210 ADC",
        0x68 | 0x69 => "ICM-42607 IMU",
        _ => "unknown",
    }
}

/// Probes every 7-bit address and logs whatever answers.
///
/// A zero-length write is the standard probe: it puts the address on the bus and
/// stops, so an ACK means something is listening and nothing is disturbed.
pub fn scan_bus(i2c: &mut I2cDriver<'static>) -> Vec<u8> {
    let timeout = TickType::from(PROBE_TIMEOUT).into();
    let found = (0x08..=0x77_u8)
        .filter(|address| i2c.write(*address, &[], timeout).is_ok())
        .collect::<Vec<_>>();

    if found.is_empty() {
        warn!("👆 I2C scan found nothing on SDA=8 SCL=18");
    } else {
        let listing = found
            .iter()
            .map(|address| format!("{address:#04x} ({})", describe(*address)))
            .collect::<Vec<_>>()
            .join(", ");
        info!("👆 I2C scan found {listing}");
    }

    found
}

/// Finds the GT911, trying both addresses it can have latched at reset.
///
/// `init` verifies the controller reports a `"911\0"` product id, so a wrong
/// guess fails cleanly rather than leaving us talking to the wrong chip.
pub fn find_touch(i2c: &mut I2cDriver<'static>) -> Option<TouchController> {
    for address in GT911_ADDRESSES {
        let controller = TouchController::new(address);
        match controller.init(i2c) {
            Ok(()) => {
                info!("👆 GT911 ready at {address:#04x}");
                return Some(controller);
            }
            Err(error) => info!("👆 No GT911 at {address:#04x}: {error:?}"),
        }
    }
    None
}

/// Maps a controller coordinate onto the frame as it is actually drawn.
///
/// Measured, not inferred: the controller's origin lands at the *bottom left* of
/// the drawn image, so only the vertical axis disagrees with
/// `embedded-graphics`, whose origin is the top left. An earlier guess flipped
/// both axes - the display is built with `flip_horizontal().flip_vertical()`, so
/// 180 degrees looked right on paper, but the touch panel is mounted turned the
/// same way and the two cancel on x.
///
/// Only the y flip is confirmed. If a touch near the top right ever reads as a
/// low x, the horizontal axis needs the same treatment; the raw values are
/// logged alongside so that is one glance rather than another guess.
#[must_use]
pub fn to_screen(point: &Point) -> (u16, u16) {
    let x = point.x.min(SCREEN_WIDTH.saturating_sub(1));
    let y = SCREEN_HEIGHT.saturating_sub(1).saturating_sub(point.y);
    (x, y)
}

/// Which half of the screen a touch landed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Right,
}

/// Splits the screen down the middle.
///
/// Decided on the *drawn* coordinate rather than the raw one, so if the
/// horizontal mapping in [`to_screen`] is ever corrected the zones follow it
/// instead of silently inverting.
///
/// No dead band in the centre: a tap that does nothing reads as a broken
/// screen and invites a harder second tap, which is worse during a talk than
/// stepping the wrong way once.
#[must_use]
pub fn zone(point: &Point) -> Zone {
    let (x, _) = to_screen(point);
    if x < SCREEN_WIDTH / 2 {
        Zone::Left
    } else {
        Zone::Right
    }
}

/// Reports one touch.
pub fn log_touch(point: &Point) {
    let (x, y) = to_screen(point);
    info!(
        "👆 Touch raw=({}, {}) screen=({x}, {y}) {:?} id={} area={}",
        point.x,
        point.y,
        zone(point),
        point.track_id,
        point.area
    );
}

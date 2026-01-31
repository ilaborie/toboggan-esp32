//! ESP32-S3-BOX-3B Toboggan Presentation Controller
//!
//! This application controls a presentation display with:
//! - WiFi connectivity
//! - REST API for talk data
//! - WebSocket for real-time slide updates
//! - LED status indicators
//! - LCD display for slide content

use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use embedded_graphics::image::Image;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::prelude::Peripherals;
use log::info;
use tinybmp::Bmp;

use crate::config::display::{BOOT_IMAGE_AREA_HEIGHT, BUFFER_SIZE};
use crate::config::env::WIFI_SSID;
use crate::config::timing::MAIN_LOOP_POLL_INTERVAL;
use crate::services::{spawn_api_thread, spawn_websocket_thread, spawn_wifi_thread};

mod wifi;
pub use self::wifi::*;

mod api;
pub use self::api::*;

mod websocket;
pub use self::websocket::*;

mod display;
pub use self::display::*;

mod state;
pub use self::state::*;

mod led;
pub use self::led::*;

mod display_manager;
pub use self::display_manager::*;

mod services;
pub use self::services::{ServiceState, ServiceTracker};

mod boot_image;
mod config;

/// Tracks consecutive hardware failures for graceful degradation
struct FailureTracker {
    display_failures: u8,
    led_failures: u8,
    max_failures: u8,
}

impl FailureTracker {
    fn new(max_failures: u8) -> Self {
        Self {
            display_failures: 0,
            led_failures: 0,
            max_failures,
        }
    }

    fn record_display_failure(&mut self) -> bool {
        self.display_failures = self.display_failures.saturating_add(1);
        self.display_failures >= self.max_failures
    }

    fn record_led_failure(&mut self) -> bool {
        self.led_failures = self.led_failures.saturating_add(1);
        self.led_failures >= self.max_failures
    }

    fn reset_display(&mut self) {
        self.display_failures = 0;
    }

    fn reset_led(&mut self) {
        self.led_failures = 0;
    }
}

/// Run the main application with synchronous threading model
///
/// # Errors
/// Returns error if application initialization fails, display initialization fails,
/// or main loop encounters unrecoverable errors
///
/// # Panics
/// Panics if thread spawning fails for `WiFi`, API, or WebSocket threads
pub fn run(peripherals: Peripherals, sysloop: EspSystemEventLoop) -> anyhow::Result<()> {
    info!("👋 Hello - Starting synchronous ESP32 application");

    let port = config::env::TOBOGGAN_PORT
        .parse::<u16>()
        .context("Expected a numeric port")?;

    let Peripherals {
        pins, spi2, modem, ..
    } = peripherals;

    // Create state diff channel for efficient updates
    let (diff_sender, diff_receiver) = mpsc::channel::<AppStateDiff>();

    // Create talk data channel
    let (talk_data_sender, talk_data_receiver) = mpsc::channel::<TalkData>();

    // Initialize state manager with diff channel (starts in Booting state by default)
    let mut state_manager = StateManager::new(diff_sender.clone());

    // Initialize display
    let mut buffer = [0_u8; BUFFER_SIZE];
    let display = display(
        spi2,
        pins.gpio7,  // sclk
        pins.gpio6,  // sdo
        pins.gpio5,  // dc
        pins.gpio4,  // cs
        pins.gpio48, // reset
        &mut buffer,
    )
    .context("display init")?;

    // Set up backlight
    let mut backlight = PinDriver::output(pins.gpio47).context("backlight")?;
    backlight.set_high().context("activate backlight")?;

    // Initialize LED controller
    let mut led_controller =
        LedController::new(pins.gpio39, pins.gpio40, pins.gpio41).context("initialize LEDs")?;
    led_controller.update(&AppState::Booting)?;

    // Initialize display with Booting state
    let mut display_manager = DisplayManager::new(display).context("create display manager")?;
    let mut talk_data: Option<TalkData> = None;
    if let Err(error) =
        display_manager.update_display(state_manager.current_state(), talk_data.as_ref())
    {
        log::warn!("Failed to initialize display: {error:?}");
    }

    // Deduplication - track last few diffs to prevent rapid cycling
    let mut last_diff: Option<AppStateDiff> = None;

    // Show boot image initially (graceful degradation if parsing fails)
    match Bmp::<Rgb565>::from_slice(boot_image::BOOT_IMAGE) {
        Ok(bmp) => {
            let image_size = bmp.bounding_box().size;
            let x = (320 - i32::try_from(image_size.width).unwrap_or(0)) / 2;
            let y = (BOOT_IMAGE_AREA_HEIGHT - i32::try_from(image_size.height).unwrap_or(0)) / 2;
            if let Err(error) =
                Image::new(&bmp, Point::new(x, y)).draw(&mut display_manager.display)
            {
                log::error!("Failed to display boot image: {error:?}");
            }
        }
        Err(error) => {
            log::error!("Failed to parse boot image: {error:?}");
        }
    }

    info!("🚀 Starting main application loop");

    // Start WiFi connection immediately since we're in Booting state
    spawn_wifi_thread(diff_sender.clone(), modem, sysloop)?;

    state_manager.transition_to(AppState::Connecting {
        ssid: WIFI_SSID.to_string(),
    });

    let mut services = ServiceTracker::default();
    let mut failure_tracker = FailureTracker::new(5);

    // Main loop: handle differential updates and transient effects
    loop {
        // Use shorter timeout when blinking to ensure smooth LED animation
        let timeout = if led_controller.is_blink_override_active() {
            Duration::from_millis(100) // 100ms for smooth blinking
        } else {
            MAIN_LOOP_POLL_INTERVAL
        };

        // Check for state diffs with dynamic timeout
        match diff_receiver.recv_timeout(timeout) {
            Ok(diff) => {
                // Skip duplicate diffs to prevent loops (except Blink which is always processed)
                if !matches!(diff, AppStateDiff::Blink) && last_diff.as_ref() == Some(&diff) {
                    continue;
                }

                info!("📡 State diff received: {diff:?}");

                // Handle blink effect (transient)
                if matches!(diff, AppStateDiff::Blink) {
                    led_controller.start_blink_override(Duration::from_secs(5));
                }

                // Apply the diff locally (don't send back to channel)
                // StateManager logs state transitions internally
                state_manager.apply_diff(&diff);

                last_diff = Some(diff);

                // Check for talk data updates (non-blocking)
                if let Ok(new_talk_data) = talk_data_receiver.try_recv() {
                    info!("📚 Talk data received: {}", new_talk_data.title);
                    talk_data = Some(new_talk_data);
                }

                // Update display with failure tracking
                info!(
                    "📺 Updating display for state: {:?}, talk_data available: {}",
                    state_manager.current_state(),
                    talk_data.is_some()
                );
                match display_manager
                    .update_display(state_manager.current_state(), talk_data.as_ref())
                {
                    Ok(()) => failure_tracker.reset_display(),
                    Err(error) => {
                        log::error!("Failed to update display: {error:?}");
                        if failure_tracker.record_display_failure() {
                            log::error!("Too many consecutive display failures, transitioning to error state");
                            state_manager.transition_to_error("Display hardware failure");
                        }
                    }
                }

                // Update LEDs with failure tracking
                match led_controller.update(state_manager.current_state()) {
                    Ok(()) => failure_tracker.reset_led(),
                    Err(error) => {
                        log::error!("Failed to update LEDs: {error:?}");
                        if failure_tracker.record_led_failure() {
                            log::error!(
                                "Too many consecutive LED failures, transitioning to error state"
                            );
                            state_manager.transition_to_error("LED hardware failure");
                        }
                    }
                }

                // Handle state-specific actions
                match state_manager.current_state() {
                    AppState::Connected { .. } if services.is_api_pending() => {
                        spawn_api_thread(diff_sender.clone(), talk_data_sender.clone(), port)?;
                        state_manager.transition_to(AppState::Loading);
                        services.mark_api_started();
                    }
                    AppState::Initialized
                        if services.is_websocket_pending() && talk_data.is_some() =>
                    {
                        spawn_websocket_thread(diff_sender.clone(), port)?;
                        services.mark_websocket_started();
                    }
                    _ => {} // Other states don't trigger new threads
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Update LEDs (handles blink animation and expiry internally)
                if let Err(error) = led_controller.update(state_manager.current_state()) {
                    log::error!("Failed to update LEDs: {error:?}");
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::error!("State diff channel disconnected");
                break;
            }
        }
    }

    Ok(())
}

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
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::prelude::Peripherals;
use log::info;
use mipidsi::TestImage;

use crate::config::display::BUFFER_SIZE;
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

mod config;

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

    // Show test image initially
    let img = TestImage::<Rgb565>::new();
    if let Err(error) = img.draw(&mut display_manager.display) {
        log::error!("Failed to display test image: {error:?}");
    }

    info!("🚀 Starting main application loop");

    // Start WiFi connection immediately since we're in Booting state
    spawn_wifi_thread(diff_sender.clone(), modem, sysloop)?;

    state_manager.transition_to(AppState::Connecting {
        ssid: WIFI_SSID.to_string(),
    });

    let mut services = ServiceTracker::default();

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
                let old_state = state_manager.current_state().clone();
                state_manager.apply_diff(&diff);
                let new_state = state_manager.current_state().clone();

                // Log state transitions for debugging
                if old_state != new_state {
                    info!("🔄 State transition: {old_state:?} -> {new_state:?}");
                }

                last_diff = Some(diff);

                // Check for talk data updates (non-blocking)
                if let Ok(new_talk_data) = talk_data_receiver.try_recv() {
                    info!("📚 Talk data received: {}", new_talk_data.title);
                    talk_data = Some(new_talk_data);
                }

                // Update display
                info!(
                    "📺 Updating display for state: {:?}, talk_data available: {}",
                    state_manager.current_state(),
                    talk_data.is_some()
                );
                if let Err(error) = display_manager
                    .update_display(state_manager.current_state(), talk_data.as_ref())
                {
                    log::error!("Failed to update display: {error:?}");
                }

                // Update LEDs based on state
                if let Err(error) = led_controller.update(state_manager.current_state()) {
                    log::error!("Failed to update LEDs: {error:?}");
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

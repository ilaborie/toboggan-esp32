//! Background service thread management
//!
//! This module handles spawning and managing to background threads for:
//! - WiFi connection
//! - API communication
//! - WebSocket real-time updates

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{Gpio18, Gpio8};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::{I2cDriver, I2C0};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::units::KiloHertz;
use log::{error, info, warn};

use crate::config::env::{WIFI_PASSWORD, WIFI_SSID};
use crate::config::reconnect::{INITIAL_DELAY, MAX_DELAY};
use crate::config::threading;
use crate::config::touch::{COMMAND_DEBOUNCE, I2C_BAUDRATE_KHZ, POLL_INTERVAL};
use crate::state::{error_chain, send_diff, AppState, AppStateDiff, TalkData};
use crate::touch::{find_touch, log_touch, scan_bus, zone, Zone};
use crate::{connect_to_ws, error_diff, server_addr, wifi_sync, Api, Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkFetch {
    Initial,
    Reload,
}

/// Tracks the life cycle state of background service threads
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ServiceState {
    #[default]
    Pending,
    Started,
}

/// Tracks all background service states
#[derive(Debug, Default)]
pub struct ServiceTracker {
    api: ServiceState,
    websocket: ServiceState,
}

impl ServiceTracker {
    pub fn is_api_pending(&self) -> bool {
        self.api == ServiceState::Pending
    }

    pub fn is_websocket_pending(&self) -> bool {
        self.websocket == ServiceState::Pending
    }

    pub fn mark_api_started(&mut self) {
        self.api = ServiceState::Started;
    }

    pub fn mark_websocket_started(&mut self) {
        self.websocket = ServiceState::Started;
    }
}

/// Spawn `WiFi` connection thread with proper error handling
pub fn spawn_wifi_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<()> {
    info!("🔄 Starting WiFi connection thread");
    thread::Builder::new()
        .name("wifi_thread".to_string())
        .stack_size(threading::WIFI_THREAD_STACK)
        .spawn(move || {
            wifi_thread(diff_sender, modem, sysloop);
        })
        .context("Failed to spawn WiFi thread")?;
    Ok(())
}

/// Spawn API loading thread with proper error handling
pub fn spawn_api_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    talk_data_sender: mpsc::Sender<TalkData>,
    port: u16,
    fetch: TalkFetch,
) -> anyhow::Result<()> {
    info!("📶 Starting API loading ({fetch:?})");
    thread::Builder::new()
        .name("api_thread".to_string())
        .stack_size(threading::API_THREAD_STACK)
        .spawn(move || {
            api_thread(diff_sender, talk_data_sender, port, fetch);
        })
        .context("Failed to spawn API thread")?;
    Ok(())
}

/// Spawn WebSocket connection thread with proper error handling
pub fn spawn_websocket_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    port: u16,
    commands: mpsc::Receiver<Command>,
) -> anyhow::Result<()> {
    info!("🔌 Starting WebSocket connection");
    thread::Builder::new()
        .name("websocket_thread".to_string())
        .stack_size(threading::WEBSOCKET_THREAD_STACK)
        .spawn(move || {
            websocket_thread(diff_sender, port, &commands);
        })
        .context("Failed to spawn WebSocket thread")?;
    Ok(())
}

/// Hands one command to the WebSocket thread, logging a dead channel.
fn send_command(commands: &mpsc::Sender<Command>, command: Command) {
    info!("👆 Tap -> {command:?}");
    if let Err(error) = commands.send(command) {
        error!("Failed to send command: {error}");
    }
}

/// Spawn the touchscreen polling thread
///
/// # Errors
/// Returns error if the thread cannot be spawned
pub fn spawn_touch_thread(
    commands: mpsc::Sender<Command>,
    i2c: I2C0<'static>,
    sda: Gpio8<'static>,
    scl: Gpio18<'static>,
) -> anyhow::Result<()> {
    info!("👆 Starting touch thread");
    thread::Builder::new()
        .name("touch_thread".to_string())
        .stack_size(threading::TOUCH_THREAD_STACK)
        .spawn(move || {
            touch_thread(&commands, i2c, sda, scl);
        })
        .context("Failed to spawn touch thread")?;
    Ok(())
}

/// Touchscreen thread
///
/// Reports failures to the log only, never as an `AppStateDiff::Error`: a box
/// with no touchscreen still shows the talk perfectly well, and the simulator
/// has no touch controller at all.
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn touch_thread(
    commands: &mpsc::Sender<Command>,
    i2c: I2C0<'static>,
    sda: Gpio8<'static>,
    scl: Gpio18<'static>,
) {
    let config = I2cConfig::new().baudrate(KiloHertz::from(I2C_BAUDRATE_KHZ).into());
    let mut i2c = match I2cDriver::new(i2c, sda, scl, &config) {
        Ok(driver) => driver,
        Err(error) => {
            warn!("👆 Could not open the I2C bus: {error}");
            return;
        }
    };

    scan_bus(&mut i2c);

    let Some(controller) = find_touch(&mut i2c) else {
        // Nothing to retry against: the controller is soldered on, so if it did
        // not answer the scan it is not going to appear later.
        warn!("👆 No touch controller found, touch input disabled");
        return;
    };

    // A finger resting on the glass reads as a press on every poll, so the
    // command fires on the press *edge* rather than on the press.
    let mut touching = false;
    let mut last_command: Option<Instant> = None;

    loop {
        match controller.get_touch(&mut i2c) {
            // A press or a move.
            Ok(Some(point)) => {
                if !touching {
                    touching = true;
                    log_touch(&point);
                    let now = Instant::now();
                    if last_command.is_none_or(|at| now.duration_since(at) >= COMMAND_DEBOUNCE) {
                        last_command = Some(now);
                        send_command(
                            commands,
                            match zone(&point) {
                                Zone::Left => Command::PreviousStep,
                                Zone::Right => Command::NextStep,
                            },
                        );
                    }
                }
            }
            // A release. `NotReady` means nothing changed since the last poll,
            // which at 50 Hz is most of them - and crucially is *not* a release,
            // so it must leave `touching` alone.
            Ok(None) => touching = false,
            Err(gt911::Error::NotReady) => {}
            Err(error) => warn!("👆 Touch read failed: {error:?}"),
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// `WiFi` connection thread
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn wifi_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
) {
    info!("📶 WiFi thread started");

    let wifi = match wifi_sync(WIFI_SSID, WIFI_PASSWORD, modem, sysloop) {
        Ok(wifi) => wifi,
        Err(error) => {
            log::error!("❌ WiFi connection failed: {error:?}");
            send_diff(
                &diff_sender,
                error_diff!("WiFi failed: {}", error_chain(&error)),
                "WiFi error",
            );
            return;
        }
    };

    info!("✅ WiFi connected successfully");
    send_diff(
        &diff_sender,
        AppStateDiff::Transition(AppState::Connected {
            ssid: WIFI_SSID.to_string(),
        }),
        "Connected",
    );

    // Keep the WiFi connection alive by holding the wifi object
    // This prevents the WiFi driver from being dropped
    let _wifi = wifi; // Move ownership to keep it alive

    // Sleep forever to keep the thread and WiFi connection alive
    loop {
        thread::sleep(Duration::from_secs(3600)); // Sleep for 1 hour at a time
    }
}

/// API loading thread
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn api_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    talk_data_sender: mpsc::Sender<TalkData>,
    port: u16,
    fetch: TalkFetch,
) {
    info!("🌐 API thread started ({fetch:?})");

    if let Err(error) = server_addr(port).and_then(|address| {
        let base_url = format!("http://{address}");
        info!("🌐 Base URL: {base_url}");
        load_talk(&talk_data_sender, base_url)
    }) {
        match fetch {
            TalkFetch::Initial => {
                log::error!("❌ Failed to load talk: {error:?}");
                send_diff(
                    &diff_sender,
                    error_diff!("{}", error_chain(&error)),
                    "Talk loading error",
                );
            }
            TalkFetch::Reload => {
                log::warn!("⚠️ Talk reload failed, keeping the previous talk: {error:?}");
            }
        }
        return;
    }

    // Only the first load announces this, and it is what starts the WebSocket.
    if fetch == TalkFetch::Initial {
        send_diff(
            &diff_sender,
            AppStateDiff::Transition(AppState::Initialized),
            "Initialized",
        );
    }
}

/// Fetches the talk and hands it to the main loop.
fn load_talk(talk_data_sender: &mpsc::Sender<TalkData>, base_url: String) -> anyhow::Result<()> {
    let mut api = Api::new(base_url).context("create the API client")?;
    let talk_data = api.talk()?;
    info!(
        "📚 Talk loaded: title='{}', slides: {}",
        talk_data.title,
        talk_data.slide_count()
    );

    talk_data_sender
        .send(talk_data)
        .context("send the talk data")?;

    Ok(())
}

/// WebSocket connection thread with automatic reconnection
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn websocket_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    port: u16,
    commands: &mpsc::Receiver<Command>,
) {
    info!("🔌 WebSocket thread started");

    let mut delay = INITIAL_DELAY;

    loop {
        // Already resolved by the API thread, which must succeed before this one
        // is ever spawned, so this is a cache hit rather than a second lookup.
        match server_addr(port).and_then(|address| connect_to_ws(address, &diff_sender, commands)) {
            Ok(()) => {
                // Connection closed gracefully, reset delay for next attempt
                info!("🔌 WebSocket connection closed, will reconnect");
                delay = INITIAL_DELAY;
            }
            Err(err) => {
                error!("🔌 WebSocket connection failed: {err:?}");
            }
        }

        info!("🔌 Reconnecting WebSocket in {:?}", delay);
        thread::sleep(delay);

        // Exponential backoff, capped at MAX_DELAY
        delay = delay.saturating_mul(2).min(MAX_DELAY);
    }
}

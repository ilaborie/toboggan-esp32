//! Background service thread management
//!
//! This module handles spawning and managing to background threads for:
//! - WiFi connection
//! - API communication
//! - WebSocket real-time updates

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use log::{error, info};

use crate::config::env::{TOBOGGAN_HOST, WIFI_PASSWORD, WIFI_SSID};
use crate::config::reconnect::{INITIAL_DELAY, MAX_DELAY};
use crate::config::threading;
use crate::state::{send_diff, AppState, AppStateDiff, TalkData};
use crate::{connect_to_ws, error_diff, wifi_sync, Api};

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
    modem: Modem,
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
) -> anyhow::Result<()> {
    info!("📶 WiFi connected, starting API loading");
    thread::Builder::new()
        .name("api_thread".to_string())
        .stack_size(threading::API_THREAD_STACK)
        .spawn(move || {
            api_thread(diff_sender, talk_data_sender, port);
        })
        .context("Failed to spawn API thread")?;
    Ok(())
}

/// Spawn WebSocket connection thread with proper error handling
pub fn spawn_websocket_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    port: u16,
) -> anyhow::Result<()> {
    info!("🔌 Starting WebSocket connection");
    thread::Builder::new()
        .name("websocket_thread".to_string())
        .stack_size(threading::WEBSOCKET_THREAD_STACK)
        .spawn(move || {
            websocket_thread(diff_sender, port);
        })
        .context("Failed to spawn WebSocket thread")?;
    Ok(())
}

/// `WiFi` connection thread
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn wifi_thread(diff_sender: mpsc::Sender<AppStateDiff>, modem: Modem, sysloop: EspSystemEventLoop) {
    info!("📶 WiFi thread started");

    match wifi_sync(WIFI_SSID, WIFI_PASSWORD, modem, sysloop) {
        Ok(wifi) => {
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
        Err(error) => {
            log::error!("❌ WiFi connection failed: {error:?}");
            send_diff(
                &diff_sender,
                error_diff!("WiFi failed: {error}"),
                "WiFi error",
            );
        }
    }
}

/// API loading thread
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn api_thread(
    diff_sender: mpsc::Sender<AppStateDiff>,
    talk_data_sender: mpsc::Sender<TalkData>,
    port: u16,
) {
    info!("🌐 API thread started");

    let base_url = format!("http://{TOBOGGAN_HOST}:{port}");
    info!("🌐 Base URL: {base_url}");

    match Api::new(base_url) {
        Ok(mut api) => {
            info!("🌐 API client created successfully");
            match api.talk() {
                Ok(talk_data) => {
                    info!(
                        "📚 Talk loaded: title='{}', slides: {}",
                        talk_data.title,
                        talk_data.slide_count()
                    );

                    // Send talk data through dedicated channel
                    if let Err(error) = talk_data_sender.send(talk_data) {
                        log::error!("Failed to send talk data: {error}");
                    }

                    // Send simple initialized state
                    send_diff(
                        &diff_sender,
                        AppStateDiff::Transition(AppState::Initialized),
                        "Initialized",
                    );
                }
                Err(error) => {
                    log::error!("❌ Failed to load talk: {error:?}");
                    send_diff(
                        &diff_sender,
                        error_diff!("Talk loading failed: {error}"),
                        "Talk loading error",
                    );
                }
            }
        }
        Err(error) => {
            log::error!("❌ Failed to create API client: {error:?}");
            send_diff(
                &diff_sender,
                error_diff!("API client failed: {error}"),
                "API client error",
            );
        }
    }
}

/// WebSocket connection thread with automatic reconnection
#[allow(clippy::needless_pass_by_value)] // Need owned values for thread
fn websocket_thread(diff_sender: mpsc::Sender<AppStateDiff>, port: u16) {
    info!("🔌 WebSocket thread started");

    let mut delay = INITIAL_DELAY;

    loop {
        match connect_to_ws(TOBOGGAN_HOST, port, &diff_sender) {
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

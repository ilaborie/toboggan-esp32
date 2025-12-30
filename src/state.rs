use std::sync::mpsc;

use crate::config::app::{BOOTING_TEXT, ERROR_PREFIX};
use crate::config::network::{CONNECTING_TEXT_PREFIX, CONNECTING_TEXT_SUFFIX, LOADING_TALK_TEXT};

/// Helper to send state diffs with consistent error logging
pub fn send_diff(sender: &mpsc::Sender<AppStateDiff>, diff: AppStateDiff, context: &str) {
    if let Err(error) = sender.send(diff) {
        log::error!("Failed to send {context} diff: {error}");
    }
}

/// Creates an `AppStateDiff::Error` with format! style arguments
///
/// # Examples
/// ```ignore
/// error_diff!("WiFi failed: {}", error)
/// error_diff!("Connection timeout after {timeout:?}")
/// ```
#[macro_export]
macro_rules! error_diff {
    ($($arg:tt)*) => {
        $crate::AppStateDiff::Error { message: format!($($arg)*) }
    };
}

/// Static talk content that doesn't change during presentation
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct TalkData {
    pub title: String,
    pub slides: Vec<String>,
    pub step_counts: Vec<usize>,
}

impl TalkData {
    #[must_use]
    pub fn new(title: String, slides: Vec<String>, step_counts: Vec<usize>) -> Self {
        Self {
            title,
            slides,
            step_counts,
        }
    }

    #[must_use]
    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    #[must_use]
    pub fn get_slide(&self, index: usize) -> Option<&str> {
        self.slides.get(index).map(String::as_str)
    }

    #[must_use]
    pub fn get_next_slide(&self, current: usize) -> Option<&str> {
        self.slides.get(current + 1).map(String::as_str)
    }

    /// Get step count for a slide (defaults to 0 if not available)
    #[must_use]
    pub fn get_step_count(&self, index: usize) -> usize {
        self.step_counts.get(index).copied().unwrap_or(0)
    }
}

/// Differential updates for efficient state management
#[derive(Debug, Clone, PartialEq)]
pub enum AppStateDiff {
    /// Transition to a completely new state
    Transition(AppState),
    /// Update slide position, step, and mode (only valid in Play state)
    UpdateSlide {
        current: usize,
        current_step: usize,
        mode: StateMode,
    },
    /// Trigger LED blink effect (transient, doesn't change core state)
    Blink,
    /// Error occurred (can happen from any state)
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum AppState {
    Booting,
    Connecting {
        ssid: String,
    },
    Connected {
        ssid: String,
    },
    Loading,
    Initialized,
    Play {
        current: usize,
        current_step: usize,
        mode: StateMode,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Hash)]
pub enum StateMode {
    Paused,
    Running,
    Done,
}

impl AppState {
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            AppState::Booting => BOOTING_TEXT.into(),
            AppState::Connecting { ssid } => {
                format!("{CONNECTING_TEXT_PREFIX}{ssid}{CONNECTING_TEXT_SUFFIX}")
            }
            AppState::Connected { ssid } => {
                format!("Connected to {ssid}")
            }
            AppState::Loading => LOADING_TALK_TEXT.into(),
            AppState::Initialized => "Talk loaded and ready".into(),
            AppState::Play {
                current,
                current_step,
                mode,
            } => {
                format!("Slide {current} step {current_step} ({mode:?})")
            }
            AppState::Error { message } => format!("{ERROR_PREFIX}{message}"),
        }
    }

    #[must_use]
    pub fn is_presentation_active(&self) -> bool {
        matches!(self, AppState::Initialized | AppState::Play { .. })
    }

    /// Apply a differential update to this state
    #[must_use]
    pub fn apply_diff(self, diff: AppStateDiff) -> Self {
        match diff {
            AppStateDiff::Transition(new_state) => new_state,
            AppStateDiff::UpdateSlide {
                current,
                current_step,
                mode,
            } => match self {
                // Transition from Initialized to Play on first slide update
                AppState::Initialized | AppState::Play { .. } => AppState::Play {
                    current,
                    current_step,
                    mode,
                },
                // Ignore slide updates in other states
                other => other,
            },
            AppStateDiff::Error { message } => AppState::Error { message },
            // Blink is transient and doesn't change the core state
            AppStateDiff::Blink => self,
        }
    }
}

pub struct StateManager {
    current_state: AppState,
    diff_sender: mpsc::Sender<AppStateDiff>,
}

impl StateManager {
    #[must_use]
    pub fn new(diff_sender: mpsc::Sender<AppStateDiff>) -> Self {
        Self {
            current_state: AppState::Booting,
            diff_sender,
        }
    }

    #[must_use]
    pub fn current_state(&self) -> &AppState {
        &self.current_state
    }

    /// Apply a differential update (internal use - doesn't send to channel)
    pub fn apply_diff(&mut self, diff: &AppStateDiff) {
        let old_state = self.current_state.clone();
        self.current_state = self.current_state.clone().apply_diff(diff.clone());

        if old_state != self.current_state {
            log::info!(
                "State change: {old_state:?} -> {:?} (via diff: {diff:?})",
                self.current_state,
            );
        }
    }

    /// Send a diff to all subscribers (and apply it locally)
    fn send_diff(&mut self, diff: AppStateDiff) {
        // Apply locally first
        self.apply_diff(&diff);

        // Then send to subscribers
        if let Err(error) = self.diff_sender.send(diff) {
            log::error!("Failed to send state diff: {error}");
        }
    }

    /// Convenience method for state transitions
    pub fn transition_to(&mut self, new_state: AppState) {
        self.send_diff(AppStateDiff::Transition(new_state));
    }

    /// Convenience method for slide updates
    pub fn update_slide(&mut self, current: usize, current_step: usize, mode: StateMode) {
        self.send_diff(AppStateDiff::UpdateSlide {
            current,
            current_step,
            mode,
        });
    }

    /// Convenience method for blink effect
    pub fn trigger_blink(&mut self) {
        self.send_diff(AppStateDiff::Blink);
    }

    /// Convenience method for errors
    pub fn transition_to_error(&mut self, error_message: impl Into<String>) {
        let message = error_message.into();
        self.send_diff(error_diff!("{message}"));
    }
}

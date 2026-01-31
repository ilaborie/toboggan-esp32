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
        let old_state = std::mem::replace(&mut self.current_state, AppState::Booting);
        self.current_state = old_state.clone().apply_diff(diff.clone());

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

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // TalkData Tests
    // =============================================================================

    #[test]
    fn talk_data_new() {
        let data = TalkData::new(
            "Test Talk".to_string(),
            vec!["Slide 1".to_string(), "Slide 2".to_string()],
            vec![2, 3],
        );

        assert_eq!(data.title, "Test Talk");
        assert_eq!(data.slides.len(), 2);
        assert_eq!(data.step_counts.len(), 2);
    }

    #[test]
    fn talk_data_slide_count() {
        let data = TalkData::new(
            "Test".to_string(),
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            vec![],
        );

        assert_eq!(data.slide_count(), 3);
    }

    #[test]
    fn talk_data_get_slide() {
        let data = TalkData::new(
            "Test".to_string(),
            vec!["First".to_string(), "Second".to_string()],
            vec![],
        );

        assert_eq!(data.get_slide(0), Some("First"));
        assert_eq!(data.get_slide(1), Some("Second"));
        assert_eq!(data.get_slide(2), None);
    }

    #[test]
    fn talk_data_get_next_slide() {
        let data = TalkData::new(
            "Test".to_string(),
            vec![
                "First".to_string(),
                "Second".to_string(),
                "Third".to_string(),
            ],
            vec![],
        );

        assert_eq!(data.get_next_slide(0), Some("Second"));
        assert_eq!(data.get_next_slide(1), Some("Third"));
        assert_eq!(data.get_next_slide(2), None);
    }

    #[test]
    fn talk_data_get_step_count_default() {
        let data = TalkData::new(
            "Test".to_string(),
            vec!["Slide".to_string()],
            vec![], // Empty step counts
        );

        // Should default to 0 when step_counts is empty
        assert_eq!(data.get_step_count(0), 0);
        assert_eq!(data.get_step_count(100), 0);
    }

    #[test]
    fn talk_data_get_step_count_with_data() {
        let data = TalkData::new(
            "Test".to_string(),
            vec!["A".to_string(), "B".to_string()],
            vec![3, 5],
        );

        assert_eq!(data.get_step_count(0), 3);
        assert_eq!(data.get_step_count(1), 5);
        assert_eq!(data.get_step_count(2), 0); // Out of bounds defaults to 0
    }

    // =============================================================================
    // AppState::apply_diff Tests
    // =============================================================================

    #[test]
    fn apply_diff_transition() {
        let state = AppState::Booting;
        let diff = AppStateDiff::Transition(AppState::Loading);

        let new_state = state.apply_diff(diff);

        assert_eq!(new_state, AppState::Loading);
    }

    #[test]
    fn apply_diff_update_slide_from_initialized() {
        let state = AppState::Initialized;
        let diff = AppStateDiff::UpdateSlide {
            current: 5,
            current_step: 2,
            mode: StateMode::Running,
        };

        let new_state = state.apply_diff(diff);

        assert_eq!(
            new_state,
            AppState::Play {
                current: 5,
                current_step: 2,
                mode: StateMode::Running,
            }
        );
    }

    #[test]
    fn apply_diff_update_slide_from_play() {
        let state = AppState::Play {
            current: 1,
            current_step: 0,
            mode: StateMode::Running,
        };
        let diff = AppStateDiff::UpdateSlide {
            current: 2,
            current_step: 1,
            mode: StateMode::Paused,
        };

        let new_state = state.apply_diff(diff);

        assert_eq!(
            new_state,
            AppState::Play {
                current: 2,
                current_step: 1,
                mode: StateMode::Paused,
            }
        );
    }

    #[test]
    fn apply_diff_update_slide_ignored_in_wrong_state() {
        let state = AppState::Booting;
        let diff = AppStateDiff::UpdateSlide {
            current: 5,
            current_step: 2,
            mode: StateMode::Running,
        };

        let new_state = state.apply_diff(diff);

        // Should remain in Booting state
        assert_eq!(new_state, AppState::Booting);
    }

    #[test]
    fn apply_diff_update_slide_ignored_in_connecting() {
        let state = AppState::Connecting {
            ssid: "test".to_string(),
        };
        let diff = AppStateDiff::UpdateSlide {
            current: 0,
            current_step: 0,
            mode: StateMode::Running,
        };

        let new_state = state.apply_diff(diff);

        assert_eq!(
            new_state,
            AppState::Connecting {
                ssid: "test".to_string()
            }
        );
    }

    #[test]
    fn apply_diff_blink_preserves_state() {
        let state = AppState::Play {
            current: 3,
            current_step: 1,
            mode: StateMode::Running,
        };
        let diff = AppStateDiff::Blink;

        let new_state = state.apply_diff(diff);

        // Blink should not change the state
        assert_eq!(
            new_state,
            AppState::Play {
                current: 3,
                current_step: 1,
                mode: StateMode::Running,
            }
        );
    }

    #[test]
    fn apply_diff_error_from_booting() {
        let state = AppState::Booting;
        let diff = AppStateDiff::Error {
            message: "Test error".to_string(),
        };

        let new_state = state.apply_diff(diff);

        assert_eq!(
            new_state,
            AppState::Error {
                message: "Test error".to_string()
            }
        );
    }

    #[test]
    fn apply_diff_error_from_play() {
        let state = AppState::Play {
            current: 5,
            current_step: 2,
            mode: StateMode::Running,
        };
        let diff = AppStateDiff::Error {
            message: "Connection lost".to_string(),
        };

        let new_state = state.apply_diff(diff);

        assert_eq!(
            new_state,
            AppState::Error {
                message: "Connection lost".to_string()
            }
        );
    }

    // =============================================================================
    // AppState::is_presentation_active Tests
    // =============================================================================

    #[test]
    fn is_presentation_active_true_for_initialized() {
        let state = AppState::Initialized;
        assert!(state.is_presentation_active());
    }

    #[test]
    fn is_presentation_active_true_for_play() {
        let state = AppState::Play {
            current: 0,
            current_step: 0,
            mode: StateMode::Running,
        };
        assert!(state.is_presentation_active());
    }

    #[test]
    fn is_presentation_active_false_for_booting() {
        let state = AppState::Booting;
        assert!(!state.is_presentation_active());
    }

    #[test]
    fn is_presentation_active_false_for_connecting() {
        let state = AppState::Connecting {
            ssid: "test".to_string(),
        };
        assert!(!state.is_presentation_active());
    }

    #[test]
    fn is_presentation_active_false_for_error() {
        let state = AppState::Error {
            message: "test".to_string(),
        };
        assert!(!state.is_presentation_active());
    }
}

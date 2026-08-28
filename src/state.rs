use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use crate::config::text::{ERROR_MESSAGE_CAPACITY, SSID_CAPACITY};

/// An error message, bounded to roughly what the panel can render.
///
/// Server error text and flattened `anyhow` chains are otherwise unbounded, and
/// this lands in an `AppState` that is cloned on every state change. Anything
/// past this was already being cut by the display.
pub type ErrorMessage = heapless::String<ERROR_MESSAGE_CAPACITY>;

/// A WiFi SSID. 802.11 caps one at 32 bytes, so this can never truncate.
pub type Ssid = heapless::String<SSID_CAPACITY>;

/// Copies as much of `text` as fits, cut at a character boundary.
///
/// Truncation is deliberate: an error the box cannot display in full is still
/// worth showing, and failing here would blank the screen instead.
#[must_use]
pub fn bounded<const N: usize>(text: &str) -> heapless::String<N> {
    let mut out = heapless::String::new();
    let end = text
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|&end| end <= N)
        .last()
        .unwrap_or(0);
    // Cannot fail: `end` was chosen to fit.
    let _ = out.push_str(&text[..end]);
    out
}

/// Helper to send state diffs with consistent error logging
pub fn send_diff(sender: &mpsc::Sender<AppStateDiff>, diff: AppStateDiff, context: &str) {
    if let Err(error) = sender.send(diff) {
        log::error!("Failed to send {context} diff: {error}");
    }
}

/// Flattens an [`anyhow::Error`] cause chain into one displayable line.
///
/// `{error}` renders only the outermost context, so a failure surfaces on screen
/// as "load the talk" with the reason it actually failed hidden in the chain.
/// This keeps every link, innermost cause last.
pub fn error_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
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
        $crate::AppStateDiff::Error {
            message: $crate::bounded(&format!($($arg)*)),
        }
    };
}

/// Hands every `TalkData` a distinct id. Only ever compared for equality, so
/// wrapping after 4 billion decks would be harmless even if it were reachable.
static NEXT_GENERATION: AtomicU32 = AtomicU32::new(0);

/// Static talk content that doesn't change during presentation
///
/// Deliberately not `Hash`: the display used to hash every slide title on every
/// pass of the 100 ms main loop just to notice a reload. `generation` answers
/// the same question in one word - see [`TalkData::generation`].
#[derive(Debug, Clone, PartialEq)]
pub struct TalkData {
    title: Box<str>,
    /// Every slide title, concatenated. One allocation for the lot rather than
    /// one per slide: a 60-slide deck was 61 separate blocks, freed and taken
    /// again on every reload - and toboggan is a watch server, so a deck is
    /// rebuilt every time a file is saved mid-talk.
    slides: Box<str>,
    /// End offset of each slide title within `slides`. Each title starts where
    /// the previous one ended, so ends alone describe every span.
    ends: Box<[usize]>,
    step_counts: Box<[usize]>,
    generation: u32,
}

impl TalkData {
    #[must_use]
    pub fn new(title: String, slides: Vec<String>, step_counts: Vec<usize>) -> Self {
        let mut text = String::with_capacity(slides.iter().map(String::len).sum());
        let mut ends = Vec::with_capacity(slides.len());
        for slide in &slides {
            text.push_str(slide);
            ends.push(text.len());
        }

        Self {
            title: title.into_boxed_str(),
            slides: text.into_boxed_str(),
            ends: ends.into_boxed_slice(),
            // `Box<[T]>` rather than `Vec<T>`: there is no spare capacity to
            // carry around, and it cannot silently regrow later.
            step_counts: step_counts.into_boxed_slice(),
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Identifies this deck. Distinct for every `TalkData` built, so a reload
    /// changes it even when the rebuilt deck has identical content.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    #[must_use]
    pub fn slide_count(&self) -> usize {
        self.ends.len()
    }

    #[must_use]
    pub fn get_slide(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)?;
        // Each title starts where the previous ended; the first at zero.
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.ends[previous]);
        self.slides.get(start..end)
    }

    #[must_use]
    pub fn get_next_slide(&self, current: usize) -> Option<&str> {
        self.get_slide(current + 1)
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
    /// Need re-fetch talk
    TalkReload,
    /// Error occurred (can happen from any state)
    Error { message: ErrorMessage },
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum AppState {
    Booting,
    Connecting {
        ssid: Ssid,
    },
    Connected {
        ssid: Ssid,
    },
    Loading,
    Initialized,
    Play {
        current: usize,
        current_step: usize,
        mode: StateMode,
    },
    Error {
        message: ErrorMessage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Hash)]
pub enum StateMode {
    Running,
    Done,
}

impl AppState {
    #[must_use]
    pub fn is_presentation_active(&self) -> bool {
        matches!(self, AppState::Initialized | AppState::Play { .. })
    }

    /// Apply a differential update to this state
    ///
    /// Takes the diff by reference and clones only in the two arms that keep
    /// its payload: `Blink`, `TalkReload` and `UpdateSlide` - the ones that
    /// arrive on every tap - now copy nothing at all.
    #[must_use]
    pub fn apply_diff(self, diff: &AppStateDiff) -> Self {
        match diff {
            AppStateDiff::Transition(new_state) => new_state.clone(),
            AppStateDiff::UpdateSlide {
                current,
                current_step,
                mode,
            } => match self {
                // Transition from Initialized to Play on first slide update
                AppState::Initialized | AppState::Play { .. } => AppState::Play {
                    current: *current,
                    current_step: *current_step,
                    mode: *mode,
                },
                // Ignore slide updates in other states
                other => other,
            },
            AppStateDiff::Error { message } => AppState::Error {
                message: message.clone(),
            },
            // Neither moves the deck: Blink is a transient LED effect, and a
            // reload only refreshes the talk data held alongside this state.
            AppStateDiff::Blink | AppStateDiff::TalkReload => self,
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
        // Kept for the comparison and the log line below. Now that `ssid` and
        // `message` are bounded, an `AppState` is entirely inline - so this
        // clone is a small memcpy and touches the allocator not at all, which
        // is the whole point of bounding them.
        let previous = old_state.clone();
        self.current_state = old_state.apply_diff(diff);

        if previous != self.current_state {
            log::info!("State change: {previous:?} -> {:?}", self.current_state,);
        }
    }

    /// Send a diff to all subscribers (and apply it locally)
    fn send_diff(&mut self, diff: AppStateDiff) {
        // Apply locally first. Borrowed, so the diff below is the same value
        // rather than a copy of it.
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
    pub fn transition_to_error(&mut self, error_message: &str) {
        self.send_diff(AppStateDiff::Error {
            message: bounded(error_message),
        });
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

        assert_eq!(data.title(), "Test Talk");
        assert_eq!(data.slide_count(), 2);
        assert_eq!(data.get_step_count(0), 2);
        assert_eq!(data.get_step_count(1), 3);
    }

    #[test]
    fn talk_data_slides_round_trip_through_the_arena() {
        // Titles are stored concatenated, so the spans have to put them back
        // exactly - including an empty one, which shares its neighbour's offset.
        let slides = vec![
            "First".to_string(),
            String::new(),
            "Troisième été".to_string(),
        ];
        let data = TalkData::new("T".to_string(), slides.clone(), vec![]);

        assert_eq!(data.slide_count(), 3);
        for (index, expected) in slides.iter().enumerate() {
            assert_eq!(data.get_slide(index), Some(expected.as_str()));
        }
        assert_eq!(data.get_slide(3), None);
    }

    #[test]
    fn talk_data_generation_is_distinct_per_deck() {
        // What the display uses to notice a reload, in place of hashing every
        // slide title on every pass of the main loop.
        let first = TalkData::new("T".to_string(), vec!["A".to_string()], vec![]);
        let second = TalkData::new("T".to_string(), vec!["A".to_string()], vec![]);
        assert_ne!(first.generation(), second.generation());
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

        let new_state = state.apply_diff(&diff);

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

        let new_state = state.apply_diff(&diff);

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
            mode: StateMode::Done,
        };

        let new_state = state.apply_diff(&diff);

        assert_eq!(
            new_state,
            AppState::Play {
                current: 2,
                current_step: 1,
                mode: StateMode::Done,
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

        let new_state = state.apply_diff(&diff);

        // Should remain in Booting state
        assert_eq!(new_state, AppState::Booting);
    }

    #[test]
    fn apply_diff_update_slide_ignored_in_connecting() {
        let state = AppState::Connecting {
            ssid: bounded("test"),
        };
        let diff = AppStateDiff::UpdateSlide {
            current: 0,
            current_step: 0,
            mode: StateMode::Running,
        };

        let new_state = state.apply_diff(&diff);

        assert_eq!(
            new_state,
            AppState::Connecting {
                ssid: bounded("test")
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

        let new_state = state.apply_diff(&diff);

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
            message: bounded("Test error"),
        };

        let new_state = state.apply_diff(&diff);

        assert_eq!(
            new_state,
            AppState::Error {
                message: bounded("Test error")
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
            message: bounded("Connection lost"),
        };

        let new_state = state.apply_diff(&diff);

        assert_eq!(
            new_state,
            AppState::Error {
                message: bounded("Connection lost")
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
            ssid: bounded("test"),
        };
        assert!(!state.is_presentation_active());
    }

    #[test]
    fn is_presentation_active_false_for_error() {
        let state = AppState::Error {
            message: bounded("test"),
        };
        assert!(!state.is_presentation_active());
    }
}

use embedded_graphics::mono_font::iso_8859_1::{FONT_9X18, FONT_9X18_BOLD};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use heapless::Vec as HeaplessVec;
use log::info;

use crate::config::display::{
    BOOT_STATUS_LINE, COLOR_BLACK, COLOR_CYAN, COLOR_ERROR_BACKGROUND, COLOR_GREEN, COLOR_ORANGE,
    COLOR_RED, COLOR_WHITE, COLOR_YELLOW, CURRENT_SLIDE_LINE_START, CURRENT_SLIDE_MAX_LINES,
    ERROR_MAX_LINES, LINE_HEIGHT, MAX_CHARS_PER_LINE, NEXT_SLIDE_LINE_START, NEXT_SLIDE_MAX_LINES,
    PROGRESS_BAR_HEIGHT, PROGRESS_BAR_LINE, PROGRESS_BAR_MARGIN, SSID_MAX_LINES,
    STEP_DOT_MAX_VISIBLE, STEP_DOT_RADIUS, STEP_DOT_SPACING, STEP_INDICATOR_LINE, TITLE_LINE_START,
    TITLE_MAX_LINES,
};
use crate::state::{AppState, StateMode, TalkData};

pub struct DisplayManager<D>
where
    D: DrawTarget<Color = Rgb565>,
{
    pub display: D,
    current_state_hash: u64,
    boot_image_displayed: bool,
}

impl<D> DisplayManager<D>
where
    D: DrawTarget<Color = Rgb565>,
{
    /// Create a new display manager with the given display
    ///
    /// # Errors
    /// Returns error if display initialization fails
    pub fn new(mut display: D) -> anyhow::Result<Self> {
        display
            .clear(COLOR_BLACK)
            .map_err(|_| anyhow::anyhow!("Failed to clear display"))?;

        Ok(Self {
            display,
            current_state_hash: 0,
            boot_image_displayed: true,
        })
    }

    /// Update the display based on the application state and talk data
    ///
    /// # Errors
    /// Returns error if display rendering fails
    pub fn update_display(
        &mut self,
        state: &AppState,
        talk_data: Option<&TalkData>,
    ) -> anyhow::Result<()> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Hash the deck's generation rather than its contents: hashing every
        // slide title here walked several KB on every pass of the 100 ms loop,
        // and a reload is exactly what a new generation means.
        let mut hasher = DefaultHasher::new();
        state.hash(&mut hasher);
        if let Some(data) = talk_data {
            data.generation().hash(&mut hasher);
        }
        let new_state_hash = hasher.finish();

        if self.current_state_hash != new_state_hash {
            info!("Updating display for state: {state:?}");
            self.current_state_hash = new_state_hash;
            self.render_state(state, talk_data)?;
        }

        Ok(())
    }

    /// Render the complete state-based display layout
    fn render_state(
        &mut self,
        state: &AppState,
        talk_data: Option<&TalkData>,
    ) -> anyhow::Result<()> {
        // Check if this is a boot state (displays boot image + status line)
        let is_boot_state = matches!(
            state,
            AppState::Booting
                | AppState::Connecting { .. }
                | AppState::Connected { .. }
                | AppState::Loading
        );

        if is_boot_state && self.boot_image_displayed {
            // Only render status line at the bottom, keep boot image visible
            match state {
                AppState::Booting => {
                    self.render_boot_status("Booting...", COLOR_ORANGE)?;
                }
                AppState::Connecting { ssid } => {
                    let text = format!("Connecting to {ssid}...");
                    self.render_boot_status(&text, COLOR_YELLOW)?;
                }
                AppState::Connected { ssid } => {
                    let text = format!("Connected to {ssid}");
                    self.render_boot_status(&text, COLOR_GREEN)?;
                }
                AppState::Loading => {
                    self.render_boot_status("Loading talk...", COLOR_WHITE)?;
                }
                _ => {}
            }
            return Ok(());
        }

        // Transition away from boot image - clear and mark as no longer displayed
        if self.boot_image_displayed {
            self.boot_image_displayed = false;
        }

        // Clear display
        let background_color = match state {
            AppState::Error { .. } => COLOR_ERROR_BACKGROUND,
            _ => COLOR_BLACK,
        };

        self.display
            .clear(background_color)
            .map_err(|_| anyhow::anyhow!("Failed to clear display"))?;

        match state {
            AppState::Play {
                current,
                current_step,
                mode,
            } => {
                if let Some(data) = talk_data {
                    self.render_play_state(data, *current, *current_step, *mode)?;
                } else {
                    self.render_simple_state("No talk data", COLOR_RED, TITLE_LINE_START + 3)?;
                }
            }
            AppState::Initialized => {
                if let Some(data) = talk_data {
                    // Title on lines 1-2, "Ready" on line 4
                    self.render_title(data.title(), COLOR_CYAN)?;
                    self.render_simple_state("Ready", COLOR_GREEN, CURRENT_SLIDE_LINE_START)?;
                } else {
                    self.render_simple_state("Loading...", COLOR_YELLOW, TITLE_LINE_START + 3)?;
                }
            }
            AppState::Error { message } => {
                self.render_simple_state("ERROR", COLOR_RED, TITLE_LINE_START)?;
                let error_lines = wrap_text::<ERROR_MAX_LINES>(message, MAX_CHARS_PER_LINE);
                self.draw_text_at_lines(
                    &error_lines,
                    CURRENT_SLIDE_LINE_START,
                    COLOR_RED,
                    &FONT_9X18,
                )?;
            }
            // Boot states with boot_image_displayed=false (shouldn't happen normally)
            AppState::Booting => {
                self.render_simple_state("Booting...", COLOR_ORANGE, TITLE_LINE_START + 3)?;
            }
            AppState::Connecting { ssid } => {
                self.render_simple_state("Connecting", COLOR_YELLOW, TITLE_LINE_START)?;
                let ssid_lines = wrap_text::<SSID_MAX_LINES>(ssid, MAX_CHARS_PER_LINE);
                self.draw_text_at_lines(
                    &ssid_lines,
                    CURRENT_SLIDE_LINE_START,
                    COLOR_YELLOW,
                    &FONT_9X18,
                )?;
            }
            AppState::Connected { ssid } => {
                self.render_simple_state("Connected", COLOR_GREEN, TITLE_LINE_START)?;
                let ssid_lines = wrap_text::<SSID_MAX_LINES>(ssid, MAX_CHARS_PER_LINE);
                self.draw_text_at_lines(
                    &ssid_lines,
                    CURRENT_SLIDE_LINE_START,
                    COLOR_WHITE,
                    &FONT_9X18,
                )?;
            }
            AppState::Loading => {
                self.render_simple_state("Loading talk...", COLOR_WHITE, TITLE_LINE_START + 3)?;
            }
        }

        Ok(())
    }

    /// Render the play state with title, current slide, step indicators, and next slide
    fn render_play_state(
        &mut self,
        talk_data: &TalkData,
        current: usize,
        current_step: usize,
        mode: StateMode,
    ) -> anyhow::Result<()> {
        // Determine title color based on play mode
        let title_color = match mode {
            StateMode::Running => COLOR_GREEN,
            StateMode::Done => COLOR_CYAN,
        };

        // Render progress bar (line 1)
        self.render_progress_bar(current, talk_data.slide_count(), mode)?;

        // Render talk title (lines 2-3)
        self.render_title(talk_data.title(), title_color)?;

        // Render step indicators (line 4) if this slide has steps
        let step_count = talk_data.get_step_count(current);
        if step_count > 0 {
            self.render_step_indicators(current_step, step_count, mode)?;
        }

        // Render current slide (lines 5-8, bold)
        if let Some(current_slide) = talk_data.get_slide(current) {
            let current_lines =
                wrap_text::<CURRENT_SLIDE_MAX_LINES>(current_slide, MAX_CHARS_PER_LINE);
            self.draw_text_at_lines(
                &current_lines,
                CURRENT_SLIDE_LINE_START,
                COLOR_WHITE,
                &FONT_9X18_BOLD,
            )?;
        }

        // Render next slide (lines 10-11) if available
        if let Some(next_slide) = talk_data.get_next_slide(current) {
            let next_lines = wrap_text::<NEXT_SLIDE_MAX_LINES>(next_slide, MAX_CHARS_PER_LINE);
            self.draw_text_at_lines(&next_lines, NEXT_SLIDE_LINE_START, COLOR_CYAN, &FONT_9X18)?;
        }

        Ok(())
    }

    /// Render step indicator dots: filled for completed/current, empty for remaining
    fn render_step_indicators(
        &mut self,
        current_step: usize,
        step_count: usize,
        mode: StateMode,
    ) -> anyhow::Result<()> {
        let display_size = self.display.bounding_box().size;
        let display_width = i32::try_from(display_size.width).expect("width fits in i32");

        // Limit visible dots (step_count is additional steps, so total positions = step_count + 1)
        let visible_count = (step_count + 1).min(STEP_DOT_MAX_VISIBLE);

        // Calculate starting X position to center the dots
        let total_width = (visible_count as i32 - 1) * STEP_DOT_SPACING;
        let start_x = (display_width - total_width) / 2;
        let y = STEP_INDICATOR_LINE * LINE_HEIGHT;

        // Choose colors based on mode
        let filled_color = match mode {
            StateMode::Running => COLOR_GREEN,
            StateMode::Done => COLOR_CYAN,
        };

        for i in 0..visible_count {
            let x = start_x + (i as i32) * STEP_DOT_SPACING;
            let top_left = Point::new(
                x - i32::try_from(STEP_DOT_RADIUS).expect("radius fits"),
                y - i32::try_from(STEP_DOT_RADIUS).expect("radius fits"),
            );

            if i <= current_step {
                // Filled dot for completed and current steps
                let style = PrimitiveStyle::with_fill(filled_color);
                Circle::new(top_left, STEP_DOT_RADIUS * 2)
                    .into_styled(style)
                    .draw(&mut self.display)
                    .map_err(|_| anyhow::anyhow!("Failed to draw filled step dot"))?;
            } else {
                // Empty dot (stroke only) for remaining steps
                let style = PrimitiveStyleBuilder::new()
                    .stroke_color(COLOR_WHITE)
                    .stroke_width(1)
                    .build();
                Circle::new(top_left, STEP_DOT_RADIUS * 2)
                    .into_styled(style)
                    .draw(&mut self.display)
                    .map_err(|_| anyhow::anyhow!("Failed to draw empty step dot"))?;
            }
        }

        Ok(())
    }

    /// Render progress bar showing presentation progress
    fn render_progress_bar(
        &mut self,
        current: usize,
        total: usize,
        mode: StateMode,
    ) -> anyhow::Result<()> {
        let display_size = self.display.bounding_box().size;
        let display_width = i32::try_from(display_size.width).expect("width fits in i32");

        // Calculate bar dimensions
        let bar_x = PROGRESS_BAR_MARGIN;
        let bar_y = PROGRESS_BAR_LINE * LINE_HEIGHT
            - i32::try_from(PROGRESS_BAR_HEIGHT / 2).expect("height fits");
        let bar_width =
            u32::try_from(display_width - 2 * PROGRESS_BAR_MARGIN).expect("bar width fits");

        // Draw background (outline)
        let bg_style = PrimitiveStyleBuilder::new()
            .stroke_color(COLOR_WHITE)
            .stroke_width(1)
            .build();
        Rectangle::new(
            Point::new(bar_x, bar_y),
            Size::new(bar_width, PROGRESS_BAR_HEIGHT),
        )
        .into_styled(bg_style)
        .draw(&mut self.display)
        .map_err(|_| anyhow::anyhow!("Failed to draw progress bar background"))?;

        // Calculate fill width using (current + 1) / total to show progress even on first slide
        // (slide 0 shows 1/N progress, not 0/N)
        let progress = (current + 1) as f32 / total as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let fill_width = (bar_width as f32 * progress) as u32;

        // Draw filled portion
        let fill_color = match mode {
            StateMode::Running => COLOR_GREEN,
            StateMode::Done => COLOR_CYAN,
        };
        let fill_style = PrimitiveStyle::with_fill(fill_color);
        Rectangle::new(
            Point::new(bar_x, bar_y),
            Size::new(fill_width, PROGRESS_BAR_HEIGHT),
        )
        .into_styled(fill_style)
        .draw(&mut self.display)
        .map_err(|_| anyhow::anyhow!("Failed to draw progress bar fill"))?;

        Ok(())
    }

    /// Render a simple centered status message
    fn render_simple_state(&mut self, text: &str, color: Rgb565, line: i32) -> anyhow::Result<()> {
        let display_size = self.display.bounding_box().size;
        let center_x = i32::try_from(display_size.width).expect("display width fits in i32") / 2;
        let y_pos = line * LINE_HEIGHT;

        let text_style = MonoTextStyle::new(&FONT_9X18, color);
        Text::with_alignment(
            text,
            Point::new(center_x, y_pos),
            text_style,
            Alignment::Center,
        )
        .draw(&mut self.display)
        .map_err(|_| anyhow::anyhow!("Failed to draw status text"))?;

        Ok(())
    }

    /// Render boot status text at the bottom of the screen (line 12)
    /// Clears only the status line area, preserving the boot image above
    fn render_boot_status(&mut self, text: &str, color: Rgb565) -> anyhow::Result<()> {
        let display_size = self.display.bounding_box().size;
        let display_width = display_size.width;

        // Clear only the status line area (bottom 20 pixels)
        let status_y = BOOT_STATUS_LINE * LINE_HEIGHT;
        let status_rect = Rectangle::new(
            Point::new(0, status_y - LINE_HEIGHT / 2),
            Size::new(
                display_width,
                u32::try_from(LINE_HEIGHT).expect("line height fits"),
            ),
        );
        status_rect
            .into_styled(PrimitiveStyle::with_fill(COLOR_BLACK))
            .draw(&mut self.display)
            .map_err(|_| anyhow::anyhow!("Failed to clear status area"))?;

        // Draw status text centered on line 12
        self.render_simple_state(text, color, BOOT_STATUS_LINE)
    }

    /// Render the talk title with proper wrapping
    fn render_title(&mut self, title: &str, color: Rgb565) -> anyhow::Result<()> {
        let title_lines = wrap_text::<TITLE_MAX_LINES>(title, MAX_CHARS_PER_LINE);
        self.draw_text_at_lines(&title_lines, TITLE_LINE_START, color, &FONT_9X18)
    }

    /// Draw text lines at specific line positions
    fn draw_text_at_lines(
        &mut self,
        lines: &[&str],
        start_line: i32,
        color: Rgb565,
        font: &embedded_graphics::mono_font::MonoFont,
    ) -> anyhow::Result<()> {
        let display_size = self.display.bounding_box().size;
        let center_x = i32::try_from(display_size.width).expect("display width fits in i32") / 2;
        let text_style = MonoTextStyle::new(font, color);

        for (index, line) in lines.iter().enumerate() {
            let y_pos =
                (start_line + i32::try_from(index).expect("line index fits in i32")) * LINE_HEIGHT;
            Text::with_alignment(
                line,
                Point::new(center_x, y_pos),
                text_style,
                Alignment::Center,
            )
            .draw(&mut self.display)
            .map_err(|_| anyhow::anyhow!("Failed to draw text line '{line}'"))?;
        }

        Ok(())
    }
}

/// Wrap text to fit within specified character width.
///
/// Returns slices of `text`: choosing split points is the whole job, so
/// there is nothing to copy. `N` caps the result, which is what every
/// caller used to do with a second `.take(N).collect()`.
///
/// Widths are counted in characters, not bytes - the previous version
/// compared `str::len()` against a character budget, so an accented title
/// wrapped a line early.
fn wrap_text<const N: usize>(text: &str, max_chars: usize) -> HeaplessVec<&str, N> {
    let mut lines = HeaplessVec::new();

    if text.chars().count() <= max_chars {
        // Cannot fail for any N >= 1, and an N of 0 renders nothing anyway.
        let _ = lines.push(text);
        return lines;
    }

    // The line being built, as a byte range into `text`. Slicing the
    // original preserves whatever whitespace it had; rebuilding from
    // `split_whitespace` would silently normalize runs of it to one space.
    let mut line: Option<(usize, usize)> = None;

    for (word_start, word_end) in word_ranges(text) {
        line = match line {
            // Measured across the whole candidate line, so the separating
            // whitespace is counted as it actually appears.
            Some((start, _)) if text[start..word_end].chars().count() <= max_chars => {
                Some((start, word_end))
            }
            Some((start, end)) => {
                if lines.push(&text[start..end]).is_err() {
                    return lines;
                }
                Some((word_start, word_end))
            }
            None => Some((word_start, word_end)),
        };
    }

    if let Some((start, end)) = line {
        let _ = lines.push(&text[start..end]);
    }

    lines
}

/// Byte ranges of the whitespace-separated words in `text`, in order.
fn word_ranges(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = None;
    text.char_indices()
        // A virtual trailing separator, so a word ending at the very end of
        // the string is closed by the same branch as every other word.
        .chain(std::iter::once((text.len(), ' ')))
        .filter_map(move |(index, character)| {
            if character.is_whitespace() {
                start.take().map(|begin| (begin, index))
            } else {
                start.get_or_insert(index);
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wrapped lines, as a plain Vec for readable assertions.
    fn wrap<const N: usize>(text: &str, max_chars: usize) -> Vec<&str> {
        wrap_text::<N>(text, max_chars).into_iter().collect()
    }

    #[test]
    fn short_text_is_one_line() {
        assert_eq!(wrap::<4>("hello", 35), vec!["hello"]);
    }

    #[test]
    fn empty_text_is_one_empty_line() {
        assert_eq!(wrap::<4>("", 35), vec![""]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(
            wrap::<4>("the quick brown fox jumps", 10),
            vec!["the quick", "brown fox", "jumps"]
        );
    }

    #[test]
    fn counts_characters_not_bytes() {
        // Ten accented characters are 20 bytes; the old byte comparison
        // wrapped this into two lines against a budget of 12.
        let text = "ééééééééé ok";
        assert_eq!(wrap::<4>(text, 12), vec![text]);
    }

    #[test]
    fn over_long_word_gets_its_own_line() {
        assert_eq!(
            wrap::<4>("antidisestablishmentarianism is long", 10),
            vec!["antidisestablishmentarianism", "is long"]
        );
    }

    #[test]
    fn stops_at_the_line_cap() {
        // Four words that each need their own line, into room for two.
        assert_eq!(wrap::<2>("aaaa bbbb cccc dddd", 4), vec!["aaaa", "bbbb"]);
    }

    #[test]
    fn returned_slices_borrow_the_input() {
        let text = "alpha beta gamma";
        let lines = wrap_text::<4>(text, 10);
        // Every line is a subslice of `text`, which is the point of the change:
        // wrapping allocates nothing.
        for line in &lines {
            let offset = line.as_ptr() as usize - text.as_ptr() as usize;
            assert!(offset + line.len() <= text.len());
        }
    }

    #[test]
    fn interior_whitespace_is_preserved() {
        // Slicing the original keeps the double space; rebuilding from
        // `split_whitespace` would have collapsed it.
        assert_eq!(
            wrap::<4>("aa  bb cccccccccc", 8),
            vec!["aa  bb", "cccccccccc"]
        );
    }

    #[test]
    fn word_ranges_finds_every_word() {
        let text = "  one two   three ";
        let words: Vec<&str> = word_ranges(text)
            .map(|(start, end)| &text[start..end])
            .collect();
        assert_eq!(words, vec!["one", "two", "three"]);
    }
}

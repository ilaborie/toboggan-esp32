use embedded_graphics::mono_font::iso_8859_1::{FONT_9X18, FONT_9X18_BOLD};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use log::info;

use crate::config::display::{
    COLOR_BLACK, COLOR_CYAN, COLOR_ERROR_BACKGROUND, COLOR_GREEN, COLOR_ORANGE, COLOR_RED,
    COLOR_WHITE, COLOR_YELLOW, CURRENT_SLIDE_LINE_START, CURRENT_SLIDE_MAX_LINES, LINE_HEIGHT,
    MAX_CHARS_PER_LINE, NEXT_SLIDE_LINE_START, NEXT_SLIDE_MAX_LINES, PROGRESS_BAR_HEIGHT,
    PROGRESS_BAR_LINE, PROGRESS_BAR_MARGIN, STEP_DOT_MAX_VISIBLE, STEP_DOT_RADIUS,
    STEP_DOT_SPACING, STEP_INDICATOR_LINE, TITLE_LINE_START, TITLE_MAX_LINES,
};
use crate::state::{AppState, StateMode, TalkData};

pub struct DisplayManager<D>
where
    D: DrawTarget<Color = Rgb565>,
{
    pub display: D,
    current_state_hash: u64,
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

        // Calculate hash of current state and talk data to detect changes
        let mut hasher = DefaultHasher::new();
        state.hash(&mut hasher);
        if let Some(data) = talk_data {
            data.hash(&mut hasher);
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
                    self.render_title(&data.title, COLOR_CYAN)?;
                    self.render_simple_state("Ready", COLOR_GREEN, CURRENT_SLIDE_LINE_START)?;
                } else {
                    self.render_simple_state("Loading...", COLOR_YELLOW, TITLE_LINE_START + 3)?;
                }
            }
            AppState::Error { message } => {
                self.render_simple_state("ERROR", COLOR_RED, TITLE_LINE_START)?;
                let error_lines = Self::wrap_text(message, MAX_CHARS_PER_LINE);
                self.draw_text_at_lines(
                    &error_lines,
                    CURRENT_SLIDE_LINE_START,
                    COLOR_RED,
                    &FONT_9X18,
                )?;
            }
            AppState::Booting => {
                self.render_simple_state("Booting...", COLOR_ORANGE, TITLE_LINE_START + 3)?;
            }
            AppState::Connecting { ssid } => {
                self.render_simple_state("Connecting", COLOR_YELLOW, TITLE_LINE_START)?;
                let ssid_lines = Self::wrap_text(ssid, MAX_CHARS_PER_LINE);
                self.draw_text_at_lines(
                    &ssid_lines,
                    CURRENT_SLIDE_LINE_START,
                    COLOR_YELLOW,
                    &FONT_9X18,
                )?;
            }
            AppState::Connected { ssid } => {
                self.render_simple_state("Connected", COLOR_GREEN, TITLE_LINE_START)?;
                let ssid_lines = Self::wrap_text(ssid, MAX_CHARS_PER_LINE);
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
            StateMode::Paused => COLOR_YELLOW,
            StateMode::Running => COLOR_GREEN,
            StateMode::Done => COLOR_CYAN,
        };

        // Render progress bar (line 1)
        self.render_progress_bar(current, talk_data.slide_count(), mode)?;

        // Render talk title (lines 2-3)
        self.render_title(&talk_data.title, title_color)?;

        // Render step indicators (line 4) if this slide has steps
        let step_count = talk_data.get_step_count(current);
        if step_count > 0 {
            self.render_step_indicators(current_step, step_count, mode)?;
        }

        // Render current slide (lines 5-8, bold)
        if let Some(current_slide) = talk_data.get_slide(current) {
            let current_lines = Self::wrap_text(current_slide, MAX_CHARS_PER_LINE);
            let limited_lines: Vec<_> = current_lines
                .into_iter()
                .take(CURRENT_SLIDE_MAX_LINES)
                .collect();
            self.draw_text_at_lines(
                &limited_lines,
                CURRENT_SLIDE_LINE_START,
                COLOR_WHITE,
                &FONT_9X18_BOLD,
            )?;
        }

        // Render next slide (lines 10-11) if available
        if let Some(next_slide) = talk_data.get_next_slide(current) {
            let next_lines = Self::wrap_text(next_slide, MAX_CHARS_PER_LINE);
            let limited_lines: Vec<_> = next_lines.into_iter().take(NEXT_SLIDE_MAX_LINES).collect();
            self.draw_text_at_lines(
                &limited_lines,
                NEXT_SLIDE_LINE_START,
                COLOR_CYAN,
                &FONT_9X18,
            )?;
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

        // Limit visible dots
        let visible_count = step_count.min(STEP_DOT_MAX_VISIBLE);

        // Calculate starting X position to center the dots
        let total_width = (visible_count as i32 - 1) * STEP_DOT_SPACING;
        let start_x = (display_width - total_width) / 2;
        let y = STEP_INDICATOR_LINE * LINE_HEIGHT;

        // Choose colors based on mode
        let filled_color = match mode {
            StateMode::Running => COLOR_GREEN,
            StateMode::Paused => COLOR_YELLOW,
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

        // Calculate fill width: (current + 1) / total
        let progress = (current + 1) as f32 / total as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let fill_width = (bar_width as f32 * progress) as u32;

        // Draw filled portion
        let fill_color = match mode {
            StateMode::Running => COLOR_GREEN,
            StateMode::Paused => COLOR_YELLOW,
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

    /// Render the talk title with proper wrapping
    fn render_title(&mut self, title: &str, color: Rgb565) -> anyhow::Result<()> {
        let title_lines = Self::wrap_text(title, MAX_CHARS_PER_LINE);
        let limited_lines: Vec<_> = title_lines.into_iter().take(TITLE_MAX_LINES).collect();
        self.draw_text_at_lines(&limited_lines, TITLE_LINE_START, color, &FONT_9X18)
    }

    /// Draw text lines at specific line positions
    fn draw_text_at_lines(
        &mut self,
        lines: &[String],
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

    /// Wrap text to fit within specified character width
    fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
        if text.len() <= max_chars {
            return vec![text.to_string()];
        }

        let mut lines = Vec::new();
        let mut current_line = String::new();

        for word in text.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= max_chars {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        lines
    }
}

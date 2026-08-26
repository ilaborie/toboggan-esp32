/// Configuration constants for the ESP32-S3-BOX-3B application
/// Following clean code principles by centralizing magic numbers and configuration
use embedded_graphics::pixelcolor::Rgb565;

/// Environment variables configuration
pub mod env {
    pub const WIFI_SSID: &str = env!("WIFI_SSID");
    pub const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
    pub const TOBOGGAN_HOST: &str = env!("TOBOGGAN_HOST");
    pub const TOBOGGAN_PORT: &str = env!("TOBOGGAN_PORT");

    /// Tried when `TOBOGGAN_HOST` does not resolve, so an mDNS `.local` name can
    /// fall back to a literal address on a network that blocks multicast.
    pub const TOBOGGAN_HOST_FALLBACK: Option<&str> = option_env!("TOBOGGAN_HOST_FALLBACK");
}

/// LED Configuration
pub mod led {
    /// LED blink intervals in milliseconds
    pub const BLINK_INTERVAL_FAST: u64 = 250;
    pub const BLINK_INTERVAL_NORMAL: u64 = 500;
}

/// Display Configuration
pub mod display {
    use super::Rgb565;

    /// Text rendering constants for FONT_9X18
    /// Font dimensions: 9 pixels wide, 18 pixels tall
    /// Display: 320x240 pixels → 35 chars/line, 12 lines max
    pub const LINE_HEIGHT: i32 = 20;
    pub const BUFFER_SIZE: usize = 512;

    /// Display layout constants (optimized for 320x240 with FONT_9X18)
    /// Progress: line 1, Title: lines 2-3, Steps: line 4, Current: lines 5-8, Next: lines 10-11
    pub const TITLE_LINE_START: i32 = 2;
    pub const TITLE_MAX_LINES: usize = 2;
    pub const CURRENT_SLIDE_LINE_START: i32 = 5;
    pub const CURRENT_SLIDE_MAX_LINES: usize = 4;
    pub const NEXT_SLIDE_LINE_START: i32 = 10;
    pub const NEXT_SLIDE_MAX_LINES: usize = 2;
    pub const MAX_CHARS_PER_LINE: usize = 35;

    /// Progress bar constants
    pub const PROGRESS_BAR_LINE: i32 = 1;
    pub const PROGRESS_BAR_HEIGHT: u32 = 8;
    pub const PROGRESS_BAR_MARGIN: i32 = 20;

    /// Step indicator constants
    pub const STEP_INDICATOR_LINE: i32 = 4;
    pub const STEP_DOT_RADIUS: u32 = 6;
    pub const STEP_DOT_SPACING: i32 = 16;
    pub const STEP_DOT_MAX_VISIBLE: usize = 15;

    /// Boot screen constants
    pub const BOOT_STATUS_LINE: i32 = 11;
    pub const BOOT_IMAGE_AREA_HEIGHT: i32 = 200; // Top 200px for boot image

    /// Display colors for different states - using explicit RGB values
    pub const COLOR_BLACK: Rgb565 = Rgb565::new(0x00, 0x00, 0x00);
    pub const COLOR_WHITE: Rgb565 = Rgb565::new(0x1F, 0x3F, 0x1F);
    pub const COLOR_RED: Rgb565 = Rgb565::new(0x1F, 0x00, 0x00);
    pub const COLOR_YELLOW: Rgb565 = Rgb565::new(0x1F, 0x3F, 0x00);
    pub const COLOR_CYAN: Rgb565 = Rgb565::new(0x00, 0x3F, 0x1F);
    pub const COLOR_GREEN: Rgb565 = Rgb565::new(0x00, 0x3F, 0x00);

    /// Fixed orange color for display text
    pub const COLOR_ORANGE: Rgb565 = Rgb565::new(0x1F, 0x0F, 0x00); // True orange

    /// Error background color
    pub const COLOR_ERROR_BACKGROUND: Rgb565 = Rgb565::new(0x10, 0x00, 0x00);
}

/// Timing Configuration
pub mod timing {
    use std::time::Duration;

    /// Main loop delays
    pub const MAIN_LOOP_POLL_INTERVAL: Duration = Duration::from_millis(100);
}

/// Application Constants
pub mod app {
    /// Default text values
    pub const ERROR_PREFIX: &str = "Error! ";
    pub const BOOTING_TEXT: &str = "Booting...";
}

/// Network Configuration
pub mod network {
    pub const CONNECTING_TEXT_PREFIX: &str = "Connecting to Wifi ";
    pub const CONNECTING_TEXT_SUFFIX: &str = "...";
    pub const LOADING_TALK_TEXT: &str = "Loading talk...";
}

/// Threading Configuration
pub mod threading {
    /// Stack sizes for worker threads
    pub const WIFI_THREAD_STACK: usize = 32 * 1024;
    pub const API_THREAD_STACK: usize = 32 * 1024;
    pub const WEBSOCKET_THREAD_STACK: usize = 16 * 1024;
}

/// WebSocket Configuration
pub mod websocket {
    use std::time::Duration;

    /// Message queue capacity
    pub const MESSAGE_QUEUE_SIZE: usize = 16;

    /// Connection timeout settings
    pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
    pub const POLL_INTERVAL: Duration = Duration::from_millis(100);
}

/// Reconnection Configuration
pub mod reconnect {
    use std::time::Duration;

    /// Initial delay before first retry attempt
    pub const INITIAL_DELAY: Duration = Duration::from_secs(5);

    /// Maximum delay between retry attempts (caps exponential backoff)
    pub const MAX_DELAY: Duration = Duration::from_secs(60);
}

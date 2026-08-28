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

    /// Offered in the `Register` frame to ask for the Presenter role.
    ///
    /// The server grants Presenter to loopback clients only; this box is on the
    /// LAN, so without a matching token it registers as Audience and every
    /// command it sends is refused. Start the server with `--presenter-token`.
    ///
    /// Baked into the binary by `env!`, exactly like `WIFI_PASSWORD` — readable
    /// with `strings` on the firmware image.
    pub const TOBOGGAN_PRESENTER_TOKEN: Option<&str> = option_env!("TOBOGGAN_PRESENTER_TOKEN");
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

    /// Panel size in pixels. Lives here rather than in `display.rs` because the
    /// touchscreen has to map its coordinates onto the same frame.
    pub const SCREEN_WIDTH: u16 = 320;
    pub const SCREEN_HEIGHT: u16 = 240;

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

    /// An SSID is capped at 32 bytes by 802.11, so it fits one line - the
    /// second is only there for a multi-byte name that renders wider.
    pub const SSID_MAX_LINES: usize = 2;

    /// Error text occupies the same four lines as a slide title would.
    /// `ERROR_MESSAGE_CAPACITY` is derived from this, so the panel renders
    /// exactly as much as the state can hold - no more, no less.
    pub const ERROR_MAX_LINES: usize = 4;

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

/// Touchscreen Configuration
pub mod touch {
    use std::time::Duration;

    /// The GT911 tops out at 400 kHz, and the I2C driver's own default is 1 MHz,
    /// so this has to be set rather than left alone.
    pub const I2C_BAUDRATE_KHZ: u32 = 400;

    /// 50 Hz. A tap lasts well under the main loop's 100 ms poll, which is why
    /// touch gets its own thread rather than riding along there.
    pub const POLL_INTERVAL: Duration = Duration::from_millis(20);

    /// Kept short: on a bus with nothing attached (the simulator) every one of
    /// the ~112 probes has to time out before the scan can report.
    pub const PROBE_TIMEOUT: Duration = Duration::from_millis(20);

    /// The controller latches its address from the INT pin level at reset, so
    /// which of the two it answers on is not knowable ahead of time.
    pub const GT911_ADDRESSES: [u8; 2] = [0x5D, 0x14];

    /// Ignore a second tap this soon after the last one. Presses are detected on
    /// the press edge, so this only guards against contact bounce - but skipping
    /// two steps mid-talk is not a mistake worth risking to save a constant.
    pub const COMMAND_DEBOUNCE: Duration = Duration::from_millis(300);
}

/// Timing Configuration
pub mod timing {
    use std::time::Duration;

    /// Main loop delays
    pub const MAIN_LOOP_POLL_INTERVAL: Duration = Duration::from_millis(100);
}

/// Bounds on the text the app stores
pub mod text {
    use super::display::{ERROR_MAX_LINES, MAX_CHARS_PER_LINE};

    /// Roughly one screenful. The panel renders at most
    /// `MAX_CHARS_PER_LINE * ERROR_MAX_LINES` characters, so a longer message
    /// was always going to be cut - this just cuts it before it is stored and
    /// cloned around.
    pub const ERROR_MESSAGE_CAPACITY: usize = MAX_CHARS_PER_LINE * ERROR_MAX_LINES;

    /// 802.11 caps an SSID at 32 bytes, so this bound can never truncate one.
    pub const SSID_CAPACITY: usize = 32;
}

/// HTTP API Configuration
pub mod api {
    /// Hard ceiling on a `/api/talk` body.
    ///
    /// Generous for a real deck (a 150-slide talk with long titles is well
    /// under 20 KB) and small enough that a wrong host or a server bug cannot
    /// walk the heap down 256 bytes at a time.
    pub const MAX_TALK_BODY: usize = 32 * 1024;
}

/// Threading Configuration
pub mod threading {
    /// Stack sizes for worker threads
    pub const WIFI_THREAD_STACK: usize = 32 * 1024;
    pub const API_THREAD_STACK: usize = 32 * 1024;
    pub const WEBSOCKET_THREAD_STACK: usize = 16 * 1024;
    /// Smaller than the rest: this one only polls I2C and logs.
    pub const TOUCH_THREAD_STACK: usize = 8 * 1024;
}

/// WebSocket Configuration
pub mod websocket {
    use std::time::Duration;

    /// Connection timeout settings
    pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
    pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

    /// How long the message loop blocks waiting for a server frame before it
    /// loops round to drain queued taps. Bounds tap latency, so it stays short.
    pub const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// Stack room for one serialized outgoing command.
    ///
    /// The longest is `Register`, at ~45 bytes plus the presenter token; the
    /// rest are under 60. Generous enough that no plausible token overflows it,
    /// and serialization errors rather than truncates if one does.
    pub const COMMAND_FRAME_CAPACITY: usize = 512;

    /// Stack for the `esp_websocket_client` task, which is where the receive
    /// callback - and so the JSON parse - actually runs. The component's own
    /// default is 4 KB, which is tight for serde's recursive descent.
    pub const TASK_STACK: usize = 8 * 1024;

    /// Receive buffer for the same task. The component defaults to 1 KB, which
    /// splits any larger frame across several callbacks.
    pub const RECEIVE_BUFFER: usize = 4 * 1024;
}

/// Reconnection Configuration
pub mod reconnect {
    use std::time::Duration;

    /// Initial delay before first retry attempt
    pub const INITIAL_DELAY: Duration = Duration::from_secs(5);

    /// Maximum delay between retry attempts (caps exponential backoff)
    pub const MAX_DELAY: Duration = Duration::from_secs(60);
}

//! Boot image data for the splash screen

/// Embedded boot image (RGB565 BMP format, 200x195 pixels)
pub const BOOT_IMAGE: &[u8] = include_bytes!("../assets/boot.bmp");

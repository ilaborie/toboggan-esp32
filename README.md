# Toboggan ESP32

ESP32-S3-BOX-3B embedded application that connects to a presentation server ([Toboggan](https://github.com/ilaborie/toboggan)) via WiFi and WebSocket to display slides on a built-in screen with RGB LED status indicators.

**Note**: This is an educational and fun project created to explore Rust's capabilities across different platforms - from embedded systems to web browsers. While fully functional, it's designed primarily for learning and experimentation rather than production use. It's a playground to demonstrate how Rust can target everything from microcontrollers to iOS apps!

## Hardware

**Target Device**: [ESP32-S3-BOX-3B](https://github.com/espressif/esp-box/blob/master/docs/hardware_overview/esp32_s3_box_3/hardware_overview_for_3.md)
- MCU: ESP32-S3 (Xtensa dual-core)
- Display: MIPIDSI 240x320 SPI
- RGB LED: GPIO 39 (red), 40 (green), 41 (blue)
- WiFi: Built-in 802.11b/g/n

## Quick Start

### Prerequisites

Install tools via [mise](https://mise.jdx.dev/):
```bash
mise install
```

This installs: `ldproxy`, `espflash`, `espup`, and `esp-generate`.

### Configuration

Create `.mise.local.toml` (gitignored):
```toml
[env]
WIFI_SSID = "your-wifi-network"
WIFI_PASSWORD = "your-password"
TOBOGGAN_HOST = "192.168.1.100"
TOBOGGAN_PORT = "8080"
```

### Build and Flash

```bash
# Build for release
cargo build --release

# Flash and monitor
cargo espflash flash --monitor --release
```

## Architecture

- **Multi-threaded**: WiFi, API, WebSocket, and main display loop
- **State machine**: Booting → Connecting → Loading → Play/Paused/Done
- **Message passing**: Workers communicate via `std::sync::mpsc` channels
- **LED indicators**: Visual feedback for each application state

## Key Crates

### ESP32 Ecosystem
- [**esp-idf-svc**](https://github.com/esp-rs/esp-idf-svc) - High-level Rust bindings for ESP-IDF services (WiFi, HTTP, WebSocket)
- [**esp-idf-sys**](https://github.com/esp-rs/esp-idf-sys) - Low-level ESP-IDF bindings (auto-generated)
- [**embuild**](https://github.com/esp-rs/embuild) - ESP-IDF build integration for Cargo
- [**embedded-svc**](https://github.com/esp-rs/embedded-svc) - Embedded service traits (WiFi, HTTP client, etc.)

### Display & Graphics
- [**mipidsi**](https://github.com/almindor/mipidsi) - MIPI Display Serial Interface driver
- [**embedded-graphics**](https://github.com/embedded-graphics/embedded-graphics) - 2D graphics library for embedded systems

### Utilities
- [**heapless**](https://github.com/rust-embedded/heapless) - Static data structures (Vec, String) without heap allocation
- [**serde**](https://github.com/serde-rs/serde) / [**serde_json**](https://github.com/serde-rs/json) - JSON serialization for WebSocket messages
- [**anyhow**](https://github.com/dtolnay/anyhow) - Flexible error handling
- [**log**](https://github.com/rust-lang/log) - Logging facade

## Resources

### Official Rust ESP32 Documentation
- [Rust on ESP Book](https://docs.esp-rs.org/book/) - Comprehensive guide
- [ESP-IDF Training](https://docs.esp-rs.org/std-training/) - Hands-on exercises
- [ESP Rust Board Support](https://github.com/esp-rs/esp-hal) - HAL and PAC crates

### Tools
- [espflash](https://github.com/esp-rs/espflash) - Flash utility (`cargo espflash`)
- [espup](https://github.com/esp-rs/espup) - Toolchain installer
- [esp-generate](https://github.com/esp-rs/esp-generate) - Project generator

### Community
- [esp-rs GitHub Organization](https://github.com/esp-rs)
- [ESP32 Rust Community Matrix](https://matrix.to/#/#esp-rs:matrix.org)
- [Awesome ESP Rust](https://github.com/esp-rs/awesome-esp-rust) - Curated list of resources

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

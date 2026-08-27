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
TOBOGGAN_HOST = "your-laptop.local"
TOBOGGAN_HOST_FALLBACK = "192.168.1.100"   # optional
TOBOGGAN_PORT = "8080"
```

Prefer your machine's Bonjour name (`mise run host`) over a literal address: the
box resolves `.local` over multicast mDNS, so the same firmware keeps working
when you move between the home network and a phone hotspot.

mDNS needs the network to forward multicast, which guest and client-isolated
WiFi often do not. `TOBOGGAN_HOST_FALLBACK` (from `mise run ip`) is tried when
the `.local` name does not resolve. The address is resolved once per boot and
shared by the HTTP and WebSocket clients, and every attempt is logged:

```
🔍 Resolving your-laptop.local:8080
🔍 Could not resolve your-laptop.local: failed to lookup address information
🔍 Falling back to 192.168.1.100 (192.168.1.100:8080): your-laptop.local did not resolve
```

### Build and Flash

```bash
# Build and flash (rebuilds for the device first, so a simulator build
# can never reach the box)
mise run flash

# Flash and monitor by hand
cargo espflash flash --monitor --release --partition-table partitions.csv
```

`--partition-table` is not optional: espflash does not read `sdkconfig`, so
without it the device gets espflash's default layout instead of the 4 MB app
partition the build was sized against. See `partitions.csv`.

## Simulating without hardware

The firmware runs in the [Wokwi](https://wokwi.com/) simulator, which emulates
the ESP32-S3 core, GPIO, SPI, and WiFi and renders the LCD. QEMU is not an
option: the Espressif build pinned by this project's ESP-IDF supports `esp32`
only, and it emulates neither the WiFi radio nor a display.

`diagram.json` wires the same GPIOs the real board uses. Wokwi has no
ESP32-S3-BOX part, so it uses the DevKitC with an ILI9341 panel. That panel is
natively portrait 240x320, while the box's ILI9342C is landscape 320x240, and
nothing in the firmware can bridge that: the simulated screen shows the frame
rotated a quarter turn with the overflowing columns wrapped down the side.
Colors and text are faithful; pixel-exact layout is not. Read it as "did the
right thing get drawn", not as a preview of the box.

Simulator builds need their own `WIFI_SSID` / `TOBOGGAN_HOST`, because
`src/config.rs` bakes them in with `env!` at compile time. `mise run sim-build`
sets them; `mise run build` targets the real box. Both write the same binary, so
`mise run flash` depends on `build` to re-bake the device values first —
otherwise the box would boot looking for `Wokwi-GUEST`.

### Headless (CI)

```bash
mise install    # provides wokwi-cli and wokwigw
# then put WOKWI_CLI_TOKEN from https://wokwi.com/dashboard/ci in .mise.local.toml
mise run sim-test
```

Runs `wokwi/boot.test.yaml` in Wokwi's cloud. The cloud cannot reach a server
on your machine, so the run deliberately ends in the talk-fetch error path —
covering boot, display init, the WiFi join, and error rendering in one go.

The two screenshots are compared against `wokwi/golden/`, which is committed.
The comparison is `wokwi/compare.py`, not wokwi-cli's own `compare-with`, and it
allows a small budget of differing pixels (50, against a 240x320 frame).

That budget is not laziness. The simulated panel is not byte-deterministic:
upgrading to embedded-graphics 0.8.2 — a release that changed nothing but
documentation links — moved a single pixel, and so did changing a screenshot
delay. The artefact is in Wokwi's ILI9341 model, not in what the firmware drew.
A real regression is never that small: one wrong character in `FONT_9X18` is up
to 162 pixels, and a moved line or wrong colour is thousands.

Every run leaves the actual frame in `wokwi/out/`. To refresh a golden after an
intentional display change, eyeball it against the old one first, then copy it
up.

### Live, against a local toboggan server

The [private gateway](https://github.com/wokwi/wokwigw) makes
`host.wokwi.internal` resolve to this machine, which is what lets the simulated
board talk to a real server. It is read from `wokwi.toml` by the **VS Code
extension only** (install `Wokwi.wokwi-vscode`) — `wokwi-cli` has no such
option, so headless runs can never reach a local server.

```bash
toboggan --host 0.0.0.0 --port 8080   # NOT from this repo: it reads
                                      # TOBOGGAN_HOST/PORT as its own bind address
mise run sim-gateway                  # wokwigw, ws://localhost:9011
mise run sim-build                    # then F1 -> "Wokwi: Start Simulator"
```

This is the only way to exercise the full WebSocket protocol — registration,
slide updates, talk reloads — without flashing the box.

## Architecture

See [docs/touch-and-imu.md](docs/touch-and-imu.md) for what the unused
touchscreen and IMU would take — the parts, the crates, and the constraints.

- **Multi-threaded**: WiFi, API, WebSocket, and main display loop
- **State machine**: Booting → Connecting → Connected → Loading → Initialized → Play (Running/Done)
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

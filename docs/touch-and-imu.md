# Touchscreen and gyroscope: what it would take

Written while upgrading to `esp-idf-svc` 0.52, and updated as the touchscreen
landed. Touch is implemented (`src/touch.rs`, tap advances a step); the IMU is
not, and this records what was established so that work does not start from
zero.

## Did the upgrade unlock either of them?

**No.** `esp-idf-hal` 0.45 already implemented `embedded_hal::i2c::I2c` for
`I2cDriver`, which is the whole of what an off-the-shelf sensor driver needs.
Neither part was ever blocked by the old versions.

The upgrade still had to come first, for a different reason: `esp-idf-hal` 0.46
deleted the `Peripheral`/`PeripheralRef` traits and gave every peripheral a
lifetime, so `I2cDriver::new` changed shape. Hardware code written against 0.45
would have been rewritten a week later.

```rust
// esp-idf-hal 0.46
I2cDriver::new(
    i2c: impl I2c + 'd,
    sda: impl InputPin + OutputPin + 'd,
    scl: impl InputPin + OutputPin + 'd,
    config: &config::Config,
) -> Result<Self, EspError>
```

One wrinkle worth knowing: on every boot ESP-IDF already logs
`W (556) i2c: This driver is an old driver, please migrate ... driver/i2c_master.h`.
`esp-idf-hal` 0.46 still wraps the legacy `driver/i2c.h`. It works, but the
deprecation is upstream's to resolve, not something to route around here.

## The hardware

Both parts sit on one I2C bus this firmware has never touched. From Espressif's
own BSP (`esp-bsp/bsp/esp-box-3/include/bsp/esp-box-3.h`), which is trustworthy
for this board because its display pins match ours exactly (6/7/5/4/48 plus
backlight 47):

| Signal | Pin |
|---|---|
| `BSP_I2C_SCL` | GPIO18 |
| `BSP_I2C_SDA` | GPIO8 |
| `BSP_LCD_TOUCH_INT` | GPIO3 |

All three are free. `src/lib.rs` destructures `Peripherals { pins, spi2, modem, .. }`
and moves GPIO fields out one at a time, so `pins.gpio8` / `gpio18` / `gpio3`
are still available — but `i2c0` and `i2c1` are currently **dropped by the `..`
rest pattern** and must be added to that destructuring first.

### Gyroscope — settled

The part is an **ICM-42607-P** (BSP header, "Inertial Measurement Unit"). The
[`icm42670`](https://crates.io/crates/icm42670) crate targets `embedded-hal` 1.0
and declares:

```rust
pub const DEVICE_IDS: [u8; 2] = [
    0x60, // ICM-42607
    0x67, // ICM-42670
];
```

So it accepts this exact part off the shelf — accelerometer, gyroscope and
temperature. It *owns* its I2C handle.

### The bus, as measured

The firmware now scans the bus at boot (`src/touch.rs`). On this board:

```
👆 I2C scan found 0x14 (GT911 touch), 0x18 (ES8311 codec),
                  0x40 (ES7210 ADC), 0x68 (ICM-42607 IMU)
👆 GT911 ready at 0x14
```

So it is the **GT911**, as predicted — but at **0x14**, the backup address, not
the 0x5D that the `gt911` crate defaults to. The controller latches its address
from the INT pin level at reset and its reset line is shared with the LCD's, so
trying both addresses is not optional. The IMU is confirmed present at 0x68.

### Touch — the older uncertainty, now resolved

The BSP probes the bus at runtime and handles two possibilities, pairing each
with a different LCD driver: **GT911** (0x5D/0x14) implies ILI9341, **TT21100**
(0x24) implies ST7789. The scan above settles it.

Rust support differs sharply between the two (the GT911 is what this board has,
so the second entry is recorded only for other revisions):

- [`gt911`](https://crates.io/crates/gt911) 0.3 — current, `embedded-hal` 1.0,
  and it *borrows* the bus per call (`get_touch(&mut i2c) -> Option<Point>`),
  so sharing is trivial.
- [`tt21100`](https://crates.io/crates/tt21100) 0.1 — from 2023, still on
  `embedded-hal` 0.2. `esp-idf-hal` does still implement those traits, so it
  would work, but it is unmaintained.

Sharing the bus is easy given the mismatch: `gt911` borrows and `icm42670` owns,
so a `RefCell<I2cDriver>` with `embedded-hal-bus`'s `RefCellDevice` for the IMU
and a plain `borrow_mut()` for the touch controller covers both from one thread.
`I2cDriver` is `Send`, so that thread can be an ordinary worker.

## Constraints any such feature will hit

1. **The display cannot leave the main thread.** `SpiInterface` borrows the
   `[0u8; 512]` scratch buffer allocated on `run()`'s stack (`src/lib.rs`), so
   the `DrawTarget` is not `'static`. A sensor thread has to feed the main loop
   over a channel, the way `wifi_thread` and `websocket_thread` already do; it
   cannot draw anything itself.

2. **Redraws are hash-gated.** `DisplayManager::update_display` hashes
   `AppState` + `TalkData` and skips rendering when unchanged. Feedback that is
   not part of that hash is silently dropped.

3. **Adding an `AppStateDiff` variant is safe** — `AppState::apply_diff` and
   `render_state` both match exhaustively with no wildcard arm, so the compiler
   lists every site that needs attention. But the dedup guard in the main loop
   names the event-like diffs explicitly (`Blink | TalkReload`); a repeated tap
   would be swallowed unless it joins that list.

4. **Tap-to-advance was a protocol problem, not a driver problem.** Now done,
   and it needed all of this:

   - a command channel from `touch_thread` through `spawn_websocket_thread`
     into `connect_to_ws`, borrowed rather than moved so it survives the
     reconnect loop, and drained on connect — taps made against a dead socket
     would otherwise all replay at once;
   - a **presenter token** in the `Register` frame. The server settles the role
     once, at registration, from the token in that frame — *not* from the
     `?token=` query parameter, which is the HTTP path. No token configured
     **server-side** also means Audience, whatever the client offers;
   - press-edge detection. At 50 Hz a resting finger reads as a press on every
     poll, so firing on the press rather than the edge sends ~50 commands;
   - and a fix to `Error` handling. The server answers a refused command with
     `"This client is watching, not presenting"` and carries on; the handler
     used to turn any `Error` into a full-screen `AppState::Error`, which would
     have blanked the deck mid-talk on every disallowed tap. It now only does
     that before registration, where no benign case exists.

5. **Neither part is simulable.** Wokwi has no GT911 or ICM-42607 model, so
   `mise run sim-test` can only ever show that their absence degrades
   gracefully — which means whatever is written must tolerate a bus that never
   ACKs.

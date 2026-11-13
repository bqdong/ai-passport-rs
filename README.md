# ai-passport-rs

Rust bring-up of the **FoloToy AI Passport** (ESP32-C3) display, ported from the
C firmware's BSP. On boot it lights the ST7789P3 panel and paints a splash
screen:

```
      Python in the work
       Rust in the board
```

Built with [`esp-hal`](https://github.com/esp-rs/esp-hal),
[`mipidsi`](https://crates.io/crates/mipidsi) and
[`embedded-graphics`](https://crates.io/crates/embedded-graphics).

## Hardware

| Item     | Value                                                        |
| -------- | ------------------------------------------------------------ |
| MCU      | ESP32-C3, 8 MB flash, no PSRAM                               |
| Display  | ST7789P3, 240×320, RGB565, factory color-inverted            |
| Bus      | SPI2, 80 MHz, mode 0, 4-line (no MISO)                       |
| Backlight| LEDC low-speed timer0 / ch0, 5 kHz, 10-bit PWM               |
| Console  | Native USB Serial/JTAG (`/dev/ttyACM0` on Linux)             |

### Pin map (`src/display.rs`)

| Signal | GPIO | Notes                                 |
| ------ | ---- | ------------------------------------- |
| SCLK   | 8    | SPI2                                  |
| MOSI   | 9    | SPI2                                  |
| CS     | 1    |                                       |
| DC     | 20   | command / data select                 |
| RST    | —    | not wired to MCU, uses software reset |
| BL     | 21   | LEDC PWM backlight                    |

### Vendor init table

The panel needs the maker's porch / power / gamma commands, not the ST7789
generic defaults — using the defaults renders wrong colors. `src/display.rs`
defines a custom `St7789P3` model that replays this table inside
`mipidsi::models::Model::init`, mirroring `ST7789P3_CMDS` in the C BSP.

## Prerequisites

- Rust (see `rust-toolchain.toml`) with the `riscv32imc-unknown-none-elf` target
  and `rust-src` (both pinned there).
- [`espflash`](https://github.com/esp-rs/espflash): `cargo install espflash`.

```bash
rustup show          # pulls the pinned toolchain/target/component
```

## Build and run

```bash
cargo build          # compile
cargo run            # build, flash and open the serial monitor
```

`.cargo/config.toml` sets the runner to
`espflash flash --monitor --chip esp32c3`, so `cargo run` flashes and monitors
in one step.

To flash without the monitor (e.g. from CI or a non-interactive shell):

```bash
cargo build
espflash flash --chip esp32c3 --port /dev/ttyACM0 \
    target/riscv32imc-unknown-none-elf/debug/ai-passport-rs
```

### Reading the serial log without a TTY

`espflash monitor` needs a real terminal. To just read output, open the port
directly; the firmware prints an `alive N` heartbeat every 500 ms:

```bash
timeout 5 cat /dev/ttyACM0
```

### WSL2 users

The ESP32-C3 exposes a native USB Serial/JTAG, which must be forwarded from
Windows into WSL with [usbipd-win](https://github.com/dorssel/usbipd-win):

```powershell
usbipd list
usbipd bind --busid <BUSID>      # once, as admin
usbipd attach --wsl --busid <BUSID>
```

Then `/dev/ttyACM0` appears in WSL. Re-`attach` after every replug. If the port
is busy, check `fuser /dev/ttyACM0` for a stale process.

## Project layout

```
src/
├── lib.rs          # library root, re-exports display module
├── display.rs      # SPI, ST7789P3 init, backlight, splash screen
└── bin/main.rs     # entry point: init display, heartbeat loop
```

## License

Not yet chosen.

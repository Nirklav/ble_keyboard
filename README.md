# ESP32-C3 Keyboard

The firmware and the tools for a custom BLE keyboard built on top of ESP32-C3.
For the matrix an MCP23S17 is used, an SSD1306 display and an optional joystick extension sit on the I2C bus.

**Config app: <https://kb.nirklav.xyz/>** — edit the keymap, combos and settings and update the firmware
right from the browser (Chrome, Edge or Opera: it needs WebHID).

## Features

- BLE HID: keyboard, media keys and mouse
- 4 BLE profiles: the keyboard remembers 4 hosts and switches between them (`BtSelect`, `BtNext`, `BtPrev`,
  `BtClear`)
- Customizable keymaps:
  - layouts and layers: `Fn` (while held / after a double tap), `Momentary`, `Toggle`, `To`, transparent keys
  - mod-tap and layer-tap keys with a tapping term and prior-idle check
  - mouse keys, cursor travel and wheel scroll keys
- Combos: multi-tap and hold combinations of keys for special actions (switch layout, select a profile,
  unpair, reset the config) and chords that send a key action
- Joystick extension (STM32F030): an analog stick as the mouse cursor with configurable speed, ramp and axes,
  its buttons take any key action of the layout
- OLED display: the current layout and BLE profile, animated pop-ups, firmware update progress; turns off
  when idle
- Config over HID: read and written by the desktop and web apps, validated on both sides, stored on the
  keyboard
- Firmware update over the air (BLE) into the other OTA slot, with rollback if the new firmware doesn't work
- Low-power operation: idle BLE link and deep sleep after a configurable time, connected or not

## Project layout

| Directory   | What                                                                  |
|-------------|-----------------------------------------------------------------------|
| `keyboard/` | ESP32-C3 firmware (ESP-IDF, NimBLE), schematic and PCB in `hardware/` |
| `joystick/` | STM32F030 firmware of the joystick extension, its hardware            |
| `protocol/` | Shared crate: config model, HID config protocol, firmware images      |
| `desktop/`  | `kb` CLI and `kb-gui` config editor, native and browser (wasm) builds |

## Config apps

The web app at <https://kb.nirklav.xyz/> is the browser build of `kb-gui`. The same editor runs natively,
and the `kb` CLI works with the same TOML files:

```sh
kb info                      # firmware and protocol versions, keyboard size and limits
kb pull config.toml          # read the config from the keyboard
kb check config.toml         # validate a config without the keyboard
kb push config.toml          # validate and write it to the keyboard
kb reset                     # restore the factory config
kb update [firmware.bin]     # update the firmware (the bundled one without a file)
kb udev-rule                 # Linux: the udev rule for access to the keyboard
```

[desktop/examples/config.toml](desktop/examples/config.toml) is the factory config, with all key actions,
combos and settings described.

## Building

- Firmware: `cd keyboard && cargo run --release` builds it and flashes it over the serial port
  (`espflash`, see [keyboard/scripts/flash.sh](keyboard/scripts/flash.sh)).
- Apps: `cargo build -p desktop --release`; [desktop/scripts/bundle.sh](desktop/scripts/bundle.sh) builds the
  firmware too and bundles it into `kb` / `kb-gui` for `kb update` and the Update button.
- Web app: `trunk serve` in `desktop/`; `desktop/scripts/bundle.sh docker` builds the nginx Docker image
  with the firmware bundled ([desktop/Dockerfile](desktop/Dockerfile)).

## License

MIT License

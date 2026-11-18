# Esp serial under wsl

The keyboard ESP32-C3 (USB-Serial-JTAG, Windows busid 3-1, VID:PID 303a:1001) is passed into WSL with usbipd.
Every chip reset re-enumerates USB and detaches it from WSL, so `espflash monitor` fails ("Failed to reopen serial port").

- Keep it attached: `usbipd.exe attach --wsl --busid 3-1 --auto-attach` (run in background from WSL).
- Flash: `espflash flash --non-interactive -p /dev/ttyACM0 --partition-table partitions.csv ../target/riscv32imc-esp-espidf/debug/keyboard` (from keyboard/).
- `espflash flash` writes the app to ota_0 only; after an OTA put another slot in otadata, the bootloader keeps
  booting that one and the USB flash silently doesn't run. keyboard/scripts/flash.sh (the cargo runner) erases otadata,
  so use it, or install test builds by OTA (kb-win.sh); check "Running firmware from ota_N" in the log. See [kb-via-windows](kb-via-windows.md).
- Read log without resetting: `stty -F /dev/ttyACM0 115200 raw -echo -hupcl; cat /dev/ttyACM0`. Stop the reader before flashing.
- In deep sleep the USB is off and the port disappears until a key wakes the keyboard.
- Since 0.2.18 auto light sleep is on; CONFIG_USJ_NO_AUTO_LS_ON_CONNECTION blocks it while a host is on the USB.
  Plugging USB into a keyboard that is light-sleeping can leave the console silent (documented C3 behaviour):
  press a key, then unplug and re-plug. A plain `cat` reader hangs on the stale port after a re-attach; use a
  reader that reopens when the device node's inode changes.
- NimBLE debug logs need CONFIG_BT_NIMBLE_LOG_LEVEL_DEBUG + CONFIG_LOG_MAXIMUM_LEVEL_DEBUG and esp_log_level_set("NimBLE", DEBUG) at runtime.

# TODO

## Firmware features

- [ ] **Tap-hold flavors.** Only permissive hold now; maybe hold-preferred / tap-preferred per key or globally,
  once home-row mods are tried on the real keyboard.
- [ ] **Firmware update over BLE (OTA).** ESP-IDF OTA partitions, upload from the desktop app over the config service
  (chunks + CRC already exist in `protocol`).
- [ ] **Key usage statistics.** Count presses per key on the keyboard, read them over the config service,
  show a heatmap in kb-gui.

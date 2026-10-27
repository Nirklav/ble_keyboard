# TODO

## Firmware features

- [ ] **Tap-hold flavors.** Only permissive hold now; maybe hold-preferred / tap-preferred per key or globally,
  once home-row mods are tried on the real keyboard.
- [ ] **Combos with ordinary actions.** `ComboAction::Action(KeyAction)` so a chord (e.g. `J+K`) sends any key,
  not only the special actions; a timeout window (~30-50 ms) for the chord keys.
- [ ] **Configurable parameters in the config.** Debounce (5 ms hardcoded in `keyboard/src/key_matrix.rs`),
  deep sleep timeouts, joystick sensitivity. `tapping_term_ms` and `require_prior_idle_ms` are already
  in `Config::settings`.
- [ ] **Firmware update over BLE (OTA).** ESP-IDF OTA partitions, upload from the desktop app over the config service
  (chunks + CRC already exist in `protocol`).
- [ ] **Key usage statistics.** Count presses per key on the keyboard, read them over the config service,
  show a heatmap in kb-gui.

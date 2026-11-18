# Web config via webhid

Since 2026-10-27 the keyboard config protocol goes only over a vendor HID collection (usage page 0xFF4B, report ID 4, 64-byte reports, PnP ID 1209:0001; protocol/src/hid.rs). The GATT config service and btleplug were dropped. Desktop/CLI use hidapi (desktop/src/transport.rs), the browser build of kb-gui (trunk, desktop/index.html) uses WebHID (desktop/src/webhid.rs).

**Why:** Web Bluetooth was tried first: Chrome on Windows only lists advertising devices, and the keyboard stops advertising once connected (MAX_CONNECTIONS=1), so it never appeared. HID is seen via the OS whenever the keyboard is connected; Windows picked up the new report map without re-pairing. The user accepted Chromium-only for the web (no Firefox/Safari).

**How to apply:** don't propose Web Bluetooth / GATT again. Linux needs the udev rule (`kb udev-rule`); WSL can't see the keyboard (paired with Windows), so test the real connection on Windows.

# Project memory index

Maintained alongside the code. Follow the loading and update rules in
[AGENTS.md](../../AGENTS.md). Initial notes came from Claude on 2026-11-18;
their October–November 2026 observations require verification before reuse.

- [kb-gui design backlog](kb-gui-design-backlog.md) — remaining design work and known UI bugs after the theme pass
- [ESP serial under WSL](esp-serial-under-wsl.md) — usbipd auto-attach, flashing, reading the log without reset
- [Config protocol is HID-only](web-config-via-webhid.md) — hidapi + WebHID, GATT dropped; Web Bluetooth can't see the connected keyboard
- [Bump versions with changes](bump-versions-with-changes.md) — firmware and desktop crate versions go up with every change, since 0.2.0
- [Compat breaks may be intentional](compat-breaks-may-be-intentional.md) — report lost firmware compatibility, but ask before fixing it
- [kb via Windows](kb-via-windows.md) — WSL can't see the BLE keyboard; kb-win.sh runs kb.exe; OTA speed baseline
- [Battery measurements](battery-measurements.md) — USB video measurement conditions, battery switch, verified power-path and dependency behavior

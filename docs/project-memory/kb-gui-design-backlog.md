# Kb gui design backlog

Design rework of kb-gui (egui) started 2026-10-25: user chose a balanced style (Vial/Oryx-like), dark theme only,
key editor as a right side panel (since 2026-10-28 a card inside the centered content column, not
glued to the window edge: `theme::MAX_WIDTH` = 1200, header/status/content all line up to `theme::column`;
reason: the web build in a full-screen browser spread everything to the corners).

Done by 2026-10-25: `theme.rs` tokens, header, Layout, side-panel editor (fits mouse buttons), start screen
(centered connect progress, "Open file…" / "Edit offline"), combo segmented toggle, combo key chips,
shifted symbol labels ("!" for Shift+1).

Still open:
- Icons: default egui 0.37 fonts (Ubuntu-Light) lack arrows/circles; options are the emoji font (~700 KB) or own icons.
- Connected states are checked with `--fake-keyboard` (WSL has no BlueZ, a real connect fails at once).

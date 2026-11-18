# Project instructions

This file applies to the entire workspace, including `keyboard/`, `desktop/`,
`joystick/`, and `protocol/`. Paths below are relative to this workspace root,
regardless of the current working directory. Full notes are indexed in
[docs/project-memory/MEMORY.md](docs/project-memory/MEMORY.md). Read the relevant
note before working on the areas below. These notes describe the project as of
October–November 2026; verify implementation details, hardware identifiers,
performance measurements, and unfinished tasks against the current state.
Current user instructions take precedence.

## User preferences

- When changing firmware or desktop crate code, bump the corresponding version
  in `keyboard/Cargo.toml` or `desktop/Cargo.toml` in the same change and mention
  the bump. Firmware versions distinguish Update from Reinstall in the GUI.
  For protocol changes, also consult the versioning rules in `protocol/src/hid.rs`.
- Report compatibility breaks with older firmware, but do not automatically add
  flags or fallbacks. The user has intentionally accepted breaks before; ask
  whether a break is intended before fixing it unless already authorized.
- GUI direction: dark theme, balanced Vial/Oryx-like style, centered content
  column with aligned header/status and an editor card inside the column.

## Architecture and development

- Configuration uses a vendor HID collection: native hidapi and browser WebHID.
  GATT/btleplug were removed because Web Bluetooth discovery did not work for
  the already-connected keyboard. Chromium-only web support was accepted;
  do not propose returning to Web Bluetooth/GATT without a new requirement.
- The BLE keyboard is paired with Windows and is not visible to native WSL
  tools. Use `desktop/scripts/kb-win.sh` for real-device CLI/OTA operations;
  use `--fake-keyboard` for connected GUI states without hardware.
- ESP32-C3 USB serial detaches from WSL on reset. See
  `docs/project-memory/esp-serial-under-wsl.md` for reattachment and log reading
  without resetting the device. Prefer `keyboard/scripts/flash.sh` for USB
  flashing: it clears OTA selection metadata so the flashed app boots.
- Avoid per-chunk OTA logging: it previously throttled transfers substantially.
  Historical measurements and threading details are in
  `docs/project-memory/kb-via-windows.md`.
- The recorded GUI backlog includes missing arrow/circle glyphs; check whether
  this is still unresolved before acting on it.

## Automatic memory workflow

- At the start of each new task, read `docs/project-memory/MEMORY.md` and select
  the notes relevant to the task using the table below. Do this without waiting
  for the user to mention memory. Read selected notes before planning changes
  or running commands in their area; unrelated notes need not be loaded.
- Apply the user preferences above to every relevant change. Notes provide
  context, not authorization for unrelated hardware or external actions.
- Treat historical measurements, old backlog items, paths, and device IDs as
  leads to verify, not guaranteed current facts. Current user instructions and
  observed implementation take precedence over old notes.
- When the user explicitly corrects a lasting project preference, or work
  confirms a durable project fact, update the appropriate note in the same
  task. Update this file too if a universal rule changed. Do not save guesses,
  credentials, transient progress, or unrelated projects' context.
- When work resolves a recorded backlog item or invalidates an old procedure,
  update that note rather than leaving conflicting instructions. Date new
  observations and label historical measurements. Keep the index current when
  adding, renaming, or deleting a note; mention meaningful memory updates in
  the final response. Documentation-only updates do not require crate bumps.

| Task area | Read before working |
| --- | --- |
| Firmware or desktop code changes, release/version work | `docs/project-memory/bump-versions-with-changes.md` |
| Protocol, OTA, transport, compatibility review | `docs/project-memory/compat-breaks-may-be-intentional.md` and `docs/project-memory/web-config-via-webhid.md` |
| GUI layout, themes, icons, connected-state testing | `docs/project-memory/kb-gui-design-backlog.md` |
| USB flashing, serial logs, reset, sleep/wake diagnosis | `docs/project-memory/esp-serial-under-wsl.md` |
| Real-device CLI, Windows/WSL connectivity, OTA performance | `docs/project-memory/kb-via-windows.md` |

These notes were initially imported from Claude on 2026-11-18. They are now
maintained here and must not depend on files under either user's `.claude`
directory.

## Shared skills

- Canonical skill sources live in `skills/<name>/SKILL.md`. Edit those files,
  not tool-specific copies. `.agents/skills/<name>` and `.claude/skills/<name>`
  are relative symlinks to the same source.
- For GUI interaction, screenshots, or visual verification of kb-gui changes,
  read and follow `skills/kb-gui/SKILL.md`. This also applies when a client does
  not automatically discover the skill.
- Keep shared instructions and skills tool-neutral: describe actions and shell
  commands rather than requiring a particular agent's tool names. Store durable
  project knowledge in `docs/project-memory/`.
- `CLAUDE.md` imports this file. Keep common rules here, not duplicated there.

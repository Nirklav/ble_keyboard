---
name: kb-gui
description: Run the desktop config editor (kb-gui, egui) under WSLg, look at it via screenshots, click and type in it. Use to see a GUI change working, screenshot a screen, or reproduce a UI bug.
---

# Running kb-gui and looking at it

The app runs under WSLg, so its window is on the user's Windows desktop. `desktop/scripts/gui.sh`
drives that window from the Windows side through `powershell.exe` (`desktop/scripts/window.ps1`).
Run it from the repo root:

```bash
desktop/scripts/gui.sh start            # builds, launches, waits for the window (1256x737 with the frame)
desktop/scripts/gui.sh shot             # prints the PNG path (target/kb-gui/shot.png) -> Read it
desktop/scripts/gui.sh click 84 323     # window coordinates, as on the screenshot
desktop/scripts/gui.sh type 'MAIN'      # ASCII text into the focused widget
desktop/scripts/gui.sh resize 300 200 1221,702  # drag the frame corner at (1221, 702) to 300x200
desktop/scripts/gui.sh key ctrl+a       # enter, esc, tab, backspace, delete, arrows, f1..f12, ctrl+s, ...
desktop/scripts/gui.sh stop
desktop/scripts/gui.sh log              # app stdout / stderr, e.g. after a crash
```

- Always Read the screenshot before deciding where to click; coordinates are pixels from the window's
  top-left corner, including the X11 frame and its shadow, so they match the screenshot 1:1
  (the app content starts at about (38, 59)). On WSL the app uses X11, not WSLg's frameless Wayland.
- Give several shots distinct names (`shot target/kb-gui/combos.png`) when comparing screens.
- Without a keyboard nearby: `gui.sh start --offline` opens the editor right away
  with the example config. "Connect" needs the keyboard as a HID device, which WSL doesn't see (it's paired with Windows).
- `gui.sh start --fake-keyboard` talks to a fake keyboard instead: connect, read, write and factory
  reset work with delays and progress, so the connected UI can be looked at. Combines with `--offline`.
- Every action brings the window to the front and moves the user's mouse: the user's desktop is in use,
  so keep sessions short and `stop` the app when done.
- `type` switches the window's input language to English while typing and restores it afterwards.
- `start` rebuilds the binary; restart after code changes.
- If system libraries (libxkbcommon, libEGL, ...) are missing, `start` unpacks them into
  `target/kb-gui/libs` on first run; `sudo apt install` from the script header removes the need.

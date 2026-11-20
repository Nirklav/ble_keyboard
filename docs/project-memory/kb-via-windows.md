# Kb via windows

The keyboard is a BLE HID device of Windows; WSL (usbipd is USB only) never sees it, so native `kb` finds nothing.
`desktop/scripts/kb-win.sh <kb args>` rsyncs the sources to %LOCALAPPDATA%\kb-win, builds `kb` with the Windows
cargo there and runs kb.exe; WSL path args are converted. Use it for `info`, `update <image>` and OTA speed checks.
Image for update: `espflash save-image --chip esp32c3 target/riscv32imc-esp-espidf/release/keyboard <out.bin>`.

On 2026-11-20, the default firmware build stalled while preparing ESP-IDF tools in network-restricted WSL.
From `keyboard/`, `ESP_IDF_TOOLS_INSTALL_DIR=fromenv cargo build --release --locked --offline` worked with
the installed ESP-IDF 5.5. Set `IDF_TOOLS_PATH` to `/home/anton/sources/rust/.embuild`, `IDF_PATH` to its
`esp-idf/v5.5` checkout, and `IDF_PYTHON_ENV_PATH` to its `python_env/idf5.5_py3.9_env`. Prepend the checkout's
`tools`, the Python environment's `bin`, and the installed CMake, Ninja, and RISC-V GCC directories to `PATH`.
The stock `export.sh` expected additional tools. Bindgen used `LIBCLANG_PATH` pointing to `esp-clang/lib`
under the installed `xtensa-esp32-elf-clang/esp-16.0.0-20230516` in the Rust `esp` toolchain.

OTA speed 2026-10-30 (650 KB compressed): ~5 min stop-and-wait; 115 s with CHUNK_WINDOW=16; 115 s still at a
7.5 ms interval (ConnParams::TRANSFER); 64 s once NimBLE logs went to WARN (it logged every notify to the
USB serial, which throttled the transfer). Logging per chunk is the thing to avoid.
Then 60 s with build-std-features = [] (no std backtrace, image 1.14 MB -> 1.00 MB, 567 KB compressed).
The thread logs "Firmware received in ..." on commit: ~45% busy (half inflate, half flash writes), rest waiting
for the link. Erasing 64 KiB blocks ahead of the writes (not a sector per write): ~50 s. The firmware thread runs at
prio 1 like the main task (pthreads default to 5 and starved the main loop), and chunks don't wake the main loop
(waking it ~200/s ate the CPU: 96 s); the firmware thread wakes it on update start/end instead.
Related: [esp-serial-under-wsl](esp-serial-under-wsl.md)

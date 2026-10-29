#!/usr/bin/env bash
# Builds the keyboard firmware (release), saves its app image to target/firmware/keyboard.bin and builds
# kb and kb-gui (release) with it bundled (KB_FIRMWARE, see desktop/build.rs): `kb update` without a file
# and the Update button of kb-gui install it.
#
#   bundle.sh [cargo build args for the desktop crate]
#
# The browser build bundles it the same way: KB_FIRMWARE=target/firmware/keyboard.bin trunk build --release
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image="$root/target/firmware/keyboard.bin"

# The firmware crate has its own target and toolchain (keyboard/.cargo/config.toml), build it from its directory.
(cd "$root/keyboard" && cargo build --release)
mkdir -p "$(dirname "$image")"
espflash save-image --chip esp32c3 "$root/target/riscv32imc-esp-espidf/release/keyboard" "$image"

KB_FIRMWARE="$image" cargo build --manifest-path "$root/desktop/Cargo.toml" --release --bins "$@"

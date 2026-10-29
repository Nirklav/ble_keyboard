#!/usr/bin/env bash
# Builds the keyboard firmware (release), saves its app image to target/firmware/keyboard.bin and builds the
# app with it bundled (KB_FIRMWARE, see desktop/build.rs): `kb update` without a file and the Update button of
# kb-gui install it.
#
#   bundle.sh [cargo build args]   kb and kb-gui (release)
#   bundle.sh docker               the browser build as the Docker image reg.thirtynineeighty.com/kb-gui
#                                  (desktop/Dockerfile)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image="$root/target/firmware/keyboard.bin"

firmware_image() {
    # The firmware crate has its own target and toolchain (keyboard/.cargo/config.toml), build it from its directory.
    (cd "$root/keyboard" && cargo build --release)
    mkdir -p "$(dirname "$image")"
    espflash save-image --chip esp32c3 "$root/target/riscv32imc-esp-espidf/release/keyboard" "$image"
}

case "${1:-}" in
    docker)
        firmware_image
        docker build -f "$root/desktop/Dockerfile" -t reg.thirtynineeighty.com/kb-gui "$root"
        ;;
    *)
        firmware_image
        KB_FIRMWARE="$image" cargo build --manifest-path "$root/desktop/Cargo.toml" --release --bins "$@"
        ;;
esac

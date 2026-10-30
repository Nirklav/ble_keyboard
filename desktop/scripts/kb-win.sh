#!/usr/bin/env bash
# Runs the Windows build of the CLI (`kb.exe`) from WSL: WSL doesn't see the keyboard, Windows has it as a HID
# device (BLE, HID over GATT).
#
#   kb-win.sh [kb args]   sync the sources to Windows, build `kb` there with the Windows cargo, run it
#
# Arguments that are existing WSL paths (a firmware image, a config) are passed as Windows paths.
# KB_FIRMWARE (a WSL path) bundles that firmware, like in the native build.
#
# Builds in %LOCALAPPDATA%\kb-win: cargo on a \\wsl$ path is slow. Needs Rust (stable, MSVC) on Windows.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
win_dir="$(cmd.exe /d /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')\\kb-win"
dir="$(wslpath -u "$win_dir")"
cargo="$(wslpath -u "$(cmd.exe /d /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")/.cargo/bin/cargo.exe"
[[ -x "$cargo" ]] || { echo "no Rust on Windows: $cargo" >&2; exit 1; }

# rust-toolchain.toml is left out: it pulls in the Linux and wasm targets, Windows needs neither.
mkdir -p "$dir/src"
rsync -a --delete --exclude target --exclude dist \
    "$root/Cargo.toml" "$root/Cargo.lock" "$root/.cargo" "$root/desktop" "$root/protocol" "$dir/src/"

firmware=""
if [[ -n "${KB_FIRMWARE:-}" ]]; then
    firmware="$(wslpath -w "$KB_FIRMWARE")"
fi
# Windows programs get the WSL variables named in WSLENV.
(cd "$dir/src" && KB_FIRMWARE="$firmware" WSLENV="KB_FIRMWARE:${WSLENV:-}" \
    "$cargo" build --quiet --release --locked --bin kb --target-dir "$win_dir\\target") >&2

args=()
for arg in "$@"; do
    if [[ -e "$arg" ]]; then
        arg="$(wslpath -w "$(realpath "$arg")")"
    fi
    args+=("$arg")
done
exec "$dir/target/release/kb.exe" "${args[@]}"

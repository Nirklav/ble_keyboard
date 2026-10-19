#!/usr/bin/env bash
# Runs kb-gui under WSLg and drives its window from the Windows side (see window.ps1).
#
#   gui.sh start [kb-gui args]   build, launch in the background and wait for the window
#   gui.sh shot [file.png]       screenshot the window (default: target/kb-gui/shot.png), prints the path
#   gui.sh click <x> <y>         left click at window coordinates (as on the screenshot)
#   gui.sh type <text>           type ASCII text into the focused widget
#   gui.sh resize <w> <h> [x,y]  drag the window corner (or x,y) to w,h, prints the resulting size
#   gui.sh key <combo>           press a key combo: enter, esc, tab, backspace, up, f2, ctrl+a, ctrl+s, ...
#   gui.sh stop                  stop the app
#   gui.sh log                   print the app's stdout / stderr
#
# Ubuntu 20.04 lacks libraries winit needs (libxkbcommon, libEGL, libXcursor, ...). Install them with
#   sudo apt install libxkbcommon0 libxkbcommon-x11-0 libxcursor1 libegl1 libegl-mesa0 libwayland-cursor0 libwayland-egl1
# otherwise `start` downloads the packages and unpacks them into target/kb-gui/libs (no root needed).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state="$root/target/kb-gui"
ps1="$(wslpath -w "$root/desktop/scripts/window.ps1")"
mkdir -p "$state"

window() {
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ps1" "$@" | tr -d '\r'
}

libs_env() {
    local lib missing=
    for lib in libxkbcommon.so.0 libxkbcommon-x11.so.0 libXcursor.so.1 libEGL.so.1; do
        ldconfig -p | grep -q "$lib" || missing=1
    done
    [[ -n "$missing" ]] || return 0
    local libs="$state/libs"
    if [[ ! -e "$libs/.done" ]]; then
        echo "system libraries missing, unpacking them into $libs" >&2
        local debs="$state/debs"
        rm -rf "$debs" "$libs" && mkdir -p "$debs" "$libs"
        (cd "$debs" && apt-get download -q libxkbcommon0 libxkbcommon-x11-0 libxcb-xkb1 libxcursor1 \
            libegl1 libegl-mesa0 libgbm1 libwayland-cursor0 libwayland-egl1 libwayland-server0 >&2)
        for deb in "$debs"/*.deb; do dpkg -x "$deb" "$libs"; done
        touch "$libs/.done"
    fi
    export LD_LIBRARY_PATH="$libs/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export __EGL_VENDOR_LIBRARY_DIRS="$libs/usr/share/glvnd/egl_vendor.d"
}

stop() {
    if [[ -f "$state/pid" ]]; then
        kill "$(cat "$state/pid")" 2>/dev/null || true
        rm -f "$state/pid"
    fi
}

case "${1:-}" in
start)
    shift
    stop
    cargo build --manifest-path "$root/desktop/Cargo.toml" --bin kb-gui >&2
    libs_env
    nohup "$root/target/debug/kb-gui" "$@" >"$state/log" 2>&1 &
    echo $! >"$state/pid"
    for _ in $(seq 1 40); do
        if ! kill -0 "$(cat "$state/pid")" 2>/dev/null; then
            echo "kb-gui exited:" >&2
            cat "$state/log" >&2
            exit 1
        fi
        if size="$(window -Action find 2>/dev/null)"; then
            echo "kb-gui running, window $size"
            exit 0
        fi
        sleep 0.5
    done
    echo "window did not appear in 20 s, see $state/log" >&2
    exit 1
    ;;
shot)
    out="$(realpath -m "${2:-$state/shot.png}")"
    sleep 0.3 # let egui repaint after the last input
    window -Action shot -Out "$(wslpath -w "$out")" >/dev/null
    echo "$out"
    ;;
click)
    window -Action click -X "$2" -Y "$3"
    ;;
type)
    window -Action type -Text "$2"
    ;;
key)
    window -Action key -Text "$2"
    ;;
resize)
    window -Action resize -X "$2" -Y "$3" ${4:+-Text "$4"}
    ;;
stop)
    stop
    ;;
log)
    cat "$state/log"
    ;;
*)
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac

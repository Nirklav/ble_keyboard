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
# Needs the libraries winit loads: sudo apt install libxkbcommon-x11-0 libxcursor1 libegl1
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state="$root/target/kb-gui"
ps1="$(wslpath -w "$root/desktop/scripts/window.ps1")"
mkdir -p "$state"

window() {
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ps1" "$@" | tr -d '\r'
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

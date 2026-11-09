#!/usr/bin/env bash
# Click on the romacs/emacs winit window at a given position.
#
# winit ignores synthetic X events (XSendEvent), so xdotool --window
# doesn't work for clicks.  This script converts window-relative
# coordinates to absolute screen coordinates and uses xdotool's
# XTest-based click, which winit does receive.
#
# Usage:
#   romacs-click.sh X Y [BUTTON]                    # click at (X,Y)
#   romacs-click.sh X Y --screenshot FILE            # click and screenshot
#   romacs-click.sh --screenshot FILE                # screenshot only
#   romacs-click.sh --list                           # list windows
#
# Options:
#   -w, --window WID      Use specific window ID
#   -d, --delay SEC       Delay between move and click (default: 0.2)
#   -n, --count N         Number of clicks (default: 1)
#   -s, --screenshot F    Take screenshot after click
#   --double              Double-click
#   --right               Right-click (button 3)
#   --list                List romacs windows and exit
#   --dry-run             Show what would happen without clicking
#
# Examples:
#   ./scripts/romacs-click.sh 100 10                        # click tab-bar
#   ./scripts/romacs-click.sh 200 300 --right               # right-click
#   ./scripts/romacs-click.sh 100 10 -s /tmp/after.png      # click + screenshot
#   ./scripts/romacs-click.sh 50 50 --double                # double-click
#   ./scripts/romacs-click.sh -s /tmp/snap.png              # screenshot only

set -euo pipefail

DISPLAY="${DISPLAY:-:0}"
export DISPLAY

WIN_ID=""
DELAY=0.2
BUTTON=1
COUNT=1
SCREENSHOT=""
DRY_RUN=false
LIST=false
X=""
Y=""

die() { echo "ERROR: $*" >&2; exit 1; }

# --- Find romacs window (skip 1x1 helper windows) ---
find_window() {
    local pids wid
    pids=$(pgrep -f './src/emacs|/emacs ' 2>/dev/null || true)
    for pid in $pids; do
        wid=$(xdotool search --pid "$pid" 2>/dev/null | while read -r id; do
            geom=$(xdotool getwindowgeometry "$id" 2>/dev/null || true)
            if ! echo "$geom" | grep -q 'Geometry: 1x1'; then
                echo "$id"
                break
            fi
        done)
        if [[ -n "$wid" ]]; then
            echo "$wid"
            return
        fi
    done
    xdotool search --name "emacs" 2>/dev/null | head -1
}

# --- Parse arguments ---
while [[ $# -gt 0 ]]; do
    case "$1" in
        --list)          LIST=true; shift ;;
        --dry-run)       DRY_RUN=true; shift ;;
        --double)        COUNT=2; shift ;;
        --right)         BUTTON=3; shift ;;
        -w|--window)     WIN_ID="$2"; shift 2 ;;
        -d|--delay)      DELAY="$2"; shift 2 ;;
        -n|--count)      COUNT="$2"; shift 2 ;;
        -s|--screenshot) SCREENSHOT="$2"; shift 2 ;;
        -h|--help)       sed -n '2,/^$/{ s/^# \?//; p }' "$0"; exit 0 ;;
        -*)              die "Unknown option: $1" ;;
        *)
            if [[ -z "$X" ]]; then X="$1"
            elif [[ -z "$Y" ]]; then Y="$1"
            else BUTTON="$1"
            fi
            shift ;;
    esac
done

# --- List mode ---
if $LIST; then
    echo "Romacs/Emacs windows:"
    pids=$(pgrep -f './src/emacs|/emacs ' 2>/dev/null || true)
    found=false
    for pid in $pids; do
        for wid in $(xdotool search --pid "$pid" 2>/dev/null); do
            geom=$(xdotool getwindowgeometry "$wid" 2>/dev/null || true)
            if ! echo "$geom" | grep -q 'Geometry: 1x1'; then
                echo "  PID=$pid  WID=$wid"
                echo "$geom" | sed 's/^/    /'
                found=true
            fi
        done
    done
    $found || echo "  (none found)"
    exit 0
fi

# --- Screenshot-only mode ---
if [[ -z "$X" && -z "$Y" && -n "$SCREENSHOT" ]]; then
    if [[ -z "$WIN_ID" ]]; then
        WIN_ID=$(find_window)
        [[ -n "$WIN_ID" ]] || die "No romacs window found"
    fi
    import -window "$WIN_ID" "$SCREENSHOT"
    echo "Screenshot saved to $SCREENSHOT"
    exit 0
fi

# --- Validate ---
[[ -n "$X" && -n "$Y" ]] || die "Usage: romacs-click.sh X Y [options]  (try --help)"

# --- Resolve window ---
if [[ -z "$WIN_ID" ]]; then
    WIN_ID=$(find_window)
    [[ -n "$WIN_ID" ]] || die "No romacs window found. Use --window WID or --list."
fi

# --- Get window geometry ---
geom_output=$(xdotool getwindowgeometry "$WIN_ID" 2>/dev/null) \
    || die "Cannot get geometry for window $WIN_ID"

win_x=$(echo "$geom_output" | grep Position | sed 's/.*Position: \([0-9]*\),.*/\1/')
win_y=$(echo "$geom_output" | grep Position | sed 's/.*Position: [0-9]*,\([0-9]*\).*/\1/')
win_w=$(echo "$geom_output" | grep Geometry | sed 's/.*Geometry: \([0-9]*\)x.*/\1/')
win_h=$(echo "$geom_output" | grep Geometry | sed 's/.*Geometry: [0-9]*x\([0-9]*\)/\1/')

abs_x=$((win_x + X))
abs_y=$((win_y + Y))

if (( X < 0 || X >= win_w || Y < 0 || Y >= win_h )); then
    echo "WARNING: ($X, $Y) is outside window bounds (${win_w}x${win_h})" >&2
fi

echo "Window:  $WIN_ID (${win_w}x${win_h} at ${win_x},${win_y})"
echo "Click:   ($X, $Y) relative -> ($abs_x, $abs_y) absolute"
echo "Button:  $BUTTON  Count: $COUNT"

if $DRY_RUN; then
    echo "(dry run)"
    exit 0
fi

# --- Move and click (XTest-based, works with winit) ---
xdotool mousemove --sync "$abs_x" "$abs_y"
sleep "$DELAY"

for ((i = 0; i < COUNT; i++)); do
    xdotool click "$BUTTON"
    (( i < COUNT - 1 )) && sleep 0.05
done

echo "Done."

# --- Optional screenshot ---
if [[ -n "$SCREENSHOT" ]]; then
    sleep 0.5
    import -window "$WIN_ID" "$SCREENSHOT"
    echo "Screenshot saved to $SCREENSHOT"
fi

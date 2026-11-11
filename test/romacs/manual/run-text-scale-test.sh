#!/usr/bin/env bash
# Test text-scale-adjust rendering (per-window font scaling, variable-height faces)
# Usage: ./test/romacs/run-text-scale-test.sh
#
# What this tests:
# - text-scale-adjust (C-x C-+ / C-x C--) per-window scaling
# - Variable-height face rendering at different scales
# - Cycling animation of scale levels (key 'c')
# - Full test suite across all scale scenarios (key 'a')
# - Crossfade animation on scale change
#
# Interactions: press 'c' to start cycling animation, wait 15s,
# then press 'a' to run all tests, wait 40s.

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/text-scale-test.log
SCREENSHOT=/tmp/text-scale-screenshot.png
TEST_NAME="Text Scale"

echo "=== $TEST_NAME Test ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/text-scale-test.el >"$LOG" 2>&1 &
EMACS_PID=$!

echo "Romacs PID: $EMACS_PID"
echo "Waiting for window to appear..."
sleep 5

# Find Romacs window
WIN_ID=$(DISPLAY=:0 xdotool search --class "romacs" 2>/dev/null | head -1)
if [ -z "$WIN_ID" ]; then
    echo "ERROR: Could not find Romacs window"
    kill $EMACS_PID 2>/dev/null || true
    exit 1
fi

echo "Found window: $WIN_ID"
DISPLAY=:0 xdotool getwindowgeometry "$WIN_ID"

# Activate window
echo "Activating window..."
DISPLAY=:0 xdotool windowactivate --sync "$WIN_ID"
sleep 1

# Press 'c' to start cycling animation through scale levels
echo ""
echo "=== Test 1: Cycling animation (pressing 'c') ==="
echo "This cycles through various text-scale levels with animation."
DISPLAY=:0 xdotool key --window "$WIN_ID" c
sleep 2

echo "Waiting 15 seconds for scale cycling animation..."
sleep 15

# Take screenshot mid-animation
echo ""
echo "=== Taking mid-animation screenshot ==="
if command -v import &>/dev/null; then
    DISPLAY=:0 import -window "$WIN_ID" "$SCREENSHOT" 2>/dev/null || true
    if [ -f "$SCREENSHOT" ]; then
        echo "Screenshot saved: $SCREENSHOT"
    else
        echo "Screenshot capture failed (non-fatal)"
    fi
else
    echo "Skipping screenshot (ImageMagick 'import' not available)"
fi

# Press 'a' to run all text-scale test scenarios
echo ""
echo "=== Test 2: Running all text-scale tests (pressing 'a') ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" a
sleep 2

echo "Waiting 40 seconds for all text-scale test scenarios to cycle..."
sleep 40

# Check logs for errors
echo ""
echo "=== Checking log entries ==="
if [ -f "$LOG" ]; then
    PANIC_COUNT=$(grep -ci "panic" "$LOG" 2>/dev/null || true)
    ERROR_COUNT=$(grep -ci "error" "$LOG" 2>/dev/null || true)

    if [ "$PANIC_COUNT" -gt 0 ]; then
        echo "WARNING: $PANIC_COUNT PANIC entries found!"
        grep -i "panic" "$LOG" | tail -5
    else
        echo "No PANIC entries detected."
    fi

    if [ "$ERROR_COUNT" -gt 0 ]; then
        echo "WARNING: $ERROR_COUNT ERROR entries found:"
        grep -i "error" "$LOG" | tail -10
    else
        echo "No ERROR entries detected."
    fi
else
    echo "Log file not found."
fi

# Cleanup
echo ""
echo "Stopping Romacs..."
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

# Summary
echo ""
echo "=== $TEST_NAME Test Summary ==="
if [ "$PANIC_COUNT" -gt 0 ]; then
    echo "RESULT: PANICS DETECTED - check log"
    exit 1
elif [ "$ERROR_COUNT" -gt 0 ]; then
    echo "RESULT: ERRORS DETECTED - check log for details"
    exit 1
else
    echo "RESULT: No panics or errors detected"
fi
echo "Full log at: $LOG"
[ -f "$SCREENSHOT" ] && echo "Screenshot at: $SCREENSHOT"

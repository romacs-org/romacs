#!/usr/bin/env bash
# Test word-wrap rendering across different modes and configurations
# Usage: ./test/romacs/manual/run-word-wrap-test.sh
#
# What this tests:
# - Word wrap with truncate-lines off
# - Visual-line-mode wrapping
# - 4-pane side-by-side comparison (key '1')
# - Cycling through all wrap scenarios (key 'a')
# - Interaction of word-wrap with variable-width fonts, faces, overlays
#
# Interactions: press '1' for 4-pane comparison, then 'a' to cycle all.

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/word-wrap-test.log
SCREENSHOT=/tmp/word-wrap-screenshot.png
TEST_NAME="Word Wrap"

echo "=== $TEST_NAME Test ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/manual/word-wrap-test.el >"$LOG" 2>&1 &
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

# Press '1' for 4-pane comparison view
echo ""
echo "=== Test 1: 4-pane comparison view (pressing '1') ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 1
sleep 3

# Take screenshot of 4-pane comparison
echo "=== Taking 4-pane screenshot ==="
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

# Press 'a' to cycle through all test scenarios
echo ""
echo "=== Test 2: Cycling through all scenarios (pressing 'a') ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" a
sleep 2

echo "Waiting 20 seconds for all wrap scenarios to cycle..."
sleep 20

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

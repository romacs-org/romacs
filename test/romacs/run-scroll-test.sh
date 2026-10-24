#!/usr/bin/env bash
# Test scroll rendering (page up/down, arrow key scrolling, scroll animations)
# Usage: ./test/romacs/run-scroll-test.sh
#
# What this tests:
# - Page scrolling with C-v (scroll-down) and M-v (scroll-up)
# - Line-by-line scrolling with arrow keys
# - Scroll slide animation transitions
# - Content rendering after scroll operations
#
# Interactions: use xdotool to scroll with C-v, M-v, and arrow keys,
# wait between scrolls, take screenshot, check log.

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/scroll-test.log
SCREENSHOT=/tmp/scroll-screenshot.png
TEST_NAME="Scroll"

echo "=== $TEST_NAME Test ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/romacs-scroll-test.el >"$LOG" 2>&1 &
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

# === Test 1: Page down scrolling ===
echo ""
echo "=== Test 1: Page down scrolling (C-v) ==="
for i in $(seq 1 3); do
    echo "Page down $i..."
    DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+v
    sleep 0.8
done
sleep 1

# === Test 2: Page up scrolling ===
echo ""
echo "=== Test 2: Page up scrolling (M-v) ==="
for i in $(seq 1 3); do
    echo "Page up $i..."
    DISPLAY=:0 xdotool key --window "$WIN_ID" alt+v
    sleep 0.8
done
sleep 1

# === Test 3: Arrow key scrolling ===
echo ""
echo "=== Test 3: Arrow key scrolling (Down/Up) ==="
echo "Scrolling down 15 lines..."
for i in $(seq 1 15); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Down
    sleep 0.1
done
sleep 0.5

echo "Scrolling up 10 lines..."
for i in $(seq 1 10); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Up
    sleep 0.1
done
sleep 1

# === Test 4: Rapid scrolling ===
echo ""
echo "=== Test 4: Rapid page scrolling ==="
echo "Rapid page down..."
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+v
    sleep 0.2
done
sleep 0.5

echo "Rapid page up..."
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" alt+v
    sleep 0.2
done
sleep 1

# Take screenshot after scroll tests
echo ""
echo "=== Taking screenshot ==="
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

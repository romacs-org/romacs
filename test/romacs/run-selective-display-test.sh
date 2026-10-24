#!/usr/bin/env bash
# Test selective display, outline mode, and code folding rendering
# Usage: ./test/romacs/run-selective-display-test.sh

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/selective-display-test.log
SCREENSHOT=/tmp/selective-display-screenshot.png

echo "=== Selective Display / Folding Test ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/selective-display-test.el >"$LOG" 2>&1 &
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

# Activate window
DISPLAY=:0 xdotool windowactivate --sync "$WIN_ID"
sleep 1

# Run all tests
echo "Running all selective-display tests (pressing 'a')..."
DISPLAY=:0 xdotool key --window "$WIN_ID" a
sleep 35

# Take screenshot
if command -v import &>/dev/null; then
    echo "Taking screenshot..."
    DISPLAY=:0 import -window "$WIN_ID" "$SCREENSHOT" 2>/dev/null && \
        echo "Screenshot: $SCREENSHOT" || echo "Screenshot failed"
else
    echo "ImageMagick not available, skipping screenshot"
fi

# Check logs
echo ""
echo "=== Checking logs ==="
PANIC_COUNT=$(grep -ci "PANIC" "$LOG" 2>/dev/null || true)
ERROR_COUNT=$(grep -ci "ERROR" "$LOG" 2>/dev/null || true)
echo "PANIC entries: $PANIC_COUNT"
echo "ERROR entries: $ERROR_COUNT"

if [ "$PANIC_COUNT" -gt 0 ]; then
    echo "--- PANIC log entries ---"
    grep -i "PANIC" "$LOG" | tail -5
fi

# Cleanup
echo ""
echo "Stopping Romacs..."
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo ""
if [ "$PANIC_COUNT" -gt 0 ]; then
    echo "RESULT: FAIL (panics detected)"
    exit 1
else
    echo "RESULT: PASS"
fi
echo "Full log: $LOG"

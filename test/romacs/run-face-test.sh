#!/usr/bin/env bash
# Test face rendering (font faces, colors, attributes, variable-height faces)
# Usage: ./test/romacs/run-face-test.sh
#        ROMACS_BIN=./target/release/romacs ./test/romacs/run-face-test.sh
#
# What this tests:
# - Face attribute rendering (foreground, background, bold, italic, underline)
# - Variable-height face rendering (:height attribute)
# - Face inheritance and composition
# - Box face borders and strike-through
#
# Interactions: wait for face rendering, take screenshot, check log.

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/face-test.log
SCREENSHOT=/tmp/face-screenshot.png
MATRIX_REPORT=/tmp/face-matrix-report.txt
TEST_NAME="Face"
ROMACS_BIN="${ROMACS_BIN:-./target/release/romacs}"
PANIC_COUNT=0
ERROR_COUNT=0

echo "=== $TEST_NAME Test ==="
echo "Starting Romacs..."

if [ ! -x "$ROMACS_BIN" ]; then
    echo "ERROR: Romacs binary not found or not executable: $ROMACS_BIN"
    echo "Run: cargo xtask fresh-build --release"
    exit 1
fi

rm -f "$LOG" "$SCREENSHOT" "$MATRIX_REPORT"

RUST_LOG=info DISPLAY="${DISPLAY:-:0}" "$ROMACS_BIN" -Q \
    -l test/romacs/romacs-face-test.el \
    --eval "(run-at-time 5 nil (lambda () (romacs-face-test-write-matrix-report \"$MATRIX_REPORT\")))" \
    >"$LOG" 2>&1 &
ROMACS_PID=$!

echo "Romacs PID: $ROMACS_PID"
echo "Waiting for window to appear..."
sleep 5

# Find Romacs window.
WIN_ID=$(DISPLAY="${DISPLAY:-:0}" xdotool search --class "romacs" 2>/dev/null | head -1)
if [ -z "$WIN_ID" ]; then
    echo "ERROR: Could not find Romacs window"
    kill $ROMACS_PID 2>/dev/null || true
    exit 1
fi

echo "Found window: $WIN_ID"
DISPLAY="${DISPLAY:-:0}" xdotool getwindowgeometry "$WIN_ID"

# Activate window
echo "Activating window..."
DISPLAY="${DISPLAY:-:0}" xdotool windowactivate --sync "$WIN_ID"
sleep 1

# Wait for face rendering to complete
echo ""
echo "=== Waiting 5 seconds for face rendering ==="
sleep 5

# Take screenshot
echo ""
echo "=== Taking screenshot ==="
if command -v import &>/dev/null; then
    DISPLAY="${DISPLAY:-:0}" import -window "$WIN_ID" "$SCREENSHOT" 2>/dev/null || true
    if [ -f "$SCREENSHOT" ]; then
        echo "Screenshot saved: $SCREENSHOT"
    else
        echo "Screenshot capture failed (non-fatal)"
    fi
else
    echo "Skipping screenshot (ImageMagick 'import' not available)"
fi

# Wait for matrix report timer.
sleep 1
if [ -f "$MATRIX_REPORT" ]; then
    echo "Face matrix report saved: $MATRIX_REPORT"
else
    echo "WARNING: Face matrix report was not written."
fi

# Check logs for errors
echo ""
echo "=== Checking log entries ==="
if [ -f "$LOG" ]; then
    PANIC_COUNT=$(grep -ci "panic" "$LOG" 2>/dev/null || true)
    ERROR_COUNT=$(grep -c " ERROR " "$LOG" 2>/dev/null || true)

    if [ "$PANIC_COUNT" -gt 0 ]; then
        echo "WARNING: $PANIC_COUNT PANIC entries found!"
        grep -i "panic" "$LOG" | tail -5
    else
        echo "No PANIC entries detected."
    fi

    if [ "$ERROR_COUNT" -gt 0 ]; then
        echo "WARNING: $ERROR_COUNT ERROR entries found:"
        grep " ERROR " "$LOG" | tail -10
    else
        echo "No ERROR entries detected."
    fi
else
    echo "Log file not found."
fi

# Cleanup
echo ""
echo "Stopping Romacs..."
kill $ROMACS_PID 2>/dev/null || true
wait $ROMACS_PID 2>/dev/null || true

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

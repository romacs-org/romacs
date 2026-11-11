#!/usr/bin/env bash
# Test child frame GPU overlay compositing using xdotool
# Usage: ./test/romacs/manual/run-child-frame-test.sh
#
# What this tests:
# 1. Basic child frame creation and positioning
# 2. Z-order stacking of overlapping child frames
# 3. Alpha transparency
# 4. Border styles
# 5. Rich face content inside child frames
# 6. Cursor focus across child frames
# 7. Style configuration (corner radius, shadow)
# 8. Rapid create/delete lifecycle
# 9. Posframe-like tooltip

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/child-frame-test.log

echo "=== Child Frame Test Suite ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/manual/child-frame-test.el >"$LOG" 2>&1 &
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
sleep 2

echo ""
echo "=== Test 1: Basic positioning (loaded on startup) ==="
echo "3 child frames should be visible at different positions."
sleep 3

# Run test 2: Z-order
echo ""
echo "=== Test 2: Z-order stacking ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 2
sleep 3

# Run test 3: Alpha transparency
echo ""
echo "=== Test 3: Alpha transparency ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 3
sleep 3

# Run test 4: Border styles
echo ""
echo "=== Test 4: Border styles ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 4
sleep 3

# Run test 5: Rich face content
echo ""
echo "=== Test 5: Rich face content ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 5
sleep 3

# Run test 6: Cursor focus
echo ""
echo "=== Test 6: Cursor focus ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 6
sleep 2

# Click on the first child frame to test focus
echo "Clicking first child frame..."
DISPLAY=:0 xdotool mousemove --window "$WIN_ID" 200 150
sleep 0.3
DISPLAY=:0 xdotool click --window "$WIN_ID" 1
sleep 1

# Type some text
echo "Typing in child frame..."
DISPLAY=:0 xdotool type --window "$WIN_ID" "hello"
sleep 1

# Click second child frame
echo "Clicking second child frame..."
DISPLAY=:0 xdotool mousemove --window "$WIN_ID" 500 150
sleep 0.3
DISPLAY=:0 xdotool click --window "$WIN_ID" 1
sleep 1
DISPLAY=:0 xdotool type --window "$WIN_ID" "world"
sleep 2

# Run test 7: Style config
echo ""
echo "=== Test 7: Style configuration ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 7
sleep 3

# Run test 8: Rapid lifecycle
echo ""
echo "=== Test 8: Rapid lifecycle ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 8
sleep 3

# Run test 9: Tooltip
echo ""
echo "=== Test 9: Tooltip ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" 9
sleep 3

# Cleanup
echo ""
echo "=== Cleaning up ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" q
sleep 1

# Check logs
echo ""
echo "=== Checking log entries ==="
if [ -f "$LOG" ]; then
    grep -ci "child" "$LOG" 2>/dev/null && echo " child frame log entries" || echo "No child frame log entries"
    grep -ci "error\|panic\|crash" "$LOG" 2>/dev/null && echo " ERROR entries found!" || echo "No errors detected"
fi

echo ""
echo "Waiting 3 seconds before shutdown..."
sleep 3

# Cleanup
echo "Stopping Romacs..."
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Child frame test complete ==="
echo "Full log at: $LOG"

#!/usr/bin/env bash
# Test cursor animation, buffer crossfade, and scroll slide using xdotool
# Usage: ./test/romacs/manual/run-cursor-animation-test.sh
#
# What this tests:
# 1. Smooth cursor: moves cursor with arrow keys — cursor should glide
# 2. Buffer crossfade: switches buffer with C-x b — should see crossfade
# 3. Scroll slide: pages down with C-v — should see slide animation

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/cursor-animation-test.log

echo "=== Cursor Animation Test ==="
echo "Starting Romacs..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/manual/cursor-animation-test.el >"$LOG" 2>&1 &
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

# === Test 1: Smooth cursor motion ===
echo ""
echo "=== Test 1: Smooth cursor motion (arrow keys) ==="
echo "Moving cursor down 10 lines..."
for i in $(seq 1 10); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Down
    sleep 0.15
done
echo "Moving cursor right 20 chars..."
for i in $(seq 1 20); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Right
    sleep 0.1
done
echo "Moving cursor up 5 lines..."
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Up
    sleep 0.15
done
sleep 1
echo "Cursor motion test done."

# === Test 2: Scroll slide animation ===
echo ""
echo "=== Test 2: Scroll slide (C-v / M-v) ==="
echo "Page down..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+v
sleep 0.5
echo "Page down..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+v
sleep 0.5
echo "Page up..."
DISPLAY=:0 xdotool key --window "$WIN_ID" alt+v
sleep 0.5
echo "Page up..."
DISPLAY=:0 xdotool key --window "$WIN_ID" alt+v
sleep 1
echo "Scroll slide test done."

# === Test 3: Buffer crossfade ===
echo ""
echo "=== Test 3: Buffer crossfade (C-x b) ==="
echo "Switching to *AnimTest-B*..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" b
sleep 0.3
DISPLAY=:0 xdotool type --window "$WIN_ID" "*AnimTest-B*"
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" Return
sleep 1

echo "Switching back to *AnimTest-A*..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" b
sleep 0.3
DISPLAY=:0 xdotool type --window "$WIN_ID" "*AnimTest-A*"
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" Return
sleep 1
echo "Buffer crossfade test done."

# === Test 4: Multi-window with split ===
echo ""
echo "=== Test 4: Split window + cursor in each ==="
echo "Splitting window horizontally..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.1
DISPLAY=:0 xdotool key --window "$WIN_ID" 3
sleep 0.5

echo "Moving cursor in left pane..."
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Down
    sleep 0.15
done

echo "Switch to right pane..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.1
DISPLAY=:0 xdotool key --window "$WIN_ID" o
sleep 0.3

echo "Scroll in right pane..."
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+v
sleep 0.5

echo "Move cursor in right pane..."
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Down
    sleep 0.15
done
sleep 1
echo "Multi-window test done."

# Check logs for animation activity
echo ""
echo "=== Checking animation log entries ==="
grep -c "Starting crossfade" "$LOG" 2>/dev/null && echo " crossfade(s) detected" || echo "No crossfade detected"
grep -c "Starting scroll slide" "$LOG" 2>/dev/null && echo " scroll slide(s) detected" || echo "No scroll slide detected"
grep -c "cursor_anim" "$LOG" 2>/dev/null && echo " cursor animation entries" || echo "No cursor animation entries"

echo ""
echo "Waiting 5 seconds before cleanup (observe animations visually)..."
sleep 5

# Cleanup
echo "Stopping Romacs..."
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Test complete ==="
echo "Full log at: $LOG"

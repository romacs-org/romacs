#!/usr/bin/env bash
# Test vertico-posframe child frame rendering
# Usage: ./test/romacs/manual/run-vertico-posframe-test.sh
#
# Installs vertico + posframe + vertico-posframe from MELPA,
# then exercises completion UI with xdotool.

set -e

cd "$(dirname "$0")/../.."

LOG=/tmp/vertico-posframe-test.log

echo "=== Vertico-Posframe Child Frame Test ==="
echo "Starting Romacs (will install packages on first run)..."

RUST_LOG=romacs_display=debug DISPLAY=:0 ./target/release/romacs -Q \
    -l test/romacs/manual/vertico-posframe-test.el >"$LOG" 2>&1 &
EMACS_PID=$!

echo "Romacs PID: $EMACS_PID"
echo "Waiting for window + package install..."
sleep 10

# Find Romacs window
WIN_ID=$(DISPLAY=:0 xdotool search --class "romacs" 2>/dev/null | head -1)
if [ -z "$WIN_ID" ]; then
    echo "ERROR: Could not find Romacs window"
    kill $EMACS_PID 2>/dev/null || true
    exit 1
fi

echo "Found window: $WIN_ID"
DISPLAY=:0 xdotool windowactivate --sync "$WIN_ID"
sleep 1

# === Test 1: M-x command completion ===
echo ""
echo "=== Test 1: M-x (command completion via posframe) ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" alt+x
sleep 1
echo "Typing 'describe-f'..."
DISPLAY=:0 xdotool type --window "$WIN_ID" --delay 80 "describe-f"
sleep 1
echo "Posframe should show filtered completions."
sleep 2
# Cancel
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+g
sleep 0.5

# === Test 2: C-x b buffer switching ===
echo ""
echo "=== Test 2: C-x b (buffer switch via posframe) ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" b
sleep 1
echo "Posframe should show buffer list."
sleep 2
# Select test-text buffer
DISPLAY=:0 xdotool type --window "$WIN_ID" --delay 80 "test-text"
sleep 0.5
DISPLAY=:0 xdotool key --window "$WIN_ID" Return
sleep 1

# === Test 3: C-x C-f file open ===
echo ""
echo "=== Test 3: C-x C-f (find-file via posframe) ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+x
sleep 0.2
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+f
sleep 1
echo "Posframe should show file completions."
sleep 2
# Cancel
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+g
sleep 0.5

# === Test 4: C-h f describe function ===
echo ""
echo "=== Test 4: C-h f (describe-function via posframe) ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+h
sleep 0.3
DISPLAY=:0 xdotool key --window "$WIN_ID" f
sleep 1
echo "Typing 'message'..."
DISPLAY=:0 xdotool type --window "$WIN_ID" --delay 80 "message"
sleep 1
echo "Posframe should show matching functions."
sleep 2
# Cancel
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+g
sleep 0.5

# === Test 5: Scroll through completions ===
echo ""
echo "=== Test 5: Scroll through M-x completions ==="
DISPLAY=:0 xdotool key --window "$WIN_ID" alt+x
sleep 1
echo "Scrolling down through completions..."
for i in $(seq 1 15); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Down
    sleep 0.15
done
echo "Scrolling up..."
for i in $(seq 1 10); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" Up
    sleep 0.15
done
sleep 1
# Cancel
DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+g
sleep 0.5

# === Test 6: Rapid open/close ===
echo ""
echo "=== Test 6: Rapid open/close (posframe create/destroy) ==="
for i in $(seq 1 5); do
    DISPLAY=:0 xdotool key --window "$WIN_ID" alt+x
    sleep 0.4
    DISPLAY=:0 xdotool key --window "$WIN_ID" ctrl+g
    sleep 0.3
done
echo "Rapid open/close done. No ghost frames should remain."
sleep 1

# Check logs
echo ""
echo "=== Checking logs ==="
if [ -f "$LOG" ]; then
    CHILD_COUNT=$(grep -ci "child" "$LOG" 2>/dev/null || true)
    ERROR_COUNT=$(grep -ci "error\|panic\|crash" "$LOG" 2>/dev/null || true)
    echo "Child frame log entries: $CHILD_COUNT"
    echo "Error entries: $ERROR_COUNT"
    if [ "$ERROR_COUNT" -gt 0 ]; then
        echo "--- Errors found: ---"
        grep -i "error\|panic\|crash" "$LOG" | tail -10
    fi
fi

echo ""
echo "Waiting 3 seconds before shutdown..."
sleep 3

# Cleanup
echo "Stopping Romacs..."
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Vertico-posframe test complete ==="
echo "Full log at: $LOG"

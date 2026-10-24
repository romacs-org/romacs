#!/usr/bin/env bash
# Test buffer scrolling with inline webkit view
cd "$(dirname "$0")/../.."

echo "=== WebKit Buffer Scroll Test ==="

RUST_LOG=warn DISPLAY=:0 ./target/release/romacs -Q -l test/romacs/webkit-scroll-buffer-test.el 2>/tmp/webkit-buffer-scroll.log &
EMACS_PID=$!
echo "Romacs PID: $EMACS_PID"

sleep 8

WIN_ID=$(DISPLAY=:0 xdotool search --class "romacs" 2>/dev/null | head -1)
echo "Window ID: $WIN_ID"

if [ -z "$WIN_ID" ]; then
    echo "ERROR: Could not find Romacs window"
    kill $EMACS_PID 2>/dev/null || true
    exit 1
fi

# Take screenshot BEFORE scrolling
echo "Taking screenshot BEFORE scrolling..."
DISPLAY=:0 import -window "$WIN_ID" /tmp/webkit-scroll-before.png
echo "Saved: /tmp/webkit-scroll-before.png"

# Activate window and scroll down with C-n multiple times
echo "Scrolling down with C-n..."
DISPLAY=:0 xdotool windowactivate --sync "$WIN_ID"
sleep 0.5

# Press C-n (next-line) many times to scroll past the webkit view
for i in {1..25}; do
    DISPLAY=:0 xdotool key ctrl+n
    sleep 0.05
done

sleep 1

# Take screenshot AFTER scrolling
echo "Taking screenshot AFTER scrolling..."
DISPLAY=:0 import -window "$WIN_ID" /tmp/webkit-scroll-after.png
echo "Saved: /tmp/webkit-scroll-after.png"

# Scroll down more
echo "Scrolling down more..."
for i in {1..20}; do
    DISPLAY=:0 xdotool key ctrl+n
    sleep 0.05
done

sleep 1

# Take another screenshot
echo "Taking screenshot AFTER more scrolling..."
DISPLAY=:0 import -window "$WIN_ID" /tmp/webkit-scroll-after2.png
echo "Saved: /tmp/webkit-scroll-after2.png"

echo ""
echo "Screenshots saved. Check /tmp/webkit-scroll-*.png"

# Cleanup
kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo "=== Test complete ==="

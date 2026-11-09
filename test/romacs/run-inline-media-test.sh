#!/usr/bin/env bash
# Test inline image and video in one buffer
# Usage: ./test/romacs/run-inline-media-test.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROMACS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
EMACS_BIN="$ROMACS_ROOT/target/release/romacs"
TEST_EL="$SCRIPT_DIR/inline-media-test.el"
LOG_FILE="/tmp/inline-media-test-$$.log"
SCREENSHOT_FILE="/tmp/inline-media-screenshot-$$.png"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Romacs Inline Media Test ==="
echo "Testing: inline image + video in one buffer"
echo ""

# Check emacs binary
if [[ ! -x "$EMACS_BIN" ]]; then
    echo -e "${RED}ERROR: Romacs binary not found at $EMACS_BIN${NC}"
    echo "Run: cargo build --release -p romacs-vm-gui"
    exit 1
fi

# Check for test media files
echo "Media files:"
if [ -f ~/Pictures/559-4K.jpg ]; then
    echo -e "  ${GREEN}Image: ~/Pictures/559-4K.jpg${NC}"
elif [ -f test/data/image/black.jpg ]; then
    echo -e "  ${GREEN}Image: test/data/image/black.jpg${NC}"
else
    echo -e "  ${YELLOW}Image: [not found - will be skipped]${NC}"
fi

if [ -f ~/Videos/4k_f1.mp4 ]; then
    echo -e "  ${GREEN}Video: ~/Videos/4k_f1.mp4${NC}"
elif [ -f test/data/video/test.mp4 ]; then
    echo -e "  ${GREEN}Video: test/data/video/test.mp4${NC}"
else
    echo -e "  ${YELLOW}Video: [not found - will be skipped]${NC}"
fi

echo ""
echo "Log file: $LOG_FILE"
echo ""

# Run emacs with timeout
echo "Starting Romacs..."
export RUST_LOG=info
timeout 35 "$EMACS_BIN" -Q -l "$TEST_EL" 2>&1 | tee "$LOG_FILE" &
EMACS_PID=$!

# Wait for window to appear and content to load
sleep 8

# Take screenshot
if command -v import &> /dev/null && [[ -n "$DISPLAY" ]]; then
    echo "Taking screenshot..."
    WINDOW_ID=$(xdotool search --class "romacs" 2>/dev/null | head -1 || true)
    if [[ -n "$WINDOW_ID" ]]; then
        import -window "$WINDOW_ID" "$SCREENSHOT_FILE" 2>/dev/null || true
        if [[ -f "$SCREENSHOT_FILE" ]]; then
            echo -e "${GREEN}Screenshot saved: $SCREENSHOT_FILE${NC}"
        fi
    else
        echo -e "${YELLOW}Could not find Romacs window for screenshot${NC}"
    fi
fi

# Wait for emacs to finish
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Analyzing results ==="
echo ""

# Check results
if grep -q "Image: OK" "$LOG_FILE" 2>/dev/null; then
    echo -e "${GREEN}[PASS] Image loaded successfully${NC}"
else
    echo -e "${YELLOW}[SKIP] Image not loaded${NC}"
fi

if grep -q "Video: OK" "$LOG_FILE" 2>/dev/null; then
    echo -e "${GREEN}[PASS] Video loaded successfully${NC}"
    if grep -q "Frame #" "$LOG_FILE" 2>/dev/null; then
        echo -e "${GREEN}[PASS] Video frames being decoded${NC}"
    fi
else
    echo -e "${YELLOW}[SKIP] Video not loaded${NC}"
fi

# Check for errors
if grep -qi "error\|panic" "$LOG_FILE" 2>/dev/null; then
    echo ""
    echo -e "${YELLOW}[WARN] Some errors in log:${NC}"
    grep -i "error\|panic" "$LOG_FILE" | head -5
fi

echo ""
echo "=== Summary ==="
if grep -q "Result: 2/2" "$LOG_FILE" 2>/dev/null; then
    echo -e "${GREEN}Both media types loaded successfully!${NC}"
else
    RESULT=$(grep "Result:" "$LOG_FILE" 2>/dev/null | tail -1 || echo "Result unknown")
    echo "$RESULT"
fi

echo ""
echo "Log file: $LOG_FILE"
if [[ -f "$SCREENSHOT_FILE" ]]; then
    echo "Screenshot: $SCREENSHOT_FILE"
fi

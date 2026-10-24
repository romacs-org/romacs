#!/usr/bin/env bash
# Test all image types (PNG, JPEG, GIF, TIFF, WebP, SVG) via Rust GPU pipeline.
#
# Requires test images in ~/Pictures/:
#   test.png  test.jpg  test.gif  test.tiff  test.webp  test.svg
#
# Usage:
#   ./test/romacs/run-image-type-test.sh          # headless (Xvfb)
#   DISPLAY=:0 ./test/romacs/run-image-type-test.sh  # real display

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROMACS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
EMACS_BIN="$ROMACS_ROOT/target/release/romacs"
TEST_EL="$SCRIPT_DIR/image-type-test.el"
LOG_FILE="/tmp/romacs-image-type-test-$$.log"
SCREENSHOT_FILE="/tmp/romacs-image-type-test-$$.png"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== Romacs Image Type Test ==="
echo ""

if [[ ! -x "$EMACS_BIN" ]]; then
    echo -e "${RED}ERROR: Romacs binary not found at $EMACS_BIN${NC}"
    echo "Run: cargo xtask fresh-build --release"
    exit 1
fi

# Check test images
MISSING=()
for img in test.png test.jpg test.gif test.tiff test.webp test.svg; do
    if [[ ! -f "$HOME/Pictures/$img" ]]; then
        MISSING+=("$img")
    fi
done
if [[ ${#MISSING[@]} -gt 0 ]]; then
    echo -e "${RED}ERROR: Missing test images in ~/Pictures/:${NC}"
    printf '  %s\n' "${MISSING[@]}"
    exit 1
fi

export RUST_LOG=info

echo "Emacs: $EMACS_BIN"
echo "Test:  $TEST_EL"
echo "Log:   $LOG_FILE"
echo ""

"$EMACS_BIN" -Q -l "$TEST_EL" >"$LOG_FILE" 2>&1 &
EMACS_PID=$!
echo "Started emacs PID=$EMACS_PID"

sleep 14

if ! ps -p $EMACS_PID > /dev/null 2>&1; then
    echo -e "${RED}Emacs died!${NC}"
    echo "Log tail:"
    tail -20 "$LOG_FILE"
    exit 1
fi

# Screenshot if display available
if command -v import &>/dev/null && [[ -n "$DISPLAY" ]]; then
    echo "Taking screenshot..."
    import -window root "$SCREENSHOT_FILE" 2>/dev/null || true
    if [[ -f "$SCREENSHOT_FILE" ]]; then
        echo -e "${GREEN}Screenshot: $SCREENSHOT_FILE${NC}"
    fi
fi

kill $EMACS_PID 2>/dev/null || true
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Log Analysis ==="

PASS=0
FAIL=0

# Check image type registrations
for type in png jpeg gif tiff webp svg; do
    if grep -q "load_image_file_scaled\|load_image_data" "$LOG_FILE"; then
        ((PASS++))
    fi
done

# Check fontdb
if grep -q "SVG fontdb: loaded" "$LOG_FILE"; then
    FONTS=$(grep "SVG fontdb" "$LOG_FILE" | grep -oP '\d+ system font faces')
    echo -e "${GREEN}[PASS] SVG fontdb: $FONTS${NC}"
else
    echo -e "${YELLOW}[WARN] No SVG font loading detected${NC}"
fi

# Check for image loading activity
IMG_COUNT=$(grep -c "Loading image" "$LOG_FILE" 2>/dev/null || true)
echo -e "${GREEN}[INFO] $IMG_COUNT images loaded${NC}"

# Check for errors
if grep -qi "panic\|SIGSEGV\|abort" "$LOG_FILE" 2>/dev/null; then
    echo -e "${RED}[FAIL] Crash detected in log${NC}"
    grep -i "panic\|SIGSEGV\|abort" "$LOG_FILE" | head -5
    FAIL=1
fi

echo ""
if [[ $FAIL -eq 0 ]]; then
    echo -e "${GREEN}IMAGE TYPE TEST: PASSED${NC}"
else
    echo -e "${RED}IMAGE TYPE TEST: FAILED${NC}"
fi

echo ""
echo "Log: $LOG_FILE"
[[ -f "$SCREENSHOT_FILE" ]] && echo "Screenshot: $SCREENSHOT_FILE"
exit $FAIL

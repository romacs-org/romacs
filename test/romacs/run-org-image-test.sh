#!/usr/bin/env bash
# Test org-mode inline image preview in Romacs (Issue #40)
#
# Verifies that org-toggle-inline-images works without
# "Invalid image specification" errors.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROMACS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
EMACS_BIN="$ROMACS_ROOT/target/release/romacs"
TEST_EL="$SCRIPT_DIR/org-image-test.el"
LOG_FILE="/tmp/romacs-org-image-test.log"
SCREENSHOT_FILE="/tmp/romacs-org-image-test.png"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== Romacs Org Image Preview Test ==="
echo ""

if [[ ! -x "$EMACS_BIN" ]]; then
    echo -e "${RED}ERROR: Romacs binary not found at $EMACS_BIN${NC}"
    echo "Run: cargo xtask fresh-build --release"
    exit 1
fi

echo "Romacs binary: $EMACS_BIN"
echo "Test file: $TEST_EL"
echo ""

export EMACS_SOURCE_DIR="$ROMACS_ROOT"

# Clean up previous results
rm -f "$LOG_FILE" "$SCREENSHOT_FILE"

# Run emacs with the test
timeout 20 "$EMACS_BIN" -Q \
    --eval "(setq inhibit-startup-screen t)" \
    -l "$TEST_EL" \
    2>/dev/null &

EMACS_PID=$!

# Wait for rendering
sleep 6

# Take screenshot
if command -v import &> /dev/null && [[ -n "$DISPLAY" ]]; then
    WINDOW_ID=$(xdotool search --class "romacs" 2>/dev/null | head -1 || true)
    if [[ -n "$WINDOW_ID" ]]; then
        import -window "$WINDOW_ID" "$SCREENSHOT_FILE" 2>/dev/null || true
    fi
fi

# Wait for emacs to finish
wait $EMACS_PID 2>/dev/null || true

echo ""
echo "=== Test Results ==="
echo ""

if [[ ! -f "$LOG_FILE" ]]; then
    echo -e "${RED}FAIL: No log file produced${NC}"
    exit 1
fi

PASS_COUNT=$(grep -c "^PASS:" "$LOG_FILE" 2>/dev/null || true)
FAIL_COUNT=$(grep -c "^FAIL:" "$LOG_FILE" 2>/dev/null || true)

# Show all results
while IFS= read -r line; do
    if [[ "$line" == PASS:* ]]; then
        echo -e "${GREEN}$line${NC}"
    elif [[ "$line" == FAIL:* ]]; then
        echo -e "${RED}$line${NC}"
    else
        echo "$line"
    fi
done < "$LOG_FILE"

echo ""
echo "=== Summary ==="
echo -e "Passed: ${GREEN}$PASS_COUNT${NC}  Failed: ${RED}$FAIL_COUNT${NC}"

if [[ -f "$SCREENSHOT_FILE" ]]; then
    echo "Screenshot: $SCREENSHOT_FILE"
fi

if [[ "$FAIL_COUNT" -gt 0 ]]; then
    echo -e "${RED}TEST FAILED${NC}"
    exit 1
else
    echo -e "${GREEN}TEST PASSED${NC}"
    exit 0
fi

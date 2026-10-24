#!/usr/bin/env bash
# Interactive romacs-term test using xdotool
# Usage: ./test/romacs/run-romacs-term-interactive-test.sh
#
# Tests:
#   1. Terminal creation (window mode)
#   2. Keyboard input and text output
#   3. ANSI color rendering
#   4. Terminal resize
#   5. Floating terminal overlay
#   6. Cleanup and destruction
#
# Requires: xdotool and an accessible DISPLAY

set -e

cd "$(dirname "$0")/../.."

TEST_ROOT="$PWD/tmp/romacs-term-interactive"
MARKER_DIR="$TEST_ROOT/markers"
LOG="$TEST_ROOT/romacs-term-interactive.log"
RESULTS="$TEST_ROOT/romacs-term-interactive-results.txt"
SCREENSHOT_DIR="$TEST_ROOT/screenshots"
TEST_NAME="Romacs-Term Interactive"
EXPECTED_PASS_COUNT=15
EMACS_PID=""

# Cleanup from previous runs
mkdir -p "$MARKER_DIR" "$SCREENSHOT_DIR" "$TEST_ROOT/runtime"
rm -f "$MARKER_DIR"/phase{2,3,4,5,6}
rm -f "$RESULTS"
rm -f "$SCREENSHOT_DIR"/romacs-term-*.png
export TMPDIR="$TEST_ROOT/runtime"
export NEO_TERM_ITEST_RESULTS="$RESULTS"
export NEO_TERM_ITEST_MARKER_DIR="$MARKER_DIR"

# Setup LD_LIBRARY_PATH for nix-based systems
setup_lib_path() {
    local xcursor_so=""
    local xkb_so=""
    local libstdcpp_so=""
    xcursor_so=$(find /nix/store -maxdepth 3 -name 'libXcursor.so.1' 2>/dev/null | head -1)
    xkb_so=$(find /nix/store -maxdepth 3 -name 'libxkbcommon-x11.so' 2>/dev/null | head -1)
    if command -v g++ >/dev/null 2>&1; then
        libstdcpp_so=$(g++ -print-file-name=libstdc++.so.6)
        if [ ! -f "$libstdcpp_so" ]; then
            libstdcpp_so=""
        fi
    fi

    local extra_path=""
    if [ -n "$xcursor_so" ]; then
        extra_path="$(dirname "$xcursor_so")"
    fi
    if [ -n "$xkb_so" ]; then
        extra_path="${extra_path:+$extra_path:}$(dirname "$xkb_so")"
    fi
    if [ -n "$libstdcpp_so" ]; then
        extra_path="${extra_path:+$extra_path:}$(dirname "$libstdcpp_so")"
    fi

    if [ -n "$extra_path" ]; then
        export LD_LIBRARY_PATH="${extra_path}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        echo "LD_LIBRARY_PATH set: $LD_LIBRARY_PATH"
    fi
}

# Validate the existing display. X11's Xvfb transport uses system-global socket
# paths, so this repository-local test intentionally does not start one.
setup_display() {
    local test_display="${DISPLAY:-}"

    if [ -z "$test_display" ] || ! DISPLAY="$test_display" xdotool getactivewindow >/dev/null 2>&1; then
        echo "ERROR: An accessible DISPLAY is required"
        exit 1
    fi

    echo "Using existing DISPLAY=$test_display"
    export DISPLAY="$test_display"
}

require_gui_probe_tools() {
    local tool
    for tool in xdotool import magick; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "ERROR: $tool is required for renderer-observed romacs-term verification"
            exit 1
        fi
    done
}

# Cleanup function
cleanup() {
    echo ""
    echo "Cleaning up..."
    if [ -n "$EMACS_PID" ]; then
        kill "$EMACS_PID" 2>/dev/null || true
        wait "$EMACS_PID" 2>/dev/null || true
    fi
    rm -f "$MARKER_DIR"/phase{2,3,4,5,6}
}
trap cleanup EXIT

echo "=== $TEST_NAME Test ==="

setup_lib_path
setup_display
require_gui_probe_tools

echo "Starting Romacs with romacs-term interactive test..."

RUST_LOG=romacs_display=debug ./target/release/romacs -Q \
    -l test/romacs/romacs-term-interactive-test.el >"$LOG" 2>&1 &
EMACS_PID=$!

echo "Romacs PID: $EMACS_PID"

# Helper: wait for Emacs window
wait_for_window() {
    local attempts=0
    local max_attempts=30
    local previous_id=""
    local stable_observations=0
    while [ $attempts -lt $max_attempts ]; do
        if ! kill -0 $EMACS_PID 2>/dev/null; then
            echo "ERROR: Emacs process died"
            echo "--- Last 20 lines of log ---"
            tail -20 "$LOG" 2>/dev/null || true
            return 1
        fi
        local candidate=""
        candidate=$(xdotool search --onlyvisible --pid "$EMACS_PID" 2>/dev/null | head -1)
        if [ -n "$candidate" ] && xdotool getwindowgeometry "$candidate" >/dev/null 2>&1; then
            if [ "$candidate" = "$previous_id" ]; then
                stable_observations=$((stable_observations + 1))
            else
                previous_id="$candidate"
                stable_observations=1
            fi

            # Romacs replaces its bootstrap window when it adopts the first
            # Emacs frame. Require two observations of the same visible XID so
            # xdotool never targets the short-lived bootstrap window.
            if [ "$stable_observations" -ge 2 ]; then
                WIN_ID="$candidate"
                echo "Found stable Emacs window: $WIN_ID"
                return 0
            fi
        else
            previous_id=""
            stable_observations=0
        fi
        sleep 0.5
        attempts=$((attempts + 1))
    done
    echo "ERROR: Emacs window not found after ${max_attempts} attempts"
    echo "--- Last 20 lines of log ---"
    tail -20 "$LOG" 2>/dev/null || true
    return 1
}

# Helper: take a screenshot with label
take_screenshot() {
    local label=$1
    local file="$SCREENSHOT_DIR/romacs-term-${label}.png"
    if [ -z "$WIN_ID" ] || ! import -window "$WIN_ID" "$file" 2>/dev/null || [ ! -s "$file" ]; then
        echo "ERROR: Failed to capture required screenshot: $file"
        return 1
    fi
    echo "  Screenshot: $file"
    LAST_SCREENSHOT="$file"
}

assert_pixel_fraction() {
    local label=$1
    local file=$2
    local expression=$3
    local minimum=$4
    local fraction
    fraction=$(pixel_fraction "$file" "$expression")
    if ! awk -v value="$fraction" -v minimum="$minimum" \
        'BEGIN { exit !(value > minimum) }'; then
        echo "ERROR: $label pixel fraction $fraction did not exceed $minimum"
        return 1
    fi
    echo "  $label pixel fraction: $fraction"
}

pixel_fraction() {
    local file=$1
    local expression=$2
    magick "$file" -alpha off -colorspace sRGB \
        -fx "$expression" -format '%[fx:mean]' info:
}

assert_pixel_fraction_drop() {
    local label=$1
    local visible_file=$2
    local hidden_file=$3
    local expression=$4
    local minimum_drop=$5
    local visible_fraction hidden_fraction
    visible_fraction=$(pixel_fraction "$visible_file" "$expression")
    hidden_fraction=$(pixel_fraction "$hidden_file" "$expression")
    if ! awk -v visible="$visible_fraction" -v hidden="$hidden_fraction" \
        -v minimum="$minimum_drop" 'BEGIN { exit !((visible - hidden) > minimum) }'; then
        echo "ERROR: $label fraction did not fall enough ($visible_fraction -> $hidden_fraction)"
        return 1
    fi
    echo "  $label fraction: $visible_fraction -> $hidden_fraction"
}

# Helper: wait for results file to contain a marker
wait_for_marker() {
    local marker=$1
    local timeout=${2:-15}
    local elapsed=0
    while [ $elapsed -lt $timeout ]; do
        if ! kill -0 $EMACS_PID 2>/dev/null; then
            echo "  WARNING: Emacs process died while waiting for '$marker'"
            return 1
        fi
        if [ -f "$RESULTS" ] && grep -q "$marker" "$RESULTS" 2>/dev/null; then
            return 0
        fi
        sleep 0.5
        elapsed=$((elapsed + 1))
    done
    echo "  WARNING: Timed out waiting for '$marker' (${timeout}s)"
    return 1
}

# Helper: send keystrokes with delay
type_text() {
    local text=$1
    local delay=${2:-50}
    xdotool type --window "$WIN_ID" --delay "$delay" "$text"
}

send_key() {
    local key=$1
    xdotool key --window "$WIN_ID" "$key"
}

# Wait for window
echo ""
echo "--- Waiting for Emacs window ---"
if ! wait_for_window; then
    exit 1
fi

# Activate and focus the newly created Romacs window.
xdotool windowactivate --sync "$WIN_ID" 2>/dev/null || true
xdotool windowfocus --sync "$WIN_ID" 2>/dev/null || true
sleep 1

# Wait for terminal to be ready
echo ""
echo "--- Phase 1: Running M-x romacs-term ---"
send_key "alt+x"
type_text "romacs-term"
send_key "Return"
if ! wait_for_marker "READY_FOR_INPUT" 15; then
    echo "ERROR: Terminal not ready"
    cat "$RESULTS" 2>/dev/null || true
    exit 1
fi
echo "  Terminal is ready"
take_screenshot "01-initial"
sleep 2

# Phase 2: Type a command
echo ""
echo "--- Phase 2: Typing 'echo hello' ---"
type_text "echo hello"
sleep 0.5
send_key "Return"
sleep 2
touch "$MARKER_DIR/phase2"
wait_for_marker "Phase 2" 10
take_screenshot "02-echo-hello"
TERMINAL_SCREENSHOT="$LAST_SCREENSHOT"

# Window terminals belong to their Emacs buffer. A real buffer switch must
# remove the GPU surface, then switching back must preserve the PTY session.
send_key "alt+x"
type_text "switch-to-buffer"
send_key "Return"
type_text "*scratch*"
send_key "Return"
sleep 1
take_screenshot "02-window-terminal-hidden"
assert_pixel_fraction_drop "owned terminal dark surface" \
    "$TERMINAL_SCREENSHOT" "$LAST_SCREENSHOT" \
    'r<0.10&&g<0.10&&b<0.10?1:0' 0.25
send_key "alt+x"
type_text "switch-to-prev-buffer"
send_key "Return"
sleep 1

# Phase 3: ANSI color output
echo ""
echo "--- Phase 3: ANSI color test ---"
sleep 1
type_text "printf '\\033[31mRED\\033[32mGREEN\\033[34mBLUE\\033[0m COLOR_TEST\\n'"
sleep 0.5
send_key "Return"
sleep 2
touch "$MARKER_DIR/phase3"
wait_for_marker "Phase 3" 10
take_screenshot "03-ansi-colors"
assert_pixel_fraction "red terminal glyph" "$LAST_SCREENSHOT" \
    'r>0.45&&r>1.5*g&&r>1.5*b?1:0' 0.00003
assert_pixel_fraction "blue terminal glyph" "$LAST_SCREENSHOT" \
    'b>0.35&&b>1.3*r&&b>1.3*g?1:0' 0.00003

# Phase 4: Resize test (driven by Elisp)
echo ""
echo "--- Phase 4: Resize test ---"
sleep 1
touch "$MARKER_DIR/phase4"
wait_for_marker "Phase 4" 10
take_screenshot "04-after-resize"

# Phase 5: Floating terminal
echo ""
echo "--- Phase 5: Floating terminal ---"
sleep 1
touch "$MARKER_DIR/phase5"
wait_for_marker "FLOATING_RENDER_READY" 10
take_screenshot "05-floating"
assert_pixel_fraction "floating magenta surface" "$LAST_SCREENSHOT" \
    'r>0.35&&b>0.35&&g<0.75*r&&g<0.75*b?1:0' 0.0003

# Phase 6: Cleanup
echo ""
echo "--- Phase 6: Cleanup ---"
touch "$MARKER_DIR/phase6"
wait_for_marker "DONE" 10
take_screenshot "06-final"

sleep 1

# Read and display results
echo ""
echo "=== Test Results ==="
if [ -f "$RESULTS" ]; then
    cat "$RESULTS"
else
    echo "WARNING: Results file not found"
fi

# Check for panics in Rust log
echo ""
echo "=== Log Analysis ==="
PANIC_COUNT=0
ERROR_COUNT=0
if [ -f "$LOG" ]; then
    PANIC_COUNT=$(grep -ci "panic" "$LOG" 2>/dev/null | head -1 || true)
    PANIC_COUNT=${PANIC_COUNT:-0}
    # Filter out known non-fatal errors
    ERROR_COUNT=$(grep -ci "error" "$LOG" 2>/dev/null | head -1 || true)
    ERROR_COUNT=${ERROR_COUNT:-0}
    echo "PANIC entries: $PANIC_COUNT"
    echo "ERROR entries: $ERROR_COUNT (includes non-fatal warnings)"

    if [ "$PANIC_COUNT" -gt 0 ]; then
        echo ""
        echo "--- PANIC log entries ---"
        grep -i "panic" "$LOG" | tail -10
    fi
else
    echo "Log file not found"
fi

# Final summary
echo ""
echo "=== Summary ==="
if [ -f "$RESULTS" ]; then
    PASS_COUNT=$(grep -c "^PASS:" "$RESULTS" 2>/dev/null || true)
    FAIL_COUNT=$(grep -c "^FAIL:" "$RESULTS" 2>/dev/null || true)
    echo "Tests passed: $PASS_COUNT"
    echo "Tests failed: $FAIL_COUNT"
    echo "Panics: $PANIC_COUNT"

    if [ "$FAIL_COUNT" -gt 0 ]; then
        echo ""
        echo "Failed tests:"
        grep "^FAIL:" "$RESULTS"
    fi

    if [ "$PASS_COUNT" -ne "$EXPECTED_PASS_COUNT" ]; then
        echo "Expected exactly $EXPECTED_PASS_COUNT completed assertions"
    fi

    if [ "$FAIL_COUNT" -gt 0 ] || [ "$PANIC_COUNT" -gt 0 ] || \
       [ "$PASS_COUNT" -ne "$EXPECTED_PASS_COUNT" ]; then
        echo ""
        echo "RESULT: FAIL"
        exit 1
    else
        echo ""
        echo "RESULT: PASS"
    fi
else
    echo "RESULT: UNKNOWN (no results file)"
    exit 1
fi

echo ""
echo "Screenshots: $SCREENSHOT_DIR/"
echo "Full log: $LOG"
echo "Test results: $RESULTS"

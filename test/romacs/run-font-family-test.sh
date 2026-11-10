#!/usr/bin/env bash
# Run the font family spacing test
# Usage: ./test/romacs/run-font-family-test.sh
#
# This opens a buffer showing bold/italic text in many font families.
# Check that character spacing is identical for normal, bold, and italic.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EMACS="${EMACS:-./target/release/romacs}"
LOG="${LOG:-/tmp/font-family-test.log}"
TIMEOUT="${TIMEOUT:-8s}"

if [ ! -x "$EMACS" ]; then
    echo "ERROR: Romacs binary not found or not executable: $EMACS"
    echo "Run: cargo build --release -p romacs-vm-gui"
    exit 1
fi

rm -f "$LOG"

set +e
timeout "$TIMEOUT" env RUST_LOG="${RUST_LOG:-info}" "$EMACS" -Q -l "$SCRIPT_DIR/font-family-test.el" >"$LOG" 2>&1
STATUS=$?
set -e

if [ "$STATUS" -ne 0 ] && [ "$STATUS" -ne 124 ]; then
    echo "ERROR: font family test exited with status $STATUS"
    tail -40 "$LOG" || true
    exit "$STATUS"
fi

if grep -q " ERROR " "$LOG"; then
    echo "ERROR: renderer errors detected in font family test"
    grep -m 20 " ERROR " "$LOG"
    echo "Full log at: $LOG"
    exit 1
fi

echo "Font family test rendered without renderer ERROR records."
echo "Full log at: $LOG"

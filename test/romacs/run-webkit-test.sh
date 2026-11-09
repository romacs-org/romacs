#!/usr/bin/env bash
# Test script for Romacs inline WebKit rendering
#
# This tests that WPE WebKit views render inline in Emacs buffers
# using GPU acceleration.
#
# Usage: ./test/romacs/run-webkit-test.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EMACS_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$EMACS_DIR"

if [ ! -x "./target/release/romacs-vm-gui" ]; then
    echo "Error: ./target/release/romacs-vm-gui not found. Run: cargo build --release -p romacs-vm-gui."
    exit 1
fi

# Check for display
if [ -z "$DISPLAY" ]; then
    export DISPLAY=:0
fi

echo "=== Romacs Inline WebKit Test ==="
echo "Emacs: $EMACS_DIR/target/release/romacs-vm-gui"
echo "Display: $DISPLAY"
echo ""

# Check for bubblewrap - if not available, disable sandbox
if ! command -v bwrap &> /dev/null; then
    echo "Warning: bubblewrap (bwrap) not found - disabling WebKit sandbox"
    export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
fi

# Check for xdg-dbus-proxy
if ! command -v xdg-dbus-proxy &> /dev/null; then
    echo "Warning: xdg-dbus-proxy not found - disabling WebKit sandbox"
    export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
fi

# Run the test
exec ./target/release/romacs-vm-gui -Q -l test/romacs/romacs-webkit-test.el "$@"

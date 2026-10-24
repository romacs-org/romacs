#!/usr/bin/env bash
# Run bold overlap test with debug logging
# Usage: ./test/romacs/run-bold-overlap-test.sh
set -e
cd "$(git rev-parse --show-toplevel)"
RUST_LOG=debug exec ./target/release/romacs -Q -l test/romacs/bold-overlap-test.el

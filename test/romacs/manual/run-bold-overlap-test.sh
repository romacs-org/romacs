#!/usr/bin/env bash
# Run bold overlap test with debug logging
# Usage: ./test/romacs/manual/run-bold-overlap-test.sh
set -e
cd "$(git rev-parse --show-toplevel)"
RUST_LOG=debug exec ./target/release/romacs -Q -l test/romacs/manual/bold-overlap-test.el

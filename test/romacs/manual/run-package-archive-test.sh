#!/usr/bin/env bash
# Test HTTPS package archive access via rustls TLS
# Usage: ./test/romacs/manual/run-package-archive-test.sh
set -e
cd "$(git rev-parse --show-toplevel)"
TMPHOME="$(mktemp -d "${TMPDIR:-/tmp}/romacs-pkgtest.XXXXXX")"
export HOME="$TMPHOME"
cleanup() { rm -rf "$TMPHOME"; }
trap cleanup EXIT
exec ./target/release/romacs -Q -l test/romacs/manual/package-archive-test.el

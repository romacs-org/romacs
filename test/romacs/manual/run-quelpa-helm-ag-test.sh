#!/usr/bin/env bash
# Test use-package + quelpa installing helm-ag from GitHub over HTTPS
# Two-phase load: bootstrap quelpa-use-package first, then use-package :quelpa
# Usage: ./test/romacs/manual/run-quelpa-helm-ag-test.sh
set -e
cd "$(git rev-parse --show-toplevel)"
TMPHOME="$(mktemp -d "${TMPDIR:-/tmp}/romacs-quelpa-test.XXXXXX")"
export HOME="$TMPHOME"
cleanup() { rm -rf "$TMPHOME"; }
trap cleanup EXIT
exec ./target/release/romacs -Q -l test/romacs/manual/quelpa-helm-ag-bootstrap.el -l test/romacs/manual/quelpa-helm-ag-test.el

#!/usr/bin/env bash
# Build the recording oracle: GNU Emacs at the pin in parity-reference.toml,
# installed into a prefix named by that revision (R228).
#
#   tools/build-gnu-oracle.sh            # then: export ROMACS_GNU_ORACLE=$ROOT/$PIN/bin/emacs
#
# Builds in a DEDICATED clone ($ROOT/src) so the shared reference checkout's
# worktree is never touched. Configure mirrors the Arch /usr/bin/emacs the old
# corpora were recorded with (pgtk, cairo, harfbuzz, native-comp aot,
# tree-sitter, modules) minus distro paths and LTO/hardening CFLAGS, so a
# drift measurement against the old corpora varies the REVISION, not features.
# `emacs-repository-version` is frozen into the dump by loadup.el from the
# clone's git HEAD; tools/lib/gnu_oracle.py reads it back from the binary.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
PIN=$(sed -n 's/^mirror_commit = "\(.*\)"$/\1/p' "$repo/parity-reference.toml")
REF=${GNU_REF:-https://github.com/emacs-mirror/emacs.git}
ROOT=${GNU_ORACLE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/gnu-emacs-pin}
mkdir -p "$ROOT"
PREFIX=$ROOT/$PIN
[ ${#PIN} -eq 40 ] || { echo "no mirror_commit in parity-reference.toml" >&2; exit 1; }
[ -d "$ROOT/src" ] || git clone -q --no-checkout "$REF" "$ROOT/src"
cd "$ROOT/src"
git checkout -q --detach "$PIN"
test "$(git rev-parse HEAD)" = "$PIN"
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "$ROOT/src is modified" >&2; exit 1; }
[ -x configure ] || ./autogen.sh
[ -f Makefile ] || ./configure --prefix="$PREFIX" \
  --disable-build-details --with-cairo --with-harfbuzz --with-libsystemd \
  --with-modules --with-native-compilation=aot --with-pgtk --with-tree-sitter \
  CFLAGS='-O2 -pipe'
make -j"${JOBS:-10}"     # -j30 was OOM-killed (collect2) beside other lanes' builds
[ ! -d "$PREFIX" ] || chmod -R u+w "$PREFIX"
make install
chmod -R a-w "$PREFIX"
ROMACS_GNU_ORACLE=$PREFIX/bin/emacs python3 "$repo/tools/lib/gnu_oracle.py" check

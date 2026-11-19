#!/bin/sh
# Install the notices the licences of what this directory builds into
# Romacs's executables ask a binary distribution to carry, into DEST.
# tools/package-release and nix/package.nix both run it.
set -eu
here=$(dirname "$0")
dest=$1
install -D -m 0644 "$here/NOTICE" "$dest/NOTICE"
install -m 0644 "$here/LICENSE" "$dest/freetype-sys-LICENSE.txt"
install -m 0644 "$here/freetype2/docs/FTL.TXT" "$dest/FreeType-FTL.txt"
install -m 0644 "$here/freetype2/src/bdf/README" "$dest/FreeType-bdf-README.txt"
install -m 0644 "$here/freetype2/src/pcf/README" "$dest/FreeType-pcf-README.txt"
install -m 0644 "$here/libpng/LICENSE" "$dest/libpng-LICENSE.txt"

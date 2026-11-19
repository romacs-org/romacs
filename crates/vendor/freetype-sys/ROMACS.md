# freetype-sys 0.23.0, as Romacs carries it

This directory is `freetype-sys` from the eval-exec/freetype-sys fork at
a4eb0a7c (branch fix/bundled-zlib-include), which the workspace used as a
git dependency before, cut down to what its bundled build compiles and
changed as listed below. The workspace's `[patch.crates-io]` points
`freetype-sys` here; `exclude` keeps it out of the workspace.

## Why it exists

FreeType came from the host, and the host's upgrade from 2.14.3 to 2.15.0
re-hinted Liberation Mono: the GUI oracle (`crates/romacs-gui-oracle`)
built its glyph templates from it and failed 9 of 20 frozen Romacs
fixtures. Built from these sources, the metric probe, bitmap and colour
glyphs and the oracle rasterise alike on every machine, and a FreeType
upgrade is a deliberate change here that re-captures the oracle's fixtures.
Ordinary text is drawn by swash, not FreeType.

## What was kept

`src/` (the Rust bindings), `build.rs`, `LICENSE`; `freetype2/include`, the
`freetype2/src` modules `build.rs` compiles (their `.c` and `.h` files and
licence READMEs), `freetype2/LICENSE.TXT` and `freetype2/docs/FTL.TXT`;
libpng's library sources and headers, `libpng/arm`, `libpng/LICENSE` and
`libpng/scripts/pnglibconf.h.prebuilt`. The fork's libz-sys submodule, the
build systems, docs, tests and tools are left out.

## Changes

- `build.rs` always builds from source (the `bundled` feature, which
  freetype-rs forwards, no longer selects anything, and the pkg-config
  route is gone). It copies `pnglibconf.h` into `OUT_DIR` instead of the
  source tree, which a Nix build's store path does not let it write.
- FreeType and libpng are compiled with `-fvisibility=hidden`, and
  `FT_PUBLIC_FUNCTION_ATTRIBUTE` is defined empty. The host's libfontconfig
  and libharfbuzz load the host's libfreetype.so: an executable exporting
  our `FT_*` captured their calls (23 symbols in romacs-vm-gui, and
  libfreetype.so's own calls to them), two versions over one set of
  objects. Hidden, every executable's dynamic table holds none.
- `freetype2/include/freetype/config/public-macros.h`: the platform choice
  of `FT_PUBLIC_FUNCTION_ATTRIBUTE` is skipped when a build defines it,
  which the item above needs.
- `Cargo.toml`: the metadata and dependencies the build above uses.
- `src/lib.rs`: `extern crate libz_sys` no longer waits for the `bundled`
  feature, which nothing enables now; and the crate expects
  `unpredictable_function_pointer_comparisons`, which upstream's derives on
  callback-holding FFI structs raise. As a git dependency its warnings were
  capped; as a path dependency they printed in every build.
- `NOTICE` and `install-notices.sh` (ours): the notices the licences ask a
  binary distribution to carry, installed by `tools/package-release` and
  `nix/package.nix`.

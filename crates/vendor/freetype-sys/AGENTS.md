# crates/vendor/freetype-sys
The FreeType binding and the FreeType 2.13.2 and libpng sources Romacs
builds into its executables; `ROMACS.md` says what was taken and changed.

## Boundary
- Callers are freetype-rs's users: `crates/romacs-font` (the metric probe),
  `crates/romacs-font-materializer` (bitmap and colour glyphs) and
  `crates/romacs-gui-oracle` (glyph templates). Ordinary text goes through
  swash.

## Contracts
- No executable exports a FreeType or libpng symbol: the host's shared
  libraries keep the host's FreeType. `nm -D --defined-only BINARY` lists
  no `FT_` or `png_` name.
- The build writes only into `OUT_DIR`.

## Tests
- `cargo test --profile gate --locked --offline -p romacs-gui-oracle` holds
  the rasterisation to the oracle's frozen fixtures.
- `cargo test --profile gate --locked --offline -p romacs-font -p romacs-font-materializer`.

## Decisions
- 2026-11-19: FreeType is vendored and pinned, never the host's, so a host
  upgrade cannot change our text or our tests; an upgrade here re-captures
  the oracle's fixtures.

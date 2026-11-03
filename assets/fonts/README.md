# Bundled fonts

Fonts Romacs ships and registers at startup (`crates/romacs-vm-gui/src/fonts.rs`).

| Font | Holds | Source | Licence |
|---|---|---|---|
| `RomacsIcons.ttf` (Romacs Icons) | The mode line's icons, U+FE001 onward (`assets/icons/codepoints.txt`), each one text column wide | Glyphs from Symbols Nerd Font Mono 3.4.0 (Nerd Fonts), imported by `scripts/import-icon-glyphs.py` into `assets/icons/*.svg` (`assets/icons/sources.txt` names each glyph and its set) and built by `scripts/build-icon-font.py` | SIL OFL 1.1 as a Modified Version of Symbols Nerd Font (`OFL.txt`, Copyright (c) 2014 Ryan L McIntyre), and per glyph set: Material Design Icons (non-brand icons) Apache-2.0; Devicon 2.16.0 (konpa), Octicons (GitHub Inc.), Nerd Fonts' own glyphs MIT (`assets/icons/LICENSE`, `assets/icons/LICENSE-Apache-2.0.txt`) |

`scripts/check-derived` checks that `RomacsIcons.ttf` was built from the
SVGs as they are (`tools/icon_font.py`). Emoji are not bundled: they come
from the system's colour emoji font, which the display draws (COLR, SVG
and bitmap glyphs).

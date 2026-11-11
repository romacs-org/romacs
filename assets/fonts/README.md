# Bundled fonts

Fonts Romacs ships and registers at startup (`crates/romacs-vm-gui/src/fonts.rs`).

| Font | Holds | Source | Licence |
|---|---|---|---|
| `RomacsIcons.ttf` (Romacs Icons) | The mode line's icons, U+FE001 onward (`assets/icons/codepoints.txt`), each one text column wide | Glyphs from Symbols Nerd Font Mono 3.4.0 (Nerd Fonts), imported by `tools/import-icon-glyphs` into `assets/icons/*.svg` (`assets/icons/sources.txt` names each glyph and its set) and built by `tools/build-icon-font` | SIL OFL 1.1 as a Modified Version of Symbols Nerd Font (`OFL.txt`, Copyright (c) 2014 Ryan L McIntyre), and per glyph set: Material Design Icons (non-brand icons) Apache-2.0; Devicon 2.16.0 (konpa), Octicons (GitHub Inc.), Nerd Fonts' own glyphs MIT (`assets/icons/LICENSE`, `assets/icons/LICENSE-Apache-2.0.txt`) |
| `Iosevka-Regular.ttf` (Iosevka, regular) | The default face's font (`lisp/romacs/romacs-startup.el`), hinted, normal width | Iosevka 34.9.1, unchanged from `https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-Iosevka-34.9.1.zip` (SHA-256 `7a77a3dfa0e2d713a8920a30624863e23df2867a1f55a5e3d6d23dad07c14c65`); this file's SHA-256 `1ba24b81d82df65a68c9d0bb44bda306fdcdb5d7763afe89b0dea29ea827d1a2` | SIL OFL 1.1, Copyright (c) 2015-2026 Renzhi Li (`Iosevka-LICENSE.md`, from the v34.9.1 tag) |
| `Iosevka-Bold.ttf` (Iosevka, bold) | The default face's font (`lisp/romacs/romacs-startup.el`), hinted, normal width | Iosevka 34.9.1, unchanged from `https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-Iosevka-34.9.1.zip` (SHA-256 `7a77a3dfa0e2d713a8920a30624863e23df2867a1f55a5e3d6d23dad07c14c65`); this file's SHA-256 `1988973571c96725d240c97bd2131b5deecdedfc537ffd30e11aff6ec0286488` | SIL OFL 1.1, Copyright (c) 2015-2026 Renzhi Li (`Iosevka-LICENSE.md`, from the v34.9.1 tag) |
| `Iosevka-Italic.ttf` (Iosevka, italic) | The default face's font (`lisp/romacs/romacs-startup.el`), hinted, normal width | Iosevka 34.9.1, unchanged from `https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-Iosevka-34.9.1.zip` (SHA-256 `7a77a3dfa0e2d713a8920a30624863e23df2867a1f55a5e3d6d23dad07c14c65`); this file's SHA-256 `8bb76fdd0bda08193781e9f1703e4021d3afca6b48a3e7bbb61b699efe989115` | SIL OFL 1.1, Copyright (c) 2015-2026 Renzhi Li (`Iosevka-LICENSE.md`, from the v34.9.1 tag) |
| `Iosevka-BoldItalic.ttf` (Iosevka, bold italic) | The default face's font (`lisp/romacs/romacs-startup.el`), hinted, normal width | Iosevka 34.9.1, unchanged from `https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-Iosevka-34.9.1.zip` (SHA-256 `7a77a3dfa0e2d713a8920a30624863e23df2867a1f55a5e3d6d23dad07c14c65`); this file's SHA-256 `13579582b40296b7b79200c3d419d99de2c8b1ef051ec92c9fbc1df841991bec` | SIL OFL 1.1, Copyright (c) 2015-2026 Renzhi Li (`Iosevka-LICENSE.md`, from the v34.9.1 tag) |

`tools/check-derived` checks that `RomacsIcons.ttf` was built from the
SVGs as they are (`tools/lib/icon_font.py`), and that every other font here
is the upstream file `sources.json` records, by SHA-256
(`tools/lib/bundled_fonts.py`).

Iosevka is the default variant, not Term or Fixed: it draws `…`, the
arrows and a few other symbols two cells wide, which Romacs's own
truncation and layer code assumes. Upgrading it is deliberate and rare:
the four files are 47.7 MB, about 14 MB in Git's packs, and every
version stays in the history. Take the new release's `PkgTTF-Iosevka`
archive, check it against the release's `SHA-256.txt`, and update the
files, `Iosevka-LICENSE.md`, `sources.json` and the rows above together.

Romacs registers these files for itself only, as Fontconfig application
fonts (`crates/romacs-vm-gui/src/fonts.rs`), and installs nothing into
the user's or the system's font directories. Where the system has an
Iosevka of its own, the bundled face wins a tie with it. Emoji are not bundled: they come
from the system's colour emoji font, which the display draws (COLR, SVG
and bitmap glyphs).

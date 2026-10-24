# Romacs artwork

The source mark is
[`window-icon.svg`](../crates/romacs-display-runtime/assets/window-icon.svg).
Its red brush-shaped R and blue crescent use the
[Modus Vivendi palette](https://protesilaos.com/emacs/modus-themes-colors):
red `#ff5f59`, blue `#2fafff`, and black `#000000`.
The artwork consists of vector paths and needs no fonts or external images.

Run `python3 scripts/generate-logo-assets.py` from the repository to regenerate
the splash images, bundled application and document icons, banner, and social
preview. This requires `rsvg-convert` (librsvg) and `magick` (ImageMagick).
Historical `emacs*` asset names remain because Lisp packages reference them.
The splash retains its 333 × 233 native size.

Linux packages install the source SVG as `romacs.svg`; the window runtime
renders it with resvg. macOS packaging uses `cargo xtask render-window-icon`
to render the same source at each icon size before assembling its ICNS.
Windows installer, shortcut, and installed-program icons use the bundled ICO.

[`wordmark.svg`](wordmark.svg) contains outlined Iosevka ExtraBold letters.
The banner and social preview compose those outlines with the source mark.
The generated concept PNGs are design references, not shipping artwork.

# Romacs artwork

The source mark is
[`window-icon.svg`](../crates/romacs-display-runtime/assets/window-icon.svg).
Its red brush-shaped R and blue crescent use the
[Modus Vivendi palette](https://protesilaos.com/emacs/modus-themes-colors):
red `#ff5f59`, blue `#2fafff`, and black `#000000`.
The artwork consists of vector paths and needs no fonts or external images.

The splash images, `etc/images/icons/hicolor/scalable/apps/emacs.svg` (the
notification icon `notifications.el` reads), the banner and the social preview
are kept as files, edited by hand from the mark; nothing regenerates them.
Historical `emacs*` asset names remain because Lisp packages reference them.
The splash retains its 333 × 233 native size.

Linux packages install the source SVG as `romacs.svg`; the window runtime
renders it with resvg.

[`wordmark.svg`](wordmark.svg) contains outlined Iosevka ExtraBold letters.
The banner and social preview compose those outlines with the source mark.
The generated concept PNGs are design references, not shipping artwork.

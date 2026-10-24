# romacs-gui-oracle

The GUI acceptance oracle: asserts a **compositor capture** of a live frame
against a **declared scene**. Its inputs are the captured pixels, the
declaration, and the font file the declaration names. It never reads what a
renderer reports about its own output (ruling D-F3), so the renderer under
test cannot grade itself.

```
romacs-gui-oracle check SPEC.json CAPTURE.png   # spec mode; exit 0 pass, 1 fail, 2 error
romacs-gui-oracle pair  SPEC.json A.png B.png   # pair mode: GNU capture vs romacs capture
romacs-gui-oracle read  SPEC.json CAPTURE.png   # the read-back grid, as JSON
romacs-gui-oracle equiv SPEC.json               # characters the font draws alike
```

Captures come from `harness/capture.py`: a private headless labwc, a
throwaway HOME, the editor (`--editor gnu` or `--editor romacs`) building the
scene from the spec's `buffer` half, and `grim`. It never touches the
ambient session.

## What is asserted, and how

| kind | from pixels | tolerance |
|---|---|---|
| Text | each cell's glyph is classified over printable ASCII + Greek + the declared characters, against FreeType templates (3 hinting modes x regular/bold/italic). Neighbours' overhang is masked first. | exact, up to characters the font draws identically |
| Colour | cell background = most frequent colour; foreground = per-channel third-most-extreme value under the matched template | bg 2, fg 8 levels per channel |
| Style | weight from ink mass against the regular and bold templates; slant from shape; underline = a full-width ink row at or below the baseline | exact, except glyphs drawn alike in both styles |
| Geometry | best template placement searched over ±2 px; every glyph must sit at offset (0, 0) of its declared cell | 0 px (measured for Liberation Mono 20 px; revalidate for another font) |
| Cursor | the declared cell has the cursor colour as background, and no other cell does | exact |
| Wrapping | via Text: each declared row is the exact text that row shows | exact |
| Stray | every pixel outside the text grid and fringes is the frame background | 2 levels |
| Fringe | fringe background, and bitmap ink exactly on the declared rows | 2 / 8 levels |

The tolerances were set from measurement, not chosen: over 20 scenes from
each of GNU 32 PGTK and romacs (2,045 inked cells each), foreground error
was at most 5 (GNU) and 4 (romacs), background error 0, and every glyph
offset (0, 0). See `PERTURBATIONS.md` for what each tolerance still catches.

## What a capture cannot tell you

- **Homoglyphs.** Liberation Mono draws 15 Latin/Greek pairs identically
  (A/Α B/Β E/Ε H/Η I/Ι K/Κ M/Μ N/Ν O/Ο P/Ρ T/Τ X/Χ Y/Υ Z/Ζ o/ο). Text is
  exact up to these classes, and every use of one is listed in the report.
  `Ωμέγα Hello` contains two (`H`, `o`).
- **Slant of glyphs the italic face draws upright** (Liberation Mono `=`,
  `+`).
- **Which font.** A different monospace font with similar shapes reads
  mostly right; it is caught by geometry (cell metrics, glyph placement), not
  by font identity.
- **Renderer-internal facts**: buffer versions, frame numbers, face-cache or
  font-resolution counters. These can only come from the renderer, so they
  are not oracle assertions.
- **A lone underline vs `_`**: an underlined blank and an underscore can be
  the same pixels; the reader prefers `_` only when the underscore template
  explains the line at its own position.

## Scope

One window, one monospace font at one pixel size, cells one column wide, no
bidi, no images, mode line off. Pair mode compares the same extracted
structure from two captures; it is not yet a driver for keystroke scenarios.

## One render target, including visible changes from today's romacs

We collapse for graphical truecolour Wayland: `romacs`, `graphic`, `pgtk`
and `gtk` match; tty/greyscale/monochrome branches are discarded and
`supports` is always true. Background is the remaining light/dark axis.
`gtk` is an intentional compatibility selector, not a claim that this
renderer is GTK or that its Lisp `featurep` says so.

S98's independent **real romacs GUI** probe found window-system `romacs`,
display-graphic-p true, featurep gtk nil. Today's faces.el matches `graphic`
but not `(type x w32 ns haiku pgtk android)` there. Wiring this crate will
select the GUI branch for nine corpus specs. `custom-button` currently gets
**no attributes at all**, not a tty fallback; it gains its GUI attributes.
A running GNU pgtk editor is not affected by today's romacs bug.
Our answer is deliberately graphical, not preservation of that blank face.

Tests: `collapse.rs::specs_written_for_pgtk_match_romacs` and
`readiness.rs::romacs_matches_graphical_specs` (including `graphic`).

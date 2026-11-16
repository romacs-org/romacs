## VM2-WIDTH1 — String widths and automatic compositions

`char-width` and `string-width` follow character.c's `char_width` over
`char-width-table`, `tab-width`, `ctl-arrow` and the buffer or standard
display table. `string-width` applies the automatic compositions GNU counts
as emoji: a run a `compose-gstring-and-emoji` rule of
`composition-function-table` matches (VS-16, ZWJ, modifier and flag
sequences) is two columns, compose-gstring-and-emoji-for-terminal's width,
as pinned GNU batch counts them; other compositions keep their characters'
widths, as compose-gstring-for-terminal leaves them. A GUI frame's GNU
measures a composition by its font's pixels, which Romacs does not.
Text with a `composition` property signals an explicit not-implemented
error, since VM2 has no composition table.

## VM2-WIDTH1 — String widths without compositions

`char-width` and `string-width` follow character.c's `char_width` over
`char-width-table`, `tab-width`, `ctl-arrow` and the buffer or standard
display table. `string-width` does not apply automatic compositions from
`composition-function-table`: each character is measured on its own, so a
sequence GNU composes is counted per character. In pinned GNU batch that
differs for a character followed by an emoji modifier or variation
selector: 👍🏽 and ❤️ are 2 columns in GNU, 4 and 1 here. Text with a `composition` property signals an
explicit not-implemented error, since VM2 has no composition table.

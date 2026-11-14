//! Markdown fontifying and structure (lisp/romacs/romacs-markdown.el): a
//! region read as one document by `crate::markdown`.
use super::Value;

// (romacs-markdown--fontify BEG END FACES &optional FROM): BEG..END of the
// current buffer fontified as one Markdown document, each kind's face from
// FACES, an alist of kind symbols (`markup', `heading-1' .. `heading-6',
// `emphasis', `strong', `strike', `code', `code-block', `language', `link',
// `url', `image', `list-marker', `task', `quote', `table', `table-header',
// `rule', `html') to faces. With FROM, only the blocks from the one before
// FROM's are written again: the text before FROM has not changed since the
// last call. Runs inside `with-silent-modifications', else signals. Returns
// (START . CODES): where the writes start, and the code blocks written,
// each (LANGUAGE BODY-START BODY-END), LANGUAGE a string or nil.
defsubr! { name = "romacs-markdown--fontify", gnu = "romacs:markdown", effect = Global,
fn markdown_fontify(cx, beg: Value, end: Value, faces: Value; optional from: Value) {
    cx.markdown_fontify(beg, end, faces, from)
}}

// (romacs-markdown--blocks BEG END): the blocks of BEG..END of the current
// buffer, one Markdown document: each (KIND START END . PLIST) in document
// order, nested ones after their container. KIND is `heading', `code',
// `table', `item', `quote', `rule' or `html'; PLIST has `:depth', and
// `:level' (heading), `:language' and `:body' (code), `:ordered' (item).
defsubr! { name = "romacs-markdown--blocks", gnu = "romacs:markdown", effect = Reads,
fn markdown_blocks(cx, beg: Value, end: Value) {
    cx.markdown_blocks(beg, end)
}}


// (romacs-markdown--table-grid SOURCE WIDTH FACES): SOURCE, a pipe table,
// drawn as a grid at most WIDTH columns wide: columns share the width by
// what they hold, cells wrap at spaces, and a table too wide for its
// narrowest columns is drawn as panels of columns, each with its header.
// Its kinds' faces come from FACES, as `romacs-markdown--fontify' takes
// them. Nil when SOURCE holds no table.
defsubr! { name = "romacs-markdown--table-grid", gnu = "romacs:markdown", effect = Reads,
fn markdown_table_grid(cx, source: Value, width: Value, faces: Value) {
    cx.markdown_table_grid(source, width, faces)
}}

// (romacs-markdown--fontify-buffer BEG END FACES): the blocks of the current
// buffer, a Markdown document, that hold BEG..END fontified as
// `romacs-markdown--fontify' does a region. The buffer's block index finds
// them and follows its edits: a change is reparsed by the blocks it
// touched, and text it changed outside them loses its `fontified'. Runs
// inside `with-silent-modifications'. Returns (START END . CODES), the
// blocks written and their code blocks, each (LANGUAGE BODY-START BODY-END).
defsubr! { name = "romacs-markdown--fontify-buffer", gnu = "romacs:markdown", effect = Global,
fn markdown_fontify_buffer(cx, beg: Value, end: Value, faces: Value) {
    cx.markdown_fontify_buffer(beg, end, faces)
}}

// (romacs-markdown--block-at POS): the top-level block of the current
// buffer, a Markdown document, holding POS, as (START . END), found by the
// buffer's block index.
defsubr! { name = "romacs-markdown--block-at", gnu = "romacs:markdown", effect = Reads,
fn markdown_block_at(cx, pos: Value) {
    cx.markdown_block_at(pos)
}}

// (romacs-markdown--links BEG END): the links of BEG..END of the current
// buffer, one Markdown document, each (START END . DESTINATION); the
// destination is empty for a reference defined outside the region.
defsubr! { name = "romacs-markdown--links", gnu = "romacs:markdown", effect = Reads,
fn markdown_links(cx, beg: Value, end: Value) {
    cx.markdown_links(beg, end)
}}

// (romacs-markdown--table-align SOURCE): SOURCE, a pipe table, its columns
// padded to their widest cell on the side its delimiter row names; nil when
// SOURCE holds no table.
defsubr! { name = "romacs-markdown--table-align", gnu = "romacs:markdown", effect = Reads,
fn markdown_table_align(cx, source: Value) {
    cx.markdown_table_align(source)
}}

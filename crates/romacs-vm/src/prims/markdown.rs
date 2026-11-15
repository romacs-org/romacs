//! Markdown fontifying and structure (lisp/textmodes/romacs-markdown.el): a
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
// the position the writes start at.
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

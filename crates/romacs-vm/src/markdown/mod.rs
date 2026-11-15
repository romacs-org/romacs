//! Markdown: `CommonMark` with GFM tables, strikethrough and task lists,
//! parsed by pulldown-cmark into what fontifying and editing need. Pure
//! functions over a `&str`, every position a byte offset into it.
//!
//! - [`spans`]: each element's extent by [`Kind`], with its markup
//!   (delimiters, fences, markers) as spans of its own;
//! - [`runs`]: those spans flattened into runs, each with its kinds
//!   innermost first, as a face list wants them;
//! - [`blocks`]: the block structure, for outline, navigation and
//!   editing;
//! - [`restart`]: where a document's last blocks begin, so a text that
//!   grows at its end is refaced from there;
//! - [`grid`]: a pipe table drawn as a grid within a width.
//!
//! The buffer side (reading a region, writing faces) is
//! `crate::prim::leaf::markdown`; the kinds' faces are Lisp's
//! (`romacs-markdown-faces`, lisp/textmodes/romacs-markdown.el).
mod blocks;
mod runs;
mod spans;
mod table;

pub(crate) use blocks::{blocks, BlockKind};
pub(crate) use runs::runs;
pub(crate) use spans::{spans, Span};
pub(crate) use table::grid;

/// What a stretch of Markdown is, for its face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Kind {
    /// Delimiters, fences, markers and table pipes.
    Markup,
    /// An ATX or setext heading of level 1 to 6.
    Heading(u8),
    Emphasis,
    Strong,
    Strike,
    /// An inline code span.
    Code,
    /// A fenced or indented code block, fences included.
    CodeBlock,
    /// A fenced code block's info string.
    Language,
    /// A link's text, or an autolink.
    Link,
    /// A link's destination part, or a link reference definition.
    Url,
    Image,
    ListMarker,
    /// A task list item's `[ ]` or `[x]`.
    Task,
    Quote,
    Table,
    TableHeader,
    /// A thematic break.
    Rule,
    /// An HTML block or inline HTML.
    Html,
}

impl Kind {
    /// Every kind, in the order of their names.
    pub(crate) const ALL: [Kind; 23] = [
        Kind::Markup,
        Kind::Heading(1),
        Kind::Heading(2),
        Kind::Heading(3),
        Kind::Heading(4),
        Kind::Heading(5),
        Kind::Heading(6),
        Kind::Emphasis,
        Kind::Strong,
        Kind::Strike,
        Kind::Code,
        Kind::CodeBlock,
        Kind::Language,
        Kind::Link,
        Kind::Url,
        Kind::Image,
        Kind::ListMarker,
        Kind::Task,
        Kind::Quote,
        Kind::Table,
        Kind::TableHeader,
        Kind::Rule,
        Kind::Html,
    ];

    /// The symbol Lisp names this kind by.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Markup => "markup",
            Kind::Heading(1) => "heading-1",
            Kind::Heading(2) => "heading-2",
            Kind::Heading(3) => "heading-3",
            Kind::Heading(4) => "heading-4",
            Kind::Heading(5) => "heading-5",
            Kind::Heading(_) => "heading-6",
            Kind::Emphasis => "emphasis",
            Kind::Strong => "strong",
            Kind::Strike => "strike",
            Kind::Code => "code",
            Kind::CodeBlock => "code-block",
            Kind::Language => "language",
            Kind::Link => "link",
            Kind::Url => "url",
            Kind::Image => "image",
            Kind::ListMarker => "list-marker",
            Kind::Task => "task",
            Kind::Quote => "quote",
            Kind::Table => "table",
            Kind::TableHeader => "table-header",
            Kind::Rule => "rule",
            Kind::Html => "html",
        }
    }

    /// This kind's index in [`Kind::ALL`].
    pub(crate) fn index(self) -> usize {
        match self {
            Kind::Heading(n) => usize::from(n.clamp(1, 6)),
            kind => Kind::ALL.iter().position(|&k| k == kind).unwrap_or(0),
        }
    }
}

/// The byte offset at which TEXT's top-level blocks may be redone when
/// it has changed only from FROM on: the start of the block before the
/// one holding FROM, since text arriving can join a block to the one
/// before it (a list continued, a setext underline). STARTS are the
/// top-level blocks' line starts, ascending ([`Parsed::tops`]).
pub(crate) fn restart(starts: &[usize], from: usize) -> usize {
    let holding = starts.partition_point(|&s| s <= from);
    starts.get(holding.saturating_sub(2)).copied().unwrap_or(0).min(from)
}

#[cfg(test)]
mod markdown_tests;

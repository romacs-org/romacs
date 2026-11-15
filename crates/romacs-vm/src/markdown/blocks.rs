//! A document's block structure: what outline, navigation, table and list
//! commands, and agents, ask of it.
use super::spans::{heading_level, line_end, options};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BlockKind {
    /// An ATX or setext heading of this level.
    Heading(u8),
    /// A code block, with its fence's language (None when indented or
    /// unnamed) and its text's range.
    Code { language: Option<String>, body: Range<usize> },
    Table,
    /// A list item, ordered or not.
    Item { ordered: bool },
    Quote,
    Rule,
    Html,
}

/// A block of KIND over RANGE (bytes), DEPTH its nesting in containers
/// (lists, quotes): 0 at the top level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    pub kind: BlockKind,
    pub range: Range<usize>,
    pub depth: u32,
}

/// TEXT's blocks in document order, nested ones after their container.
pub(crate) fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    // The open elements: the index of their block in OUT, if they are one,
    // and, for a list, whether it is ordered.
    let mut open: Vec<(Option<usize>, bool)> = Vec::new();
    let mut depth = 0u32;
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let ordered = matches!(tag, Tag::List(Some(_)));
                let in_ordered = open.last().is_some_and(|&(_, o)| o);
                let kind = match &tag {
                    Tag::Heading { level, .. } => Some(BlockKind::Heading(heading_level(*level))),
                    Tag::CodeBlock(kind) => Some(code(text, kind, &range)),
                    Tag::Table(_) => Some(BlockKind::Table),
                    Tag::Item => Some(BlockKind::Item { ordered: in_ordered }),
                    Tag::BlockQuote(_) => Some(BlockKind::Quote),
                    Tag::HtmlBlock => Some(BlockKind::Html),
                    _ => None,
                };
                let index = kind.map(|kind| {
                    out.push(Block { kind, range: range.clone(), depth });
                    out.len() - 1
                });
                if matches!(tag, Tag::Item | Tag::BlockQuote(_)) {
                    depth += 1;
                }
                open.push((index, ordered));
            }
            Event::End(_) => {
                if let Some((Some(index), _)) = open.pop()
                    && matches!(out[index].kind, BlockKind::Item { .. } | BlockKind::Quote)
                {
                    depth = depth.saturating_sub(1);
                }
            }
            Event::Rule => out.push(Block { kind: BlockKind::Rule, range, depth }),
            Event::Text(_) => {
                // A code block's text: its body runs from its first text to its last.
                if let Some(&(Some(index), _)) = open.last()
                    && let BlockKind::Code { body, .. } = &mut out[index].kind
                {
                    if body.start == body.end {
                        *body = range.clone();
                    }
                    body.end = body.end.max(range.end);
                    body.start = body.start.min(range.start);
                }
            }
            _ => {}
        }
    }
    out
}

/// A code block opening at RANGE: its language, and an empty body at its
/// text's first possible place until its text arrives.
fn code(text: &str, kind: &CodeBlockKind<'_>, range: &Range<usize>) -> BlockKind {
    match kind {
        CodeBlockKind::Fenced(info) => {
            let language = info.split_whitespace().next().filter(|l| !l.is_empty()).map(str::to_owned);
            let first = line_end(text, range.start).min(range.end);
            BlockKind::Code { language, body: first..first }
        }
        CodeBlockKind::Indented => BlockKind::Code { language: None, body: range.start..range.start },
    }
}

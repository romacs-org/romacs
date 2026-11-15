//! pulldown-cmark's events as spans by [`Kind`].
use super::Kind;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};
use std::ops::Range;

/// KIND over START..END, DEPTH its element's nesting: where spans
/// overlap, the deeper one's face comes first. Markup is deepest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub depth: u32,
}

/// A document's spans, and the line starts of its top-level blocks
/// (ascending), from which [`super::restart`] picks.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub spans: Vec<Span>,
    pub tops: Vec<usize>,
}

/// Markup's depth: above every element's.
const MARKUP: u32 = u32::MAX;

/// The dialect: `CommonMark`, with GFM's tables, strikethrough, task lists
/// and quote alerts.
pub(crate) fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM
}

/// How an element's markup is found when it closes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// Its markup is what its children leave of its range at either end:
    /// `#`s, `*`s, `[`, `](url)`. A link's or image's trailing part is
    /// its destination.
    Around(Kind),
    /// A fenced code block: fence lines around its text.
    Fenced,
    /// A table: pipes outside its cells, and its delimiter row.
    Table,
    /// A block quote, its markers found as it opens.
    Quote,
    /// Nothing more at its close.
    Plain,
}

/// An element open while its events arrive.
struct Open {
    frame: Frame,
    range: Range<usize>,
    /// The first child's start and the last child's end, once there is one.
    children: Option<(usize, usize)>,
    /// A table's cells, and where its header row ends.
    cells: Vec<Range<usize>>,
    head_end: Option<usize>,
    first_row: Option<usize>,
}

/// TEXT's spans and top-level block starts.
pub(crate) fn spans(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut open: Vec<Open> = Vec::new();
    let mut parser = Parser::new_ext(text, options()).into_offset_iter();
    for (event, range) in parser.by_ref() {
        let depth = u32::try_from(open.len()).unwrap_or(u32::MAX - 1);
        match event {
            Event::Start(tag) => {
                note_child(&mut open, &range);
                if depth == 0 {
                    out.tops.push(line_start(text, range.start));
                }
                let frame = start(text, &tag, &range, depth, &mut open, &mut out.spans);
                open.push(Open { frame, range, children: None, cells: Vec::new(), head_end: None, first_row: None });
            }
            Event::End(_) => {
                if let Some(element) = open.pop() {
                    close(text, &element, &mut out.spans);
                }
            }
            Event::Code(_) => {
                note_child(&mut open, &range);
                out.spans.push(Span { start: range.start, end: range.end, kind: Kind::Code, depth });
                let ticks = text[range.clone()].bytes().take_while(|&b| b == b'`').count();
                if range.len() >= 2 * ticks {
                    markup(&mut out.spans, range.start, range.start + ticks);
                    markup(&mut out.spans, range.end - ticks, range.end);
                }
            }
            Event::InlineHtml(_) => {
                note_child(&mut open, &range);
                out.spans.push(Span { start: range.start, end: range.end, kind: Kind::Html, depth });
            }
            Event::TaskListMarker(_) => {
                note_child(&mut open, &range);
                out.spans.push(Span { start: range.start, end: range.end, kind: Kind::Task, depth });
            }
            Event::Rule => {
                if depth == 0 {
                    out.tops.push(line_start(text, range.start));
                }
                out.spans.push(Span { start: range.start, end: range.end, kind: Kind::Rule, depth });
            }
            _ => note_child(&mut open, &range),
        }
    }
    for (_, definition) in parser.reference_definitions().iter() {
        let span = definition.span.clone();
        out.spans.push(Span { start: span.start, end: span.end, kind: Kind::Url, depth: 0 });
    }
    out.tops.dedup();
    out
}

/// The spans an element opening at RANGE adds at once, and how its markup
/// is found when it closes.
fn start(text: &str, tag: &Tag<'_>, range: &Range<usize>, depth: u32, open: &mut [Open], spans: &mut Vec<Span>) -> Frame {
    let element = |kind| Span { start: range.start, end: range.end, kind, depth };
    match tag {
        Tag::Heading { level, .. } => {
            let kind = Kind::Heading(heading_level(*level));
            spans.push(element(kind));
            Frame::Around(kind)
        }
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. } => {
            let kind = match tag {
                Tag::Emphasis => Kind::Emphasis,
                Tag::Strong => Kind::Strong,
                Tag::Strikethrough => Kind::Strike,
                Tag::Link { .. } => Kind::Link,
                _ => Kind::Image,
            };
            spans.push(element(kind));
            Frame::Around(kind)
        }
        Tag::CodeBlock(kind) => {
            spans.push(element(Kind::CodeBlock));
            if matches!(kind, CodeBlockKind::Fenced(_)) { Frame::Fenced } else { Frame::Plain }
        }
        Tag::BlockQuote(_) => {
            spans.push(element(Kind::Quote));
            let nesting = 1 + open.iter().filter(|o| o.frame == Frame::Quote).count();
            quote_markers(text, range, nesting, spans);
            Frame::Quote
        }
        Tag::Item => {
            let marker = list_marker(&text[range.clone()]);
            if marker > 0 {
                spans.push(Span { start: range.start, end: range.start + marker, kind: Kind::ListMarker, depth });
            }
            Frame::Plain
        }
        Tag::Table(_) => {
            spans.push(element(Kind::Table));
            Frame::Table
        }
        Tag::TableHead => {
            spans.push(element(Kind::TableHeader));
            if let Some(table) = open_table(open) {
                table.head_end = Some(range.end);
            }
            Frame::Plain
        }
        Tag::TableRow => {
            if let Some(table) = open_table(open) {
                table.first_row.get_or_insert(range.start);
            }
            Frame::Plain
        }
        Tag::TableCell => {
            if let Some(table) = open_table(open) {
                table.cells.push(range.clone());
            }
            Frame::Plain
        }
        Tag::HtmlBlock => {
            spans.push(element(Kind::Html));
            Frame::Plain
        }
        _ => Frame::Plain,
    }
}

/// The innermost table open.
fn open_table(open: &mut [Open]) -> Option<&mut Open> {
    open.iter_mut().rev().find(|o| o.frame == Frame::Table)
}

/// The markup an element adds as it closes.
fn close(text: &str, element: &Open, spans: &mut Vec<Span>) {
    let Range { start, end } = element.range.clone();
    match element.frame {
        Frame::Around(kind) => {
            let trailing = if matches!(kind, Kind::Link | Kind::Image) { Kind::Url } else { Kind::Markup };
            match element.children {
                Some((first, last)) => {
                    markup(spans, start, first);
                    if trailing == Kind::Url && end - last > 1 {
                        spans.push(Span { start: last, end, kind: Kind::Url, depth: MARKUP });
                    } else {
                        markup(spans, last, end);
                    }
                }
                None if trailing == Kind::Url => spans.push(Span { start, end, kind: Kind::Url, depth: MARKUP }),
                None => markup(spans, start, end),
            }
        }
        Frame::Fenced => {
            let first_line = line_end(text, start).min(end);
            let (body_start, body_end) = element.children.unwrap_or((first_line, first_line));
            // The opening fence is markup but for its language.
            let fence_end = body_start.max(first_line);
            match fence_info(&text[start..first_line]) {
                Some(info) => {
                    markup(spans, start, start + info.start);
                    spans.push(Span { start: start + info.start, end: start + info.end, kind: Kind::Language, depth: MARKUP });
                    markup(spans, start + info.end, fence_end);
                }
                None => markup(spans, start, fence_end),
            }
            markup(spans, body_end.max(body_start), end);
        }
        Frame::Table => {
            let inside = |at: usize| element.cells.iter().any(|cell| cell.contains(&at));
            for (at, _) in text[start..end].match_indices('|') {
                if !inside(start + at) {
                    markup(spans, start + at, start + at + 1);
                }
            }
            if let Some(head_end) = element.head_end {
                let rows = element.first_row.unwrap_or(end);
                markup(spans, head_end, rows);
            }
        }
        Frame::Quote | Frame::Plain => {}
    }
}

/// A child element or text at RANGE: its parent's markup ends there.
fn note_child(open: &mut [Open], range: &Range<usize>) {
    if let Some(parent) = open.last_mut() {
        let (first, last) = parent.children.unwrap_or((range.start, range.end));
        parent.children = Some((first.min(range.start), last.max(range.end)));
    }
}

fn markup(spans: &mut Vec<Span>, start: usize, end: usize) {
    if start < end {
        spans.push(Span { start, end, kind: Kind::Markup, depth: MARKUP });
    }
}

/// The `>` markers of a block quote at RANGE, NESTING quotes deep (itself
/// counted): its own on its first line, which RANGE starts at, then on
/// each later line the NESTING markers that lead it, its own and its outer
/// quotes'. A `>` past those is the quote's text.
fn quote_markers(text: &str, range: &Range<usize>, nesting: usize, spans: &mut Vec<Span>) {
    let bytes = text.as_bytes();
    let mut line = range.start;
    let mut limit = 1;
    while line < range.end {
        let mut at = line;
        for _ in 0..limit {
            while at < range.end && matches!(bytes[at], b' ' | b'\t') {
                at += 1;
            }
            if at >= range.end || bytes[at] != b'>' {
                break;
            }
            markup(spans, at, at + 1);
            at += 1;
        }
        line = line_end(text, line);
        limit = nesting;
    }
}

/// The length of the list marker ITEM starts with: a bullet, or digits
/// with `.` or `)`.
fn list_marker(item: &str) -> usize {
    let bytes = item.as_bytes();
    match bytes.first() {
        Some(b'-' | b'*' | b'+') => 1,
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if matches!(bytes.get(digits), Some(b'.' | b')')) { digits + 1 } else { 0 }
        }
        _ => 0,
    }
}

/// The info string of an opening fence LINE, as a range in it.
fn fence_info(line: &str) -> Option<Range<usize>> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let fence = rest.bytes().take_while(|&b| b == b'`' || b == b'~').count();
    let after = &rest[fence..];
    let lead = after.len() - after.trim_start().len();
    let info = after.trim();
    (!info.is_empty()).then(|| {
        let start = indent + fence + lead;
        start..start + info.len()
    })
}

pub(crate) fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// The start of the line holding AT.
pub(crate) fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// The start of the line after the one holding AT, or TEXT's end.
pub(crate) fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i + 1)
}

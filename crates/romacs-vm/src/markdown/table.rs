//! A pipe table drawn as a grid within a width: its columns share the
//! width by what they hold, cells wrap at spaces, and a table too wide for
//! its narrowest columns is drawn as panels of columns, each with its
//! header. The cells keep their own Markdown's faces.
use super::spans::spans;
use super::runs::{runs, Run};
use super::{Kind, Span};
use pulldown_cmark::{Event, Parser, Tag};
use std::ops::Range;

/// A grid's text and its runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub text: String,
    pub runs: Vec<Run>,
}

/// One cell: its text (tabs as spaces) and its runs, relative to it.
struct Cell {
    text: String,
    runs: Vec<Run>,
}

/// Borders above every element, and cells' own kinds above the row's.
const BORDER: u32 = u32::MAX;
const HEADER: u32 = 1;
const INLINE: u32 = 16;

/// SOURCE, a pipe table, as a grid at most WIDTH columns wide (at least
/// 6), each character as many columns as COLUMNS says. None when SOURCE
/// holds no table.
pub(crate) fn grid(source: &str, width: usize, columns: &dyn Fn(char) -> usize) -> Option<Grid> {
    let rows = rows(source);
    if rows.is_empty() {
        return None;
    }
    let width = width.max(6);
    let count = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let minimum = rows.iter().flatten().flat_map(|c| c.text.chars().map(columns)).max().unwrap_or(0).max(2);
    let panel = ((width - 1) / (minimum + 3)).max(1);
    let mut out = Out { text: String::new(), spans: Vec::new() };
    for (n, first) in (0..count).step_by(panel).enumerate() {
        if n > 0 {
            out.text.push('\n');
        }
        let last = (first + panel).min(count);
        draw(&mut out, &rows, first..last, width, minimum, columns);
    }
    out.spans.push(Span { start: 0, end: out.text.len(), kind: Kind::Table, depth: 0 });
    Some(Grid { runs: runs(&out.spans), text: out.text })
}

struct Out {
    text: String,
    spans: Vec<Span>,
}

impl Out {
    fn border(&mut self, s: &str) {
        let start = self.text.len();
        self.text.push_str(s);
        self.spans.push(Span { start, end: self.text.len(), kind: Kind::Markup, depth: BORDER });
    }
}

/// Columns COLS of ROWS, the first the header, as one grid.
fn draw(out: &mut Out, rows: &[Vec<Cell>], cols: Range<usize>, width: usize, minimum: usize, columns: &dyn Fn(char) -> usize) {
    let text_width = |s: &str| s.chars().map(columns).sum::<usize>();
    let blank = Cell { text: String::new(), runs: Vec::new() };
    let natural: Vec<usize> = cols.clone().map(|i| rows.iter().map(|r| text_width(&cell_at(r, i, &blank).text)).max().unwrap_or(0).max(minimum)).collect();
    let widths = share(&natural, width, minimum);
    let rule = format!("+-{}-+\n", widths.iter().map(|&w| "-".repeat(w)).collect::<Vec<_>>().join("-+-"));
    out.border(&rule);
    for (r, row) in rows.iter().enumerate() {
        let lines: Vec<Vec<Range<usize>>> = cols.clone().zip(&widths).map(|(i, &w)| wrap(&cell_at(row, i, &blank).text, w, columns)).collect();
        let height = lines.iter().map(Vec::len).max().unwrap_or(1);
        for l in 0..height {
            out.border("| ");
            for (k, (i, &w)) in cols.clone().zip(&widths).enumerate() {
                if k > 0 {
                    out.border(" | ");
                }
                let c = cell_at(row, i, &blank);
                let piece = lines[k].get(l).cloned().unwrap_or(0..0);
                let start = out.text.len();
                out.text.push_str(&c.text[piece.clone()]);
                carry(&mut out.spans, &c.runs, piece.clone(), start);
                if r == 0 && !piece.is_empty() {
                    out.spans.push(Span { start, end: out.text.len(), kind: Kind::TableHeader, depth: HEADER });
                }
                let pad = w.saturating_sub(text_width(&c.text[piece]));
                out.text.extend(std::iter::repeat_n(' ', pad));
            }
            out.border(" |\n");
        }
        out.border(&rule);
    }
}

/// ROW's cell I, or BLANK past its end (a ragged row).
fn cell_at<'a>(row: &'a [Cell], i: usize, blank: &'a Cell) -> &'a Cell {
    row.get(i).unwrap_or(blank)
}

/// Widths for columns wanting NATURAL within WIDTH: each MINIMUM, then a
/// column at a time one more while room is left and it wants more.
fn share(natural: &[usize], width: usize, minimum: usize) -> Vec<usize> {
    let mut widths = vec![minimum; natural.len()];
    let mut room = width.saturating_sub(1 + natural.len() * (minimum + 3));
    while room > 0 && widths != natural {
        for (w, &n) in widths.iter_mut().zip(natural) {
            if room > 0 && *w < n {
                *w += 1;
                room -= 1;
            }
        }
    }
    widths
}

/// TEXT cut into lines of at most WIDTH columns, as byte ranges: a cut
/// goes after the last space that fits, else where the line is full; a
/// character wider than a line has one to itself.
fn wrap(text: &str, width: usize, columns: &dyn Fn(char) -> usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let (mut start, mut used, mut space) = (0usize, 0usize, None::<usize>);
    for (at, c) in text.char_indices() {
        let w = columns(c);
        if used > 0 && used + w > width {
            let cut = space.filter(|&s| s > start).unwrap_or(at);
            lines.push(start..cut);
            used = text[cut..at].chars().map(columns).sum();
            start = cut;
            space = None;
            // What the cut kept may still leave no room for this character.
            if used > 0 && used + w > width {
                lines.push(start..at);
                start = at;
                used = 0;
            }
        }
        used += w;
        if c == ' ' {
            space = Some(at + 1);
        }
    }
    lines.push(start..text.len());
    lines
}

/// RUNS of a cell, those over PIECE of it, as spans of the grid from AT.
fn carry(spans: &mut Vec<Span>, runs: &[Run], piece: Range<usize>, at: usize) {
    for run in runs.iter().filter(|r| r.start < piece.end && r.end > piece.start) {
        let (start, end) = (run.start.max(piece.start) - piece.start + at, run.end.min(piece.end) - piece.start + at);
        // Innermost first: the first kind deepest.
        for (depth, &kind) in (INLINE..).zip(run.kinds.iter().rev()) {
            spans.push(Span { start, end, kind, depth });
        }
    }
}

/// SOURCE's table rows, header first: each cell's text and inline runs.
fn rows(source: &str) -> Vec<Vec<Cell>> {
    let all = runs(&spans(source).spans);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut in_table = false;
    for (event, range) in Parser::new_ext(source, super::spans::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_)) if rows.is_empty() => in_table = true,
            Event::End(pulldown_cmark::TagEnd::Table) => in_table = false,
            Event::Start(Tag::TableHead | Tag::TableRow) if in_table => rows.push(Vec::new()),
            Event::Start(Tag::TableCell) if in_table => {
                if let Some(row) = rows.last_mut() {
                    row.push(cell(source, range, &all));
                }
            }
            _ => {}
        }
    }
    rows
}

/// The cell at RANGE of SOURCE, trimmed, with the inline kinds of ALL
/// (the source's runs) over it.
fn cell(source: &str, range: Range<usize>, all: &[Run]) -> Cell {
    let raw = &source[range.clone()];
    let lead = raw.len() - raw.trim_start().len();
    let body = (range.start + lead)..(range.start + lead + raw.trim().len());
    let text = source[body.clone()].replace('\t', " ");
    let runs = all.iter().filter(|r| r.start < body.end && r.end > body.start).filter_map(|r| {
        let kinds: Vec<Kind> = r.kinds.iter().copied().filter(|k| !matches!(k, Kind::Table | Kind::TableHeader)).collect();
        (!kinds.is_empty()).then(|| Run { start: r.start.max(body.start) - body.start, end: r.end.min(body.end) - body.start, kinds })
    }).collect();
    Cell { text, runs }
}

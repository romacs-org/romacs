//! What a root picture of one frame must repaint to show the next: nothing,
//! some regions, or all of it. Pure: frames in, a plan out.
//!
//! A frame's glyphs fall into groups, one per window and vertical span (a
//! glyph's y and height, bit for bit), each in frame order: a window's row
//! of text is a group, as is a window's background or a scroll bar. Layout
//! emits a group's glyphs together, so a frame is indexed once as runs of
//! consecutive glyphs of one group (`Indexed`), tens of them, and groups are
//! compared run by run. A group present in one frame and not the other, or
//! whose glyphs differ, damages the rectangle its glyphs cover in both
//! frames.
//!
//! Everything a glyph's paint reads besides itself is compared once per
//! frame: the frame's setting (`FrameGlyphBuffer::sets_glyphs_like`), and
//! the tables a glyph names by id, of which only the entries both frames
//! hold must agree: an entry one frame alone holds belongs to glyphs that
//! differ. A frame whose layout identity is the shown frame's carries the
//! same glyphs and tables by the layout's promise, and is not compared.
use std::collections::HashMap;
use std::ops::Range;

use rustc_hash::FxHashMap;

use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer};
use romacs_display_protocol::types::{DisplayWindowId, Rect};

use super::moves::{self, Move};
use super::reach::Inks;

/// A group's name: its window (0 for glyphs of none) and its glyphs' span.
pub(crate) type Key = (i64, u32, u32);

/// Each group's runs, by position in its frame's runs.
pub(crate) type Groups = FxHashMap<Key, Vec<usize>>;

/// The two frames a plan compares, with their groups.
pub(crate) struct Sides<'a> {
    pub(crate) old: &'a Indexed,
    pub(crate) before: Groups,
    pub(crate) new: &'a FrameGlyphBuffer,
    pub(crate) runs: &'a [Run],
    pub(crate) after: Groups,
}

/// Consecutive glyphs of one group, and the rectangle they cover.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub(crate) key: Key,
    pub(crate) glyphs: Range<usize>,
    pub(crate) bounds: Rect,
    /// It holds characters or stretches: a row's text, which underline and
    /// box runs span.
    pub(crate) text: bool,
    /// Its visual row (`assign_rows`): text runs of one window that overlap
    /// vertically share one; any other run is a row of its own.
    pub(crate) row: usize,
}

/// A frame and its runs.
pub(crate) struct Indexed {
    pub(crate) frame: FrameGlyphBuffer,
    pub(crate) runs: Vec<Run>,
}

impl Indexed {
    #[cfg(test)]
    pub(crate) fn new(frame: FrameGlyphBuffer) -> Self {
        let runs = runs(&frame.glyphs);
        Self { frame, runs }
    }
}

/// How a picture of one frame comes to show the next.
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    /// Nothing the picture shows changed. ROWS text rows are kept.
    Clean { rows: usize },
    /// Make `moves`, then repaint `regions` (logical pixels, disjoint).
    /// `rows_kept` text rows of the next frame are kept or moved, outside
    /// the regions.
    Regions {
        moves: Vec<Move>,
        regions: Vec<Rect>,
        rows_kept: usize,
    },
    /// Repaint all of it.
    Whole,
}

/// Past this share of the frame's area in regions, a whole repaint.
const WHOLE_SHARE: f32 = 0.5;
/// Past this many changed runs, a whole repaint, before they are joined.
const MAX_DAMAGED_RUNS: usize = 256;
/// Past this many regions, a whole repaint: each is a pass of its own, with
/// a setup that does not shrink with its size.
const MAX_REGIONS: usize = 6;

/// Whether NEW repeats OLD by the layout's promise: one nonzero layout
/// identity, the same setting. Its glyphs and tables need no compare.
pub(crate) fn repeats(old: &FrameGlyphBuffer, new: &FrameGlyphBuffer) -> bool {
    old.layout_identity != 0 && old.layout_identity == new.layout_identity && old.sets_glyphs_like(new)
}

/// The text rows of INDEXED whose run KEPT holds: its groups that hold text.
pub(crate) fn text_rows(indexed: &Indexed, kept: impl Fn(&Run) -> bool) -> usize {
    let mut keys: Vec<Key> = indexed
        .runs
        .iter()
        .filter(|r| kept(r) && has_text(&indexed.frame, r))
        .map(|r| r.key)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.len()
}

/// What a picture of OLD does to show NEW, whose runs are `new_runs`: its
/// ink reaches INKS past its cells, at SCALE device pixels per logical one.
pub(crate) fn plan(old: &Indexed, new: &FrameGlyphBuffer, new_runs: &[Run], inks: Inks<'_>, scale: f32) -> Plan {
    let rows = |runs: &[Run], kept: &dyn Fn(&Run) -> bool| {
        let mut keys: Vec<Key> = runs
            .iter()
            .filter(|r| kept(r) && has_text(new, r))
            .map(|r| r.key)
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.len()
    };
    if !old.frame.sets_glyphs_like(new) || !tables_agree(&old.frame, new) {
        return Plan::Whole;
    }
    let sides = Sides {
        old,
        before: groups(&old.runs),
        new,
        runs: new_runs,
        after: groups(new_runs),
    };
    let (before, after) = (&sides.before, &sides.after);
    // Each group compared once: the keys of those that differ, or that one
    // frame alone holds.
    let mut changed: Vec<Key> = before
        .keys()
        .chain(after.keys().filter(|key| !before.contains_key(*key)))
        .filter(|key| match (before.get(*key), after.get(*key)) {
            (Some(a), Some(b)) => !glyphs_of(&old.frame, &old.runs, a).eq(glyphs_of(new, new_runs, b)),
            _ => true,
        })
        .copied()
        .collect();
    changed.sort_unstable();
    let same = |key: &Key| changed.binary_search(key).is_err();
    let moved = moves::find(&sides, &changed, inks, scale);
    let placed = |key: &Key| moved.placed.binary_search(key).is_ok();
    // A changed group the moves place needs no repaint; nor does one of the
    // old frame whose ink lies inside a band a move overwrites.
    let mut damaged: Vec<Rect> = Vec::new();
    for (key, indices) in after {
        if !same(key) && !placed(key) {
            damaged.extend(indices.iter().map(|&i| new_runs[i].bounds));
        }
    }
    for (key, indices) in before {
        if !same(key) {
            damaged.extend(
                indices
                    .iter()
                    .map(|&i| old.runs[i].bounds)
                    .filter(|bounds| !moved.bands.iter().any(|band| moves::holds_ink(*band, *bounds, inks.drawn(*bounds), new))),
            );
        }
    }
    // Past this many changed runs the frame changed nearly everywhere, and
    // joining them pairwise would cost more than the whole repaint saves.
    if damaged.len() > MAX_DAMAGED_RUNS {
        return Plan::Whole;
    }
    let regions = merge(damaged);
    if regions.is_empty() && moved.moves.is_empty() {
        return Plan::Clean {
            rows: rows(new_runs, &|_| true),
        };
    }
    let area: f32 = regions.iter().map(|r| r.width * r.height).sum();
    if regions.len() > MAX_REGIONS || area > WHOLE_SHARE * new.width * new.height {
        return Plan::Whole;
    }
    let rows_kept = rows(new_runs, &|run| {
        (same(&run.key) || placed(&run.key)) && !regions.iter().any(|r| overlaps(*r, run.bounds))
    });
    Plan::Regions {
        moves: moved.moves,
        regions,
        rows_kept,
    }
}

/// Whether RUN of FRAME holds text.
fn has_text(frame: &FrameGlyphBuffer, run: &Run) -> bool {
    frame.glyphs[run.glyphs.clone()]
        .iter()
        .any(|g| matches!(g, FrameGlyph::Char { .. }))
}

/// GLYPHS as runs, in frame order.
pub(crate) fn runs(glyphs: &[FrameGlyph]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (at, glyph) in glyphs.iter().enumerate() {
        let bounds = glyph.geometry().unwrap_or_default();
        // An image fills its face's background over its box and slot, which
        // can be larger than itself.
        let painted = match glyph {
            FrameGlyph::Image { slot_rect, box_rect, .. } => union(union(bounds, *slot_rect), *box_rect),
            _ => bounds,
        };
        let text = matches!(glyph, FrameGlyph::Char { .. } | FrameGlyph::Stretch { .. });
        let key = (
            glyph.window_id().map_or(0, DisplayWindowId::get),
            bounds.y.to_bits(),
            bounds.height.to_bits(),
        );
        match out.last_mut() {
            Some(run) if run.key == key => {
                run.glyphs.end = at + 1;
                run.bounds = union(run.bounds, painted);
                run.text |= text;
            }
            _ => out.push(Run {
                key,
                glyphs: at..at + 1,
                bounds: painted,
                text,
                row: 0,
            }),
        }
    }
    assign_rows(&mut out);
    out
}

/// Give RUNS their visual rows: a sweep over each window's text runs by y
/// joins those whose vertical extents overlap (an underline or a box run
/// may span glyphs of two heights); every other run is a row of its own.
fn assign_rows(runs: &mut [Run]) {
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&runs[a], &runs[b]);
        (a.key.0, !a.text).cmp(&(b.key.0, !b.text)).then(a.bounds.y.total_cmp(&b.bounds.y))
    });
    // The window, bottom and row of the text row being swept.
    let mut open: Option<(i64, f32, usize)> = None;
    for (id, &at) in order.iter().enumerate() {
        let run = &runs[at];
        let bottom = run.bounds.y + run.bounds.height;
        runs[at].row = match open {
            Some((window, end, row)) if run.text && window == run.key.0 && run.bounds.y < end => {
                open = Some((window, end.max(bottom), row));
                row
            }
            _ => {
                open = run.text.then_some((run.key.0, bottom, id));
                id
            }
        };
    }
}

/// Each group's runs, by position in RUNS.
fn groups(runs: &[Run]) -> Groups {
    let mut groups = Groups::default();
    for (at, run) in runs.iter().enumerate() {
        groups.entry(run.key).or_default().push(at);
    }
    groups
}

/// The glyphs of the runs at INDICES of FRAME's RUNS, in order.
fn glyphs_of<'a>(
    frame: &'a FrameGlyphBuffer,
    runs: &'a [Run],
    indices: &'a [usize],
) -> impl Iterator<Item = &'a FrameGlyph> {
    indices
        .iter()
        .flat_map(move |&i| frame.glyphs[runs[i].glyphs.clone()].iter())
}

/// Whether A and B share any area.
pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// The smallest rectangle holding A and B.
pub(crate) fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        (a.x + a.width).max(b.x + b.width) - x,
        (a.y + a.height).max(b.y + b.height) - y,
    )
}

/// Whether A and B overlap or share an edge.
fn meet(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.width && b.x <= a.x + a.width && a.y <= b.y + b.height && b.y <= a.y + a.height
}

/// RECTS joined where they meet, until none do; empty ones dropped. A row
/// whose glyphs differ in height is groups side by side: one region.
pub(crate) fn merge(rects: Vec<Rect>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for mut rect in rects.into_iter().filter(|r| r.width > 0.0 && r.height > 0.0) {
        while let Some(at) = out.iter().position(|other| meet(*other, rect)) {
            rect = union(out.swap_remove(at), rect);
        }
        out.push(rect);
    }
    out.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    out
}

/// Whether every face, font and glyph answer OLD and NEW both hold is the
/// same in each.
fn tables_agree(old: &FrameGlyphBuffer, new: &FrameGlyphBuffer) -> bool {
    agree(&old.faces, &new.faces)
        && agree(&old.fonts, &new.fonts)
        && nested_agree(&old.char_fonts, &new.char_fonts)
        && nested_agree(&old.shaped_clusters, &new.shaped_clusters)
}

fn agree<K, V, S>(a: &HashMap<K, V, S>, b: &HashMap<K, V, S>) -> bool
where
    K: std::hash::Hash + Eq,
    V: PartialEq,
    S: std::hash::BuildHasher,
{
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small
        .iter()
        .all(|(key, value)| large.get(key).is_none_or(|other| other == value))
}

fn nested_agree<K, L, V, S>(a: &HashMap<K, HashMap<L, V, S>, S>, b: &HashMap<K, HashMap<L, V, S>, S>) -> bool
where
    K: std::hash::Hash + Eq,
    L: std::hash::Hash + Eq,
    V: PartialEq,
    S: std::hash::BuildHasher,
{
    a.iter()
        .all(|(key, inner)| b.get(key).is_none_or(|other| agree(inner, other)))
}

#[cfg(test)]
mod tests;

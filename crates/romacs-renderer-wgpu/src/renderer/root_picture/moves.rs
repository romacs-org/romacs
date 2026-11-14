//! Rows that only moved. A scroll, or a line inserted or deleted above
//! others, moves a band of a window's rows by one distance: the new frame's
//! rows there are the old frame's, glyph for glyph, but for their y. The
//! picture holds their pixels already, a distance away, so the band is
//! copied there, and the plan repaints what changed around it.
//!
//! A band moves only when it passes one predicate, which makes the copy
//! byte-equal to drawing the band whole (display-renderer-damage.md, §5
//! step 4, gives each clause's reason):
//!
//! 1. Exact on the device: the scale is a whole number, and the distance
//!    and every y, height and baseline the band's glyphs carry are whole
//!    device pixels (`on_grid`). Every edge the glyph pass derives from
//!    them then lies on a half pixel and moves by exactly the distance.
//! 2. The same rows: each of the band's visual rows is entire, and is the
//!    old frame's row the distance away, glyph for glyph but y and slot
//!    (`moved`, `whole_rows`). A row draws with its whole visual row and
//!    nothing else: underline runs and box runs end at a window's edge.
//! 3. Nothing drawn from y: no face with a background gradient, no tiled
//!    fringe bitmap, no strike-through or wave underline (placed off the
//!    half-pixel grid), no clip cutting the band's ink in either place
//!    (`reads_y`, `moved`).
//! 4. Nothing else in the band: the band is the rows' ink box (their cells
//!    grown by how far their own ink reached when the picture drew them, out
//!    to device pixels), and whatever else
//!    the new frame draws there, or the old frame at its source, is a
//!    uniform rectangle spanning both places (a window's background, an
//!    unclipped border, a pane's dim), or, in the band's place only, a pane
//!    edge that is opaque and the same in both frames, which every pass
//!    draws last and the picture draws over each band it moves (`cover` in
//!    root_picture.rs), so it covers whatever the copy put under it. No
//!    other glyph's ink, no pane corner, no pane edge in the source
//!    (`alone`).
//!
//! A frame's glyphs that pass are copied; the plan repaints its changed
//! groups as for any frame but the moved rows, whose ink the band holds,
//! and old rows whose ink lies in the band, which the copy overwrites.
use std::hash::{Hash, Hasher};

use rustc_hash::{FxHashMap, FxHasher};

use romacs_display_protocol::face::{FaceAttributes, UnderlineStyle};
use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer};
use romacs_display_protocol::types::Rect;

use super::damage::{self, Key, Run, Sides};
use super::reach::{Inks, Reach};

/// A band of the picture to move: its place before, logical pixels on
/// whole device pixels, and how far down it goes (up when negative).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Move {
    pub(crate) from: Rect,
    pub(crate) dy: f32,
}

/// A frame's moves.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub(crate) moves: Vec<Move>,
    /// The new frame's groups the moves place, sorted.
    pub(crate) placed: Vec<Key>,
    /// Where each move lands.
    pub(crate) bands: Vec<Rect>,
}

/// Fewest groups a move carries: one alone is as cheap repainted.
const MIN_GROUPS: usize = 2;
/// Most distances checked per window, best voted first: signatures agree on
/// rows alike in text, so the best voted can prove wrong. A wrong one costs
/// a few glyphs a row: a row that did not move differs early.
const MAX_TRIES: usize = 8;

/// The moves that bring a picture of SIDES' old frame toward its new one,
/// whose groups at CHANGED (sorted) differ in place; INKS is how far ink
/// leaves its cells, SCALE the device pixels per logical one.
pub(crate) fn find(sides: &Sides<'_>, changed: &[Key], inks: Inks<'_>, scale: f32) -> Found {
    let mut found = Found::default();
    if !on_pixels(1.0, scale) {
        return found;
    }
    let candidates = distances(sides, changed, scale);
    let mut windows: Vec<i64> = candidates.iter().map(|c| c.0).collect();
    windows.dedup();
    for window in windows {
        // The first distance, best voted first, whose band passes.
        let Some((step, area, placed)) = candidates
            .iter()
            .filter(|c| c.0 == window)
            .take(MAX_TRIES)
            .find_map(|&(_, dy)| band(sides, changed, window, dy, inks, scale))
        else {
            continue;
        };
        found.moves.push(step);
        found.bands.push(area);
        found.placed.extend(placed);
    }
    found.placed.sort_unstable();
    found
}

/// The band of WINDOW's rows that moved DY, if one passes the predicate:
/// its move, where it lands, and the groups it places. CHANGED (sorted) are
/// the groups that differ in place, which the picture has not drawn.
fn band(sides: &Sides<'_>, changed: &[Key], window: i64, dy: f32, inks: Inks<'_>, scale: f32) -> Option<(Move, Rect, Vec<Key>)> {
    let mut placed: Vec<Key> = sides
        .after
        .keys()
        .filter(|key| key.0 == window && moved(sides, **key, dy, inks, scale))
        .copied()
        .collect();
    placed.sort_unstable();
    let placed = whole_rows(sides, &placed, dy);
    if placed.len() < MIN_GROUPS {
        return None;
    }
    // The rows' ink: each group's bounds grown by how far its ink reached
    // where the picture drew it, a distance away.
    let rows = placed
        .iter()
        .flat_map(|key| sides.after[key].iter().map(|&at| sides.runs[at].bounds))
        .map(|r| inks.drawn(Rect::new(r.x, r.y - dy, r.width, r.height)).grow(r))
        .reduce(damage::union)?;
    let frame = Rect::new(0.0, 0.0, sides.new.width, sides.new.height);
    let area = outward(intersection(rows, frame)?, scale);
    let from = Rect::new(area.x, area.y - dy, area.width, area.height);
    let inside = from.y >= 0.0 && from.y + from.height <= sides.new.height;
    let step = Move { from, dy };
    (inside && alone(sides, changed, &placed, (area, step), inks, scale)).then_some((step, area, placed))
}

/// Of PLACED (sorted), the groups whose whole visual row moved DY from a
/// whole visual row: a row's groups draw together (an underline run keeps
/// its first glyph's line, a box spans them), so a group moves only with
/// every group of its row, from a row of exactly their sources.
fn whole_rows(sides: &Sides<'_>, placed: &[Key], dy: f32) -> Vec<Key> {
    let source = |key: &Key| (key.0, (f32::from_bits(key.1) - dy).to_bits(), key.2);
    // Each frame's visual rows, as the sorted keys of their groups.
    let rows = |runs: &[Run]| {
        let mut rows: FxHashMap<usize, Vec<Key>> = FxHashMap::default();
        for run in runs {
            rows.entry(run.row).or_default().push(run.key);
        }
        for keys in rows.values_mut() {
            keys.sort_unstable();
            keys.dedup();
        }
        rows
    };
    let (new_rows, old_rows) = (rows(sides.runs), rows(&sides.old.runs));
    let old_row_of: FxHashMap<Key, usize> = sides.old.runs.iter().map(|run| (run.key, run.row)).collect();
    let whole = |keys: &Vec<Key>| {
        let mut sources: Vec<Key> = keys.iter().map(source).collect();
        sources.sort_unstable();
        keys.iter().all(|key| placed.binary_search(key).is_ok())
            && old_row_of.get(&sources[0]).is_some_and(|row| old_rows[row] == sources)
    };
    let mut kept: Vec<Key> = new_rows
        .values()
        .filter(|keys| keys.iter().any(|key| placed.binary_search(key).is_ok()) && whole(keys))
        .flatten()
        .copied()
        .collect();
    kept.sort_unstable();
    kept
}

/// The distances changed text groups moved by, by window and best voted
/// first: those at least `MIN_GROUPS` of a window's groups' signatures
/// vote for, by whole device pixels at SCALE.
fn distances(sides: &Sides<'_>, changed: &[Key], scale: f32) -> Vec<(i64, f32)> {
    let text = |groups: &damage::Groups, runs: &[Run], key: &Key| {
        key.0 != 0 && groups.get(key).is_some_and(|at| at.iter().any(|&i| runs[i].text))
    };
    let moving: Vec<&Key> = changed.iter().filter(|key| text(&sides.after, sides.runs, key)).collect();
    if moving.len() < MIN_GROUPS {
        return Vec::new();
    }
    let mut windows: Vec<i64> = moving.iter().map(|key| key.0).collect();
    windows.sort_unstable();
    windows.dedup();
    // The old frame's text groups in those windows, by window, signature and
    // height: where each stood.
    let mut stood: FxHashMap<(i64, u64, u32), Vec<f32>> = FxHashMap::default();
    for (key, at) in &sides.before {
        if windows.binary_search(&key.0).is_ok() && text(&sides.before, &sides.old.runs, key) {
            let signed = signature(&sides.old.frame, &sides.old.runs, at);
            stood.entry((key.0, signed, key.2)).or_default().push(f32::from_bits(key.1));
        }
    }
    let mut votes: FxHashMap<(i64, u32), usize> = FxHashMap::default();
    for key in moving {
        let signed = signature(sides.new, sides.runs, &sides.after[key]);
        for &y in stood.get(&(key.0, signed, key.2)).into_iter().flatten() {
            let dy = f32::from_bits(key.1) - y;
            if dy != 0.0 && on_pixels(dy, scale) {
                *votes.entry((key.0, dy.to_bits())).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(i64, usize, f32)> = votes
        .into_iter()
        .filter(|vote| vote.1 >= MIN_GROUPS)
        .map(|((window, dy), n)| (window, n, f32::from_bits(dy)))
        .collect();
    // By window; then most votes, the shorter distance, and its bits, so the
    // order is the same whatever the map's.
    ranked.sort_unstable_by_key(|&(window, n, dy)| {
        (window, std::cmp::Reverse(n), dy.abs().to_bits(), dy.to_bits())
    });
    ranked.into_iter().map(|(window, _, dy)| (window, dy)).collect()
}

/// A hash of the glyphs at AT of FRAME's RUNS that leaves their y out: of
/// where they start and of each one's kind, character and face.
fn signature(frame: &FrameGlyphBuffer, runs: &[Run], at: &[usize]) -> u64 {
    let mut hasher = FxHasher::default();
    let mut glyphs = at.iter().flat_map(|&i| frame.glyphs[runs[i].glyphs.clone()].iter());
    // Where it starts, then every glyph's kind and character: rows that
    // sign alike are alike in text, and `moved` checks every field.
    if let Some(bounds) = glyphs.clone().next().and_then(FrameGlyph::geometry) {
        bounds.x.to_bits().hash(&mut hasher);
    }
    for glyph in &mut glyphs {
        match glyph {
            FrameGlyph::Char { char, face_id, .. } => (u32::from(*char), face_id.get()).hash(&mut hasher),
            _ => std::mem::discriminant(glyph).hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// Whether the new frame's group KEY is the old frame's group DY above it,
/// moved, by clauses 1 to 3: glyph for glyph equal but for y, on whole
/// device pixels in both places, drawing nothing from y, its ink clear of
/// its clips' tops and bottoms in both places (their sides cut it alike in
/// either).
fn moved(sides: &Sides<'_>, key: Key, dy: f32, inks: Inks<'_>, scale: f32) -> bool {
    let source = (key.0, (f32::from_bits(key.1) - dy).to_bits(), key.2);
    let (Some(was), Some(is)) = (sides.before.get(&source), sides.after.get(&key)) else {
        return false;
    };
    let Some(bounds) = is.iter().map(|&at| sides.runs[at].bounds).reduce(damage::union) else {
        return false;
    };
    let reach = inks.drawn(Rect::new(bounds.x, bounds.y - dy, bounds.width, bounds.height));
    let inked = reach.grow(bounds);
    let before = Rect::new(inked.x, inked.y - dy, inked.width, inked.height);
    let inside = |clip: Rect, rect: Rect| clip.y <= rect.y && rect.y + rect.height <= clip.y + clip.height;
    let clear = |glyph: &FrameGlyph| {
        !reads_y(sides.new, glyph) && clip(glyph).is_none_or(|clip| inside(clip, inked) && inside(clip, before))
    };
    let mut olds = was.iter().flat_map(|&at| sides.old.frame.glyphs[sides.old.runs[at].glyphs.clone()].iter());
    let mut news = is.iter().flat_map(|&at| sides.new.glyphs[sides.runs[at].glyphs.clone()].iter());
    loop {
        match (olds.next(), news.next()) {
            (None, None) => return true,
            (Some(old), Some(new))
                if clear(new) && on_grid(old, scale) && on_grid(new, scale) && moved_glyph(old, new, dy) => {}
            _ => return false,
        }
    }
}

/// Whether NEW is OLD moved DY down, its slot aside.
fn moved_glyph(old: &FrameGlyph, new: &FrameGlyph, dy: f32) -> bool {
    if let Some(moved) = moved_text(old, new, dy) {
        return moved;
    }
    let mut moved = old.clone();
    match &mut moved {
        FrameGlyph::Image { y, slot_rect, box_rect, slot_id, .. } => {
            *y += dy;
            slot_rect.y += dy;
            box_rect.y += dy;
            *slot_id = new.slot_id();
        }
        FrameGlyph::FringeBitmap { y, .. } | FrameGlyph::Border { y, .. } => *y += dy,
        FrameGlyph::Background { bounds, .. } => bounds.y += dy,
        // Text of two kinds; or drawn from outside the frame's glyphs (a
        // video, a surface, a widget), or not by row (a scroll bar).
        _ => return false,
    }
    moved == *new
}

/// For OLD and NEW both a character or both a stretch, the most of a row:
/// whether NEW is OLD moved DY down, its slot aside, compared field by field
/// (every field named, so one added to the glyph is compared here too).
fn moved_text(old: &FrameGlyph, new: &FrameGlyph, dy: f32) -> Option<bool> {
    let same = |a: f32, b: f32| a.to_bits() == b.to_bits();
    match (old, new) {
        (
            FrameGlyph::Char {
                window_id,
                row_role,
                clip_rect,
                slot_id: _,
                bidi_level,
                char,
                composed,
                x,
                y,
                baseline,
                width,
                height,
                ascent,
                face_id,
                box_vertical_edges,
            },
            FrameGlyph::Char {
                window_id: window_id2,
                row_role: row_role2,
                clip_rect: clip_rect2,
                slot_id: _,
                bidi_level: bidi_level2,
                char: char2,
                composed: composed2,
                x: x2,
                y: y2,
                baseline: baseline2,
                width: width2,
                height: height2,
                ascent: ascent2,
                face_id: face_id2,
                box_vertical_edges: box_vertical_edges2,
            },
        ) => Some(
            char == char2
                && same(*x, *x2)
                && same(*y + dy, *y2)
                && same(*baseline + dy, *baseline2)
                && same(*width, *width2)
                && same(*height, *height2)
                && same(*ascent, *ascent2)
                && face_id == face_id2
                && window_id == window_id2
                && row_role == row_role2
                && clip_rect == clip_rect2
                && bidi_level == bidi_level2
                && composed == composed2
                && box_vertical_edges == box_vertical_edges2,
        ),
        (
            FrameGlyph::Stretch {
                window_id,
                row_role,
                clip_rect,
                slot_id: _,
                bidi_level,
                x,
                y,
                width,
                height,
                bg,
                face_id,
                box_vertical_edges,
            },
            FrameGlyph::Stretch {
                window_id: window_id2,
                row_role: row_role2,
                clip_rect: clip_rect2,
                slot_id: _,
                bidi_level: bidi_level2,
                x: x2,
                y: y2,
                width: width2,
                height: height2,
                bg: bg2,
                face_id: face_id2,
                box_vertical_edges: box_vertical_edges2,
            },
        ) => Some(
            same(*x, *x2)
                && same(*y + dy, *y2)
                && same(*width, *width2)
                && same(*height, *height2)
                && bg == bg2
                && face_id == face_id2
                && window_id == window_id2
                && row_role == row_role2
                && clip_rect == clip_rect2
                && bidi_level == bidi_level2
                && box_vertical_edges == box_vertical_edges2,
        ),
        _ => None,
    }
}

/// Whether V logical pixels are a whole number of device pixels at SCALE,
/// exactly.
fn on_pixels(v: f32, scale: f32) -> bool {
    let device = v * scale;
    device.to_bits() == device.round().to_bits()
}

/// Clause 1: whether every y, height and baseline GLYPH carries is a whole
/// number of device pixels at SCALE; false for a kind a band does not carry.
fn on_grid(glyph: &FrameGlyph, scale: f32) -> bool {
    let all = |values: &[f32]| values.iter().all(|&v| on_pixels(v, scale));
    match glyph {
        FrameGlyph::Char { y, height, baseline, .. } => all(&[*y, *height, *baseline]),
        FrameGlyph::Stretch { y, height, .. }
        | FrameGlyph::FringeBitmap { y, height, .. }
        | FrameGlyph::Border { y, height, .. } => all(&[*y, *height]),
        FrameGlyph::Image { y, height, slot_rect, box_rect, .. } => {
            all(&[*y, *height, slot_rect.y, slot_rect.height, box_rect.y, box_rect.height])
        }
        FrameGlyph::Background { bounds, .. } => all(&[bounds.y, bounds.height]),
        _ => false,
    }
}

/// Clause 3: whether GLYPH, in FRAME, draws something from its y beyond
/// moving with it: a background gradient sampled at y, a fringe bitmap
/// tiled by y, or a strike-through or wave underline, placed off the
/// half-pixel grid.
fn reads_y(frame: &FrameGlyphBuffer, glyph: &FrameGlyph) -> bool {
    if let FrameGlyph::FringeBitmap { bitmap_index, .. } = glyph {
        return frame.fringe_bitmaps.get(bitmap_index).is_some_and(|bitmap| bitmap.period > 0);
    }
    face(glyph).and_then(|id| frame.faces.get(&id)).is_some_and(|face| {
        face.background_gradient.is_some()
            || face.attributes.contains(FaceAttributes::STRIKE_THROUGH)
            || (face.attributes.contains(FaceAttributes::UNDERLINE) && face.underline_style == UnderlineStyle::Wave)
    })
}

/// The clip GLYPH is drawn under, if any.
fn clip(glyph: &FrameGlyph) -> Option<Rect> {
    match glyph {
        FrameGlyph::Char { clip_rect, .. }
        | FrameGlyph::Stretch { clip_rect, .. }
        | FrameGlyph::Image { clip_rect, .. }
        | FrameGlyph::FringeBitmap { clip_rect, .. }
        | FrameGlyph::Border { clip_rect, .. } => *clip_rect,
        _ => None,
    }
}

/// The face GLYPH's text or fill is drawn in, if any.
fn face(glyph: &FrameGlyph) -> Option<romacs_display_protocol::types::FaceId> {
    match glyph {
        FrameGlyph::Char { face_id, .. } | FrameGlyph::Stretch { face_id, .. } => Some(*face_id),
        _ => None,
    }
}

/// Where A and B overlap, if they do.
pub(crate) fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.width).min(b.x + b.width), (a.y + a.height).min(b.y + b.height));
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Whether BAND holds the ink of a group with BOUNDS, ink reaching REACH
/// past it, on FRAME: ink past the frame's edges is never drawn.
pub(crate) fn holds_ink(band: Rect, bounds: Rect, reach: Reach, frame: &FrameGlyphBuffer) -> bool {
    intersection(reach.grow(bounds), Rect::new(0.0, 0.0, frame.width, frame.height))
        .is_none_or(|ink| contains(band, ink))
}

/// Whether OUTER holds INNER.
pub(crate) fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

/// RECT grown out to whole device pixels at SCALE.
fn outward(rect: Rect, scale: f32) -> Rect {
    let [x0, y0] = [rect.x, rect.y].map(|v| (v * scale).floor() / scale);
    let [x1, y1] = [rect.x + rect.width, rect.y + rect.height].map(|v| (v * scale).ceil() / scale);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Clause 4: whether the band at AREA, moving by STEP from its source, holds nothing
/// but the groups it PLACES (sorted): whatever else either frame draws in
/// its place there (the new frame at AREA, the old at FROM), its ink grown
/// by REACH, is a uniform rectangle spanning both places, which looks alike
/// moved within itself. Of the panes' chrome, drawn over the rows and fixed
/// where they move: a dim spans both places or touches neither, no corner
/// touches either, no edge touches FROM, and an edge in AREA is opaque and
/// the same in both frames, drawn over the band after the copy.
fn alone(sides: &Sides<'_>, changed: &[Key], placed: &[Key], (area, step): (Rect, Move), inks: Inks<'_>, scale: f32) -> bool {
    let Move { from, dy } = step;
    let span = (from.y.min(area.y), (from.y + from.height).max(area.y + area.height));
    let spans = |r: Rect| r.y <= span.0 && r.y + r.height >= span.1;
    // A window's background, or a border as its clip cuts it.
    let uniform = |glyph: &FrameGlyph| match glyph {
        FrameGlyph::Background { bounds, .. } => Some(Some(*bounds)),
        FrameGlyph::Border { x, y, width, height, clip_rect, .. } => {
            let bounds = Rect::new(*x, *y, *width, *height);
            Some(clip_rect.map_or(Some(bounds), |clip| intersection(bounds, clip)))
        }
        _ => None,
    };
    // A group the old picture drew reaches as far as it did there; one the
    // new frame adds, as far as any ink the picture drew.
    let fits = |frame: &FrameGlyphBuffer, run: &Run, place: Rect, reach: Reach| {
        let glyphs = &frame.glyphs[run.glyphs.clone()];
        if glyphs.iter().all(|g| uniform(g).is_some()) {
            glyphs
                .iter()
                .filter_map(|g| uniform(g).flatten())
                .all(|r| spans(r) || !damage::overlaps(r, place))
        } else {
            !damage::overlaps(reach.grow(run.bounds), place)
        }
    };
    let mut carried: Vec<Key> = placed.iter().map(|k| (k.0, (f32::from_bits(k.1) - dy).to_bits(), k.2)).collect();
    carried.sort_unstable();
    let others = |frame: &FrameGlyphBuffer, runs: &[Run], groups: &damage::Groups, moving: &[Key], place: Rect, new: bool| {
        groups.iter().filter(|(key, _)| moving.binary_search(key).is_err()).all(|(key, at)| {
            let drawn = !new || changed.binary_search(key).is_err();
            at.iter().all(|&i| {
                let reach = if drawn { inks.drawn(runs[i].bounds) } else { inks.all };
                fits(frame, &runs[i], place, reach)
            })
        })
    };
    let chrome = |frame: &FrameGlyphBuffer| super::super::panes::pane_geometry(&frame.panes, frame.background, scale);
    let (now, was) = (chrome(sides.new), chrome(&sides.old.frame));
    let touches = |r: Rect| damage::overlaps(r, area) || damage::overlaps(r, from);
    let covering = |(rect, color): &(Rect, romacs_display_protocol::types::Color)| {
        color.a >= 1.0 && was.edges.iter().any(|(r, c)| r == rect && c == color)
    };
    let chrome_clear = now.dims.iter().chain(&was.dims).all(|(r, _)| spans(*r) || !touches(*r))
        && now.corners.iter().chain(&was.corners).all(|corner| !touches(corner.square))
        && was.edges.iter().all(|(r, _)| !damage::overlaps(*r, from))
        && now.edges.iter().all(|edge| !damage::overlaps(edge.0, from) && (!damage::overlaps(edge.0, area) || covering(edge)));
    others(sides.new, sides.runs, &sides.after, placed, area, true)
        && others(&sides.old.frame, &sides.old.runs, &sides.before, &carried, from, false)
        && chrome_clear
}

#[cfg(test)]
mod tests;

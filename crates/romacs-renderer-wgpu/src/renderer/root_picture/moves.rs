//! Rows that only moved. A scroll, or a line inserted or deleted above
//! others, moves a band of a window's rows by one distance: the new frame's
//! groups there are the old frame's, glyph for glyph, but for their y. The
//! picture holds their pixels already, elsewhere, so the band is copied to
//! its new place and only what entered it, or lies around it, is repainted.
//!
//! A window's distance is the one most of its changed text groups moved by,
//! found from a signature of each group's glyphs that leaves y out, and
//! checked glyph by glyph. A move is taken only where it is exact on the
//! surface: a whole number of device pixels, and every glyph clear of its
//! clip's top and bottom in both places and of any gradient that reads its
//! y. A glyph's slot id is not compared: slots are derived from pixels and
//! only place cursors, which the picture does not hold.
//!
//! The band copied is the moved groups' box, widened by how far ink reaches
//! sideways and out to whole device pixels (ink a row casts sideways moves
//! with it), and narrowed to whole device pixels top and bottom, the
//! slivers left repainted. What the copy brings into the band besides the
//! moved groups, and what the new frame draws there besides them, is
//! repainted: a group of either frame that touches the band, other than a
//! solid rectangle that spans it (a window's background); and so are the
//! strips above and below it that the moved groups' ink, and the ink of
//! what they replace, reaches.
use std::hash::{Hash, Hasher};

use rustc_hash::{FxHashMap, FxHasher};

use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer};
use romacs_display_protocol::types::Rect;

use super::damage::{self, Key, Run, Sides};
use super::reach::Reach;

/// A band of the picture to move: its place before, logical pixels on
/// whole device pixels, and how far down it goes (up when negative).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Move {
    pub(crate) from: Rect,
    pub(crate) dy: f32,
}

/// A frame's moves, and what they leave to repaint.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub(crate) moves: Vec<Move>,
    /// The new frame's groups the moves place, sorted.
    pub(crate) placed: Vec<Key>,
    /// Where each move lands.
    pub(crate) bands: Vec<Rect>,
    /// What must be repainted once they are made.
    pub(crate) damage: Vec<Rect>,
}

/// Fewest groups a move carries: one alone is as cheap repainted.
const MIN_GROUPS: usize = 2;
/// Most distances checked per window, best voted first: signatures can
/// agree on rows that differ, so the best voted can prove wrong. A wrong
/// one costs a few glyphs a row: a row that did not move differs early.
const MAX_TRIES: usize = 8;

/// The moves that bring a picture of SIDES' old frame toward its new one,
/// whose groups at CHANGED (sorted) differ in place; REACH is how far ink
/// leaves its cells, SCALE the device pixels per logical one.
pub(crate) fn find(sides: &Sides<'_>, changed: &[Key], reach: Reach, scale: f32) -> Found {
    let mut found = Found::default();
    let candidates = distances(sides, changed, scale);
    let mut windows: Vec<i64> = candidates.iter().map(|c| c.0).collect();
    windows.dedup();
    for window in windows {
        // The first distance, best voted first, that places enough groups.
        let Some((dy, placed)) = candidates
            .iter()
            .filter(|c| c.0 == window)
            .take(MAX_TRIES)
            .find_map(|&(_, dy)| {
                let mut placed: Vec<Key> = sides
                    .after
                    .keys()
                    .filter(|key| key.0 == window && moved(sides, **key, dy, reach, scale))
                    .copied()
                    .collect();
                placed.sort_unstable();
                let placed = whole_rows(sides, &placed, dy);
                (placed.len() >= MIN_GROUPS).then_some((dy, placed))
            })
        else {
            continue;
        };
        let Some(band) = placed
            .iter()
            .flat_map(|key| sides.after[key].iter().map(|&at| sides.runs[at].bounds))
            .reduce(damage::union)
        else {
            continue;
        };
        let left = (band.x - reach.left).max(0.0);
        let right = (band.x + band.width + reach.right).min(sides.new.width);
        let Some(area) = snapped(Rect::new(left, band.y, right - left, band.height), scale) else {
            continue;
        };
        let from = Rect::new(area.x, area.y - dy, area.width, area.height);
        if from.y < 0.0 || from.y + from.height > sides.new.height {
            continue;
        }
        found.damage.extend(slivers(band, area));
        found.damage.extend(spoiled(sides, &placed, area, from, dy, reach));
        found.damage.extend(pane_chrome(sides.new, area, from, dy, scale));
        found.damage.extend(ring(band, area, reach));
        found.moves.push(Move { from, dy });
        found.bands.push(area);
        found.placed.extend(placed);
    }
    found.placed.sort_unstable();
    found
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
            if dy != 0.0 && whole(dy, scale) {
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

/// Whether DY logical pixels are a whole number of device pixels at SCALE,
/// exactly.
fn whole(dy: f32, scale: f32) -> bool {
    let device = dy * scale;
    device.to_bits() == device.round().to_bits()
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
/// moved: glyph for glyph equal but for y, each clear of its clip's top and
/// bottom in both places (its sides cut it alike in either) and of any
/// gradient.
fn moved(sides: &Sides<'_>, key: Key, dy: f32, reach: Reach, scale: f32) -> bool {
    let source = (key.0, (f32::from_bits(key.1) - dy).to_bits(), key.2);
    let (Some(was), Some(is)) = (sides.before.get(&source), sides.after.get(&key)) else {
        return false;
    };
    let Some(bounds) = is.iter().map(|&at| sides.runs[at].bounds).reduce(damage::union) else {
        return false;
    };
    let inked = reach.grow(bounds);
    let before = Rect::new(inked.x, inked.y - dy, inked.width, inked.height);
    let inside = |clip: Rect, rect: Rect| clip.y <= rect.y && rect.y + rect.height <= clip.y + clip.height;
    let clear = |glyph: &FrameGlyph| {
        in_phase(sides.new, glyph, dy)
            && clip(glyph).is_none_or(|clip| inside(clip, inked) && inside(clip, before))
            && face(glyph).is_none_or(|face| {
                sides.new.faces.get(&face).is_none_or(|face| face.background_gradient.is_none())
            })
    };
    let mut olds = was.iter().flat_map(|&at| sides.old.frame.glyphs[sides.old.runs[at].glyphs.clone()].iter());
    let mut news = is.iter().flat_map(|&at| sides.new.glyphs[sides.runs[at].glyphs.clone()].iter());
    loop {
        match (olds.next(), news.next()) {
            (None, None) => return true,
            (Some(old), Some(new)) if clear(new) && moved_glyph(old, new, dy) && snaps(old, new, dy, scale) => {}
            _ => return false,
        }
    }
}

/// Whether a character NEW, OLD moved DY down, lands DY lower in device
/// pixels at SCALE: the text pass snaps a baseline to whole device pixels,
/// and the snapped place of a moved one can round apart from the distance.
fn snaps(old: &FrameGlyph, new: &FrameGlyph, dy: f32, scale: f32) -> bool {
    match (old, new) {
        (FrameGlyph::Char { baseline: was, .. }, FrameGlyph::Char { baseline: is, .. }) => {
            ((*is * scale).round() - (*was * scale).round()).to_bits() == (dy * scale).to_bits()
        }
        _ => true,
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

/// Whether GLYPH, moved DY down from where it was, draws as it did: a
/// fringe bitmap that repeats samples its rows by its y, rounded, modulo its
/// period, so it moves only by whole periods.
#[expect(clippy::cast_possible_truncation, reason = "frame pixels, far inside i64")]
fn in_phase(frame: &FrameGlyphBuffer, glyph: &FrameGlyph, dy: f32) -> bool {
    let FrameGlyph::FringeBitmap { y, bitmap_index, .. } = glyph else {
        return true;
    };
    let period = frame.fringe_bitmaps.get(bitmap_index).map_or(0, |bitmap| i64::from(bitmap.period));
    let phase = |y: f32| (y.max(0.0).round() as i64).rem_euclid(period.max(1));
    period == 0 || phase(*y) == phase(*y - dy)
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
fn intersection(a: Rect, b: Rect) -> Option<Rect> {
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

/// RECT on whole device pixels at SCALE: out to them at its sides, in to
/// them at its top and bottom; None if nothing is left.
fn snapped(rect: Rect, scale: f32) -> Option<Rect> {
    let snap = |v: f32, up: bool| {
        let device = v * scale;
        (if up { device.ceil() } else { device.floor() }) / scale
    };
    let (x0, x1) = (snap(rect.x, false), snap(rect.x + rect.width, true));
    let (y0, y1) = (snap(rect.y, true), snap(rect.y + rect.height, false));
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// What of BAND's height AREA, snapped from it, leaves: under a device
/// pixel above and below.
fn slivers(band: Rect, area: Rect) -> [Rect; 2] {
    [
        Rect::new(area.x, band.y, area.width, area.y - band.y),
        Rect::new(area.x, area.y + area.height, area.width, band.y + band.height - area.y - area.height),
    ]
}

/// The strips above and below BAND, as wide as AREA, that ink drawn in it
/// reaches.
fn ring(band: Rect, area: Rect, reach: Reach) -> [Rect; 2] {
    [
        Rect::new(area.x, band.y - reach.up, area.width, reach.up),
        Rect::new(area.x, band.y + band.height, area.width, reach.down),
    ]
}

/// What the move of FROM by DY to BAND leaves wrong of FRAME's panes, drawn
/// over the rows and fixed where they move: a corner or edge the band
/// covers, or one the copy brings from FROM; but a uniform piece (a dim, a
/// side edge) that spans both places, which looks alike moved within itself.
fn pane_chrome(frame: &FrameGlyphBuffer, band: Rect, from: Rect, dy: f32, scale: f32) -> Vec<Rect> {
    if frame.panes.is_empty() {
        return Vec::new();
    }
    let span = (from.y.min(band.y), (from.y + from.height).max(band.y + band.height));
    let spans = |r: &Rect| r.y <= span.0 && r.y + r.height >= span.1;
    let geometry = super::super::panes::pane_geometry(&frame.panes, frame.background, scale);
    let uniform = geometry.dims.iter().chain(&geometry.edges).map(|(rect, _)| *rect).filter(|r| !spans(r));
    let mut out = Vec::new();
    for rect in uniform.chain(geometry.corners.iter().map(|corner| corner.square)) {
        if damage::overlaps(rect, band) {
            out.push(rect);
        }
        if damage::overlaps(rect, from) {
            out.push(Rect::new(rect.x, rect.y + dy, rect.width, rect.height));
        }
    }
    out
}

/// What the move of FROM by DY to BAND leaves wrong in BAND: each group of
/// the new frame there it did not place (PLACED), and each group of the old
/// frame in FROM it did not carry, where that lands; but a solid rectangle
/// spanning both places, which looks alike moved within itself.
fn spoiled(sides: &Sides<'_>, placed: &[Key], band: Rect, from: Rect, dy: f32, reach: Reach) -> Vec<Rect> {
    let span = (from.y.min(band.y), (from.y + from.height).max(band.y + band.height));
    let carried: Vec<Key> = {
        let mut keys: Vec<Key> = placed.iter().map(|k| (k.0, (f32::from_bits(k.1) - dy).to_bits(), k.2)).collect();
        keys.sort_unstable();
        keys
    };
    // A solid rectangle looks alike moved within itself: one that spans
    // both places, as it is drawn (a border, cut by its clip).
    let alike = |glyph: &FrameGlyph| {
        let painted = match glyph {
            FrameGlyph::Background { bounds, .. } => Some(*bounds),
            FrameGlyph::Border { x, y, width, height, clip_rect, .. } => {
                let bounds = Rect::new(*x, *y, *width, *height);
                clip_rect.map_or(Some(bounds), |clip| intersection(bounds, clip))
            }
            _ => return false,
        };
        painted.is_none_or(|r| r.y <= span.0 && r.y + r.height >= span.1)
    };
    let spoils = |frame: &FrameGlyphBuffer, run: &Run, area: Rect| {
        if frame.glyphs[run.glyphs.clone()].iter().all(alike) {
            false
        } else {
            damage::overlaps(reach.grow(run.bounds), area)
        }
    };
    let mut out = Vec::new();
    for (key, at) in &sides.after {
        if placed.binary_search(key).is_err() {
            out.extend(at.iter().map(|&i| &sides.runs[i]).filter(|run| spoils(sides.new, run, band)).map(|run| run.bounds));
        }
    }
    for (key, at) in &sides.before {
        if carried.binary_search(key).is_err() {
            out.extend(
                at.iter()
                    .map(|&i| &sides.old.runs[i])
                    .filter(|run| spoils(&sides.old.frame, run, from))
                    .map(|run| Rect::new(run.bounds.x, run.bounds.y + dy, run.bounds.width, run.bounds.height)),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests;

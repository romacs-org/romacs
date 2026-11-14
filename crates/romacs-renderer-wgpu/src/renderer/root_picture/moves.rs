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

/// The moves that bring a picture of SIDES' old frame toward its new one,
/// whose groups at CHANGED (sorted) differ in place; REACH is how far ink
/// leaves its cells, SCALE the device pixels per logical one.
pub(crate) fn find(sides: &Sides<'_>, changed: &[Key], reach: Reach, scale: f32) -> Found {
    let mut found = Found::default();
    for (window, dy) in distances(sides, changed, scale) {
        let mut placed: Vec<Key> = sides
            .after
            .keys()
            .filter(|key| key.0 == window && moved(sides, **key, dy, reach))
            .copied()
            .collect();
        if placed.len() < MIN_GROUPS {
            continue;
        }
        placed.sort_unstable();
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
        found.damage.extend(ring(band, area, reach));
        found.moves.push(Move { from, dy });
        found.bands.push(area);
        found.placed.extend(placed);
    }
    found.placed.sort_unstable();
    found
}

/// Per window, the distance most of its changed text groups moved by: at
/// least `MIN_GROUPS` of them, by whole device pixels at SCALE.
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
    let mut best: Vec<(i64, usize, f32)> = Vec::new();
    for ((window, dy), n) in votes {
        let dy = f32::from_bits(dy);
        if n < MIN_GROUPS {
            continue;
        }
        let rank = |(n, dy): (usize, f32)| (n, std::cmp::Reverse(dy.abs().to_bits()), dy.to_bits());
        match best.iter_mut().find(|b| b.0 == window) {
            Some(b) if rank((n, dy)) > rank((b.1, b.2)) => *b = (window, n, dy),
            Some(_) => {}
            None => best.push((window, n, dy)),
        }
    }
    best.sort_unstable_by_key(|b| b.0);
    best.into_iter().map(|(window, _, dy)| (window, dy)).collect()
}

/// Whether DY logical pixels are a whole number of device pixels at SCALE.
fn whole(dy: f32, scale: f32) -> bool {
    let device = dy * scale;
    (device - device.round()).abs() < 1e-3
}

/// A hash of the glyphs at AT of FRAME's RUNS that leaves their y out.
fn signature(frame: &FrameGlyphBuffer, runs: &[Run], at: &[usize]) -> u64 {
    let mut hasher = FxHasher::default();
    for glyph in at.iter().flat_map(|&i| frame.glyphs[runs[i].glyphs.clone()].iter()) {
        std::mem::discriminant(glyph).hash(&mut hasher);
        if let FrameGlyph::Char { char, face_id, .. } = glyph {
            char.hash(&mut hasher);
            face_id.get().hash(&mut hasher);
        }
        if let Some(bounds) = glyph.geometry() {
            bounds.x.to_bits().hash(&mut hasher);
            bounds.width.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Whether the new frame's group KEY is the old frame's group DY above it,
/// moved: glyph for glyph equal but for y, each clear of its clip's top and
/// bottom in both places (its sides cut it alike in either) and of any
/// gradient.
fn moved(sides: &Sides<'_>, key: Key, dy: f32, reach: Reach) -> bool {
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
        clip(glyph).is_none_or(|clip| inside(clip, inked) && inside(clip, before))
            && face(glyph).is_none_or(|face| {
                sides.new.faces.get(&face).is_none_or(|face| face.background_gradient.is_none())
            })
    };
    let mut olds = was.iter().flat_map(|&at| sides.old.frame.glyphs[sides.old.runs[at].glyphs.clone()].iter());
    let mut news = is.iter().flat_map(|&at| sides.new.glyphs[sides.runs[at].glyphs.clone()].iter());
    loop {
        match (olds.next(), news.next()) {
            (None, None) => return true,
            (Some(old), Some(new)) if clear(new) && moved_glyph(old, new, dy) => {}
            _ => return false,
        }
    }
}

/// Whether NEW is OLD moved DY down, its slot aside.
fn moved_glyph(old: &FrameGlyph, new: &FrameGlyph, dy: f32) -> bool {
    let mut moved = old.clone();
    match &mut moved {
        FrameGlyph::Char { y, baseline, slot_id, .. } => {
            *y += dy;
            *baseline += dy;
            *slot_id = new.slot_id().unwrap_or(*slot_id);
        }
        FrameGlyph::Stretch { y, slot_id, .. } => {
            *y += dy;
            *slot_id = new.slot_id().unwrap_or(*slot_id);
        }
        FrameGlyph::Image { y, slot_rect, box_rect, slot_id, .. } => {
            *y += dy;
            slot_rect.y += dy;
            box_rect.y += dy;
            *slot_id = new.slot_id();
        }
        FrameGlyph::FringeBitmap { y, .. } | FrameGlyph::Border { y, .. } => *y += dy,
        FrameGlyph::Background { bounds, .. } => bounds.y += dy,
        // Drawn from outside the frame's glyphs (a video, a surface, a
        // widget), or not by row (a scroll bar).
        _ => return false,
    }
    moved == *new
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
        let near = device.round();
        let device = if (device - near).abs() < 1e-3 {
            near
        } else if up {
            device.ceil()
        } else {
            device.floor()
        };
        device / scale
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
    let spoils = |frame: &FrameGlyphBuffer, run: &Run, area: Rect| {
        let solid = frame.glyphs[run.glyphs.clone()]
            .iter()
            .all(|g| matches!(g, FrameGlyph::Background { .. } | FrameGlyph::Border { .. }));
        if solid {
            damage::overlaps(run.bounds, area)
                && !(run.bounds.y <= span.0 && run.bounds.y + run.bounds.height >= span.1)
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

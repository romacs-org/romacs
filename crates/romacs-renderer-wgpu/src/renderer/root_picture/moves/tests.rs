use romacs_display_protocol::frame_glyphs::{FrameGlyphBuffer, GlyphRowRole};
use romacs_display_protocol::types::{Color, DisplayWindowId, Rect};

use super::super::damage::{self, Indexed, Plan};
use super::super::reach::Reach;
use super::{Move, moved_glyph, snapped, whole};

const ROW: f32 = 10.0;

/// A window of LINES from FIRST, ROWS of them, clipped to them, above a
/// mode line.
#[expect(clippy::cast_precision_loss, reason = "a test's few rows and columns")]
fn window(lines: &[&str], first: usize, rows: usize) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(100.0, ROW * (rows + 1) as f32);
    let body = ROW * rows as f32;
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, Some(Rect::new(0.0, 0.0, 100.0, body)));
    frame.add_background(0.0, 0.0, 100.0, body, Color::BLACK);
    for (row, line) in lines.iter().skip(first).take(rows).enumerate() {
        for (col, ch) in line.chars().enumerate() {
            frame.add_char(ch, 1.0 + 6.0 * col as f32, ROW * row as f32, 6.0, ROW, 8.0, false);
        }
    }
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::ModeLine, None);
    frame.add_char('M', 1.0, body, 6.0, ROW, 8.0, false);
    frame
}

fn plan(old: &FrameGlyphBuffer, new: &FrameGlyphBuffer, scale: f32) -> Plan {
    let old = Indexed::new(old.clone());
    damage::plan(&old, new, &damage::runs(&new.glyphs), Reach::default(), scale)
}

const LINES: [&str; 8] = ["one", "two", "three", "four", "five", "six", "seven", "eight"];

#[test]
fn a_scroll_by_a_line_moves_the_rows_that_stay_and_repaints_the_one_that_entered() {
    let Plan::Regions { moves, regions, rows_kept } = plan(&window(&LINES, 0, 5), &window(&LINES, 1, 5), 1.0) else {
        panic!("a scroll is a move and a region");
    };
    // Rows two to five, at 10..50 and as wide as "three", move up a row;
    // six enters at 40..50, where five was.
    assert_eq!(moves, [Move { from: Rect::new(1.0, 10.0, 6.0 * 5.0, 40.0), dy: -ROW }]);
    assert_eq!(regions, [Rect::new(1.0, 40.0, 6.0 * 4.0, ROW)]);
    assert_eq!(rows_kept, 5, "four moved, and the mode line");
}

#[test]
fn a_scroll_down_by_two_lines_moves_the_rows_that_stay_down() {
    let Plan::Regions { moves, .. } = plan(&window(&LINES, 3, 5), &window(&LINES, 1, 5), 1.0) else {
        panic!("a scroll is a move");
    };
    assert_eq!(moves.iter().map(|m| (m.from.y, m.dy)).collect::<Vec<_>>(), [(0.0, 2.0 * ROW)]);
}

#[test]
fn a_jump_past_a_window_moves_nothing() {
    let plan = plan(&window(&LINES, 0, 3), &window(&LINES, 5, 3), 1.0);
    assert!(!matches!(&plan, Plan::Regions { moves, .. } if !moves.is_empty()), "{plan:?}");
}

#[test]
fn a_row_moved_across_its_clip_is_repainted() {
    // The window's clip cuts its last row: that row moves out of the cut,
    // and is drawn anew.
    let mut new = window(&LINES, 1, 5);
    let mut old = window(&LINES, 0, 5);
    for frame in [&mut new, &mut old] {
        for glyph in &mut frame.glyphs {
            if let romacs_display_protocol::frame_glyphs::FrameGlyph::Char { clip_rect: Some(clip), .. } = glyph {
                clip.height = 45.0;
            }
        }
    }
    let Plan::Regions { moves, regions, .. } = plan(&old, &new, 1.0) else {
        panic!("a scroll is a move and regions");
    };
    assert_eq!(moves.iter().map(|m| (m.from.y, m.from.height)).collect::<Vec<_>>(), [(10.0, 30.0)], "rows two to four");
    assert!(regions.iter().any(|r| r.y <= 30.0 && r.y + r.height >= 50.0), "rows five and six: {regions:?}");
}

#[test]
fn a_distance_is_whole_device_pixels_and_a_band_snaps_out_at_its_sides_and_in_at_its_ends() {
    assert!(whole(-10.0, 1.5) && !whole(-11.0, 1.5) && whole(-11.0, 1.0));
    // At 1.5, 0.75..15.75 and 1.5..31.5 device pixels: 0..16 and 2..31.
    let band = snapped(Rect::new(0.5, 1.0, 10.0, 20.0), 1.5).unwrap();
    let device = [band.x, band.y, band.x + band.width, band.y + band.height].map(|v| (v * 1.5).round());
    assert_eq!(device, [0.0, 2.0, 16.0, 31.0]);
    assert_eq!(snapped(Rect::new(0.0, 0.0, 10.0, 0.5), 1.0), None);
}

#[test]
fn a_glyph_moved_is_the_same_but_for_its_y_and_its_slot() {
    use romacs_display_protocol::frame_glyphs::FrameGlyph;
    use romacs_display_protocol::types::FaceId;
    let frame = window(&LINES, 0, 1);
    for old in frame.glyphs.iter().filter(|g| matches!(g, FrameGlyph::Char { .. } | FrameGlyph::Background { .. })) {
        let mut new = old.clone();
        match &mut new {
            FrameGlyph::Char { y, baseline, slot_id, .. } => {
                (*y, *baseline, slot_id.row) = (*y + ROW, *baseline + ROW, slot_id.row + 1);
            }
            FrameGlyph::Background { bounds, .. } => bounds.y += ROW,
            _ => unreachable!(),
        }
        assert!(moved_glyph(old, &new, ROW), "{old:?}");
        assert!(!moved_glyph(old, &new, 2.0 * ROW), "another distance: {old:?}");
        if let FrameGlyph::Char { face_id, .. } = &mut new {
            *face_id = FaceId::new(9);
            assert!(!moved_glyph(old, &new, ROW), "another face");
        }
    }
}

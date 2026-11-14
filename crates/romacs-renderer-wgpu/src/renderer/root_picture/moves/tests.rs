use romacs_display_protocol::frame_glyphs::{FrameGlyphBuffer, GlyphRowRole};
use romacs_display_protocol::types::{Color, DisplayWindowId, Rect};

use super::super::damage::{self, Indexed, Plan};
use super::super::reach::Reach;
use super::{Move, moved_glyph, on_pixels, outward};

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
fn a_distance_is_whole_device_pixels_exactly_and_a_band_grows_out_to_them() {
    assert!(on_pixels(-10.0, 1.5) && !on_pixels(-11.0, 1.5) && on_pixels(-11.0, 1.0));
    // A hair from twelve device pixels at 1.25 is not twelve.
    assert!(!on_pixels(-9.6001, 1.25));
    // At 1.5, 0.75..15.75 and 1.5..31.5 device pixels: 0..16 and 1..32.
    let band = outward(Rect::new(0.5, 1.0, 10.0, 20.0), 1.5);
    let device = [band.x, band.y, band.x + band.width, band.y + band.height].map(|v| (v * 1.5).round());
    assert_eq!(device, [0.0, 1.0, 16.0, 32.0]);
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

#[test]
fn rows_alike_at_their_ends_still_move_the_way_they_went() {
    // Every row has three glyphs, an "a" first and a "z" last: their
    // signatures agree, and votes tie between the two directions.
    let lines = ["a0z", "a1z", "a2z", "a3z", "a4z", "a5z", "a6z"];
    for (old, new, dy) in [(1, 0, ROW), (0, 1, -ROW), (2, 0, 2.0 * ROW), (0, 2, -2.0 * ROW)] {
        let Plan::Regions { moves, .. } = plan(&window(&lines, old, 5), &window(&lines, new, 5), 1.0) else {
            panic!("a scroll is a move");
        };
        assert_eq!(moves.iter().map(|m| m.dy).collect::<Vec<_>>(), [dy], "from line {old} to {new}");
    }
}

#[test]
fn a_row_moves_whole_or_not_at_all() {
    use romacs_display_protocol::frame_glyphs::FrameGlyph;
    use romacs_display_protocol::types::FaceId;
    // "three" set as two groups of one visual row: its tail a pixel
    // shorter. As the window scrolls a line, its head changes face; its tail
    // does not, but draws with its head (an underline takes its line from
    // the run's first glyph), so it is repainted, not moved.
    let shape = |frame: &mut FrameGlyphBuffer, row: f32, face: Option<FaceId>| {
        for glyph in &mut frame.glyphs {
            if let FrameGlyph::Char { x, y, height, face_id, .. } = glyph
                && (*y - row * ROW).abs() < 0.5
            {
                if *x >= 13.0 {
                    *height = ROW - 1.0;
                } else if let Some(face) = face {
                    *face_id = face;
                }
            }
        }
    };
    let mut old = window(&LINES, 0, 5);
    shape(&mut old, 2.0, None);
    let mut new = window(&LINES, 1, 5);
    shape(&mut new, 1.0, Some(FaceId::new(7)));
    // The row lies among the rows that moved: no band moves round it, and
    // its tail is repainted with its head.
    let tail = Rect::new(13.0, ROW, 18.0, ROW - 1.0);
    match plan(&old, &new, 1.0) {
        Plan::Whole => {}
        Plan::Regions { regions, .. } => assert!(
            regions.iter().any(|r| r.x <= tail.x && r.y <= tail.y && r.x + r.width >= tail.x + tail.width && r.y + r.height >= tail.y + tail.height),
            "the tail is repainted with its head: {regions:?}"
        ),
        Plan::Clean { .. } => panic!("the frame changed"),
    }
}

#[test]
fn ink_of_a_row_that_left_reaching_past_the_band_is_repainted() {
    // The rows that stay end at x=25; ink reaches 10 past a cell, so the band
    // copied ends at 35. The row that leaves ends at 31: inside the band, its
    // ink out to 41 is not.
    let lines = ["gone!", "abc1", "abc2", "abc3", "abc4", "abc5", "abc6"];
    let old = Indexed::new(window(&lines, 0, 5));
    let new = window(&lines, 1, 5);
    let reach = Reach { right: 10.0, ..Reach::default() };
    let Plan::Regions { moves, regions, .. } = damage::plan(&old, &new, &damage::runs(&new.glyphs), reach, 1.0) else {
        panic!("a scroll is a move and regions");
    };
    assert_eq!(moves.len(), 1);
    assert!(regions.iter().any(|r| r.y < ROW && r.y + r.height > 0.0), "where the row that left was: {regions:?}");
}

#[test]
fn a_band_moves_under_a_pane_only_clear_of_its_edges_and_corners() {
    // The pane's chrome is drawn over the rows and stays where they move:
    // with its top edge and corners on the rows, nothing moves; with the
    // pane round the rows, clear of them, the rows move.
    let moves = |pane: Rect| {
        let mut old = window(&LINES, 0, 5);
        let mut new = window(&LINES, 1, 5);
        for frame in [&mut old, &mut new] {
            frame.panes.push(romacs_display_protocol::frame_glyphs::Pane {
                bounds: pane,
                radius: 6.0,
                edge: Some(Color::WHITE),
                dim: 0.0,
            });
        }
        match plan(&old, &new, 1.0) {
            Plan::Regions { moves, .. } => moves.len(),
            _ => 0,
        }
    };
    assert_eq!(moves(Rect::new(0.0, 0.0, 100.0, 50.0)), 0);
    assert_eq!(moves(Rect::new(-20.0, -20.0, 140.0, 100.0)), 1);
}

#[test]
fn a_band_moves_only_on_whole_device_pixels_at_a_whole_scale() {
    use romacs_display_protocol::frame_glyphs::FrameGlyph;
    let moves = |old: &FrameGlyphBuffer, new: &FrameGlyphBuffer, scale| match plan(old, new, scale) {
        Plan::Regions { moves, .. } => moves.len(),
        _ => 0,
    };
    let (old, new) = (window(&LINES, 0, 5), window(&LINES, 1, 5));
    assert_eq!((moves(&old, &new, 1.0), moves(&old, &new, 2.0)), (1, 1));
    assert_eq!(moves(&old, &new, 1.5), 0, "a scale that is no whole number");
    // A baseline half a pixel off the device grid: its row is drawn anew.
    let (mut old, mut new) = (old, new);
    for frame in [&mut old, &mut new] {
        for glyph in &mut frame.glyphs {
            if let FrameGlyph::Char { baseline, .. } = glyph {
                *baseline += 0.5;
            }
        }
    }
    assert_eq!(moves(&old, &new, 1.0), 0);
}

#[test]
fn rows_alike_but_inside_still_find_a_scroll_of_many_lines() {
    // Fifteen rows alike at their ends and middles, told apart by one
    // character each: a scroll of five lines either way is found.
    let lines: Vec<String> = (0..15).map(|i| format!("a{}m{}z", (b'a' + i) as char, (b'a' + i) as char)).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let lines = &lines[..];
    let tall = |first| {
        let mut frame = FrameGlyphBuffer::with_size(100.0, ROW * 11.0);
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, Some(Rect::new(0.0, 0.0, 100.0, ROW * 10.0)));
        for (row, line) in lines.iter().skip(first).take(10).enumerate() {
            for (col, ch) in line.chars().enumerate() {
                frame.add_char(ch, 1.0 + 6.0 * col as f32, ROW * row as f32, 6.0, ROW, 8.0, false);
            }
        }
        frame
    };
    for (old, new, dy) in [(5, 0, 5.0 * ROW), (0, 5, -5.0 * ROW)] {
        let Plan::Regions { moves, .. } = plan(&tall(old), &tall(new), 1.0) else {
            panic!("a scroll is a move");
        };
        assert_eq!(moves.iter().map(|m| m.dy).collect::<Vec<_>>(), [dy], "from line {old} to {new}");
    }
}

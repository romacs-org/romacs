use std::collections::HashMap;

use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer, GlyphRowRole};
use romacs_display_protocol::types::{Color, DisplayWindowId, FaceId, Rect};

use super::super::reach::{Inks, Reach, Rows};
use super::{Indexed, Plan, agree, merge, nested_agree, plan, repeats, runs};

const ROW: f32 = 20.0;

/// A 400x400 frame: window 1 on its left half shows LEFT, window 2 on its
/// right half RIGHT, rows 20 px tall, each window's background first.
#[expect(clippy::cast_precision_loss, reason = "a test's few rows and columns")]
fn frame(left: &[&str], right: &[&str]) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(400.0, 400.0);
    frame.background = Color::BLACK;
    frame.set_face(
        FaceId::new(0),
        Color::WHITE,
        None,
        400,
        false,
        0,
        None,
        0,
        None,
        0,
        None,
    );
    for (window, x0, lines) in [(1, 0.0, left), (2, 200.0, right)] {
        frame.set_draw_context(DisplayWindowId::new(window), GlyphRowRole::Text, None);
        frame.add_background(x0, 0.0, 200.0, 400.0, Color::BLACK);
        for (row, line) in lines.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                frame.add_char(
                    ch,
                    x0 + col as f32 * 10.0,
                    row as f32 * ROW,
                    10.0,
                    ROW,
                    15.0,
                    false,
                );
            }
        }
    }
    frame
}

fn planned(old: &FrameGlyphBuffer, new: &FrameGlyphBuffer) -> Plan {
    plan(&Indexed::new(old.clone()), new, &runs(&new.glyphs), Inks { all: Reach::default(), rows: &Rows::default() }, 1.0)
}

fn regions(plan: Plan) -> Vec<Rect> {
    match plan {
        Plan::Regions { regions, .. } => regions,
        other => panic!("regions, not {other:?}"),
    }
}

#[test]
fn an_equal_frame_repaints_nothing_whatever_its_cursors() {
    let old = frame(&["one", "two", "three"], &[]);
    let mut new = old.clone();
    new.presentation_id = romacs_display_protocol::PresentationId::new(9);
    new.window_cursors.clear();
    assert_eq!(planned(&old, &new), Plan::Clean { rows: 3 });
}

#[test]
fn a_repeat_of_the_layout_needs_no_compare_but_a_new_setting_does() {
    let mut old = frame(&["one"], &[]);
    old.layout_identity = 7;
    let mut new = frame(&["two"], &[]);
    new.layout_identity = 7;
    assert!(repeats(&old, &new), "the layout's promise is taken");
    new.background = Color::WHITE;
    assert!(!repeats(&old, &new));
    old.layout_identity = 0;
    assert!(!repeats(&old, &old.clone()), "identity zero promises nothing");
}

#[test]
fn a_character_typed_repaints_its_row_of_its_window_alone() {
    let lines = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    let old = frame(&lines, &lines);
    let mut typed = lines;
    typed[2] = "thrxee";
    let new = frame(&typed, &lines);
    assert_eq!(
        planned(&old, &new),
        Plan::Regions {
            moves: Vec::new(),
            regions: vec![Rect::new(0.0, 40.0, 60.0, 20.0)],
            rows_kept: 19
        }
    );
}

#[test]
fn rows_changed_apart_are_regions_apart_and_meeting_ones_join() {
    let lines = ["aaaa"; 12];
    let old = frame(&lines, &[]);
    let mut new = lines;
    new[1] = "b";
    new[2] = "bbbbbb";
    new[9] = "b";
    assert_eq!(
        regions(planned(&old, &frame(&new, &[]))),
        vec![
            Rect::new(0.0, 20.0, 60.0, 40.0),
            Rect::new(0.0, 180.0, 40.0, 20.0)
        ],
        "rows that meet are one region"
    );
    let side_by_side = [
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(10.0, 2.0, 5.0, 6.0),
        Rect::new(30.0, 0.0, 1.0, 1.0),
    ];
    assert_eq!(
        merge(side_by_side.to_vec()),
        vec![Rect::new(0.0, 0.0, 15.0, 10.0), Rect::new(30.0, 0.0, 1.0, 1.0)],
        "a row of glyphs of two heights"
    );
}

#[test]
fn a_row_that_moved_damages_where_it_was_and_where_it_is() {
    let old = frame(&["x", "", "", "", "", "", "", "", "", "y"], &[]);
    let new = frame(&["x", "", "", "", "", "", "", "", "y", ""], &[]);
    assert_eq!(
        regions(planned(&old, &new)),
        vec![Rect::new(0.0, 160.0, 10.0, 40.0)]
    );
}

#[test]
fn many_regions_or_most_of_the_frame_repaint_it_whole() {
    // Rows as wide as their window.
    let (a, b) = ("a".repeat(20), "b".repeat(20));
    let lines = |pick: &dyn Fn(usize) -> bool| -> Vec<&str> {
        (0..20)
            .map(|i| if pick(i) { b.as_str() } else { a.as_str() })
            .collect()
    };
    let old = frame(&lines(&|_| false), &lines(&|_| false));
    assert_eq!(
        planned(&old, &frame(&lines(&|_| true), &lines(&|i| i == 0))),
        Plan::Whole,
        "over half the frame's area"
    );
    assert_eq!(
        planned(&old, &frame(&lines(&|i| i % 2 == 0), &lines(&|_| false))),
        Plan::Whole,
        "ten regions are ten passes"
    );
    assert!(matches!(
        planned(&old, &frame(&lines(&|i| i % 4 == 0), &lines(&|_| false))),
        Plan::Regions { .. }
    ));
}

#[test]
fn a_change_of_setting_or_of_a_face_both_frames_name_repaints_whole() {
    let old = frame(&["one", "two"], &[]);
    let mut background = old.clone();
    background.background = Color::WHITE;
    assert_eq!(planned(&old, &background), Plan::Whole);
    let mut face = old.clone();
    face.faces.get_mut(&FaceId::new(0)).unwrap().foreground = Color::rgb(1.0, 0.0, 0.0);
    assert_eq!(planned(&old, &face), Plan::Whole);
    let mut added = old.clone();
    added.set_face(
        FaceId::new(7),
        Color::WHITE,
        None,
        700,
        false,
        0,
        None,
        0,
        None,
        0,
        None,
    );
    assert_eq!(
        planned(&old, &added),
        Plan::Clean { rows: 2 },
        "a face no glyph names paints nothing"
    );
}

#[test]
fn tables_agree_on_the_entries_both_hold() {
    let a = HashMap::from([(1, 'a'), (2, 'b')]);
    assert!(agree(&a, &HashMap::from([(1, 'a'), (3, 'c')])));
    assert!(!agree(&a, &HashMap::from([(2, 'x')])));
    let nested = |inner: HashMap<u32, char>| HashMap::from([(0u32, inner)]);
    assert!(nested_agree(
        &nested(a.clone()),
        &nested(HashMap::from([(1, 'a'), (5, 'e')]))
    ));
    assert!(!nested_agree(&nested(a), &nested(HashMap::from([(1, 'z')]))));
}

#[test]
fn changes_on_too_many_runs_are_a_whole_repaint_before_they_are_joined() {
    // Every other row of a tall frame changed: hundreds of runs, none
    // meeting, which joining would compare pairwise.
    let tall = |pick: bool| {
        let mut f = FrameGlyphBuffer::with_size(400.0, 20_000.0);
        f.set_face(FaceId::new(0), Color::WHITE, None, 400, false, 0, None, 0, None, 0, None);
        f.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
        for row in 0..1000u16 {
            let ch = if pick && row % 2 == 0 { 'b' } else { 'a' };
            f.add_char(ch, 0.0, f32::from(row) * ROW, 10.0, ROW, 15.0, false);
        }
        f
    };
    assert_eq!(planned(&tall(false), &tall(true)), Plan::Whole);
}

#[test]
fn text_runs_that_overlap_in_one_window_share_a_row_and_nothing_else_does() {
    let mut f = frame(&["ab", "cd"], &["ef"]);
    // The first row's second glyph two pixels shorter: a run of its own,
    // inside its row.
    if let Some(FrameGlyph::Char { height, .. }) = f.glyphs.iter_mut().filter(|g| matches!(g, FrameGlyph::Char { .. })).nth(1) {
        *height = ROW - 2.0;
    }
    let runs = runs(&f.glyphs);
    let row_of = |ch: char| {
        let at = f.glyphs.iter().position(|g| matches!(g, FrameGlyph::Char { char: c, .. } if *c == ch)).unwrap();
        runs.iter().find(|run| run.glyphs.contains(&at)).unwrap().row
    };
    assert_eq!(row_of('a'), row_of('b'), "two heights, one row");
    assert_ne!(row_of('a'), row_of('c'), "rows that only meet are apart");
    assert_ne!(row_of('a'), row_of('e'), "the other window's row is its own");
    let backgrounds: Vec<usize> = runs.iter().filter(|run| !run.text).map(|run| run.row).collect();
    assert!(backgrounds.iter().all(|row| runs.iter().filter(|run| run.row == *row).count() == 1));
}

#[test]
fn a_run_s_bounds_are_its_glyphs_bit_for_bit() {
    // A lone glyph's run is its rectangle exactly, at coordinates whose
    // width a rebuild from its edges would round (15.719279 to 15.719238);
    // a longer run is the rectangles joined one by one.
    let mut frame = FrameGlyphBuffer::with_size(2000.0, 100.0);
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    let (x, width) = (1_480.890_5, 15.719_279);
    frame.add_char('a', x, 0.0, width, ROW, 15.0, false);
    frame.add_char('b', 10.3, 40.0, 7.7, ROW, 15.0, false);
    frame.add_char('c', 18.0, 40.0, 7.7, ROW, 15.0, false);
    let runs = runs(&frame.glyphs);
    let bits = |r: Rect| [r.x, r.y, r.width, r.height].map(f32::to_bits);
    assert_eq!(bits(runs[0].bounds), bits(Rect::new(x, 0.0, width, ROW)));
    let joined = super::union(Rect::new(10.3, 40.0, 7.7, ROW), Rect::new(18.0, 40.0, 7.7, ROW));
    assert_eq!(bits(runs[1].bounds), bits(joined));
}

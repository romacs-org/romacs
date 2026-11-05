//! An emoji or variation sequence is one composed glyph, measured as
//! painted, with point inside it on that glyph (GNU's composition).
use romacs_display_protocol::{Color, FrameGlyph, FrameGlyphBuffer};
use romacs_layout2::LayoutEngine;
use romacs_layout_input::{Face, FaceHandle, FaceRun, FrameSnapshot, MeasureInput, PlacedWindowSnapshot, TextSlice, WindowSnapshot};

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
const HEART: &str = "\u{2764}\u{FE0F}";

fn face() -> Face {
    Face { decor: Default::default(), foreground: [255; 3], background: [0; 3], family: "Liberation Mono".into(), pixel_size: 20.0, weight: 400, italic: false, width: 5 }
}

fn slice(text: &str) -> TextSlice {
    TextSlice { extends: vec![], raw: vec![], utf8: text.into(), face_runs: vec![] }
}

fn lay_out(text: &str, point_byte: usize) -> FrameGlyphBuffer {
    lay_out_in(slice(text), point_byte, 400, vec![face()])
}

fn lay_out_in(text: TextSlice, point_byte: usize, width: u32, faces: Vec<Face>) -> FrameGlyphBuffer {
    let window = WindowSnapshot { underline_rules: Default::default(), id: 1, width, height: 100, start_byte: 0, point_byte, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, hscroll: Default::default(),
        tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(),
        text, faces };
    let frame = FrameSnapshot { font_selection: Default::default(), font_alternatives: Default::default(), width: 400, height: 100,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: window }], background_alpha: 1.0, children: vec![], tab_bar: None,
        chrome: Default::default(), status_bar: None, no_font: Default::default() };
    LayoutEngine::new().unwrap().layout_frame(&frame, 1, Some((1, Color::from_pixel(0xffffff)))).unwrap().0
}

fn measured(text: &str) -> Vec<u32> {
    let input = MeasureInput { font_selection: Default::default(), font_alternatives: Default::default(), faces: vec![face()], text: slice(text), no_font: Default::default() };
    LayoutEngine::new().unwrap().measure(&input).unwrap().glyphs.iter().map(|g| g.0).collect()
}

/// Each text character glyph: its character, composed text, x and width.
fn glyphs(frame: &FrameGlyphBuffer) -> Vec<(char, Option<String>, f32, f32)> {
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, composed, x, width, .. } => Some((*char, composed.as_deref().map(str::to_owned), *x, *width)),
        _ => None,
    }).collect()
}

#[test]
fn a_zwj_sequence_is_one_glyph_as_wide_as_measured() {
    let text = format!("a{FAMILY}b");
    let shown = glyphs(&lay_out(&text, 0));
    assert_eq!(shown.iter().map(|g| g.0).collect::<String>(), "a\u{1F468}b", "{shown:?}");
    assert_eq!(shown[1].1.as_deref(), Some(FAMILY));
    let widths = measured(&text);
    // a, the sequence's five characters, b: the glyph at the first, the rest none.
    assert_eq!(widths.len(), 7);
    assert_eq!(widths[1] as f32, shown[1].3);
    assert!(widths[2..6].iter().all(|&w| w == 0), "{widths:?}");
    assert_eq!(shown[2].2, shown[1].2 + shown[1].3, "b follows the one glyph");
}

#[test]
fn vs16_composes_its_base_for_the_emoji_presentation() {
    let text = format!("{HEART} \u{2764}");
    let shown = glyphs(&lay_out(&text, 0));
    assert_eq!(shown[0].1.as_deref(), Some(HEART), "{shown:?}");
    assert_eq!(shown.last().unwrap().1, None, "a heart without VS-16 keeps its text presentation");
    // Its glyph is published from the emoji font, not shaped from the text font.
    let frame = lay_out(&text, 0);
    let published = frame.shaped_clusters.values().find_map(|by_text| by_text.get(HEART)).expect("VS-16's glyph is published");
    let emoji = &frame.fonts[&published[0].resolved_font_id];
    assert_ne!(emoji.family, "Liberation Mono", "{}", emoji.family);
    assert_eq!(published[0].x_advance, shown[0].3, "measured as painted");
}

#[test]
fn point_inside_a_sequence_shows_on_its_glyph() {
    let text = format!("a{FAMILY}b");
    let frame = lay_out(&text, 1 + "\u{1F468}\u{200D}".len());
    let shown = glyphs(&frame);
    let cursor = &frame.window_cursors[0];
    assert_eq!((cursor.x, cursor.width), (shown[1].2, shown[1].3));
}


#[test]
fn a_face_boundary_inside_vs16_keeps_the_emoji_glyph() {
    let base = "\u{2764}".len();
    for runs in [vec![FaceRun { bytes: 0..base, face: FaceHandle(1) }], vec![FaceRun { bytes: base..HEART.len(), face: FaceHandle(1) }]] {
        let mut other = face();
        other.foreground = [255, 0, 0];
        let frame = lay_out_in(TextSlice { face_runs: runs.clone(), ..slice(HEART) }, 0, 400, vec![face(), other]);
        let published = frame.shaped_clusters.values().find_map(|by_text| by_text.get(HEART)).unwrap_or_else(|| panic!("{runs:?}"));
        assert_ne!(frame.fonts[&published[0].resolved_font_id].family, "Liberation Mono", "{runs:?}");
        assert_eq!(glyphs(&frame)[0].3, published[0].x_advance, "{runs:?}");
    }
}

#[test]
fn no_row_breaks_inside_a_sequence_wider_than_the_window() {
    let wide = lay_out(FAMILY, 0);
    let glyph = glyphs(&wide)[0].3;
    let starts: Vec<usize> = FAMILY.char_indices().map(|(at, _)| at).collect();
    for at in starts {
        let frame = lay_out_in(slice(&format!("{FAMILY}\n")), at, glyph as u32 - 4, vec![face()]);
        let cursor = &frame.window_cursors[0];
        assert_eq!((cursor.x, cursor.y), (0.0, 0.0), "point at byte {at}");
        assert_eq!(glyphs(&frame).iter().filter(|g| g.1.is_some()).count(), 1);
    }
}

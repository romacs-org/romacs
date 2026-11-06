//! A character no installed font has a glyph for shows as GNU's glyphless
//! glyph for the frame's `no-font' method, measured as it is painted,
//! and never stops layout.
use romacs_display_protocol::{FrameGlyph, FrameGlyphBuffer};
use romacs_layout2::LayoutEngine;
use romacs_layout_input::{BandMarks, Face, FaceHandle, FaceRun, FrameSnapshot, MeasureInput, NoFont, PlacedWindowSnapshot, Segment, StatusBarSnapshot, TabBarSnapshot, TextSlice, WindowSnapshot};

/// An unassigned code point: no font has a glyph for it.
const NONE: char = '\u{0378}';

fn face(family: &str) -> Face {
    Face { decor: Default::default(), foreground: [255, 255, 255], background: [0, 0, 64], family: family.into(), pixel_size: 20.0, weight: 400, italic: false, width: 5 }
}

fn slice(text: &str) -> TextSlice {
    TextSlice { extends: vec![], raw: vec![], utf8: text.into(), face_runs: vec![] }
}

fn window(text: &str, faces: Vec<Face>) -> WindowSnapshot {
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(), id: 1, width: 400, height: 100, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: Some(slice(&format!("m{NONE}l"))), mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], images: vec![], wrap_flips: vec![],
        line_numbers: None, chrome: Default::default(), text: slice(text), faces }
}

fn frame(text: &str, no_font: NoFont) -> FrameSnapshot {
    FrameSnapshot { font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), width: 400, height: 100,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: window(text, vec![face("Liberation Mono")]) }], background_alpha: 1.0,
        children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, no_font }
}

fn lay_out(text: &str, no_font: NoFont) -> FrameGlyphBuffer {
    LayoutEngine::new().unwrap().layout_frame(&frame(text, no_font), 1, None).expect("a character no font has never stops layout").0
}

fn measured(text: &str, no_font: NoFont) -> Vec<u32> {
    let input = MeasureInput { font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), faces: vec![face("Liberation Mono")],
        text: slice(text), no_font };
    LayoutEngine::new().unwrap().measure(&input).expect("measuring a character no font has succeeds").glyphs.iter().map(|g| g.0).collect()
}

/// The text row's characters: each one's x and width.
fn chars(frame: &FrameGlyphBuffer) -> Vec<(char, f32, f32, f32)> {
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, width, row_role: romacs_display_protocol::GlyphRowRole::Text, .. } => Some((*char, *x, *y, *width)),
        _ => None,
    }).collect()
}

/// The stretches in the text row whose face's background is FG: the box's bars.
fn bars(frame: &FrameGlyphBuffer, fg: [u8; 3]) -> Vec<(f32, f32, f32, f32)> {
    let fg = romacs_display_protocol::Color::from_pixel((u32::from(fg[0]) << 16) | (u32::from(fg[1]) << 8) | u32::from(fg[2]));
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Stretch { x, y, width, height, face_id, row_role: romacs_display_protocol::GlyphRowRole::Text, .. }
            if frame.faces.get(face_id).is_some_and(|f| f.background == fg) => Some((*x, *y, *width, *height)),
        _ => None,
    }).collect()
}

#[test]
fn hex_code_paints_a_box_of_the_code_as_wide_as_measured() {
    let frame = lay_out(&format!("a{NONE}b"), NoFont::HexCode);
    let widths = measured(&format!("a{NONE}b"), NoFont::HexCode);
    let shown = chars(&frame);
    let (a, b) = (shown.iter().find(|c| c.0 == 'a').unwrap(), shown.iter().find(|c| c.0 == 'b').unwrap());
    // `b' starts where the glyph the measurement answered ends.
    assert_eq!(b.1 - (a.1 + a.3), widths[1] as f32, "{shown:?} {widths:?}");
    assert!(widths[1] > widths[0], "GNU's two rows of four digits and bars are wider than a cell here: {widths:?}");
    // Its code in two rows, "03" over "78", in smaller characters, inside it.
    let digits: Vec<_> = shown.iter().filter(|c| c.1 > a.1 && c.1 < b.1).collect();
    assert_eq!(digits.iter().map(|c| c.0).collect::<String>(), "0378", "{shown:?}");
    assert!(digits.iter().all(|c| c.3 < a.3), "the code is in `glyphless-char''s smaller font: {digits:?}");
    // Its box: four one-pixel bars round the glyph.
    let bars = bars(&frame, [255, 255, 255]);
    let x0 = a.1 + a.3;
    assert_eq!(bars.len(), 4, "{bars:?}");
    assert!(bars.iter().all(|&(x, _, w, h)| x >= x0 && x + w <= b.1 && (w == 1.0 || h == 1.0)), "{bars:?}");
    assert!(bars.iter().any(|&(x, _, w, _)| x == x0 && w == widths[1] as f32), "{bars:?}");
    // The mode line shows one too.
    assert!(frame.glyphs.iter().any(|g| matches!(g, FrameGlyph::Char { char: '7', row_role: romacs_display_protocol::GlyphRowRole::ModeLine, .. })));
    // Nothing is drawn for the character itself.
    assert!(!frame.glyphs.iter().any(|g| matches!(g, FrameGlyph::Char { char: NONE, .. })));
}

#[test]
fn thin_space_is_a_blank_pixel() {
    let frame = lay_out(&format!("a{NONE}b"), NoFont::ThinSpace);
    assert_eq!(measured(&format!("a{NONE}b"), NoFont::ThinSpace)[1], 1);
    assert_eq!(chars(&frame).iter().map(|c| c.0).collect::<String>(), "ab");
    assert!(bars(&frame, [255, 255, 255]).is_empty());
}

#[test]
fn empty_box_is_a_cell_wide_box() {
    let frame = lay_out(&format!("a{NONE}b"), NoFont::EmptyBox);
    let widths = measured(&format!("x{NONE}"), NoFont::EmptyBox);
    assert_eq!(widths[1], widths[0], "an empty box is the base font's average width");
    assert_eq!(chars(&frame).iter().map(|c| c.0).collect::<String>(), "ab");
    assert_eq!(bars(&frame, [255, 255, 255]).len(), 4);
}

#[test]
fn an_acronym_shows_its_text() {
    let frame = lay_out(&format!("a{NONE}b"), NoFont::Acronym("NF".into()));
    assert_eq!(chars(&frame).iter().map(|c| c.0).collect::<String>(), "aNFb");
}

#[test]
fn a_face_without_a_usable_family_falls_back_to_the_default_face() {
    let mut faces = vec![face("Liberation Mono"), face("")];
    faces[1].pixel_size = f32::NAN;
    let mut snapshot = window("ab", faces);
    snapshot.text.face_runs = vec![FaceRun { bytes: 1..2, face: FaceHandle(1) }];
    let frame = romacs_layout2::layout(&snapshot, 1).expect("an unusable face falls back");
    assert_eq!(chars(&frame).iter().map(|c| c.0).collect::<String>(), "ab");
}

#[test]
fn the_bars_clicks_find_one_glyph_per_character() {
    let text = format!("a{NONE}b c");
    let mut snapshot = frame("x", NoFont::HexCode);
    snapshot.tab_bar = Some(TabBarSnapshot { height: 20, text: slice(&text), face: FaceHandle(0), faces: vec![face("Liberation Mono")] });
    let marks = BandMarks { segments: vec![Segment { bytes: 4..5, name: "right-align".into(), pill: None }], icons: vec![] };
    let bar = StatusBarSnapshot { x: 0, y: 60, width: 400, height: 20, text: slice(&text), face: FaceHandle(0),
        faces: vec![face("Liberation Mono")], window: 1, focused: true, marks };
    snapshot.status_bar = Some(bar.clone());
    let painted = LayoutEngine::new().unwrap().layout_frame(&snapshot, 1, None).unwrap().0;
    let tabs = romacs_layout2::tab_bar_extents(&painted);
    assert_eq!(tabs.len(), text.chars().count(), "{tabs:?}");
    assert!(tabs[1].1 > tabs[0].1 && tabs[2].0 >= tabs[1].0 + tabs[1].1, "the box's extent lies between its neighbours': {tabs:?}");
    let (extents, marker) = romacs_layout2::status_bar_extents(&painted, &bar);
    assert_eq!((extents.len(), marker), (text.chars().count(), vec![3]));
    assert!(extents.iter().all(Option::is_some), "{extents:?}");
}

#[test]
fn a_retained_engine_lays_out_again_for_another_method() {
    let mut engine = LayoutEngine::new().unwrap();
    let text = format!("a{NONE}b");
    let digits = |frame: &FrameGlyphBuffer| chars(frame).len();
    let hex = engine.layout_frame(&frame(&text, NoFont::HexCode), 1, None).unwrap().0;
    let thin = engine.layout_frame(&frame(&text, NoFont::ThinSpace), 2, None).unwrap().0;
    assert_eq!((digits(&hex), digits(&thin)), (6, 2));
}

#[test]
fn a_glyphless_character_ending_a_box_run_closes_it() {
    use romacs_display_protocol::{BoxVerticalEdges, FaceId, GlyphRowRole};
    use romacs_layout_input::{BoxLine, Decor};
    let mut boxed = face("Liberation Mono");
    boxed.decor = Decor { boxed: Some(BoxLine { vertical: 2, horizontal: 1, color: None, style: 1 }), ..Default::default() };
    let text = format!("a{NONE}");
    let mut snapshot = window(&text, vec![face("Liberation Mono"), boxed]);
    snapshot.mode_line = None;
    snapshot.text.face_runs = vec![FaceRun { bytes: 0..text.len(), face: FaceHandle(1) }];
    let frame = romacs_layout2::layout(&snapshot, 1).unwrap();
    let run: Vec<_> = frame.glyphs.iter()
        .filter(|g| g.face_id() == Some(FaceId::new(1)) && g.row_role() == Some(GlyphRowRole::Text))
        .map(|g| (matches!(g, FrameGlyph::Stretch { .. }), g.box_vertical_edges())).collect();
    assert_eq!(run.first().map(|g| g.1), Some(Some(BoxVerticalEdges::Left)), "{run:?}");
    assert_eq!(run.last(), Some(&(true, Some(BoxVerticalEdges::Right))), "the box's cell closes the run: {run:?}");
    // Its digits are inside the run, in a face of their own without a box.
    assert!(chars(&frame).iter().any(|c| c.0 == '7'));
}

/// A character no font has stays inside a boxed tab bar's run: its cell
/// has no edges of its own, and its blank character, there for a click,
/// no box.
#[test]
fn a_glyphless_character_stays_inside_a_boxed_tab_bar_run() {
    use romacs_display_protocol::{BoxVerticalEdges::{Left, Neither, Right}, FrameGlyph, frame_glyphs::GlyphRowRole};
    use romacs_layout_input::{BoxLine, Decor};
    let text = format!("a{NONE}b");
    let boxed = Face { decor: Decor { boxed: Some(BoxLine { vertical: 1, horizontal: 1, color: Some([200, 10, 10]), style: 1 }), ..Default::default() },
                       ..face("Liberation Mono") };
    let mut snapshot = frame("x", NoFont::ThinSpace);
    snapshot.tab_bar = Some(TabBarSnapshot { height: 20, text: slice(&text), face: FaceHandle(0), faces: vec![boxed] });
    let painted = LayoutEngine::new().unwrap().layout_frame(&snapshot, 1, None).unwrap().0;
    let cells: Vec<_> = painted.glyphs.iter().filter_map(|glyph| match glyph {
        FrameGlyph::Char { box_vertical_edges, row_role: GlyphRowRole::TabBar, .. }
        | FrameGlyph::Stretch { box_vertical_edges, row_role: GlyphRowRole::TabBar, .. } => Some(*box_vertical_edges),
        _ => None,
    }).collect();
    assert_eq!(cells, [Left, Neither, Neither, Right]);
}

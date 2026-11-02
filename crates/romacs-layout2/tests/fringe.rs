//! Fringes and the tab bar as the GUI host paints them (GNU fringe.c,
//! xdisp.c display_tab_bar), on a 10-pixel cell.
use romacs_display_protocol::{FrameGlyph, frame_glyphs::{FringeSide, GlyphRowRole}};
use romacs_layout_input::{BitmapAlign, Face, FaceHandle, FaceRun, FrameSnapshot, FringeBitmap, Indicators, LineWrap,
    PlacedWindowSnapshot, TabBarSnapshot, TextSlice, WindowChrome, WindowSnapshot};

fn face(foreground: [u8; 3], background: [u8; 3]) -> Face {
    Face { foreground, background, family: "Liberation Mono".into(), pixel_size: 16.0 }
}

fn bitmap(number: u16) -> FringeBitmap {
    FringeBitmap { number, bits: vec![0x1800; 8], width: 8, align: BitmapAlign::Center, period: 0, face: FaceHandle(1) }
}

/// A window of TEXT 100 pixels wide with 8-pixel fringes in face 1, the
/// standard continuation (7, 8) and truncation (3, 4) bitmaps.
fn window(text: &str, wrap: LineWrap) -> WindowSnapshot {
    let indicators = Indicators {
        truncation: [Some(bitmap(3)), Some(bitmap(4))],
        continuation: [Some(bitmap(7)), Some(bitmap(8))],
        ..Indicators::default()
    };
    WindowSnapshot {
        id: 1, width: 100, height: 200, start_byte: 0, point_byte: 0, buffer_version: 1,
        text: TextSlice { utf8: text.into(), face_runs: vec![], raw: vec![] },
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap, cursor_off: false, stretches: vec![], line_numbers: None,
        faces: vec![face([255; 3], [0; 3]), face([1, 2, 3], [200, 0, 0]), face([0, 255, 0], [0; 3])],
        chrome: WindowChrome { left_fringe: 8, right_fringe: 8, fringe_face: FaceHandle(1), indicators, ..WindowChrome::default() },
    }
}

fn frame(snapshot: WindowSnapshot, tab_bar: Option<TabBarSnapshot>) -> FrameSnapshot {
    let y = tab_bar.as_ref().map_or(0, |t| t.height);
    FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), width: 116, height: 200 + y,
        windows: vec![PlacedWindowSnapshot { x: 8, y, snapshot }], background_alpha: 1.0, children: vec![], tab_bar, chrome: Default::default(),
    }
}

fn lay_out(snapshot: FrameSnapshot) -> romacs_display_protocol::FrameGlyphBuffer {
    romacs_layout2::LayoutEngine::new().unwrap().layout_frame(&snapshot, 1, None).unwrap().0
}

fn bitmaps(frame: &romacs_display_protocol::FrameGlyphBuffer) -> Vec<(u16, FringeSide, f32, f32)> {
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::FringeBitmap { bitmap_index, side, x, y, .. } => Some((*bitmap_index, *side, *x, *y)),
        _ => None,
    }).collect()
}

fn chars(frame: &romacs_display_protocol::FrameGlyphBuffer, role: GlyphRowRole) -> String {
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, row_role, .. } if *row_role == role => Some(*char),
        _ => None,
    }).collect()
}

#[test]
fn fringes_paint_beside_the_text_area_in_the_fringe_face() {
    let painted = lay_out(frame(window("ab", LineWrap::Window), None));
    let fringes: Vec<_> = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Background { bounds, color } if *color == romacs_display_protocol::Color::from_pixel(0xc80000) => Some((bounds.x, bounds.width)),
        _ => None,
    }).collect();
    assert_eq!(fringes, vec![(0.0, 8.0), (108.0, 8.0)]);
}

#[test]
fn a_right_fringe_takes_the_continuation_glyph_and_shows_curly_arrows() {
    let text = "0123456789abcdef";
    let cell = romacs_layout2::frame_metrics(&face([255; 3], [0; 3])).unwrap().1 as usize;
    let fits = 100 / cell;
    let painted = lay_out(frame(window(text, LineWrap::Window), None));
    // Every column holds text: no `\\' glyph is drawn.
    assert!(!chars(&painted, GlyphRowRole::Text).contains('\\'));
    let marks = bitmaps(&painted);
    assert_eq!(marks[0].0, 8, "the continued row's right-curly-arrow: {marks:?}");
    assert_eq!((marks[0].1, marks[0].2), (FringeSide::Right, 108.0));
    assert_eq!((marks[1].0, marks[1].1, marks[1].2), (7, FringeSide::Left, 0.0), "{marks:?}");
    assert!(marks[1].3 > marks[0].3);
    assert!(text.len() > fits);
}

#[test]
fn a_right_fringe_takes_the_truncation_glyph_and_shows_an_arrow() {
    let painted = lay_out(frame(window("0123456789abcdefghij\nx", LineWrap::Truncate), None));
    assert!(!chars(&painted, GlyphRowRole::Text).contains('$'));
    let marks = bitmaps(&painted);
    assert_eq!(marks.iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(), vec![(4, FringeSide::Right)]);
}

#[test]
fn without_a_right_fringe_the_glyph_marks_the_break() {
    let mut snapshot = window("0123456789abcdef", LineWrap::Window);
    snapshot.chrome.right_fringe = 0;
    snapshot.chrome.indicators.continuation[1] = None;
    let painted = lay_out(frame(snapshot, None));
    assert!(chars(&painted, GlyphRowRole::Text).contains('\\'));
}

#[test]
fn a_window_with_a_left_neighbour_draws_the_vertical_border_in_its_fringe() {
    let mut snapshot = window("ab", LineWrap::Window);
    snapshot.chrome.border = Some(FaceHandle(2));
    let painted = lay_out(frame(snapshot, None));
    let borders: Vec<_> = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Border { x, width, .. } => Some((*x, *width)),
        _ => None,
    }).collect();
    assert_eq!(borders, vec![(0.0, 1.0)]);
}

#[test]
fn the_tab_bar_paints_its_row_above_the_windows() {
    let tab_bar = TabBarSnapshot {
        height: 20,
        text: TextSlice { utf8: " a  b ".into(), face_runs: vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }], raw: vec![] },
        face: FaceHandle(0),
        faces: vec![face([0; 3], [217; 3]), face([0; 3], [191; 3])],
    };
    let painted = lay_out(frame(window("x", LineWrap::Window), Some(tab_bar)));
    assert_eq!(chars(&painted, GlyphRowRole::TabBar), " a  b ");
    let extents = romacs_layout2::tab_bar_extents(&painted);
    assert_eq!(extents.len(), 6);
    assert_eq!(extents[0].0, 0.0);
    assert!(extents.windows(2).all(|w| w[0].0 + w[0].1 == w[1].0));
}

/// Panes (VM2-PANE-CHROME): each window's box, fringes included, but the
/// mini-window's; the selected window's edge in the focus colour, the
/// others' in the hairline's and dimmed; none in GNU's look.
#[test]
fn every_window_but_the_mini_window_is_a_pane() {
    let mut snapshot = frame(window("a", LineWrap::Window), None);
    snapshot.width = 300;
    let mut other = window("b", LineWrap::Window);
    other.id = 2;
    let mut mini = window("", LineWrap::Window);
    mini.id = 3;
    snapshot.windows.push(PlacedWindowSnapshot { x: 160, y: 0, snapshot: other });
    snapshot.windows.push(PlacedWindowSnapshot { x: 8, y: 0, snapshot: mini });
    assert!(lay_out(snapshot.clone()).panes.is_empty(), "GNU_LOOK_DRAWS_NO_PANES");
    snapshot.chrome = romacs_layout_input::FrameChrome { internal_border: 6, radius: 6, dim: 0.12, focus: Some([212, 138, 32]),
                                                         border: Some([40, 40, 40]), selected: 1, minibuffer: 3 };
    let panes = lay_out(snapshot).panes;
    let accent = romacs_display_protocol::Color::from_pixel(0xd48a20);
    let line = romacs_display_protocol::Color::from_pixel(0x282828);
    assert_eq!(panes.iter().map(|p| (p.bounds.x, p.bounds.width, p.bounds.height, p.radius, p.edge, p.dim)).collect::<Vec<_>>(),
               [(0.0, 116.0, 200.0, 6.0, Some(accent), 0.0), (152.0, 116.0, 200.0, 6.0, Some(line), 0.12)]);
}

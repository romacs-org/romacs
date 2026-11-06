//! Fringes and the tab bar as the GUI host paints them (GNU fringe.c,
//! xdisp.c display_tab_bar), on a 10-pixel cell.
use romacs_display_protocol::{FrameGlyph, frame_glyphs::{FringeSide, GlyphRowRole}};
use romacs_layout_input::{BitmapAlign, Face, FaceHandle, FaceRun, FrameSnapshot, FringeBitmap, Indicators, LineWrap,
    PlacedWindowSnapshot, BandMarks, Pill, Segment, StatusBarSnapshot, TabBarSnapshot, TextSlice, WindowChrome, WindowSnapshot};

fn face(foreground: [u8; 3], background: [u8; 3]) -> Face {
    Face { decor: Default::default(), foreground, background, family: "Liberation Mono".into(), pixel_size: 16.0, weight: 400, italic: false, width: 5 }
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
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: 1, width: 100, height: 200, start_byte: 0, point_byte: 0, buffer_version: 1,
        text: TextSlice { extends: vec![], utf8: text.into(), face_runs: vec![], raw: vec![] },
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap, cursor_off: false, no_cursor: false, stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None,
        faces: vec![face([255; 3], [0; 3]), face([1, 2, 3], [200, 0, 0]), face([0, 255, 0], [0; 3])],
        chrome: WindowChrome { left_fringe: 8, right_fringe: 8, fringe_face: FaceHandle(1), indicators, ..WindowChrome::default() },
    }
}

fn frame(snapshot: WindowSnapshot, tab_bar: Option<TabBarSnapshot>) -> FrameSnapshot {
    let y = tab_bar.as_ref().map_or(0, |t| t.height);
    FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), no_font: Default::default(), width: 116, height: 200 + y,
        windows: vec![PlacedWindowSnapshot { x: 8, y, snapshot }], background_alpha: 1.0, children: vec![], tab_bar, chrome: Default::default(), status_bar: None,
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
fn a_row_as_wide_as_the_window_shows_no_truncation_arrow() {
    // window()'s text area holds a whole number of columns: a row of
    // exactly that many characters fits, a longer one is truncated.
    let fits = (0..200).map(|n| n.to_string()).collect::<String>();
    let probe = lay_out(frame(window(&fits, LineWrap::Truncate), None));
    let shown = chars(&probe, GlyphRowRole::Text).len();
    let exact: String = fits.chars().take(shown).collect();
    let painted = lay_out(frame(window(&format!("{exact}\nx"), LineWrap::Truncate), None));
    assert_eq!(chars(&painted, GlyphRowRole::Text), format!("{exact}x"));
    assert!(bitmaps(&painted).iter().all(|m| m.1 != FringeSide::Right), "{:?}", bitmaps(&painted));
    let longer: String = fits.chars().take(shown + 1).collect();
    let painted = lay_out(frame(window(&format!("{longer}\nx"), LineWrap::Truncate), None));
    assert_eq!(bitmaps(&painted).iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(), vec![(4, FringeSide::Right)]);
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
        text: TextSlice { extends: vec![], utf8: " a  b ".into(), face_runs: vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }], raw: vec![] },
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

/// The status bar (VM2-STATUS-BAR): its band where the snapshot puts it,
/// every character on its face's background, those past its width
/// clipped; its extents are the painted characters in text order.
#[test]
fn the_status_bar_paints_its_band_and_clips_its_text() {
    let mut snapshot = frame(window("x", LineWrap::Window), None);
    snapshot.status_bar = Some(StatusBarSnapshot {
        x: 4, y: 150, width: 116 - 8, height: 20,
        text: TextSlice { extends: vec![], utf8: "abcdefghijklmnop".into(), face_runs: vec![FaceRun { bytes: 0..2, face: FaceHandle(1) }], raw: vec![] },
        face: FaceHandle(0), faces: vec![face([0; 3], [30, 40, 50]), face([9; 3], [200, 0, 0])], window: 1, focused: true,
        marks: Default::default(),
    });
    let painted = lay_out(snapshot.clone());
    let (extents, marker) = romacs_layout2::status_bar_extents(&painted, snapshot.status_bar.as_ref().unwrap());
    let shown = chars(&painted, GlyphRowRole::StatusBar);
    assert!(shown.len() < 16 && "abcdefghijklmnop".starts_with(&shown), "{shown}");
    assert_eq!((extents.len(), marker), (16, vec![]));
    assert_eq!(extents[0].unwrap().0, 4.0 + 12.0, "inside the padding");
    assert!(extents[shown.len() - 1].is_some_and(|(x, w)| x + w <= 4.0 + 108.0 - 12.0));
    assert!(extents[shown.len()..].iter().all(Option::is_none));
    assert_eq!(painted.status_bar.map(|r| (r.x, r.y, r.width, r.height)), Some((4.0, 150.0, 108.0, 20.0)));
    // A run's face keeps the bar's background, as a window's band does.
    let face = painted.glyphs.iter().find_map(|g| match g {
        FrameGlyph::Char { row_role: GlyphRowRole::StatusBar, face_id, .. } => Some(*face_id),
        _ => None,
    }).unwrap();
    assert_eq!(painted.faces[&face].background, romacs_display_protocol::Color::from_pixel(0x1e2832));
}

/// A band's own overline is a rule across it, in the line's colour, not only
/// over its text; a band without one has none.
#[test]
fn the_status_bar_s_overline_runs_across_it() {
    let bar = |line: Option<romacs_layout_input::Line>| {
        let mut bar_face = face([0; 3], [30, 40, 50]);
        bar_face.decor.overline = line;
        let mut snapshot = frame(window("x", LineWrap::Window), None);
        snapshot.status_bar = Some(StatusBarSnapshot {
            x: 4, y: 150, width: 108, height: 20,
            text: TextSlice { extends: vec![], utf8: "ab".into(), face_runs: vec![], raw: vec![] },
            face: FaceHandle(0), faces: vec![bar_face], window: 1, focused: true, marks: Default::default(),
        });
        lay_out(snapshot).glyphs.iter().filter_map(|g| match g {
            FrameGlyph::Background { bounds, color } if bounds.height == 1.0 => Some((bounds.x, bounds.y, bounds.width, *color)),
            _ => None,
        }).collect::<Vec<_>>()
    };
    let line = romacs_layout_input::Line { style: 1, color: Some([0x2e, 0x29, 0x22]), position: None };
    assert_eq!(bar(Some(line)), [(4.0, 150.0, 108.0, romacs_display_protocol::Color::from_pixel(0x2e2922))]);
    assert_eq!(bar(None), []);
}

/// A pill's text takes its foreground; an icon in a colour of its own keeps
/// it where it reads there. Solarized Light's state on its state pill is
/// 1.5:1, so its icons take the pill's text; the last pair is just under 3:1.
#[test]
fn an_icon_keeps_its_colour_on_a_pill_where_it_reads() {
    let (own, faint, under, pill, text) = ([0xff; 3], [0x0a, 0x6d, 0xaa], [0x76, 0x05, 0x4e], [0x26, 0x8b, 0xd2], [1, 2, 3]);
    let run = |bytes: std::ops::Range<usize>, face: u32| romacs_layout_input::FaceRun { bytes, face: FaceHandle(face) };
    let mut snapshot = frame(window("x", LineWrap::Window), None);
    snapshot.status_bar = Some(StatusBarSnapshot {
        x: 4, y: 150, width: 108, height: 20,
        text: TextSlice { extends: vec![], utf8: "\u{FE001}\u{FE002}\u{FE003}\u{FE004}a".into(),
                          face_runs: vec![run(0..4, 1), run(4..8, 2), run(8..12, 3), run(16..17, 1)], raw: vec![] },
        face: FaceHandle(0), faces: vec![face([9; 3], [30, 40, 50]), face(own, [30, 40, 50]), face(faint, [30, 40, 50]), face(under, [30, 40, 50])],
        window: 1, focused: true,
        marks: BandMarks { segments: vec![Segment { bytes: 0..17, name: "buffer".into(),
                                                    pill: Some(Pill { background: pill, radius: 6, foreground: Some(text) }) }],
                           icons: vec![] },
    });
    let painted = lay_out(snapshot);
    let colours: Vec<_> = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { row_role: GlyphRowRole::StatusBar, face_id, .. } => Some(painted.faces[face_id].foreground),
        _ => None,
    }).collect();
    let pixel = |[r, g, b]: [u8; 3]| romacs_display_protocol::Color::from_pixel(u32::from_be_bytes([0, r, g, b]));
    assert_eq!(colours, [pixel(own), pixel(text), pixel(text), pixel(text), pixel(text)]);
}

/// The status bar's marks (VM2-STATUS-BAR): the part after the
/// `right-align' marker flush with the right edge and whole, the part
/// before it clipped where that starts, the marker spanning the room; a
/// pill's characters in a boxed face on its background, rounded; an icon
/// drawn as its icon, one column wide; the accent block at the left edge.
#[test]
fn the_status_bar_splits_at_its_marker_and_draws_pills_and_icons() {
    // The GUI host registers Romacs Icons at startup (vm-gui fonts.rs).
    assert!(romacs_font::register_font_file(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/RomacsIcons.ttf")));
    let band = [30, 40, 50];
    let bar = |text: &str, marks: BandMarks| StatusBarSnapshot {
        x: 0, y: 150, width: 116, height: 20,
        text: TextSlice { extends: vec![], utf8: text.into(), face_runs: vec![], raw: vec![] },
        face: FaceHandle(0), faces: vec![face([200; 3], band)], window: 1, focused: true, marks,
    };
    let marker = |at: usize| Segment { bytes: at..at + 1, name: "right-align".into(), pill: None };
    let mut snapshot = frame(window("x", LineWrap::Window), None);
    // 4 + marker + 3 characters fit: the right side flush right.
    snapshot.status_bar = Some(bar("abcd xyz", BandMarks { segments: vec![marker(4)], icons: vec![] }));
    let painted = lay_out(snapshot.clone());
    let (extents, index) = romacs_layout2::status_bar_extents(&painted, snapshot.status_bar.as_ref().unwrap());
    assert_eq!(chars(&painted, GlyphRowRole::StatusBar), "abcdxyz");
    assert_eq!(index, vec![4]);
    let (right_x, right_w) = extents[7].unwrap();
    assert!((right_x + right_w - (116.0 - 12.0)).abs() < 0.01, "{extents:?}");
    assert_eq!(extents[0].unwrap().0, 12.0, "inside the padding");
    let (gap_x, gap_w) = extents[4].unwrap();
    assert!((gap_x + gap_w - extents[5].unwrap().0).abs() < 0.01);
    // A left side too long for the room is clipped where the right starts.
    let long = format!("{} xyz", "l".repeat(40));
    snapshot.status_bar = Some(bar(&long, BandMarks { segments: vec![marker(40)], icons: vec![] }));
    let painted = lay_out(snapshot.clone());
    let (extents, _) = romacs_layout2::status_bar_extents(&painted, snapshot.status_bar.as_ref().unwrap());
    assert!(chars(&painted, GlyphRowRole::StatusBar).ends_with("xyz"));
    assert!(extents[39].is_none() && extents[41..].iter().all(Option::is_some));
    assert!(extents[..40].iter().flatten().all(|(x, w)| x + w <= extents[41].unwrap().0 + 0.01));
    // A pill and an icon.
    let pill = Pill { background: [90, 20, 20], radius: 6, foreground: Some([1, 2, 3]) };
    snapshot.status_bar = Some(bar(" ab*", BandMarks {
        segments: vec![Segment { bytes: 1..3, name: "buffer".into(), pill: Some(pill) }], icons: vec![(3, '\u{FE001}')] }));
    let painted = lay_out(snapshot);
    let glyphs: Vec<_> = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { row_role: GlyphRowRole::StatusBar, char, face_id, width, .. } => Some((*char, *face_id, *width)),
        _ => None,
    }).collect();
    let fill = painted.rounded_fills[0];
    assert_eq!((fill.radius, fill.color), (6.0, romacs_display_protocol::Color::from_pixel(0x5a1414)));
    let (_, a) = (glyphs[1], glyphs[1]);
    let first = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { row_role: GlyphRowRole::StatusBar, x, .. } => Some(*x), _ => None }).collect::<Vec<_>>();
    assert_eq!(fill.bounds.x, first[1], "{a:?}");
    assert!((fill.bounds.x + fill.bounds.width - (first[2] + glyphs[2].2)).abs() < 0.01);
    assert!(fill.bounds.y > 150.0 && fill.bounds.y + fill.bounds.height < 170.0);
    // The pill's text in its own foreground; outside it, the face's.
    assert_eq!(painted.faces[&glyphs[1].1].foreground, romacs_display_protocol::Color::from_pixel(0x010203));
    assert_ne!(painted.faces[&glyphs[3].1].foreground, romacs_display_protocol::Color::from_pixel(0x010203));
    assert_eq!(glyphs[3].0, '\u{FE001}');
    assert_eq!(glyphs[3].2, glyphs[0].2, "an icon is one column");
    // The accent block: the second background, 3 pixels at the left edge.
    let accent = painted.glyphs.iter().filter_map(|g| match g { FrameGlyph::Background { bounds, .. } => Some(*bounds), _ => None })
        .find(|b| b.y == 150.0 && b.width == 3.0);
    assert!(accent.is_some());
}

/// The status bar's centred zone (modeline.md §14): the characters after
/// the `centre-align' marker centred on the bar, its marker's and the
/// `right-align' marker's room no click, each zone's glyphs in text order.
#[test]
fn the_status_bar_centres_its_middle_zone() {
    let marker = |at: usize, name: &str| Segment { bytes: at..at + 1, name: name.into(), pill: None };
    let mut snapshot = frame(window("x", LineWrap::Window), None);
    snapshot.status_bar = Some(StatusBarSnapshot {
        x: 0, y: 150, width: 116, height: 20,
        text: TextSlice { extends: vec![], utf8: "a m z".into(), face_runs: vec![], raw: vec![] },
        face: FaceHandle(0), faces: vec![face([200; 3], [30, 40, 50])], window: 1, focused: true,
        marks: BandMarks { segments: vec![marker(1, "centre-align"), marker(3, "right-align")], icons: vec![] },
    });
    let painted = lay_out(snapshot.clone());
    let (extents, markers) = romacs_layout2::status_bar_extents(&painted, snapshot.status_bar.as_ref().unwrap());
    assert_eq!(chars(&painted, GlyphRowRole::StatusBar), "amz");
    assert_eq!(markers, vec![1, 3]);
    let (mx, mw) = extents[2].unwrap();
    assert!((mx + mw / 2.0 - 58.0).abs() < 0.51, "centred on the bar: {extents:?}");
    let (zx, zw) = extents[4].unwrap();
    assert!((zx + zw - 104.0).abs() < 0.01, "flush inside the padding: {extents:?}");
    assert!((extents[1].unwrap().0 - (extents[0].unwrap().0 + extents[0].unwrap().1)).abs() < 0.01);
    assert!((extents[3].unwrap().0 - (mx + mw)).abs() < 0.01 && (extents[3].unwrap().0 + extents[3].unwrap().1 - zx).abs() < 0.01);
}

/// An icon in buffer text is one column, as in a band, and measuring the
/// text gives the advances painting it does (VM2-MODE-LINE): the icon
/// font's own advance differs from a 20-pixel mono cell.
#[test]
fn an_icon_in_text_paints_and_measures_one_column() {
    assert!(romacs_font::register_font_file(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/RomacsIcons.ttf")));
    let text = "a\u{FE001}b";
    let mut snapshot = window(text, LineWrap::Window);
    snapshot.faces = vec![Face { pixel_size: 20.0, ..face([255; 3], [0; 3]) }; 3];
    let painted = lay_out(frame(snapshot.clone(), None));
    let widths: Vec<(char, f32)> = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { row_role: GlyphRowRole::Text, char, width, .. } => Some((*char, *width)),
        _ => None,
    }).collect();
    assert_eq!(widths.iter().map(|w| w.0).collect::<String>(), text);
    assert_eq!(widths[1].1, widths[0].1, "an icon is one column: {widths:?}");
    let measured = romacs_layout2::LayoutEngine::new().unwrap().measure(&romacs_layout_input::MeasureInput {
        font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), no_font: Default::default(),
        faces: snapshot.faces.clone(), text: snapshot.text.clone(),
    }).unwrap();
    let advances: Vec<u32> = measured.glyphs.iter().map(|g| g.0).collect();
    assert_eq!(advances, widths.iter().map(|w| w.1.round() as u32).collect::<Vec<_>>());
}

/// A line folded into the bar from another window (a `context' segment)
/// keeps its own face's background (`mode-line-inactive''s), so its text
/// reads as on its own line, whatever the bar's background.
#[test]
fn a_context_segment_keeps_its_own_background() {
    let band = [30, 40, 50];
    let inactive = [240, 240, 240];
    let mut snapshot = frame(window("x", LineWrap::Window), None);
    snapshot.status_bar = Some(StatusBarSnapshot {
        x: 0, y: 150, width: 116, height: 20,
        text: TextSlice { extends: vec![], utf8: "ab | cd".into(), face_runs: vec![FaceRun { bytes: 2..7, face: FaceHandle(1) }], raw: vec![] },
        face: FaceHandle(0), faces: vec![face([200; 3], band), face([0; 3], inactive)], window: 1, focused: true,
        marks: BandMarks { segments: vec![Segment { bytes: 2..7, name: "context".into(), pill: None }], icons: vec![] },
    });
    let painted = lay_out(snapshot);
    let found = painted.glyphs.iter().any(|g| matches!(g, FrameGlyph::Background { bounds, color }
        if bounds.y == 150.0 && bounds.x > 3.0 && *color == romacs_display_protocol::Color::from_pixel(0xf0f0f0)));
    assert!(found, "the context's own background behind it");
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
                                                         border: Some([40, 40, 40]), cursor: None, selected: 1, minibuffer: 3, style: Default::default() };
    let panes = lay_out(snapshot).panes;
    let accent = romacs_display_protocol::Color::from_pixel(0xd48a20);
    let line = romacs_display_protocol::Color::from_pixel(0x282828);
    assert_eq!(panes.iter().map(|p| (p.bounds.x, p.bounds.width, p.bounds.height, p.radius, p.edge, p.dim)).collect::<Vec<_>>(),
               [(0.0, 116.0, 200.0, 6.0, Some(accent), 0.0), (152.0, 116.0, 200.0, 6.0, Some(line), 0.12)]);
}

/// Boxed characters side by side in the tab bar are one box run, with no
/// edge inside it: text over a boxed `tab-bar' takes its box.
#[test]
fn the_tab_bar_s_boxed_text_is_one_box_run() {
    use romacs_display_protocol::BoxVerticalEdges::{self, Left, Neither, Right};
    use romacs_layout_input::{BoxLine, Decor};
    let boxed = Face { decor: Decor { boxed: Some(BoxLine { vertical: 1, horizontal: 1, color: Some([200, 10, 10]), style: 1 }), ..Default::default() },
                       ..face([200; 3], [30, 40, 50]) };
    let text = TextSlice { extends: vec![], utf8: "abc d".into(), raw: vec![], face_runs: vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }] };
    let tabs = TabBarSnapshot { height: 20, text, face: FaceHandle(0), faces: vec![face([200; 3], [30, 40, 50]), boxed] };
    let painted = lay_out(frame(window("x", LineWrap::Window), Some(tabs)));
    let cells: Vec<BoxVerticalEdges> = painted.glyphs.iter().filter_map(|glyph| match glyph {
        FrameGlyph::Char { box_vertical_edges, row_role: GlyphRowRole::TabBar, .. } => Some(*box_vertical_edges),
        _ => None,
    }).collect();
    assert_eq!(cells[..3], [Left, Neither, Right]);
}

/// A boxed band is one box run, as a mode line is: its characters and the
/// stretches filling its room, every cell the band's height on one
/// baseline, and no character a box of its own, as each was. A segment
/// with a box of its own is a run over its characters; a run ends at what
/// is not painted against it.
#[test]
fn a_bar_s_box_is_one_run_and_a_segment_s_its_own() {
    use romacs_display_protocol::BoxVerticalEdges::{self, Both, Left, Neither, Right};
    use romacs_layout_input::{BoxLine, Decor};
    let line = |color, width| Decor { boxed: Some(BoxLine { vertical: width, horizontal: width, color: Some(color), style: 1 }), ..Default::default() };
    let boxed = |decor| Face { decor, ..face([200; 3], [30, 40, 50]) };
    let bar = |text: &str, runs, faces, marks| StatusBarSnapshot {
        x: 0, y: 150, width: 116, height: 22,
        text: TextSlice { extends: vec![], utf8: text.into(), raw: vec![], face_runs: runs },
        face: FaceHandle(0), faces, window: 1, focused: true, marks,
    };
    // What the bar paints, in x order: a character or `_' for a stretch,
    // its edges, its top and bottom, and a character's baseline.
    let cells = |bar| {
        let mut snapshot = frame(window("x", LineWrap::Window), None);
        snapshot.status_bar = Some(bar);
        let painted = lay_out(snapshot);
        let mut cells: Vec<(f32, char, BoxVerticalEdges, f32, f32, Option<f32>)> = painted.glyphs.iter().filter_map(|g| match g {
            FrameGlyph::Char { char, x, y, height, baseline, box_vertical_edges, row_role: GlyphRowRole::StatusBar, .. } =>
                Some((*x, *char, *box_vertical_edges, *y, *y + *height, Some(*baseline))),
            FrameGlyph::Stretch { x, y, height, box_vertical_edges, row_role: GlyphRowRole::StatusBar, .. } =>
                Some((*x, '_', *box_vertical_edges, *y, *y + *height, None)),
            _ => None,
        }).collect();
        cells.sort_by(|a, b| a.0.total_cmp(&b.0));
        cells
    };
    let shape = |cells: &[(f32, char, BoxVerticalEdges, f32, f32, Option<f32>)]| cells.iter().map(|c| (c.1, c.2)).collect::<Vec<_>>();
    // The band's box over a segment with its own: one baseline, and every
    // cell fills the band, the box's lines on its edges.
    let own = vec![FaceRun { bytes: 0..2, face: FaceHandle(1) }];
    let two = || vec![boxed(line([200, 10, 10], 1)), boxed(line([10, 200, 10], 1))];
    let shown = cells(bar("abcd", own.clone(), two(), BandMarks::default()));
    assert_eq!(shape(&shown), [('_', Both), ('a', Left), ('b', Right), ('c', Left), ('d', Neither), ('_', Right)], "{shown:?}");
    assert!(shown.iter().all(|c| c.3 == 150.0 && c.4 == 172.0), "{shown:?}");
    let baselines: Vec<_> = shown.iter().filter_map(|c| c.5).collect();
    assert!(baselines.windows(2).all(|w| w[0] == w[1]), "{baselines:?}");
    // A character whose face has no box stays in the band's run: a stretch
    // of the band's lies under it, as a mode line's faces merge over its own.
    let none = vec![boxed(line([200, 10, 10], 1)), boxed(Decor::default())];
    let shown = cells(bar("ab", vec![FaceRun { bytes: 0..1, face: FaceHandle(1) }], none, BandMarks::default()));
    let stretches: Vec<_> = shown.iter().filter(|c| c.1 == '_').map(|c| c.2).collect();
    assert_eq!(stretches, [Left, Neither, Right], "the run goes on under `a': {shown:?}");
    assert_eq!(shown.iter().find(|c| c.1 == 'b').map(|c| c.2), Some(Neither));
    // A band of no box: no stretches, and a segment's run ends with it.
    let plain = || vec![boxed(Decor::default()), boxed(line([10, 200, 10], 1))];
    let shown = cells(bar("abcd", own.clone(), plain(), BandMarks::default()));
    assert_eq!(shape(&shown).iter().take(2).copied().collect::<Vec<_>>(), [('a', Left), ('b', Right)]);
    assert!(!shown.iter().any(|c| c.1 == '_'));
    // A marker's room between two characters of one box, with no band box
    // to fill it: two runs.
    let marker = Segment { bytes: 1..2, name: "right-align".into(), pill: None };
    let all = vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }];
    let shown = cells(bar("a b", all, plain(), BandMarks { segments: vec![marker], icons: vec![] }));
    assert_eq!(shape(&shown), [('a', Both), ('b', Both)], "{shown:?}");
    // A character the band clips ends the run before it.
    let long = "l".repeat(40);
    let run = vec![FaceRun { bytes: 0..40, face: FaceHandle(1) }];
    let shown = cells(bar(&long, run, plain(), BandMarks::default()));
    assert_eq!(shown.last().map(|c| c.2), Some(Right), "{shown:?}");
}

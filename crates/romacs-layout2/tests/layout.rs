use romacs_display_protocol::{Color, FrameGlyph};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};
fn scene() -> WindowSnapshot {
    WindowSnapshot {
        id: 1,
        width: 400,
        height: 300,
        start_byte: 0,
        point_byte: 0,
        buffer_version: 1,
        mode_line: None,
        mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, tab_width: 8, wrap: Default::default(), cursor_off: false,
        text: TextSlice {
            utf8: "Ωμέγα Hello\nΔ".into(),
            face_runs: vec![FaceRun {
                bytes: 11..16,
                face: FaceHandle(1),
            }],
        },
        faces: vec![
            Face {
                foreground: [255; 3],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0,
            },
            Face {
                foreground: [255, 0, 0],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0,
            },
        ],
    }
}
#[test]
fn owned_scene_projects_text_faces_fonts_and_logical_lines() {
    let frame = romacs_layout2::layout(&scene(), 1).unwrap();
    let chars: Vec<_> = frame
        .glyphs
        .iter()
        .filter_map(|g| {
            if let FrameGlyph::Char {
                char,
                x,
                y,
                width,
                face_id,
                ..
            } = g
            {
                Some((*char, *x, *y, *width, *face_id))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        chars.iter().map(|c| c.0).collect::<String>(),
        "Ωμέγα HelloΔ"
    );
    assert_eq!((chars[0].1, chars[0].2, chars[0].3), (0.0, 0.0, 12.0));
    assert_eq!((chars[11].1, chars[11].2), (0.0, 23.0));
    assert_eq!(frame.faces[&chars[6].4].foreground, Color::RED);
    assert!(!frame.fonts.is_empty());
    assert!(!frame.char_fonts.is_empty());
    assert_eq!(frame.width, 400.0);
}
#[test]
fn malformed_snapshot_is_rejected() {
    let mut s = scene();
    s.text.face_runs[0].bytes = 1..4;
    assert!(romacs_layout2::layout(&s, 1).is_err());
}
/// A TAB is a stretch to the next tab stop, a control character GNU's
/// escape: `^L` and, beyond ASCII, octal.
#[test]
fn tabs_stretch_to_tab_stops_and_controls_show_escapes() {
    let mut s = scene();
    s.tab_width = 4;
    s.text = TextSlice { utf8: "a\tb\u{c}\u{85}\n\tc".into(), face_runs: vec![] };
    let frame = romacs_layout2::layout(&s, 1).unwrap();
    let shown: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, .. } => Some((*char, *x, *y)),
        FrameGlyph::Stretch { x, y, width, .. } => Some(('\t', *x, *y + *width * 1000.0)),
        _ => None,
    }).collect();
    // Liberation Mono at 20px: 12px columns, 23px rows; stops every 48px.
    assert_eq!(shown, [('a', 0.0, 0.0), ('\t', 12.0, 36000.0), ('b', 48.0, 0.0), ('^', 60.0, 0.0), ('L', 72.0, 0.0),
        ('\\', 84.0, 0.0), ('2', 96.0, 0.0), ('0', 108.0, 0.0), ('5', 120.0, 0.0), ('\t', 0.0, 23.0 + 48000.0), ('c', 48.0, 23.0)]);
}
/// Without fringes a row ends where its next glyph would pass the window
/// less the `\' (continued) or `$' (truncated) glyph, which shows there.
#[test]
fn rows_continue_and_truncate_as_gnu_without_fringes() {
    let mut s = scene();
    s.width = 120; // ten 12px columns
    s.text = TextSlice { utf8: "abcdefghijkl\nxy".into(), face_runs: vec![] };
    let shown = |s: &WindowSnapshot| -> Vec<(char, f32, f32)> {
        romacs_layout2::layout(s, 1).unwrap().glyphs.iter().filter_map(|g| match g {
            FrameGlyph::Char { char, x, y, .. } => Some((*char, *x, *y)),
            _ => None,
        }).collect()
    };
    let continued = shown(&s);
    assert_eq!(&continued[8..12], &[('i', 96.0, 0.0), ('\\', 108.0, 0.0), ('j', 0.0, 23.0), ('k', 12.0, 23.0)]);
    assert_eq!(continued.last(), Some(&('y', 12.0, 46.0)));
    s.wrap = romacs_layout_input::LineWrap::Truncate;
    let truncated = shown(&s);
    assert_eq!(&truncated[8..], &[('i', 96.0, 0.0), ('$', 108.0, 0.0), ('x', 0.0, 23.0), ('y', 12.0, 23.0)]);
}
/// A window starting on a continuation row lays it out as from the line's
/// start: the rows above count toward a TAB's stop.
#[test]
fn a_window_start_on_a_continuation_row_keeps_the_rows_above() {
    let mut s = scene();
    s.width = 120; // ten 12px columns: a row holds nine before its `\'
    s.text = TextSlice { utf8: "abcdefghijkl\tz".into(), face_runs: vec![] };
    let z = |s: &WindowSnapshot| romacs_layout2::layout(s, 1).unwrap().glyphs.iter().find_map(|g| match g {
        FrameGlyph::Char { char: 'z', x, y, .. } => Some((*x, *y)),
        _ => None,
    }).unwrap();
    assert_eq!(z(&s), (84.0, 23.0), "from the line start");
    s.start_byte = 9;
    assert_eq!(z(&s), (84.0, 0.0), "from the continuation row");
}

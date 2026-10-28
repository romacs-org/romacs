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
        vscroll_px: 0,
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

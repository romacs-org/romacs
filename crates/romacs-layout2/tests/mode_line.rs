use romacs_display_protocol::{Color, FrameGlyph, GlyphRowRole};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

fn scene() -> WindowSnapshot {
    WindowSnapshot {
        id: 1,
        width: 100,
        height: 60,
        start_byte: 0,
        point_byte: 0,
        buffer_version: 1,
        text: TextSlice {
            utf8: "body\nnext\nlast".into(),
            face_runs: vec![],
        },
        mode_line: Some(TextSlice {
            utf8: "g1 Ω v=0".into(),
            face_runs: vec![FaceRun {
                bytes: 3..5,
                face: FaceHandle(2),
            }],
        }),
        mode_line_face: FaceHandle(1),
        vscroll_px: 0,
        faces: vec![
            Face {
                foreground: [255; 3],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0,
            },
            Face {
                foreground: [255; 3],
                background: [0, 0, 255],
                family: "Liberation Mono".into(),
                pixel_size: 20.0,
            },
            Face {
                foreground: [255, 0, 0],
                background: [0, 255, 0],
                family: "Liberation Mono".into(),
                pixel_size: 30.0,
            },
        ],
    }
}

#[test]
fn band_is_bottom_anchored_font_measured_and_body_clipped() {
    let frame = romacs_layout2::layout(&scene(), 1).unwrap();
    let (bounds, color) = frame
        .glyphs
        .iter()
        .find_map(|g| match g {
            FrameGlyph::Background { bounds, color } => Some((*bounds, *color)),
            _ => None,
        })
        .expect("mode-line background missing");
    assert_eq!(color, Color::BLUE);
    assert_eq!(bounds.x, 0.0);
    assert_eq!(bounds.width, frame.width);
    assert_eq!(bounds.y + bounds.height, frame.height);
    let font = frame.fonts.values().find(|f| f.pixel_size == 30.0).unwrap();
    assert_eq!(bounds.height, (font.ascent_px + font.descent_px).ceil());
    let mut band = String::new();
    let mut body = String::new();
    for glyph in &frame.glyphs {
        if let FrameGlyph::Char {
            char,
            row_role,
            clip_rect,
            y,
            face_id,
            ..
        } = glyph
        {
            let clip = clip_rect.unwrap();
            if *row_role == GlyphRowRole::ModeLine {
                band.push(*char);
                assert_eq!(clip, bounds);
                assert_eq!(*y, bounds.y);
                assert_eq!(frame.faces[face_id].background, Color::BLUE);
                assert_eq!(frame.faces[face_id].id, *face_id);
                if *char == 'Ω' {
                    assert_eq!(frame.faces[face_id].foreground, Color::RED);
                }
                if char.is_ascii() {
                    assert!(
                        frame.faces[face_id]
                            .default_resolved_font_id
                            .is_some_and(|id| frame.fonts.contains_key(&id))
                    );
                } else {
                    assert!(frame.char_fonts[face_id].contains_key(char));
                }
            } else {
                body.push(*char);
                assert_eq!(*row_role, GlyphRowRole::Text);
                assert_eq!(clip.y, 0.0);
                assert_eq!(clip.height, bounds.y);
                assert!(*y < bounds.y);
            }
        }
    }
    assert_eq!(band, "g1 Ω v=0");
    assert_eq!(body, "bodynext");
}

#[test]
fn band_only_characters_are_published_for_every_used_face() {
    let mut s = scene();
    s.text.utf8.clear();
    let frame = romacs_layout2::layout(&s, 1).unwrap();
    assert!(frame.glyphs.iter().any(|g| matches!(g, FrameGlyph::Char { char: 'Ω', face_id, row_role: GlyphRowRole::ModeLine, .. } if frame.char_fonts.get(face_id).is_some_and(|m| m.contains_key(&'Ω')))));
}

#[test]
fn malformed_band_runs_and_handle_are_errors_not_panics() {
    for runs in [
        vec![FaceRun {
            bytes: 4..5,
            face: FaceHandle(0),
        }],
        vec![FaceRun {
            bytes: 0..99,
            face: FaceHandle(0),
        }],
        vec![FaceRun {
            bytes: 5..3,
            face: FaceHandle(0),
        }],
        vec![
            FaceRun {
                bytes: 0..3,
                face: FaceHandle(0),
            },
            FaceRun {
                bytes: 2..3,
                face: FaceHandle(0),
            },
        ],
        vec![FaceRun {
            bytes: 0..1,
            face: FaceHandle(99),
        }],
    ] {
        let mut s = scene();
        s.mode_line.as_mut().unwrap().face_runs = runs;
        assert!(
            romacs_layout2::layout(&s, 1).is_err(),
            "malformed band accepted"
        );
    }
    let mut s = scene();
    s.mode_line_face = FaceHandle(99);
    assert!(romacs_layout2::layout(&s, 1).is_err());
}

#[test]
fn vscroll_moves_body_not_band_and_tiny_windows_are_clipped() {
    let s = scene();
    let first = romacs_layout2::layout(&s, 1).unwrap();
    let mut scrolled = s.clone();
    scrolled.vscroll_px = 7;
    let next = romacs_layout2::layout(&scrolled, 2).unwrap();
    let positions = |frame: &romacs_display_protocol::FrameGlyphBuffer, role| {
        frame
            .glyphs
            .iter()
            .filter_map(|g| match g {
                FrameGlyph::Char { y, row_role, .. } if *row_role == role => Some(*y),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        positions(&next, GlyphRowRole::Text)[0],
        positions(&first, GlyphRowRole::Text)[0] - 7.0
    );
    assert_eq!(
        positions(&next, GlyphRowRole::ModeLine),
        positions(&first, GlyphRowRole::ModeLine)
    );
    scrolled.height = 1;
    let tiny = romacs_layout2::layout(&scrolled, 3).unwrap();
    assert!(positions(&tiny, GlyphRowRole::Text).is_empty());
    assert!(tiny.glyphs.iter().any(|g| matches!(g, FrameGlyph::Background { bounds, .. } if bounds.y == 0.0 && bounds.height == 1.0)));
}

#[test]
fn empty_band_still_reserves_height_and_none_preserves_body() {
    let mut s = scene();
    s.mode_line.as_mut().unwrap().utf8.clear();
    s.mode_line.as_mut().unwrap().face_runs.clear();
    let band = romacs_layout2::layout(&s, 1).unwrap();
    assert!(
        band.glyphs
            .iter()
            .any(|g| matches!(g, FrameGlyph::Background { .. }))
    );
    s.mode_line = None;
    let body = romacs_layout2::layout(&s, 2).unwrap();
    assert!(
        !body
            .glyphs
            .iter()
            .any(|g| matches!(g, FrameGlyph::Background { .. }))
    );
    let chars: String = body
        .glyphs
        .iter()
        .filter_map(|g| match g {
            FrameGlyph::Char { char, .. } => Some(*char),
            _ => None,
        })
        .collect();
    assert_eq!(chars, "bodynextlast");
}

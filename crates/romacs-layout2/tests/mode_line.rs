use romacs_display_protocol::{Color, FrameGlyph, GlyphRowRole};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

fn scene() -> WindowSnapshot {
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: 1,
        width: 100,
        height: 60,
        start_byte: 0,
        point_byte: 0,
        buffer_version: 1,
        text: TextSlice { extends: vec![],
            utf8: "body\nnext\nlast".into(),
            face_runs: vec![],
            raw: vec![],
        },
        mode_line: Some(TextSlice { extends: vec![],
            raw: vec![],
            utf8: "g1 Ω v=0".into(),
            face_runs: vec![FaceRun {
                bytes: 3..5,
                face: FaceHandle(2),
            }],
        }),
        mode_line_face: FaceHandle(1), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, cursor: Default::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(),
        faces: vec![
            Face { decor: Default::default(),
                foreground: [255; 3],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0, weight: 400, italic: false, width: 5,
            },
            Face { decor: Default::default(),
                foreground: [255; 3],
                background: [0, 0, 255],
                family: "Liberation Mono".into(),
                pixel_size: 20.0, weight: 400, italic: false, width: 5,
            },
            Face { decor: Default::default(),
                foreground: [255, 0, 0],
                background: [0, 255, 0],
                family: "Liberation Mono".into(),
                pixel_size: 30.0, weight: 400, italic: false, width: 5,
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

/// A header line paints at the window's top in its face's band and the
/// text starts below it; a window with only a header line leaves no gap in
/// the face table, so the next window's faces are its own.
#[test]
fn header_line_is_top_anchored_and_its_faces_stay_the_windows() {
    let mut header = scene();
    header.header_line = Some(TextSlice { extends: vec![], utf8: "head".into(), face_runs: vec![], raw: vec![] });
    header.header_line_face = FaceHandle(1);
    header.mode_line = None;
    let mut below = scene();
    below.id = 2;
    let frame_snapshot = romacs_layout_input::FrameSnapshot {
        font_selection: romacs_layout_input::FontSelectionState::default(),
        font_alternatives: romacs_layout_input::FontAlternativeState::default(), fontsets: Default::default(), no_font: Default::default(),
        width: 100, height: 120,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 0, snapshot: header },
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 60, snapshot: below },
        ],
    };
    let (frame, _) = romacs_layout2::layout_frame(&frame_snapshot, 1).unwrap();
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, row_role, y, window_id, face_id, .. } => Some((*char, *row_role, *y, window_id.get(), *face_id)),
        _ => None,
    }).collect();
    let band: String = chars.iter().filter(|c| c.1 == GlyphRowRole::HeaderLine).map(|c| c.0).collect();
    assert_eq!(band, "head");
    let band_height = (frame.fonts.values().find(|f| f.pixel_size == 20.0).map(|f| f.ascent_px + f.descent_px).unwrap()).ceil();
    let first = chars.iter().find(|c| c.1 == GlyphRowRole::Text && c.3 == 1).unwrap();
    assert_eq!((first.0, first.2), ('b', band_height), "text starts below the header line");
    assert!(chars.iter().all(|c| c.1 != GlyphRowRole::HeaderLine || c.2 == 0.0));
    // The window below paints its text in its own faces, not the header's copies.
    let header_faces: Vec<_> = chars.iter().filter(|c| c.1 == GlyphRowRole::HeaderLine).map(|c| c.4).collect();
    assert!(chars.iter().filter(|c| c.3 == 2).all(|c| !header_faces.contains(&c.4)));
    assert!(chars.iter().all(|c| frame.faces.contains_key(&c.4)));
}

/// A band's stretch (VM2-MODE-LINE-STRETCH): its `:align-to' counted from
/// the text area's left edge, past the left fringe; blank, but a face's
/// underline runs across it as a stretch glyph of its width.
#[test]
fn a_band_stretch_aligns_from_the_text_area_and_keeps_its_lines() {
    use romacs_layout_input::{BandStretches, Decor, Line, Stretch, StretchWidth};
    let mut s = scene();
    s.width = 300;
    s.chrome.left_fringe = 8;
    s.mode_line = Some(TextSlice { extends: vec![], utf8: "a b".into(), face_runs: vec![], raw: vec![] });
    s.band_stretches = BandStretches { header_line: vec![], mode_line: vec![Stretch { byte: 1, width: StretchWidth::AlignTo(100) }] };
    s.faces[1].decor = Decor { underline: Some(Line { style: 1, color: None, position: None }), overline: None, strike: None, boxed: None };
    let frame = romacs_layout2::layout(&s, 1).unwrap();
    let band: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, row_role: GlyphRowRole::ModeLine, x, .. } => Some((*char, *x)),
        _ => None,
    }).collect();
    let a = band.iter().find(|g| g.0 == 'a').unwrap().1;
    let b = band.iter().find(|g| g.0 == 'b').unwrap().1;
    assert_eq!(band.len(), 2, "the stretch paints no character: {band:?}");
    assert!((b - a - 108.0).abs() < 0.01, "{band:?}: b at the text area's 100th pixel");
    let stretch = frame.glyphs.iter().find_map(|g| match g {
        FrameGlyph::Stretch { row_role: GlyphRowRole::ModeLine, x, width, .. } if (*x - a).abs() > 0.5 => Some((*x, *width)),
        _ => None,
    }).expect("the stretch's underline");
    assert!((stretch.0 + stretch.1 - b).abs() < 0.01, "{stretch:?} ends where b starts at {b}");
}

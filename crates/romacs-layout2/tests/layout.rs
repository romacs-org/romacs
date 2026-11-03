use romacs_display_protocol::{Color, FrameGlyph};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};
fn scene() -> WindowSnapshot {
    WindowSnapshot { underline_rules: Default::default(),
        id: 1,
        width: 400,
        height: 300,
        start_byte: 0,
        point_byte: 0,
        buffer_version: 1,
        mode_line: None,
        mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(),
        text: TextSlice { extends: vec![],
            raw: vec![],
            utf8: "Ωμέγα Hello\nΔ".into(),
            face_runs: vec![FaceRun {
                bytes: 11..16,
                face: FaceHandle(1),
            }],
        },
        faces: vec![
            Face { decor: Default::default(),
                foreground: [255; 3],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0, weight: 400, italic: false, width: 5,
            },
            Face { decor: Default::default(),
                foreground: [255, 0, 0],
                background: [0; 3],
                family: "Liberation Mono".into(),
                pixel_size: 20.0, weight: 400, italic: false, width: 5,
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
    s.text = TextSlice { extends: vec![], utf8: "a\tb\u{c}\u{85}\n\tc".into(), face_runs: vec![], raw: vec![] };
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
    s.text = TextSlice { extends: vec![], utf8: "abcdefghijkl\nxy".into(), face_runs: vec![], raw: vec![] };
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
    s.text = TextSlice { extends: vec![], utf8: "abcdefghijkl\tz".into(), face_runs: vec![], raw: vec![] };
    let z = |s: &WindowSnapshot| romacs_layout2::layout(s, 1).unwrap().glyphs.iter().find_map(|g| match g {
        FrameGlyph::Char { char: 'z', x, y, .. } => Some((*x, *y)),
        _ => None,
    }).unwrap();
    assert_eq!(z(&s), (84.0, 23.0), "from the line start");
    s.start_byte = 9;
    assert_eq!(z(&s), (84.0, 0.0), "from the continuation row");
}

/// A placeholder space the snapshot names a stretch paints as one: a
/// `:width' as given, an `:align-to' to its target from the line's start,
/// and on a continued row one that would pass the last visible x ends a
/// pixel before it (GNU produce_stretch_glyph). A stretch row ends with
/// the newline after it, and the window end counts the stretch.
#[test]
fn stretches_paint_as_gnus_window_system_stretches() {
    use romacs_layout_input::{Stretch, StretchWidth};
    let mut snapshot = scene();
    snapshot.text = TextSlice { extends: vec![], utf8: "a b\nc d\n \nz".into(), face_runs: vec![], raw: vec![] };
    snapshot.stretches = vec![
        Stretch { byte: 1, width: StretchWidth::Width { width: 30, zero_ok: true } },
        Stretch { byte: 5, width: StretchWidth::AlignTo(200) },
        Stretch { byte: 8, width: StretchWidth::AlignTo(10_000) },
    ];
    let frame = romacs_layout2::layout(&snapshot, 1).unwrap();
    let stretches: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Stretch { x, y, width, .. } => Some((*x, *y, *width)),
        _ => None,
    }).collect();
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, .. } => Some((*char, *x, *y)),
        _ => None,
    }).collect();
    let (a, c, z) = (chars[0], chars[2], chars[4]);
    let backslash = 400.0 - frame.char_width;
    assert_eq!(stretches[0], (a.1 + frame.char_width, a.2, 30.0));
    assert_eq!(chars[1], ('b', a.1 + frame.char_width + 30.0, a.2));
    assert_eq!(stretches[1], (c.1 + frame.char_width, c.2, 200.0 - frame.char_width));
    assert_eq!(chars[3], ('d', 200.0, c.2));
    // The text area less the continuation glyph, less a pixel.
    assert_eq!(stretches[2].2, backslash - 1.0);
    assert_eq!((z.0, z.1, z.2 > stretches[2].1), ('z', 0.0, true));
    assert_eq!(chars.len(), 5, "no continuation glyph: {chars:?}");
    // Not on its placeholder, or out of order: refused.
    snapshot.stretches = vec![Stretch { byte: 0, width: StretchWidth::AlignTo(10) }];
    assert!(romacs_layout2::layout(&snapshot, 1).is_err());
}

/// A face's `:weight' and `:slant' choose the font: bold text is set in
/// the family's bold face and italic in its italic, not in the regular.
#[test]
fn a_bold_or_italic_face_is_set_in_that_font() {
    let mut s = scene();
    // "Hello" bold, the "Δ" on the next line italic.
    s.text.face_runs = vec![FaceRun { bytes: 11..16, face: FaceHandle(1) }, FaceRun { bytes: 17..19, face: FaceHandle(2) }];
    s.faces[1] = Face { weight: 700, ..s.faces[0].clone() };
    s.faces.push(Face { italic: true, ..s.faces[0].clone() });
    let frame = romacs_layout2::layout(&s, 1).unwrap();
    let font_of = |ch: char| {
        let face = frame.glyphs.iter().find_map(|g| match g {
            FrameGlyph::Char { char, face_id, .. } if *char == ch => Some(*face_id),
            _ => None,
        }).unwrap();
        let id = frame.faces[&face].default_resolved_font_id.expect("a resolved font");
        (frame.faces[&face].attributes, &frame.fonts[&id])
    };
    let ((regular_attrs, regular), (bold_attrs, bold), (italic_attrs, italic)) = (font_of('Ω'), font_of('H'), font_of('Δ'));
    assert!(bold_attrs.contains(romacs_display_protocol::FaceAttributes::BOLD) && !regular_attrs.contains(romacs_display_protocol::FaceAttributes::BOLD));
    assert!(italic_attrs.contains(romacs_display_protocol::FaceAttributes::ITALIC));
    assert!(regular.weight < 700 && bold.weight >= 700, "{regular:?} {bold:?}");
    assert_eq!(regular.slant, romacs_display_protocol::font::FontSlantKind::Normal);
    assert_ne!(italic.slant, romacs_display_protocol::font::FontSlantKind::Normal, "{italic:?}");
}

/// A row ending in a newline with an end-of-line face paints past its text
/// to the text area's edge in that face (xdisp.c
/// extend_face_to_end_of_line); a row without one, or the last row with no
/// newline, paints nothing there.
#[test]
fn a_row_s_extend_face_paints_to_the_text_area_s_edge() {
    use romacs_layout_input::Extend;
    let mut snapshot = scene();
    snapshot.faces.push(Face { background: [0x20, 0x30, 0x40], ..snapshot.faces[0].clone() });
    snapshot.text = TextSlice { utf8: "ab\ncd\nef".into(), face_runs: vec![], raw: vec![],
                                extends: vec![Extend { byte: 2, face: FaceHandle(2) }] };
    let frame = romacs_layout2::layout(&snapshot, 1).unwrap();
    let stretches: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Stretch { x, y, width, bg, .. } => Some((*x, *y, *width, *bg)),
        _ => None,
    }).collect();
    let b = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char: 'b', x, y, width, .. } => Some((*x, *y, *width)), _ => None }).unwrap();
    assert_eq!(stretches.len(), 1, "{stretches:?}");
    let (x, y, width, bg) = stretches[0];
    assert_eq!((x, y, x + width), (b.0 + b.2, b.1, 400.0));
    assert_eq!(bg, Color::from_pixel(0x203040));
}

/// A face's underline goes where xterm.c puts it: at the descent line less
/// `:position' pixels, else the font's own position (at least the window's
/// minimum offset); and a short mode line's face, lines included, fills the
/// rest of its band (display_mode_line).
#[test]
fn underlines_go_where_gnu_puts_them_and_a_band_s_face_fills_it() {
    use romacs_display_protocol::face::UnderlinePosition;
    use romacs_layout_input::{Decor, Line, UnderlineRules};
    let mut snapshot = scene();
    let underline = |position| Decor { underline: Some(Line { style: 1, color: None, position }), ..Default::default() };
    snapshot.faces[1].decor = underline(Some(2));
    snapshot.faces.push(Face { decor: underline(None), ..snapshot.faces[0].clone() });
    snapshot.faces.push(Face { decor: Decor { overline: Some(Line { style: 1, color: None, position: None }), ..Default::default() }, ..snapshot.faces[0].clone() });
    snapshot.text = TextSlice { utf8: "ab".into(), face_runs: vec![FaceRun { bytes: 0..1, face: FaceHandle(1) }, FaceRun { bytes: 1..2, face: FaceHandle(2) }], raw: vec![], extends: vec![] };
    snapshot.underline_rules = UnderlineRules { minimum_offset: 7, at_descent_line: false, use_position: false };
    snapshot.mode_line = Some(TextSlice { utf8: "x".into(), face_runs: vec![], raw: vec![], extends: vec![] });
    snapshot.mode_line_face = FaceHandle(3);
    let frame = romacs_layout2::layout(&snapshot, 1).unwrap();
    let face = |index: u32| frame.faces.get(&romacs_display_protocol::FaceId::new(index)).unwrap();
    assert_eq!(face(1).underline_placement, UnderlinePosition::DescentLine { pixels_above: 2 });
    // Half the descent would be above the minimum offset of 7.
    assert_eq!(face(2).underline_placement, UnderlinePosition::FontMetric { offset_from_baseline: 7 });
    // The mode line's rest: a stretch to the band's edge in its face.
    let rest = frame.glyphs.iter().filter_map(|g| match g { FrameGlyph::Stretch { x, width, face_id, .. } => Some((*x, *width, *face_id)), _ => None }).last().unwrap();
    assert_eq!(rest.0 + rest.1, 400.0);
    assert!(frame.faces[&rest.2].attributes.contains(romacs_display_protocol::FaceAttributes::OVERLINE));
}

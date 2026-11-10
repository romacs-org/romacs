use romacs_display_protocol::{Color, FrameGlyph};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};
fn scene() -> WindowSnapshot {
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: 1,
        width: 400,
        height: 300,
        start_byte: 0,
        point_byte: 0,
        buffer_version: 1,
        mode_line: None,
        mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, cursor: Default::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(),
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
    let frame = romacs_layout::layout(&scene(), 1).unwrap();
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
    assert!(romacs_layout::layout(&s, 1).is_err());
}
/// A TAB is a stretch to the next tab stop, a control character GNU's
/// escape: `^L` and, beyond ASCII, octal.
#[test]
fn tabs_stretch_to_tab_stops_and_controls_show_escapes() {
    let mut s = scene();
    s.tab_width = 4;
    s.text = TextSlice { extends: vec![], utf8: "a\tb\u{c}\u{85}\n\tc".into(), face_runs: vec![], raw: vec![] };
    let frame = romacs_layout::layout(&s, 1).unwrap();
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
        romacs_layout::layout(s, 1).unwrap().glyphs.iter().filter_map(|g| match g {
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
/// A truncated row exactly as wide as the window less the `$' column
/// shows whole, with no `$': only a glyph past the edge truncates it.
#[test]
fn a_row_that_fits_exactly_is_not_truncated() {
    let mut s = scene();
    s.width = 120; // ten 12px columns: nine before the `$' column
    s.wrap = romacs_layout_input::LineWrap::Truncate;
    s.text = TextSlice { extends: vec![], utf8: "abcdefghi\nabcdefghij\nabcdefghi".into(), face_runs: vec![], raw: vec![] };
    let rows: Vec<String> = {
        let glyphs = romacs_layout::layout(&s, 1).unwrap().glyphs;
        let mut rows = vec![String::new(); 3];
        for g in &glyphs { if let FrameGlyph::Char { char, y, .. } = g { rows[(*y / 23.0) as usize].push(*char); } }
        rows
    };
    assert_eq!(rows, ["abcdefghi", "abcdefghi$", "abcdefghi"]);
    // A display table's glyph in place of `$' reserves the same column.
    s.chrome.line_glyphs = [Some('…'), Some(' ')];
    let rows: Vec<String> = {
        let glyphs = romacs_layout::layout(&s, 1).unwrap().glyphs;
        let mut rows = vec![String::new(); 3];
        for g in &glyphs { if let FrameGlyph::Char { char, y, .. } = g { rows[(*y / 23.0) as usize].push(*char); } }
        rows
    };
    assert_eq!(rows, ["abcdefghi", "abcdefghi…", "abcdefghi"]);
}
/// A window starting on a continuation row lays it out as from the line's
/// start: the rows above count toward a TAB's stop.
#[test]
fn a_window_start_on_a_continuation_row_keeps_the_rows_above() {
    let mut s = scene();
    s.width = 120; // ten 12px columns: a row holds nine before its `\'
    s.text = TextSlice { extends: vec![], utf8: "abcdefghijkl\tz".into(), face_runs: vec![], raw: vec![] };
    let z = |s: &WindowSnapshot| romacs_layout::layout(s, 1).unwrap().glyphs.iter().find_map(|g| match g {
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
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
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
    assert!(romacs_layout::layout(&snapshot, 1).is_err());
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
    let frame = romacs_layout::layout(&s, 1).unwrap();
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
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
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
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let face = |index: u32| frame.faces.get(&romacs_display_protocol::FaceId::new(index)).unwrap();
    assert_eq!(face(1).underline_placement, UnderlinePosition::DescentLine { pixels_above: 2 });
    // Half the descent would be above the minimum offset of 7.
    assert_eq!(face(2).underline_placement, UnderlinePosition::FontMetric { offset_from_baseline: 7 });
    // The mode line's rest: a stretch to the band's edge in its face.
    let rest = frame.glyphs.iter().filter_map(|g| match g { FrameGlyph::Stretch { x, width, face_id, .. } => Some((*x, *width, *face_id)), _ => None }).next_back().unwrap();
    assert_eq!(rest.0 + rest.1, 400.0);
    assert!(frame.faces[&rest.2].attributes.contains(romacs_display_protocol::FaceAttributes::OVERLINE));
}

/// A box run with edges outside its text (xdisp.c produce_glyphs): spacers
/// of the left and right width own the side edges and add to the run's
/// advance, and the top and bottom widths add to its row's height; with
/// edges inside, the run's end glyphs own them and nothing moves.
#[test]
fn a_box_run_owns_its_edges_and_widens_its_row() {
    use romacs_display_protocol::BoxVerticalEdges;
    use romacs_layout_input::{BoxLine, Decor};
    let mut snapshot = scene();
    let boxed = |vertical, horizontal| Decor { boxed: Some(BoxLine { vertical, horizontal, color: None, style: 1 }), ..Default::default() };
    snapshot.faces[1].decor = boxed(3, 2);
    snapshot.faces.push(Face { decor: boxed(-1, -1), ..snapshot.faces[0].clone() });
    snapshot.text = TextSlice { utf8: "xabcy\nxdey".into(), raw: vec![], extends: vec![],
                                face_runs: vec![FaceRun { bytes: 1..4, face: FaceHandle(1) }, FaceRun { bytes: 7..9, face: FaceHandle(2) }] };
    let plain = { let mut s = snapshot.clone(); s.faces[1].decor = Default::default(); s.faces[2].decor = Default::default();
                  romacs_layout::layout(&s, 1).unwrap() };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let chars = |frame: &romacs_display_protocol::FrameGlyphBuffer| frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, height, box_vertical_edges, .. } => Some((*char, *x, *y, *height, *box_vertical_edges)), _ => None }).collect::<Vec<_>>();
    let (with, without) = (chars(&frame), chars(&plain));
    let at = |list: &[(char, f32, f32, f32, BoxVerticalEdges)], c| *list.iter().find(|g| g.0 == c).unwrap();
    // Outside edges: the text starts 3 pixels in, the next glyph after the
    // run 6 pixels on; the run's glyphs are 4 pixels taller.
    assert_eq!(at(&with, 'a').1, at(&without, 'a').1 + 3.0);
    assert_eq!(at(&with, 'y').1, at(&without, 'y').1 + 6.0);
    assert_eq!(at(&with, 'a').3, at(&without, 'a').3 + 4.0);
    // The left side is a spacer; the right, the last glyph's own cell
    // (so a cursor on it covers it), which owns that edge.
    assert_eq!([at(&with, 'a').4, at(&with, 'b').4, at(&with, 'c').4], [BoxVerticalEdges::Neither, BoxVerticalEdges::Neither, BoxVerticalEdges::Right]);
    let spacers: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Stretch { width, box_vertical_edges, .. } => Some((*width, *box_vertical_edges)), _ => None }).collect();
    assert_eq!(spacers, [(3.0, BoxVerticalEdges::Left)]);
    let width = |frame: &romacs_display_protocol::FrameGlyphBuffer, c| frame.glyphs.iter().find_map(|g| match g {
        FrameGlyph::Char { char, width, .. } if *char == c => Some(*width), _ => None }).unwrap();
    assert_eq!(width(&frame, 'c'), width(&plain, 'c') + 3.0);
    // Inside edges: no glyph moves, the ends own the edges.
    assert_eq!(at(&with, 'e').1 - at(&with, 'd').1, at(&without, 'e').1 - at(&without, 'd').1);
    assert_eq!((at(&with, 'd').4, at(&with, 'e').4), (BoxVerticalEdges::Left, BoxVerticalEdges::Right));
}

/// Measuring counts a box run's outside side edges in its end glyphs'
/// advances and its top and bottom in its face's height, as painting does.
#[test]
fn measuring_counts_a_box_run_s_edges() {
    use romacs_layout_input::{BoxLine, Decor, MeasureInput};
    let mut snapshot = scene();
    let plain = snapshot.faces.clone();
    snapshot.faces[1].decor = Decor { boxed: Some(BoxLine { vertical: 3, horizontal: 2, color: None, style: 1 }), ..Default::default() };
    let text = TextSlice { utf8: "xaby".into(), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 1..3, face: FaceHandle(1) }] };
    let mut engine = romacs_layout::LayoutEngine::new().unwrap();
    let mut measure = |faces: Vec<Face>| engine.measure(&MeasureInput {
        font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), faces, text: text.clone(), no_font: Default::default() }).unwrap();
    let (boxed, unboxed) = (measure(snapshot.faces.clone()), measure(plain));
    let advance = |m: &romacs_layout_input::Measured, i: usize| m.glyphs[i].0;
    assert_eq!((advance(&boxed, 0), advance(&boxed, 1), advance(&boxed, 2), advance(&boxed, 3)),
               (advance(&unboxed, 0), advance(&unboxed, 1) + 3, advance(&unboxed, 2) + 3, advance(&unboxed, 3)));
    assert_eq!(boxed.face_heights[1], unboxed.face_heights[1] + 4);
}

/// A boxed run and its row's other text share one baseline, the row as
/// tall as the box (the row's ascent is its glyphs' largest, a box's top
/// included); a run continued on the next row opens no new edge there;
/// a boxed word wraps whole, sides included; the cursor on a run's first
/// character covers its side.
#[test]
fn a_boxed_run_shares_its_row_s_baseline_and_carries_over_rows() {
    use romacs_display_protocol::BoxVerticalEdges;
    use romacs_layout_input::{BoxLine, Decor, LineWrap};
    let mut snapshot = scene();
    snapshot.faces[1].decor = Decor { boxed: Some(BoxLine { vertical: 4, horizontal: 2, color: None, style: 1 }), ..Default::default() };
    snapshot.text = TextSlice { utf8: "xab".into(), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 1..3, face: FaceHandle(1) }] };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, baseline, width, box_vertical_edges, .. } => Some((*char, *x, *baseline, *width, *box_vertical_edges)), _ => None }).collect();
    assert_eq!(chars[0].2, chars[1].2, "one baseline: {chars:?}");
    // The cursor at the run's start covers its left side and its glyph.
    snapshot.point_byte = 1;
    let placed = romacs_layout_input::FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), width: 400, height: 300,
        windows: vec![romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 0, snapshot: snapshot.clone() }], background_alpha: 1.0,
        children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, no_font: Default::default(),
    };
    let (frame, _) = romacs_layout::layout_frame_with_cursors(&placed, 1, snapshot.id, Color::from_pixel(0xffffff)).unwrap();
    let cursor = frame.active_cursor().unwrap();
    assert_eq!((cursor.x, cursor.width), (chars[1].1 - 4.0, chars[1].3 + 4.0));
    // A run wider than the row: its continued part opens no left edge.
    let cells = (400.0 / frame.char_width) as usize;
    let long = "b".repeat(cells + 4);
    snapshot.text = TextSlice { utf8: long.clone(), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 0..long.len(), face: FaceHandle(1) }] };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let lefts = frame.glyphs.iter().filter(|g| matches!(g, FrameGlyph::Stretch { box_vertical_edges: BoxVerticalEdges::Left, .. })).count();
    assert_eq!(lefts, 1);
    // Word wrap: a boxed word that fits only without its sides goes whole
    // to the next row.
    let word = "c".repeat(cells - 2);
    snapshot.wrap = LineWrap::Word;
    snapshot.text = TextSlice { utf8: format!("a {word}"), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 2..2 + word.len(), face: FaceHandle(1) }] };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let rows: std::collections::BTreeSet<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char: 'c', y, .. } => Some(*y as i64), _ => None }).collect();
    assert_eq!(rows.len(), 1, "the word stays on one row: {rows:?}");
    let a = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char: 'a', y, .. } => Some(*y as i64), _ => None }).unwrap();
    assert!(rows.iter().all(|&y| y > a), "and leaves the first");
}

/// A boxed mode line is one run: its first character owns the left edge,
/// the fill its face paints the band's rest with owns the right.
#[test]
fn a_boxed_mode_line_is_one_run_to_its_band_s_end() {
    use romacs_display_protocol::BoxVerticalEdges;
    use romacs_layout_input::{BoxLine, Decor};
    let mut snapshot = scene();
    snapshot.faces.push(Face { decor: Decor { boxed: Some(BoxLine { vertical: -1, horizontal: -1, color: None, style: 1 }), ..Default::default() }, ..snapshot.faces[0].clone() });
    snapshot.mode_line = Some(TextSlice { utf8: "abc".into(), face_runs: vec![], raw: vec![], extends: vec![] });
    snapshot.mode_line_face = FaceHandle(2);
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let band: Vec<_> = frame.glyphs.iter().filter(|g| g.row_role() == Some(romacs_display_protocol::frame_glyphs::GlyphRowRole::ModeLine))
        .filter_map(|g| g.box_vertical_edges()).collect();
    assert_eq!(band, [BoxVerticalEdges::Left, BoxVerticalEdges::Neither, BoxVerticalEdges::Neither, BoxVerticalEdges::Right]);
}

/// A band's stretch in a boxed mode line is in the run: blank, its width
/// kept (IT_APPLY_FACE_BOX), between the first character's left edge and
/// the fill's right.
#[test]
fn a_band_stretch_in_a_boxed_mode_line_stays_in_its_run() {
    use romacs_display_protocol::BoxVerticalEdges;
    use romacs_layout_input::{BandStretches, BoxLine, Decor, Stretch, StretchWidth};
    let mut snapshot = scene();
    snapshot.faces.push(Face { decor: Decor { boxed: Some(BoxLine { vertical: -1, horizontal: -1, color: None, style: 1 }), ..Default::default() }, ..snapshot.faces[0].clone() });
    snapshot.mode_line = Some(TextSlice { utf8: "a b".into(), face_runs: vec![], raw: vec![], extends: vec![] });
    snapshot.mode_line_face = FaceHandle(2);
    snapshot.band_stretches = BandStretches { header_line: vec![], mode_line: vec![Stretch { byte: 1, width: StretchWidth::Width { width: 30, zero_ok: false } }] };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let band: Vec<_> = frame.glyphs.iter().filter(|g| g.row_role() == Some(romacs_display_protocol::frame_glyphs::GlyphRowRole::ModeLine))
        .filter_map(|g| Some((g.box_vertical_edges()?, matches!(g, FrameGlyph::Stretch { .. })))).collect();
    assert_eq!(band, [(BoxVerticalEdges::Left, false), (BoxVerticalEdges::Neither, true), (BoxVerticalEdges::Neither, false), (BoxVerticalEdges::Right, true)]);
    let x_of = |c| frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char, x, width, .. } if *char == c => Some((*x, *width)), _ => None }).unwrap();
    let ((a, a_width), (b, _)) = (x_of('a'), x_of('b'));
    let stretch = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Stretch { x, width, .. } if *x == a + a_width => Some(*width), _ => None }).expect("the stretch after a");
    assert!(stretch > 0.0 && (a + a_width + stretch - b).abs() < 0.01, "b follows the stretch: {stretch} {a} {b}");
    // A stretch that opens a run under a box with sides owns its left edge
    // and keeps its width: no side spacer pads it.
    snapshot.faces[2].decor.boxed = Some(BoxLine { vertical: 2, horizontal: -1, color: None, style: 1 });
    snapshot.mode_line = Some(TextSlice { utf8: " b".into(), face_runs: vec![], raw: vec![], extends: vec![] });
    snapshot.band_stretches.mode_line = vec![Stretch { byte: 0, width: StretchWidth::Width { width: 30, zero_ok: false } }];
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let mode_line = Some(romacs_display_protocol::frame_glyphs::GlyphRowRole::ModeLine);
    let (x, width, edge) = frame.glyphs.iter().filter(|g| g.row_role() == mode_line).find_map(|g| match g {
        FrameGlyph::Stretch { x, width, box_vertical_edges, .. } => Some((*x, *width, *box_vertical_edges)),
        _ => None,
    }).expect("the opening stretch");
    assert_eq!((width, edge), (30.0, BoxVerticalEdges::Left));
    let b = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char: 'b', x, .. } => Some(*x), _ => None }).unwrap();
    assert!((b - x - 30.0).abs() < 0.01, "b right after the stretch's 30 pixels: {x} {b}");
}

/// A TAB in a boxed run is in the run (its stretch carries the run's
/// edges); a boxed mode line its text overflows closes at the band's edge
/// on its last glyph shown; a boxed segment between plain mode-line text
/// shares its baseline.
#[test]
fn tabs_and_bands_keep_their_box_runs_whole() {
    use romacs_display_protocol::{BoxVerticalEdges, frame_glyphs::GlyphRowRole};
    use romacs_layout_input::{BoxLine, Decor};
    let mut snapshot = scene();
    let boxed = |vertical, horizontal| Decor { boxed: Some(BoxLine { vertical, horizontal, color: None, style: 1 }), ..Default::default() };
    snapshot.faces[1].decor = boxed(2, -1);
    snapshot.text = TextSlice { utf8: "a\tb".into(), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }] };
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let lefts = frame.glyphs.iter().filter(|g| matches!(g.box_vertical_edges(), Some(BoxVerticalEdges::Left | BoxVerticalEdges::Both))).count();
    assert_eq!(lefts, 1, "one run: {:?}", frame.glyphs.iter().filter_map(|g| g.box_vertical_edges()).collect::<Vec<_>>());
    let tab = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Stretch { box_vertical_edges, .. } if *box_vertical_edges == BoxVerticalEdges::Neither => Some(()), _ => None });
    assert!(tab.is_some(), "the TAB is inside the run");
    // A TAB alone in a box keeps GNU's stop: no sides join its advance.
    let unboxed = { let mut s = snapshot.clone(); s.faces[1].decor = Decor::default(); romacs_layout::layout(&s, 1).unwrap() };
    let x_of = |frame: &romacs_display_protocol::FrameGlyphBuffer, c| frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char, x, .. } if *char == c => Some(*x), _ => None }).unwrap();
    snapshot.text.face_runs = vec![FaceRun { bytes: 1..2, face: FaceHandle(1) }];
    let lone = romacs_layout::layout(&snapshot, 1).unwrap();
    assert_eq!(x_of(&lone, 'b'), x_of(&unboxed, 'b'));
    let tab = lone.glyphs.iter().find_map(|g| match g { FrameGlyph::Stretch { box_vertical_edges, .. } => Some(*box_vertical_edges), _ => None });
    assert_eq!(tab, Some(BoxVerticalEdges::Both));
    // A display stretch in a box keeps its width: one of zero is no glyph.
    {
        use romacs_layout_input::{Stretch, StretchWidth};
        let mut zero = snapshot.clone();
        zero.text = TextSlice { utf8: "a b".into(), raw: vec![], extends: vec![], face_runs: vec![FaceRun { bytes: 0..3, face: FaceHandle(1) }] };
        zero.stretches = vec![Stretch { byte: 1, width: StretchWidth::Width { width: 0, zero_ok: true } }];
        let frame = romacs_layout::layout(&zero, 1).unwrap();
        assert!(!frame.glyphs.iter().any(|g| matches!(g, FrameGlyph::Stretch { width, .. } if *width > 0.0 && *width < 2.0)));
        let a = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char: 'a', x, width, .. } => Some(*x + *width), _ => None }).unwrap();
        let b = frame.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { char: 'b', x, .. } => Some(*x), _ => None }).unwrap();
        assert_eq!(a, b, "the zero stretch takes no room");
    }
    // A boxed mode line longer than its band.
    snapshot.faces.push(Face { decor: boxed(-1, -1), ..snapshot.faces[0].clone() });
    snapshot.mode_line = Some(TextSlice { utf8: "m".repeat(200), face_runs: vec![], raw: vec![], extends: vec![] });
    snapshot.mode_line_face = FaceHandle(2);
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let band: Vec<_> = frame.glyphs.iter().filter(|g| g.row_role() == Some(GlyphRowRole::ModeLine)).filter_map(|g| g.box_vertical_edges()).collect();
    assert_eq!((band.first(), band.last()), (Some(&BoxVerticalEdges::Left), Some(&BoxVerticalEdges::Right)));
    // The closing glyph's cell ends at the band's edge, where its border shows.
    let last = frame.glyphs.iter().rfind(|g| g.row_role() == Some(GlyphRowRole::ModeLine)).unwrap();
    let (x, width) = match last { FrameGlyph::Char { x, width, .. } => (*x, *width), _ => panic!("{last:?}") };
    assert_eq!(x + width, 400.0);
    // A segment with a box outside its text between plain characters.
    snapshot.faces[2].decor = Decor::default();
    snapshot.faces[1].decor = boxed(2, 3);
    snapshot.mode_line = Some(TextSlice { utf8: "xyz".into(), face_runs: vec![FaceRun { bytes: 1..2, face: FaceHandle(1) }], raw: vec![], extends: vec![] });
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let baselines: Vec<_> = frame.glyphs.iter().filter(|g| g.row_role() == Some(GlyphRowRole::ModeLine)).filter_map(|g| match g {
        FrameGlyph::Char { baseline, .. } => Some(*baseline), _ => None }).collect();
    assert_eq!(baselines.len(), 3);
    assert!(baselines.iter().all(|&b| b == baselines[0]), "{baselines:?}");
}

/// An image's slot and slice (produce_image_glyph): its body inside its
/// margins, the slice as a fraction of the image, the row grown to its
/// ascent and descent with the text on the same baseline, a relief's
/// strips around the body, and an image that failed to load as an outline.
#[test]
fn images_take_their_slots_on_the_rows_baseline() {
    use romacs_layout_input::InlineImage;
    let mut snapshot = scene();
    snapshot.text = TextSlice { extends: vec![], utf8: "a b\nc d\nz".into(), face_runs: vec![], raw: vec![] };
    let image = InlineImage { byte: 1, image: 7, surface: 0, width: 48, ascent: 40, descent: 10, body_x: 4, body_y: 5,
        source: (10, 0, 40, 40), full: (80, 40), relief: 2, relief_background: None };
    snapshot.images = vec![image, InlineImage { byte: 5, image: 0, surface: 0, width: 30, ascent: 15, descent: 15, body_x: 0, body_y: 0,
        source: (0, 0, 30, 30), full: (30, 30), relief: 0, relief_background: None }];
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let images: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Image { image_id, x, y, width, height, slot_rect, source_rect, .. } =>
            Some((image_id.get(), *x, *y, *width, *height, (slot_rect.x, slot_rect.y, slot_rect.width, slot_rect.height), *source_rect)),
        _ => None,
    }).collect();
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, baseline, .. } => Some((*char, *x, *baseline)),
        _ => None,
    }).collect();
    let (a, b) = (chars[0], chars[1]);
    assert_eq!(images.len(), 1, "a failed image draws no image: {images:?}");
    let (id, x, y, width, height, slot, source) = images[0];
    assert_eq!((id, width, height), (7, 40.0, 40.0));
    // The slot after `a', its top the row's ascent less its own above the baseline.
    let top = a.2 - 40.0;
    assert_eq!(slot, (a.1 + frame.char_width, top, 48.0, 50.0));
    assert_eq!((x, y), (slot.0 + 4.0, top + 5.0));
    assert_eq!(source, romacs_display_protocol::image::ImageSourceRect::new(0.125, 0.0, 0.5, 1.0).unwrap());
    // `b' after the slot, on the same baseline.
    assert_eq!((b.0, b.1, b.2), ('b', slot.0 + 48.0, a.2));
    // The relief: the face's background lighter above the body, darker
    // below, two pixels each; no sides, as the slice has neither of the
    // image's side edges.
    let strips: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Stretch { x, y, width, height, bg, .. } if *width == 2.0 || *height == 2.0 => Some((*x, *y, *width, *height, *bg)),
        _ => None,
    }).collect();
    assert_eq!(strips, [(x - 2.0, y - 2.0, 44.0, 2.0, Color::from_pixel(0x4c4c4c)), (x - 2.0, y + 40.0, 44.0, 2.0, Color::from_pixel(0x404040))]);
    // The failed image on the next row: a one-pixel outline in the face's foreground.
    let outline = frame.glyphs.iter().filter(|g| matches!(g, FrameGlyph::Stretch { bg, width, height, .. }
        if *bg == Color::from_pixel(0xffffff) && (*width == 1.0 || *height == 1.0))).count();
    assert_eq!(outline, 4);
}

/// A native surface (a model) takes an image's slot and draws as a surface
/// glyph on the row's baseline, with no failed-image outline.
#[test]
fn a_surface_takes_its_slot_on_the_rows_baseline() {
    use romacs_layout_input::InlineImage;
    let mut snapshot = scene();
    snapshot.text = TextSlice { extends: vec![], utf8: "a b".into(), face_runs: vec![], raw: vec![] };
    snapshot.images = vec![InlineImage { byte: 1, image: 0, surface: 9, width: 64, ascent: 48, descent: 0, body_x: 0,
        body_y: 0, source: (0, 0, 64, 48), full: (64, 48), relief: 0, relief_background: None }];
    let frame = romacs_layout::layout(&snapshot, 1).unwrap();
    let surfaces: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Surface { surface_id, x, y, width, height, .. } => Some((surface_id.get(), *x, *y, *width, *height)),
        _ => None,
    }).collect();
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, baseline, .. } => Some((*char, *x, *baseline)),
        _ => None,
    }).collect();
    let a = chars[0];
    assert_eq!(surfaces, [(9, a.1 + frame.char_width, a.2 - 48.0, 64.0, 48.0)]);
    assert_eq!((chars[1].0, chars[1].1, chars[1].2), ('b', a.1 + frame.char_width + 64.0, a.2));
    assert!(!frame.glyphs.iter().any(|g| matches!(g, FrameGlyph::Image { .. })));
    let outline = frame.glyphs.iter().filter(|g| matches!(g, FrameGlyph::Stretch { width, height, .. }
        if *width == 1.0 || *height == 1.0)).count();
    assert_eq!(outline, 0);
}

/// A character of another font that wraps to a new row sets that row on
/// its baseline, though it was reached on the row before.
#[test]
fn a_fallback_character_at_a_wrap_shares_its_rows_baseline() {
    let mut s = scene();
    let probe = romacs_layout::LayoutEngine::new().unwrap().layout(&s, 1).unwrap();
    let cell = probe.glyphs.iter().find_map(|g| match g { FrameGlyph::Char { width, .. } => Some(*width), _ => None }).unwrap();
    let text = "xxxx\u{6f22}x";
    s.width = (cell * 5.0) as u32;
    s.text.utf8 = text.into();
    s.text.face_runs = vec![FaceRun { bytes: 0..text.len(), face: FaceHandle(0) }];
    let frame = romacs_layout::LayoutEngine::new().unwrap().layout(&s, 1).unwrap();
    let baseline = |wanted: char, skip: usize| frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char: c, baseline, .. } if *c == wanted => Some(*baseline),
        _ => None,
    }).nth(skip).unwrap();
    assert_eq!(baseline('\u{6f22}', 0), baseline('x', 4));
}

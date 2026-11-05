//! A text animation (romacs_display_protocol::text_animation) drawn over a
//! frame layout2 laid out: every cell of its block shows the animation's
//! character in its colour, wherever the window sits in the frame.
use romacs_display_protocol::text_animation::{paint, TextAnimation, TextAnimationKind};
use romacs_display_protocol::{Color, FrameGlyph, FrameGlyphBuffer};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

fn window(text: &str, face_runs: Vec<FaceRun>) -> WindowSnapshot {
    let face = |foreground| Face { decor: Default::default(), foreground, background: [0; 3], family: "Liberation Mono".into(), pixel_size: 20.0, weight: 400, italic: false, width: 5 };
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: 7, width: 600, height: 400, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: true, stretches: vec![], wrap_flips: vec![], line_numbers: None,
        chrome: Default::default(),
        text: TextSlice { extends: vec![], raw: vec![], utf8: text.into(), face_runs },
        faces: vec![face([200; 3]), face([0x75, 0x6b, 0x59]), face([0xf4, 0xe7, 0xc3])],
    }
}

fn glyph<T>(frame: &FrameGlyphBuffer, row: u32, col: u16, f: impl Fn(&FrameGlyph) -> Option<T>) -> T {
    frame.glyphs.iter().find(|g| matches!(g, FrameGlyph::Char { slot_id, .. } if slot_id.window_id.get() == 7 && slot_id.row == row && slot_id.col == col))
        .and_then(f).unwrap()
}

fn face(g: &FrameGlyph) -> Option<romacs_display_protocol::FaceId> {
    match g { FrameGlyph::Char { face_id, .. } => Some(*face_id), _ => None }
}

fn shown(frame: &FrameGlyphBuffer, row: u32) -> String {
    let mut cells: Vec<(u16, char)> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { slot_id, char, .. } if slot_id.window_id.get() == 7 && slot_id.row == row => Some((slot_id.col, *char)),
        _ => None,
    }).collect();
    cells.sort();
    cells.into_iter().map(|(_, c)| c).collect()
}

fn logo(top: i32, cols: u32, rows: u32) -> TextAnimation {
    TextAnimation { window: 7, top, cols, rows, fps: 25, moving: true, still: false,
                    kind: TextAnimationKind::Logo { cell_aspect: 0.5, epoch: 0, colours: Default::default() } }
}

/// CELLS from a string: each character red, a space empty.
fn cells(text: &str) -> Vec<(char, Option<u32>)> {
    text.chars().map(|c| (c, (c != ' ').then_some(0xff5f59))).collect()
}

fn foreground(frame: &FrameGlyphBuffer, row: u32, col: u16) -> Color {
    let id = glyph(frame, row, col, face);
    frame.faces[&id].foreground
}

#[test]
fn the_block_shows_the_animation_cells_in_their_colours() {
    // Row 1 is shorter than the block; the text below stays.
    let mut frame = romacs_layout2::layout(&window("abcd\n x\nbelow", vec![]), 1).unwrap();
    let plain = glyph(&frame, 0, 0, face);
    paint(&mut frame, &logo(0, 4, 2), &cells("\u{2847}\u{28ff} ab  c"));
    assert_eq!(shown(&frame, 0), "\u{2847}\u{28ff} a");
    assert_eq!(shown(&frame, 1), "b  c");
    assert_eq!(shown(&frame, 2), "below");
    assert_eq!(foreground(&frame, 0, 0), Color::from_pixel(0xff5f59));
    assert_eq!(glyph(&frame, 0, 2, face), plain, "an empty cell keeps the evaluator's face");
    // A cell row 1 had no glyph for is placed on the grid like its row's.
    let x = |row, col| glyph(&frame, row, col, |g| match g { FrameGlyph::Char { x, .. } => Some(*x), _ => None });
    assert_eq!(x(1, 3), x(0, 3));
}

#[test]
fn a_block_scrolled_above_the_window_shows_its_lower_rows() {
    let mut frame = romacs_layout2::layout(&window("xxxx\nyyyy", vec![]), 1).unwrap();
    paint(&mut frame, &logo(-1, 4, 2), &cells("1234@@@@"));
    assert_eq!(shown(&frame, 0), "@@@@");
    assert_eq!(shown(&frame, 1), "yyyy");
}

#[test]
fn a_window_below_and_right_of_the_frame_corner_shows_its_block() {
    // A window placed down and to the right: its slots count from the
    // frame's corner, the block from the window's.
    let mut snapshot = window("xxxx\nyyyy\nzzzz", vec![]);
    snapshot.id = 9;
    let frame_snapshot = romacs_layout_input::FrameSnapshot {
        font_selection: Default::default(), font_alternatives: Default::default(), width: 1200, height: 900,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None,
        windows: vec![romacs_layout_input::PlacedWindowSnapshot { x: 300, y: 400, snapshot }],
    };
    let (mut frame, _) = romacs_layout2::LayoutEngine::new().unwrap().layout_frame(&frame_snapshot, 1, None).unwrap();
    let mut animation = logo(0, 4, 2);
    animation.window = 9;
    paint(&mut frame, &animation, &cells("@@@@::::"));
    let rows: Vec<String> = {
        let mut cells: Vec<(u32, u16, char)> = frame.glyphs.iter().filter_map(|g| match g {
            FrameGlyph::Char { slot_id, char, .. } if slot_id.window_id.get() == 9 => Some((slot_id.row, slot_id.col, *char)),
            _ => None,
        }).collect();
        cells.sort();
        let first = cells[0].0;
        (0..3).map(|r| cells.iter().filter(|c| c.0 == first + r).map(|c| c.2).collect()).collect()
    };
    assert_eq!(rows, ["@@@@", "::::", "zzzz"]);
}

#[test]
fn a_character_keeps_the_font_layout_resolved_for_it() {
    // Layout resolved U+2847 through a fallback font under the evaluator's
    // face; drawn in a new colour, the character keeps that font and glyph.
    let mut frame = romacs_layout2::layout(&window("ab", vec![]), 1).unwrap();
    let base = glyph(&frame, 0, 0, face);
    let fallback = romacs_display_protocol::font::ResolvedCharGlyph {
        resolved_font_id: romacs_display_protocol::font::ResolvedFontId(77),
        glyph_id: romacs_display_protocol::font::ResolvedGlyphId::new(1234),
        advance_px: 10.0,
    };
    frame.char_fonts.entry(base).or_default().insert('\u{2847}', fallback);
    paint(&mut frame, &logo(0, 2, 1), &cells("\u{2847} "));
    let coloured = glyph(&frame, 0, 0, face);
    assert_ne!(coloured, base);
    assert_eq!(frame.char_fonts[&coloured][&'\u{2847}'], fallback);
}

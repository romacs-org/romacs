//! A text animation (romacs_display_protocol::text_animation) drawn over a
//! frame layout2 laid out: every cell of its block shows the animation's
//! character, in the face the evaluator's frame drew that shade in.
use romacs_display_protocol::text_animation::{paint, TextAnimation, TextAnimationKind};
use romacs_display_protocol::{FrameGlyph, FrameGlyphBuffer};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

fn window(text: &str, face_runs: Vec<FaceRun>) -> WindowSnapshot {
    let face = |foreground| Face { foreground, background: [0; 3], family: "Liberation Mono".into(), pixel_size: 20.0 };
    WindowSnapshot {
        id: 7, width: 600, height: 400, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, tab_width: 8, wrap: Default::default(), cursor_off: true, stretches: vec![], line_numbers: None,
        chrome: Default::default(),
        text: TextSlice { raw: vec![], utf8: text.into(), face_runs },
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

fn sphere(top: i32, cols: u32, rows: u32, shades: u32) -> TextAnimation {
    TextAnimation { window: 7, top, cols, rows, fps: 25, moving: true, shades,
                    kind: TextAnimationKind::Armillary { pitch: -0.25, yaw: 0.4, zoom: 1.0, cell_aspect: 0.5, epoch: 0 } }
}

#[test]
fn the_block_shows_the_animation_cells_in_the_evaluators_shade_faces() {
    // The evaluator's still frame: "." in the dim shade's face, "@" in the
    // bright one's; row 1 is shorter than the block; text below stays.
    let runs = vec![FaceRun { bytes: 0..1, face: FaceHandle(1) }, FaceRun { bytes: 1..2, face: FaceHandle(2) }];
    let mut frame = romacs_layout2::layout(&window(".@  \n x\nbelow", runs), 1).unwrap();
    let (dim, bright, plain) = (glyph(&frame, 0, 0, face), glyph(&frame, 0, 1, face), glyph(&frame, 1, 1, face));
    paint(&mut frame, &sphere(0, 4, 2, 2), b"@,  : +.");
    assert_eq!(shown(&frame, 0), "@,  ");
    assert_eq!(shown(&frame, 1), ": +.");
    assert_eq!(shown(&frame, 2), "below");
    assert_eq!(glyph(&frame, 0, 0, face), bright);
    assert_eq!(glyph(&frame, 0, 1, face), dim);
    assert_eq!(glyph(&frame, 1, 0, face), dim);
    assert_eq!(glyph(&frame, 1, 2, face), bright, "a star is the brightest shade");
    assert_ne!(plain, bright);
    // A cell row 1 had no glyph for is placed on the grid like its row's.
    let x = |row, col| glyph(&frame, row, col, |g| match g { FrameGlyph::Char { x, .. } => Some(*x), _ => None });
    assert_eq!(x(1, 3), x(0, 3));
}

#[test]
fn a_shade_the_still_frame_lacks_takes_the_nearest_one() {
    let runs = vec![FaceRun { bytes: 0..1, face: FaceHandle(1) }];
    let mut frame = romacs_layout2::layout(&window(".", runs), 1).unwrap();
    let dim = glyph(&frame, 0, 0, face);
    paint(&mut frame, &sphere(0, 2, 1, 4), b"@.");
    assert_eq!(shown(&frame, 0), "@.");
    assert_eq!(glyph(&frame, 0, 0, face), dim);
}

#[test]
fn a_block_scrolled_above_the_window_shows_its_lower_rows() {
    let mut frame = romacs_layout2::layout(&window("xxxx\nyyyy", vec![]), 1).unwrap();
    paint(&mut frame, &sphere(-1, 4, 2, 4), b"1234@@@@");
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
        background_alpha: 1.0, children: vec![], tab_bar: None,
        windows: vec![romacs_layout_input::PlacedWindowSnapshot { x: 300, y: 400, snapshot }],
    };
    let (mut frame, _) = romacs_layout2::LayoutEngine::new().unwrap().layout_frame(&frame_snapshot, 1, None).unwrap();
    let mut animation = sphere(0, 4, 2, 4);
    animation.window = 9;
    paint(&mut frame, &animation, b"@@@@::::");
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

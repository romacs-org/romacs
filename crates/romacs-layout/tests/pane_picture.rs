//! O1b-1a: an owner's layout engine lays its fonts out in its slot (commit
//! 1a-2), and a frame it lays out round-trips as a pane picture that the
//! slot's receiver accepts, drawn as the master would lay out the same
//! window (commits 1a-1 and 1a-2; design O1b.md rev 3, §2.3, §6.2).
//!
//! The interface these tests drive beyond `romacs_display_protocol::
//! pane_picture` (see `romacs-display-protocol/src/tests/pane_picture_test.rs`):
//!
//! ```ignore
//! impl LayoutEngine {
//!     /// An engine whose fonts intern from owner slot SLOT's base,
//!     /// (SLOT+1) << 24 (`romacs_font::instance::FontIdBase::owner`).
//!     /// Err for 255.
//!     pub fn with_font_base(slot: u8) -> Result<Self, String>;
//! }
//! ```
use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture};
use romacs_display_protocol::{Color, DisplayWindowId, FrameGlyphBuffer};
use romacs_layout::LayoutEngine;
use romacs_layout_input::{Face, FaceHandle, FrameSnapshot, PlacedWindowSnapshot, TextSlice, WindowSnapshot};

const SLOT: u8 = 3;
const OWN: u64 = 5;
const MASTER: u64 = 900;

/// A frame of one window, ID, showing plain text and an emoji with point
/// at the start.
fn snapshot(id: u64) -> FrameSnapshot {
    let face = Face { decor: Default::default(), foreground: [255; 3], background: [0; 3], family: "Liberation Mono".into(), pixel_size: 20.0, weight: 400, italic: false, width: 5 };
    let text = TextSlice { extends: vec![], raw: vec![], utf8: "abc \u{2764}\u{FE0F}\nnext line".into(), face_runs: vec![] };
    let window = WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(), id, width: 400, height: 100, start_byte: 0, point_byte: 0, buffer_version: 1,
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0), vscroll_px: 0, hscroll: Default::default(),
        tab_width: 8, wrap: Default::default(), cursor_off: false, cursor: Default::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(),
        text, faces: vec![face] };
    FrameSnapshot { font_selection: Default::default(), font_alternatives: Default::default(), fontsets: Default::default(), width: 400, height: 100,
        windows: vec![PlacedWindowSnapshot { x: 0, y: 0, snapshot: window }], background_alpha: 1.0, children: vec![], tab_bar: None,
        chrome: Default::default(), status_bar: None, no_font: Default::default() }
}

/// Window ID laid out by ENGINE, with its cursor.
fn lay_out(engine: &mut LayoutEngine, id: u64) -> FrameGlyphBuffer {
    engine.layout_frame(&snapshot(id), 1, Some((id, Color::from_pixel(0xffffff)))).unwrap().0
}

/// Every font id FRAME names: its table's keys and ids, its characters'
/// and shaped clusters' fonts, and its faces' primary fonts.
fn font_ids(frame: &FrameGlyphBuffer) -> Vec<u32> {
    let mut ids: Vec<u32> = frame.fonts.iter().flat_map(|(key, font)| [key.0, font.id.0]).collect();
    ids.extend(frame.char_fonts.values().flat_map(|chars| chars.values().map(|glyph| glyph.resolved_font_id.0)));
    ids.extend(frame.shaped_clusters.values().flat_map(|clusters| clusters.values().flatten().map(|glyph| glyph.resolved_font_id.0)));
    ids.extend(frame.faces.values().filter_map(|face| face.default_resolved_font_id).map(|id| id.0));
    ids
}

/// Whether ID is in owner slot SLOT's range.
fn in_slot(id: u32, slot: u8) -> bool {
    let base = (u64::from(slot) + 1) << 24;
    (base..base + (1 << 24)).contains(&u64::from(id))
}

#[test]
fn an_owner_engine_lays_its_fonts_out_in_its_slot() {
    let frame = lay_out(&mut LayoutEngine::with_font_base(SLOT).unwrap(), OWN);
    assert!(!frame.fonts.is_empty() && !frame.char_fonts.is_empty(), "the text resolved fonts");
    let ids = font_ids(&frame);
    assert!(ids.iter().all(|&id| in_slot(id, SLOT)), "ids outside slot {SLOT}: {ids:x?}");
    assert!(ids.contains(&((u32::from(SLOT) + 1) << 24)), "the first font interned has the slot's first id: {ids:x?}");
}

#[test]
fn two_owner_engines_never_share_a_font_id() {
    let zero = font_ids(&lay_out(&mut LayoutEngine::with_font_base(0).unwrap(), OWN));
    let one = font_ids(&lay_out(&mut LayoutEngine::with_font_base(1).unwrap(), OWN));
    assert!(!zero.is_empty() && !one.is_empty());
    assert!(zero.iter().all(|&id| in_slot(id, 0)), "{zero:x?}");
    assert!(one.iter().all(|&id| in_slot(id, 1)), "{one:x?}");
    assert!(zero.iter().all(|id| !one.contains(id)), "shared: {zero:x?} {one:x?}");
}

#[test]
fn slot_255_is_refused() {
    assert!(LayoutEngine::with_font_base(255).is_err());
    assert!(LayoutEngine::with_font_base(254).is_ok());
}

#[test]
fn a_laid_out_pane_round_trips_and_draws_as_the_masters_window() {
    let owned = lay_out(&mut LayoutEngine::with_font_base(SLOT).unwrap(), OWN);
    assert_eq!(owned.window_cursors.len(), 1, "the window's cursor is laid out");
    let head = PaneHead { owner: 41, window: MASTER, view: 11, request: 13, v: 17, c: 19 };
    let picture = PanePicture::from_frame(
        head,
        PaneFrame { frame: owned, animations: vec![], submitted: None },
        DisplayWindowId::new(OWN as i64),
    );
    let decoded = PanePicture::decode(&picture.encode(), SLOT).expect("slot SLOT's receiver accepts its owner's picture");
    assert_eq!(decoded, picture);
    assert_eq!((decoded.width, decoded.height), (400, 100));
    // The same window laid out under the master's id, by an engine of the
    // same slot: what the pane must draw.
    let expected = lay_out(&mut LayoutEngine::with_font_base(SLOT).unwrap(), MASTER);
    let drawn = decoded.into_frame();
    assert_eq!(drawn.glyphs, expected.glyphs);
    assert_eq!(drawn.window_cursors, expected.window_cursors);
    assert_eq!(drawn.faces, expected.faces);
    assert_eq!(drawn.fonts, expected.fonts);
    assert_eq!(drawn.char_fonts, expected.char_fonts);
    assert_eq!(drawn.shaped_clusters, expected.shaped_clusters);
    assert_eq!(drawn.fringe_bitmaps, expected.fringe_bitmaps);
    assert_eq!(
        (drawn.width, drawn.height, drawn.background, drawn.char_width, drawn.char_height),
        (expected.width, expected.height, expected.background, expected.char_width, expected.char_height),
    );
}

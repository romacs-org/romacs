use romacs_display_protocol::FrameGlyph;
use romacs_vm::Vm;

#[test]
fn frame_layout_consumes_real_vm_window_tree() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    assert!(line > 0 && column > 0);
    vm.set_frame_geometry(400, 300, line, column).unwrap();
    let lower = (300 - line) / 2;
    let upper = 300 - line - lower;
    vm.eval_source(&format!(r#"(progn
      (setq a (selected-window)) (set-window-new-pixel a {upper})
      (setq b (split-window-internal a {lower} nil 0.5))
      (set-buffer (get-buffer-create "top")) (insert "TOP") (set-window-buffer a (current-buffer))
      (set-buffer (get-buffer-create "lower")) (insert "LOWER") (set-window-buffer b (current-buffer))
      (setq mode-line-format '("%b")))"#)).unwrap();
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    let (frame, ends) = romacs_layout2::layout_frame(&snapshot, 7).unwrap();
    assert_eq!((frame.width, frame.height), (400.0, 300.0), "frame cannot be first leaf only");
    let chars: Vec<_> = frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, clip_rect, face_id, .. } => Some((*char, *x, *y, *clip_rect, *face_id)),
        _ => None,
    }).collect();
    assert!(chars.iter().any(|(c, _, y, clip, _)| *c == 'L' && *y == upper as f32 && clip.is_some_and(|r| r.y == upper as f32)), "lower window must be offset/clipped: {chars:?}");
    let end_of = |id| ends.iter().find(|end| end.id == id).map(|end| (end.end, end.visible_end));
    // Each window shows its whole text, end position included.
    assert_eq!(end_of(snapshot.windows[0].snapshot.id), Some((3, 4)));
    assert_eq!(end_of(snapshot.windows[1].snapshot.id), Some((5, 6)));
    assert!(frame.faces.len() >= 3 * 2, "window faces must not collide");
    assert!(chars.iter().all(|(_,_,_,_,id)| frame.faces.contains_key(id)));
    vm.display_apply_layout(&ends).unwrap();
    let end = vm.eval_source("(window-end b)").unwrap();
    assert_eq!(end.display().unwrap(), "6");
    let tiny = romacs_layout_input::Face { pixel_size: 10.0, ..face.clone() };
    let (tiny_line, _) = romacs_layout2::frame_metrics(&tiny).unwrap();
    assert!(tiny_line < line, "metrics must come from resolved font, not constants");
}

/// `layout_frame` must reject a window placement that overlaps the frame or
/// reuses an id, rather than drawing it.
///
/// Mutation found this uncovered: replacing the bounds-and-duplicate check with
/// `|| false` left all eight layout tests green. An unchecked placement means
/// two windows can claim the same id (so `window-end` results alias) or a window
/// can extend past the frame (so glyphs land outside the surface) with nothing
/// failing.
#[test]
fn invalid_window_placement_is_rejected_not_drawn() {
    fn snap(id: u64, w: u32, h: u32, face: &romacs_layout_input::Face) -> romacs_layout_input::WindowSnapshot {
        romacs_layout_input::WindowSnapshot {
            id, width: w, height: h, start_byte: 0, point_byte: 0, buffer_version: 0,
            text: romacs_layout_input::TextSlice { utf8: String::new(), face_runs: vec![], raw: vec![] },
            mode_line: None, mode_line_face: romacs_layout_input::FaceHandle(0), header_line: None, header_line_face: romacs_layout_input::FaceHandle(0),
            vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], line_numbers: None, chrome: Default::default(), faces: vec![face.clone()],
        }
    }
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();

    // Two windows sharing one id: `ends` would carry two entries for the same
    // window and the later would silently win.
    let duplicate = romacs_layout_input::FrameSnapshot { font_selection: romacs_layout_input::FontSelectionState::default(),
        font_alternatives: romacs_layout_input::FontAlternativeState::default(),
        width: 400, height: 300,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 0, snapshot: snap(7, 400, 150, &face) },
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 150, snapshot: snap(7, 400, 150, &face) },
        ],
    };
    assert!(romacs_layout2::layout_frame(&duplicate, 1).is_err(),
            "duplicate window id must be rejected");

    // A window taller than the frame: its glyphs would be placed outside the
    // presented surface.
    let overflow = romacs_layout_input::FrameSnapshot { font_selection: romacs_layout_input::FontSelectionState::default(),
        font_alternatives: romacs_layout_input::FontAlternativeState::default(),
        width: 400, height: 300,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![romacs_layout_input::PlacedWindowSnapshot {
            x: 0, y: 200, snapshot: snap(1, 400, 200, &face) }],
    };
    assert!(romacs_layout2::layout_frame(&overflow, 1).is_err(),
            "placement extending past the frame must be rejected");

    // The honest control: the same windows, placed legally, do lay out.
    let valid = romacs_layout_input::FrameSnapshot { font_selection: romacs_layout_input::FontSelectionState::default(),
        font_alternatives: romacs_layout_input::FontAlternativeState::default(),
        width: 400, height: 300,
        background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None, windows: vec![
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 0, snapshot: snap(1, 400, 150, &face) },
            romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 150, snapshot: snap(2, 400, 150, &face) },
        ],
    };
    assert!(romacs_layout2::layout_frame(&valid, 1).is_ok(),
            "legal placement must still lay out");
}

/// Margins that consume a window's body must not terminate redisplay.
///
/// GNU validates margins once against the window they are assigned to
/// (`src/window.c:8477`), copies them to the new window on split without
/// re-validating (`:5610-5612`), and clamps the resulting body width at zero
/// under an explicit "Don't return a negative value" (`:1136-1137`) while
/// redisplay continues normally. Measured on a 640px root at char-width 8:
/// margins `(30 . 30)` leave a 160px body, and splitting into two 320px windows
/// leaves both bodies at 0 with redisplay still returning.
///
/// So a zero-width body is legal and VM2's zero-width snapshot is parity. What
/// must not happen is `layout_frame` rejecting it, which propagates out of the
/// GUI loop and kills redisplay -- the failure mode s121 was repaired to
/// prevent.
#[test]
fn margins_consuming_the_body_still_render_the_frame() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    // The geometry has to satisfy both halves of GNU's behaviour: 60 columns of
    // margin must FIT the window they are assigned to (or GNU and VM2 both
    // rightly refuse them, src/window.c:8477), yet must EXCEED a half-width
    // window after the split. So frame > 60 cols and half < 60 cols, i.e. a
    // frame of 61..120 columns. The lane measured this at 640px with an 8px
    // cell -- 80 columns -- so use 80 columns here too: margins take 60,
    // leaving a 20-column body, and each 40-column half is fully consumed.
    let width = column * 80;
    vm.set_frame_geometry(width, 300, line, column).unwrap();
    let half = width / 2;
    vm.eval_source(&format!(r#"(progn
      (setq a (selected-window))
      (set-buffer (get-buffer-create "left")) (insert "LEFT") (set-window-buffer a (current-buffer))
      (set-window-margins a 30 30)
      (set-window-new-pixel a {half})
      (setq b (split-window-internal a {half} t 0.5))
      (set-buffer (get-buffer-create "right")) (insert "RIGHT") (set-window-buffer b (current-buffer)))"#))
        .unwrap();

    let snapshot = vm.frame_snapshot(width, 300).unwrap();
    // Parity with GNU: the publisher reports a zero-width body, it does not refuse.
    assert!(snapshot.windows.iter().any(|w| w.snapshot.width == 0),
            "margins should consume at least one body, as GNU does: {:?}",
            snapshot.windows.iter().map(|w| w.snapshot.width).collect::<Vec<_>>());

    // The MUST: laying this out must succeed and keep the frame renderable.
    let (frame, ends) = romacs_layout2::layout_frame(&snapshot, 11)
        .expect("a body consumed by margins must not fail layout");
    assert_eq!((frame.width, frame.height), (width as f32, 300.0));
    assert_eq!(ends.len(), snapshot.windows.len(),
               "every window must still report a window-end");
    // An empty body displays nothing, so window-end equals window-start.
    for (placed, romacs_layout_input::WindowEnd { id, end, .. }) in snapshot.windows.iter().zip(&ends) {
        assert_eq!(*id, placed.snapshot.id);
        if placed.snapshot.width == 0 {
            assert_eq!(*end, placed.snapshot.start_byte,
                       "an empty body displays nothing, so end == start");
        }
    }
}

/// An empty body whose ORIGIN falls outside the frame must still lay out.
///
/// The symmetric case above keeps the body origin inside the frame, so it never
/// exercised the bounds check. With ASYMMETRIC margins the origin itself leaves
/// the frame: 80 columns with margins `(70 . 0)` is accepted on assignment,
/// because GNU only requires the remaining width to clear
/// `MIN_SAFE_WINDOW_PIXEL_WIDTH` against the window as it is then
/// (`src/window.c:8477-8481`), and 10 columns do. Splitting in half leaves the
/// margined window 40 columns wide with a 70-column left margin, so its body
/// origin sits past the right frame edge and its width clamps to zero under
/// GNU's "Don't return a negative value" (`:1137`). GNU redisplays this.
///
/// The split must put the new window on the LEFT (`side` = `'left`, honoured at
/// `window.rs:431-432` as GNU documents at `window.c:5378+`), so the margined
/// window shifts right and its body origin becomes x plus margin. With the
/// default right split the margined window stays at x=0 and the origin never
/// leaves the frame, which is why the first attempt at this fixture produced an
/// in-bounds empty body and the guard below rejected it.
///
/// `layout_frame` validated the BODY origin against the frame width, so it
/// rejected the whole frame and the error propagated out of the GUI loop,
/// killing redisplay. Total-window bounds must still be enforced; an empty body
/// draws nothing and cannot overflow anything.
#[test]
fn an_empty_body_positioned_outside_the_frame_still_renders() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    let width = column * 80;
    vm.set_frame_geometry(width, 300, line, column).unwrap();
    let half = width / 2;
    vm.eval_source(&format!(r#"(progn
      (setq a (selected-window))
      (set-buffer (get-buffer-create "wide")) (insert "WIDE") (set-window-buffer a (current-buffer))
      (set-window-margins a 70 nil)
      (set-window-new-pixel a {half})
      (setq b (split-window-internal a {half} 'left 0.5))
      (set-buffer (get-buffer-create "other")) (insert "OTHER") (set-window-buffer b (current-buffer)))"#))
        .unwrap();

    let snapshot = vm.frame_snapshot(width, 300).unwrap();
    // The condition this test exists for: an empty body placed beyond the frame.
    let offending: Vec<_> = snapshot.windows.iter()
        .filter(|w| w.snapshot.width == 0 && w.x + w.snapshot.width > snapshot.width)
        .map(|w| (w.x, w.snapshot.width))
        .collect();
    assert!(!offending.is_empty(),
        "fixture must produce an empty body whose origin leaves the frame; got {:?}",
        snapshot.windows.iter().map(|w| (w.x, w.snapshot.width)).collect::<Vec<_>>());

    // The MUST: this must lay out, not abort the frame.
    let (frame, ends) = romacs_layout2::layout_frame(&snapshot, 11)
        .expect("an empty body outside the frame must not fail layout");
    assert_eq!((frame.width, frame.height), (width as f32, 300.0));
    assert_eq!(ends.len(), snapshot.windows.len());
    for (placed, romacs_layout_input::WindowEnd { id, end, .. }) in snapshot.windows.iter().zip(&ends) {
        assert_eq!(*id, placed.snapshot.id);
        if placed.snapshot.width == 0 {
            assert_eq!(*end, placed.snapshot.start_byte,
                       "an empty body displays nothing, so end == start");
        }
    }
}

/// The frame's `alpha-background' reaches the painted frame, including a
/// repaint that reuses the last layout.
#[test]
fn background_opacity_reaches_every_paint() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(400, 300, line, column).unwrap();
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    let mut snapshot = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(engine.layout_frame(&snapshot, 1, None).unwrap().0.background_alpha, 1.0);
    snapshot.background_alpha = 0.8;
    assert!(snapshot.lays_out_like(&snapshot.clone()), "opacity alone needs no new layout");
    assert_eq!(engine.layout_frame(&snapshot, 2, None).unwrap().0.background_alpha, 0.8);
    // A child frame paints with its own.
    vm.eval_source("(romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 3)))").unwrap();
    let mut snapshot = vm.frame_snapshot(400, 300).unwrap();
    snapshot.children[0].background_alpha = 0.6;
    let (children, _) = engine.layout_children(&snapshot, 3, None).unwrap();
    assert_eq!(children.iter().map(|c| c.background_alpha).collect::<Vec<_>>(), [0.6]);
}

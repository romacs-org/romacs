//! Child frames from a real VM: each lays out as a frame of its own,
//! placed over the root frame with its border and background, and the root
//! frame's glyphs are the same with or without them.
use romacs_display_protocol::{DisplayFrameId, FrameGlyph};
use romacs_vm::Vm;

fn chars(frame: &romacs_display_protocol::FrameGlyphBuffer) -> Vec<(char, f32, f32)> {
    frame.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::Char { char, x, y, .. } => Some((*char, *x, *y)),
        _ => None,
    }).collect()
}

#[test]
fn a_child_frame_is_laid_out_apart_over_an_unchanged_root() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(400, 300, line, column).unwrap();
    vm.eval_source("(progn (insert \"ROOT TEXT UNDER THE LAYER\") (set-buffer (get-buffer-create \"popup\")) (insert \"POPUP\") (setq mode-line-format nil))").unwrap();
    let before = vm.frame_snapshot(400, 300).unwrap();
    assert!(before.children.is_empty());
    vm.eval_source("(let ((c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 2) (left . 30) (top . 5) (child-frame-border-width . 1) (background-color . \"#102030\"))))) \
                      (set-window-buffer (frame-root-window c) \"popup\"))").unwrap();
    let after = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(after.children.len(), 1);
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    let (root_before, _) = engine.layout_frame(&before, 7, None).unwrap();
    let (root_after, _) = engine.layout_frame(&after, 8, None).unwrap();
    assert_eq!(chars(&root_before), chars(&root_after), "a child frame moved the root frame's glyphs");
    let (children, ends) = engine.layout_children(&after, 8, None).unwrap();
    let child = &children[0];
    let placement = child.frame_placement;
    assert_eq!(placement.parent(), Some(DisplayFrameId::new(1)));
    assert_ne!(placement.frame(), DisplayFrameId::new(1));
    assert_eq!(placement.presentation(), root_after.presentation_id);
    let rect = placement.outer_in_parent();
    assert_eq!((rect.x(), rect.y(), rect.width(), rect.height()),
               (30.0, 5.0, (10 * column + 2) as f32, (2 * line + 2) as f32));
    assert_eq!(child.border_width, 1.0);
    assert_eq!(child.background, romacs_display_protocol::Color::from_pixel(0x102030));
    // The child's text starts inside its border, in its own coordinates.
    let text = chars(child);
    assert_eq!(text.first().map(|&(c, x, y)| (c, x, y)), Some(('P', 1.0, 1.0)), "{text:?}");
    assert_eq!(ends.len(), 1, "the child's window reports its end");
}

/// A layer's animation, edge and selection reach its glyphs from Lisp: the
/// band is its first window's selected row, in the face that row starts
/// in; a root frame laid out again unchanged keeps its layout identity,
/// which lets the host reuse its picture under a changing layer.
#[test]
fn a_layer_publishes_how_it_animates_and_its_selection_band() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(400, 300, line, column).unwrap();
    vm.eval_source("(progn (set-buffer (get-buffer-create \"popup\")) (insert \"one\\ntwo\\nthree\") (setq mode-line-format nil) \
                      (put-text-property 5 8 'face '(:background \"#ff0000\")))").unwrap();
    vm.eval_source("(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 3) (left . 0) (top . 5)))) \
                    (set-window-buffer (frame-root-window c) \"popup\")").unwrap();
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    let (root, _) = engine.layout_frame(&snapshot, 2, None).unwrap();
    let child = &snapshot.children[0];
    // Unset (romacs-layer.el not loaded): no animation, bottom, no selection.
    assert_eq!((child.animation, child.edge, child.selection),
               (romacs_layout_input::LayerAnimation::Off, romacs_layout_input::LayerEdge::Bottom, None));
    vm.eval_source("(setq romacs-layer-animation 'reduced) (modify-frame-parameters c '((romacs-layer-edge . top) (romacs-layer-selection . 1)))")
                    .unwrap();
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    let (again, _) = engine.layout_frame(&snapshot, 3, None).unwrap();
    assert_ne!(root.layout_identity, 0);
    assert_eq!(again.layout_identity, root.layout_identity, "an unchanged root is the same layout");
    let (children, _) = engine.layout_children(&snapshot, 3, None).unwrap();
    let layer = children[0].layer;
    assert_eq!((layer.animation, layer.edge),
               (romacs_display_protocol::LayerAnimation::Reduced, romacs_display_protocol::LayerEdge::Top));
    let band = layer.selection.expect("row 1 is shown");
    assert_eq!((band.y, band.height), (line as f32, line as f32));
    assert_eq!(band.color, romacs_display_protocol::Color::from_pixel(0xff0000));
    // The window's whole width, painted as a background after its row's text.
    assert_eq!((band.x, band.width), (0.0, children[0].width));
    assert!(matches!(children[0].glyphs.last(), Some(romacs_display_protocol::FrameGlyph::Background { bounds, color })
        if bounds.y == band.y && bounds.width == band.width && *color == band.color));
    vm.eval_source("(setq romacs-layer-animation t) (modify-frame-parameters c '((romacs-layer-selection . 7)))").unwrap();
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    let (children, _) = engine.layout_children(&snapshot, 4, None).unwrap();
    assert_eq!(children[0].layer.animation, romacs_display_protocol::LayerAnimation::Full);
    assert_eq!(children[0].layer.selection, None, "no band past the rows shown");
    // Glass: rounded and lit, its border padding (unpainted), and its
    // `background-color' its text's background, as a frame's default face's.
    vm.eval_source("(modify-frame-parameters c '((romacs-layer-radius . 12) (romacs-layer-shadow . t) \
                    (romacs-layer-glow . \"#d48a20\") (child-frame-border-width . 10) (background-color . \"#102030\")))").unwrap();
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    let (children, _) = engine.layout_children(&snapshot, 5, None).unwrap();
    let child = &children[0];
    assert_eq!((child.layer.radius, child.layer.shadow, child.layer.glow),
               (12.0, true, Some(romacs_display_protocol::Color::from_pixel(0xd48a20))));
    assert_eq!(child.border_width, 0.0);
    let glass = romacs_display_protocol::Color::from_pixel(0x102030);
    assert!(child.glyphs.iter().any(|g| matches!(g, FrameGlyph::Char { char: 'o', face_id, .. }
        if child.resolved_face(*face_id).bg == glass)), "text shows the frame's background");
}

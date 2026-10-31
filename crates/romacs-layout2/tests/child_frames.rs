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

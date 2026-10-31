use romacs_display_protocol::{Color, CursorStyle};
use romacs_vm::Vm;

#[test]
fn point_uses_text_geometry_and_selection_switches_cursor_kind() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, col) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(800, 600, line, col).unwrap();
    vm.eval_source("(setq a (selected-window)) (set-window-new-pixel a 400) (setq b (split-window-internal a 400 t 0.5)) (set-buffer (get-buffer-create \"cursor\")) (insert \"abc\\nλz\\n\") (set-window-buffer a (current-buffer)) (set-window-buffer b (current-buffer)) (setq mode-line-format '(\"%b\")) (select-window a) (goto-char 2)").unwrap();
    let mut snap = vm.frame_snapshot(800, 600).unwrap();
    let first = snap.windows[0].snapshot.id;
    let second = snap.windows[1].snapshot.id;
    let rum = Color::from_pixel(0x9099d9);
    let (frame, _) = romacs_layout2::layout_frame_with_cursors(&snap, 1, first, rum).unwrap();
    assert_eq!(frame.window_cursors.len(), 2, "minibuffer has no cursor");
    let cursor = frame.active_cursor().unwrap();
    assert_eq!(cursor.window_id.get(), first as i64);
    assert_eq!(cursor.style, CursorStyle::FilledBox);
    assert_eq!(cursor.color, rum);
    assert_eq!(cursor.x, col as f32);
    assert_eq!(frame.window_cursors[1].style, CursorStyle::Hollow);
    snap.windows[0].snapshot.point_byte = snap.windows[0].snapshot.text.utf8.len();
    let (frame, _) = romacs_layout2::layout_frame_with_cursors(&snap, 2, second, rum).unwrap();
    assert_eq!(frame.active_cursor().unwrap().window_id.get(), second as i64);
    assert_eq!(frame.window_cursors[0].y, 2.0 * line as f32, "EOF after newline owns a cursor slot");
    assert_eq!(frame.window_cursors[0].x, 0.0);
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    for revision in 3..8 {
        let (cached, cached_ends) = engine.layout_frame(&snap, revision, Some((second, rum))).unwrap();
        let (fresh, fresh_ends) = romacs_layout2::layout_frame_with_cursors(&snap, revision, second, rum).unwrap();
        assert_eq!(cached.glyphs, fresh.glyphs);
        assert_eq!(cached_ends, fresh_ends);
        assert_eq!(cached.fonts.len(), fresh.fonts.len());
        assert_eq!(cached.active_cursor().unwrap().x, fresh.active_cursor().unwrap().x);
        // The snapshot and resolver are wholly owned; VM compaction cannot invalidate either.
        vm.collect_garbage();
    }
}

#[test]
fn unused_faces_do_not_multiply_unicode_bindings() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let snapshot = romacs_layout_input::WindowSnapshot {
        id: 1, width: 800, height: 400, start_byte: 0, point_byte: 0, buffer_version: 1,
        text: romacs_layout_input::TextSlice { utf8: "Καλημέρα Жизнь".into(), face_runs: vec![], raw: vec![] },
        mode_line: None, mode_line_face: romacs_layout_input::FaceHandle(0), header_line: None, header_line_face: romacs_layout_input::FaceHandle(0), vscroll_px: 0, tab_width: 8, wrap: Default::default(), cursor_off: false,
        faces: vec![face; 24],
    };
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    let frame = engine.layout(&snapshot, 1).unwrap();
    assert_eq!(frame.char_fonts.len(), 1, "only the face that uses Unicode needs bindings");
    assert!(frame.char_fonts[&romacs_display_protocol::FaceId::new(0)].contains_key(&'Ж'));
}


/// An ACTIVE minibuffer (a prompt waiting for keys) is the selected window and
/// shows the selected cursor at point, as GNU's does; the window that called
/// the prompt keeps the non-selected hollow cursor. An inactive echo area
/// still has none (the test above).
#[test]
fn selected_minibuffer_shows_the_active_cursor_at_point() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, col) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(800, 600, line, col).unwrap();
    vm.eval_source("(setq mode-line-format '(\"%b\")) (set-buffer (get-buffer-create \" *Minibuf-1*\")) (insert \"Name: abc\") (set-window-buffer (minibuffer-window) (current-buffer)) (select-window (minibuffer-window))").unwrap();
    let snap = vm.frame_snapshot(800, 600).unwrap();
    let mini = snap.windows.last().unwrap().snapshot.id;
    let main = snap.windows[0].snapshot.id;
    let rum = Color::from_pixel(0x9099d9);
    let (frame, _) = romacs_layout2::layout_frame_with_cursors(&snap, 1, mini, rum).unwrap();
    let cursor = frame.active_cursor().expect("the active minibuffer has a cursor");
    assert_eq!(cursor.window_id.get(), mini as i64);
    assert_eq!(cursor.style, CursorStyle::FilledBox);
    assert_eq!(cursor.x, 9.0 * col as f32, "after \"Name: abc\"");
    let main_cursor = frame.window_cursors.iter().find(|c| c.window_id.get() == main as i64).unwrap();
    assert_eq!(main_cursor.style, CursorStyle::Hollow);
}

/// `internal-show-cursor' (dispnew.c, GNU's blink-cursor-mode) turns one
/// window's cursor off: the frame publishes none for it, the other window
/// keeps its own, and turning it on publishes it again.
#[test]
fn a_cursor_shown_off_is_not_published() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, col) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(800, 600, line, col).unwrap();
    vm.eval_source("(setq a (selected-window)) (set-window-new-pixel a 400) (setq b (split-window-internal a 400 t 0.5)) (set-buffer (get-buffer-create \"blink\")) (insert \"abc\") (set-window-buffer a (current-buffer)) (set-window-buffer b (current-buffer)) (setq mode-line-format '(\"%b\")) (select-window a)").unwrap();
    let rum = Color::from_pixel(0x9099d9);
    let cursors = |vm: &mut Vm| {
        let snap = vm.frame_snapshot(800, 600).unwrap();
        let (selected, other) = (snap.windows[0].snapshot.id, snap.windows[1].snapshot.id);
        let (frame, _) = romacs_layout2::layout_frame_with_cursors(&snap, 1, selected, rum).unwrap();
        let ids: Vec<i64> = frame.window_cursors.iter().map(|c| c.window_id.get()).collect();
        (ids, selected as i64, other as i64)
    };
    let (ids, selected, other) = cursors(&mut vm);
    assert_eq!(ids, [selected, other]);
    vm.eval_source("(internal-show-cursor nil nil)").unwrap();
    assert_eq!(vm.eval_source("(internal-show-cursor-p)").unwrap().display().unwrap(), "nil");
    assert_eq!(cursors(&mut vm).0, [other], "the selected window's cursor is off");
    vm.eval_source("(internal-show-cursor nil t)").unwrap();
    assert_eq!(cursors(&mut vm).0, [selected, other]);
}

/// A frame that differs from the last one only in which cursors are off is
/// shown from the last layout, stamped with its own presentation; any other
/// change is laid out again.
#[test]
fn a_cursor_only_change_reuses_the_last_layout() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, col) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(800, 600, line, col).unwrap();
    vm.eval_source("(set-buffer (get-buffer-create \"reuse\")) (insert \"abc\\ndef\") (set-window-buffer nil (current-buffer)) (setq mode-line-format '(\"%b\")) (goto-char 2)").unwrap();
    let rum = Color::from_pixel(0x9099d9);
    let mut snap = vm.frame_snapshot(800, 600).unwrap();
    let id = snap.windows[0].snapshot.id;
    let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
    let (on, ends) = engine.layout_frame(&snap, 1, Some((id, rum))).unwrap();
    snap.windows[0].snapshot.cursor_off = true;
    snap.windows[0].snapshot.buffer_version += 1;
    let (off, off_ends) = engine.layout_frame(&snap, 2, Some((id, rum))).unwrap();
    assert_eq!((off.glyphs == on.glyphs, off_ends == ends), (true, true));
    assert!(off.window_cursors.iter().all(|c| c.window_id.get() != id as i64), "the hidden cursor is not published");
    assert_eq!(off.presentation_id.get(), 2);
    snap.windows[0].snapshot.cursor_off = false;
    let (again, _) = engine.layout_frame(&snap, 3, Some((id, rum))).unwrap();
    assert_eq!(again.window_cursors.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>(), on.window_cursors.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>());
    snap.windows[0].snapshot.point_byte = 5;
    let (moved, _) = engine.layout_frame(&snap, 4, Some((id, rum))).unwrap();
    assert_ne!(moved.active_cursor().unwrap().y, on.active_cursor().unwrap().y, "a moved point is laid out again");
}

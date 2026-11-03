use romacs_vm::Vm;
#[test]
fn native_face_names_use_the_shared_color_resolver() {
    let mut vm = Vm::new();
    vm.eval_source("(defface named '((t (:foreground \"LightSteelBlue\" :background \"gray20\"))) \"named\") (insert \"x\") (put-text-property 1 2 'face 'named)").unwrap();
    let snapshot = vm.display_snapshot(640, 480).unwrap();
    assert!(snapshot.faces.iter().any(|face| face.foreground == [176, 196, 222] && face.background == [51, 51, 51]));
}
#[test]
fn source_scene_and_external_mutation_with_low_gc_floor() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(SCENE).unwrap();
    let before = vm.display_snapshot(640, 480).expect("low-floor snapshot");
    vm.eval_source("(let ((i 0)) (while (< i 5000) (vector i) (setq i (1+ i))))")
        .unwrap();
    vm.eval_source(r#"(with-current-buffer "g1" (goto-char (point-max)) (insert "Δ"))"#)
        .unwrap();
    let after = vm.display_snapshot(640, 480).unwrap();
    assert_eq!(before.text.utf8, "Ωμέγα Hello\n");
    assert_eq!(after.text.utf8, "Ωμέγα Hello\nΔ");
    assert!(after.buffer_version > before.buffer_version);
    assert!(vm.heap_stats().collections > 0);
}

#[test]
fn point_only_mutation_changes_display_revision() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    let before = vm.display_snapshot(640, 480).unwrap();
    vm.eval_source("(with-current-buffer \"g1\" (goto-char 1))")
        .unwrap();
    let after = vm.display_snapshot(640, 480).unwrap();
    assert_eq!(after.point_byte, 0);
    assert!(
        after.buffer_version > before.buffer_version,
        "point-only redraw"
    );
}
/// buffer-swap-text changes the shown text with no edit of the shown
/// buffer: here its modification tick after the swap equals the one its
/// last snapshot read, yet the window shows the other text.
#[test]
fn swapped_text_is_shown_although_the_tick_matches() {
    let mut vm = Vm::new();
    vm.eval_source(r#"(progn (setq swap-a (current-buffer)) (insert "a") (insert "aa") (setq swap-b (get-buffer-create "swap-b")) (with-current-buffer swap-b (insert "bbb")))"#).unwrap();
    assert_eq!(vm.display_snapshot(640, 480).unwrap().text.utf8, "aaa");
    vm.eval_source("(buffer-swap-text swap-b)").unwrap();
    assert_eq!(vm.eval_source("(buffer-modified-tick)").unwrap().display().unwrap(), "4");
    assert_eq!(vm.display_snapshot(640, 480).unwrap().text.utf8, "bbb");
}
#[test]
fn window_exists_before_first_query_and_scratch_identity_is_unique() {
    let mut vm = Vm::new();
    vm.eval_source(r#"(setq scratch (get-buffer-create "*scratch*"))"#)
        .unwrap();
    let same = vm.eval_source("(eq scratch (current-buffer))").unwrap();
    assert_eq!(same.display().unwrap(), "t", "one scratch identity");
    vm.eval_source(r#"(set-buffer (get-buffer-create "other")) (insert "not displayed")"#)
        .unwrap();
    assert_eq!(
        vm.display_snapshot(640, 480).unwrap().text.utf8,
        "",
        "window existed before current-buffer switch"
    );
}

const SCENE: &str = r##"(progn
 (defface g1-face '((t (:foreground "#ff0000"))) "G1")
 (set-buffer (get-buffer-create "g1"))
 (insert "Ωμέγα Hello\n")
 (put-text-property 7 12 'face 'g1-face)
 (set-window-buffer (selected-window) (current-buffer)))"##;
#[test]
fn source_scene_snapshot_and_external_mutation() {
    let mut vm = Vm::new();
    let empty = vm.display_snapshot(640, 480).expect("snapshot API");
    assert_eq!(empty.faces[0].family, "Liberation Mono");
    assert_eq!(empty.faces[0].pixel_size, 20.0);
    vm.eval_source(SCENE).expect("source scene");
    let s = vm.display_snapshot(640, 480).unwrap();
    assert_eq!(s.text.utf8, "Ωμέγα Hello\n");
    let red = s
        .text
        .face_runs
        .iter()
        .find(|r| s.faces[r.face.0 as usize].foreground == [255, 0, 0])
        .expect("red face");
    assert_eq!(&s.text.utf8[red.bytes.clone()], "Hello");
    assert_eq!(
        vm.display_snapshot(640, 480).unwrap().buffer_version,
        s.buffer_version
    );
    vm.eval_source(r#"(with-current-buffer "g1" (goto-char (point-max)) (insert "Δ"))"#)
        .unwrap();
    let after = vm.display_snapshot(640, 480).unwrap();
    assert_eq!(after.text.utf8, "Ωμέγα Hello\nΔ");
    assert!(after.buffer_version > s.buffer_version);
    assert_eq!(s.text.utf8, "Ωμέγα Hello\n");
}
#[test]
fn one_window_frame_owns_buffer_independent_of_current_buffer() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).expect("source scene");
    vm.display_snapshot(711, 433).unwrap();
    let v=vm.eval_source("(list (eq (selected-window) (selected-window)) (eq (selected-frame) (selected-frame)) (eq (window-buffer) (current-buffer)) (window-start) (window-point) (frame-pixel-width) (frame-pixel-height))").unwrap();
    assert_eq!(v.display().unwrap(), "(t t t 1 13 711 433)");
    vm.eval_source(r#"(set-buffer (get-buffer-create "other"))"#)
        .unwrap();
    assert_eq!(
        vm.display_snapshot(711, 433).unwrap().text.utf8,
        "Ωμέγα Hello\n"
    );
}

/// xfaces.c face_at_buffer_position sorts the overlays covering each
/// position: A [1,8) with secondary priority 1 wins over C [3,9) at 7,
/// though B [2,6), which A loses to, covers neither 7 nor C's end.
#[test]
fn overlay_faces_rank_the_overlays_covering_each_position() {
    let mut vm = Vm::new();
    vm.eval_source("(progn (set-buffer (get-buffer-create \"o\")) (insert \"0123456789\")
        (set-window-buffer (selected-window) (current-buffer))
        (overlay-put (make-overlay 2 6) 'face '(:foreground \"#ff0000\")) (overlay-put (car (overlays-at 2)) 'priority '(0 . 0))
        (let ((c (make-overlay 3 9))) (overlay-put c 'face '(:foreground \"#00ff00\")) (overlay-put c 'priority '(0 . 0)))
        (let ((a (make-overlay 1 8))) (overlay-put a 'face '(:foreground \"#0000ff\")) (overlay-put a 'priority '(0 . 1))))").unwrap();
    let s = vm.display_snapshot(640, 480).unwrap();
    let at = |byte: usize| s.text.face_runs.iter().find(|r| r.bytes.contains(&byte)).map(|r| s.faces[r.face.0 as usize].foreground);
    assert_eq!(at(6), Some([0, 0, 255]), "position 7");
}

/// An overlay without a face still takes part in the ranking: B [2,6)
/// lifts C [3,9) over A [1,8) at 4, as GNU's sort of all three does.
#[test]
fn a_faceless_overlay_still_ranks_the_overlay_faces() {
    let mut vm = Vm::new();
    vm.eval_source("(progn (set-buffer (get-buffer-create \"o\")) (insert \"0123456789\")
        (set-window-buffer (selected-window) (current-buffer))
        (let ((a (make-overlay 1 8))) (overlay-put a 'face '(:foreground \"#0000ff\")) (overlay-put a 'priority '(0 . 1)))
        (let ((b (make-overlay 2 6))) (overlay-put b 'priority '(0 . 0)))
        (let ((c (make-overlay 3 9))) (overlay-put c 'face '(:foreground \"#00ff00\")) (overlay-put c 'priority '(0 . 0))))").unwrap();
    let s = vm.display_snapshot(640, 480).unwrap();
    let at = |byte: usize| s.text.face_runs.iter().find(|r| r.bytes.contains(&byte)).map(|r| s.faces[r.face.0 as usize].foreground);
    assert_eq!(at(3), Some([0, 255, 0]), "position 4");
}

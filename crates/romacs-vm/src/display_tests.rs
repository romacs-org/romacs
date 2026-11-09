use super::*;
#[test]
fn snapshot_survives_source_string_and_property_compaction() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    vm.set_collection_floor(1_000_000);
    vm.eval_source_raw("(progn (vector 9) (cons 8 nil))").unwrap();
    vm.eval_source_raw(
        r##"(progn
      (defface g1-face '((t (:foreground "#ff0000"))) "G1")
      (setq source (copy-sequence "Ωμέγα Hello\n"))
      (setq property (list 'g1-face))
      (put-text-property 6 11 'face property source)
      (set-buffer (get-buffer-create "g1"))
      (insert source)
      (set-window-buffer nil (current-buffer)))"##,
    )
    .unwrap();
    let source = vm.eval_source_raw("source").unwrap();
    let property = vm.eval_source_raw("property").unwrap();
    let mut roots = [source, property];
    let snapshot = vm.display_snapshot(640, 480).unwrap();
    vm.collect(&mut roots);
    assert_ne!(roots[0], source, "source string must really move");
    assert_ne!(roots[1], property, "source property cons must really move");
    assert_eq!(vm.source_string(roots[0]).unwrap(), "Ωμέγα Hello\n");
    let after = vm
        .display_snapshot(640, 480)
        .expect("target extraction after compaction");
    assert_eq!(snapshot, after);
    let red = snapshot
        .text
        .face_runs
        .iter()
        .find(|r| snapshot.faces[r.face.0 as usize].foreground == [255, 0, 0])
        .expect("owned red run after GC");
    assert_eq!(&snapshot.text.utf8[red.bytes.clone()], "Hello");
}
/// D3 / VM2-FACEFN1: the lface keeps a function :height as GNU does; the
/// theme, which holds data only, gets the function's value over the default
/// face's height. Validation calls the function with 10 (merge_face_heights)
/// and refuses one that yields no positive integer, signalling or not.
#[test]
fn a_function_height_stays_in_the_lface_and_realizes_over_the_default_height() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let eval = |vm: &mut Vm, s: &str| vm.eval_source(s).map(|v| v.display().unwrap()).unwrap_or_else(|e| format!("VM-ERROR {e}"));
    eval(&mut vm, "(internal-set-lisp-face-attribute 'default :height 200 (selected-frame))");
    eval(&mut vm, "(internal-make-lisp-face 'fh-face)");
    eval(&mut vm, "(defun fh-half (h) (if (integerp h) (/ h 2) h))");
    assert_eq!(eval(&mut vm, "(progn (internal-set-lisp-face-attribute 'fh-face :height 'fh-half (selected-frame)) (internal-get-lisp-face-attribute 'fh-face :height (selected-frame)))"), "fh-half");
    assert_eq!(vm.snapshot_face("fh-face").unwrap().pixel_size, 100.0 * 96.0 / 720.0);
    for bad in ["(lambda (_) nil)", "(lambda (_) (error \"x\"))"] {
        assert_eq!(eval(&mut vm, &format!("(condition-case e (internal-set-lisp-face-attribute 'fh-face :height {bad} (selected-frame)) (error (car (cdr e))))")),
                   "\"Face height does not produce a positive integer\"", "{bad}");
    }
    assert_eq!(eval(&mut vm, "(internal-get-lisp-face-attribute 'fh-face :height (selected-frame))"), "fh-half");
}

#[test]
fn a_band_shows_control_characters_as_gnu_display_does() {
    // TAB, ^A and U+0085 in a header line: caret forms and an octal
    // escape, the face runs moved with the text they cover.
    let (text, runs, _) = super::shown_band("a\tb\u{1}c\u{85}d", vec![(0..1, 'x'), (1..4, 'y'), (4..8, 'z')], &[]);
    assert_eq!(text, "a^Ib^Ac\\205d");
    assert_eq!(runs, vec![(0..1, 'x'), (1..6, 'y'), (6..12, 'z')]);
    let (plain, same, _) = super::shown_band("plain", vec![(0..5, 'x')], &[]);
    assert_eq!((plain.as_str(), same), ("plain", vec![(0..5, 'x')]));
    // A `(space ...)' run shows as one placeholder at its stretch, the
    // runs after it moved back.
    let align = romacs_layout_input::StretchWidth::AlignTo(40);
    let (spaced, runs, stretches) = super::shown_band("ab   cd", vec![(0..2, 'x'), (2..5, 'y'), (5..7, 'z')], &[(2..5, align)]);
    assert_eq!(spaced, "ab cd");
    assert_eq!(runs, vec![(0..2, 'x'), (2..3, 'y'), (3..5, 'z')]);
    assert_eq!(stretches, vec![romacs_layout_input::Stretch { byte: 2, width: align }]);
}

/// The value of `romacs-mode-line-format`.
fn segment(vm: &mut Vm) -> Value {
    let slot = vm.slot("romacs-mode-line-format");
    vm.slots[slot].value
}

/// The form's propertized string for `romacs-mode-line-format` as it is
/// now: a rooted handle, the same one while the form is the cached one.
fn segment_handle(vm: &mut Vm) -> std::sync::Arc<crate::handle::Rooted> {
    let value = segment(vm);
    match vm.band_format_form(value) {
        Form::List(items) => match &items[1] {
            Form::Object(object) => object.0.clone(),
            other => panic!("a propertized string is an object: {other:?}"),
        },
        other => panic!("a list segment is a list: {other:?}"),
    }
}

/// Set `romacs-mode-line-format` to a list with a propertized string, made
/// after garbage, so that a moving collection has somewhere to move it.
fn set_segment(vm: &mut Vm) {
    vm.eval_source(
        "(progn (defvar romacs-mode-line-format nil)
                (let ((garbage (make-list 2000 (copy-sequence \"g\"))))
                  (setq romacs-mode-line-format (list \"a\" (propertize \"b\" 'face 'bold) (length garbage)))))",
    )
    .unwrap();
}

/// A segment's form survives a collection, in place or moving: the entry's
/// saved value is still the slot's, and the cached form comes back.
#[test]
fn a_mode_line_form_survives_collections() {
    let mut vm = Vm::new();
    set_segment(&mut vm);
    let first = segment_handle(&mut vm);
    let slot = vm.slot("romacs-mode-line-format");
    for moving in [false, true] {
        vm.set_gc_moving(moving);
        let before = segment(&mut vm);
        vm.collect_garbage();
        // Stress collections may have compacted the segment already.
        if moving && std::env::var_os("ROMACS_GC_STRESS").is_none() {
            assert_ne!(segment(&mut vm), before, "the fixture must move the segment");
        }
        let saved = vm.mode_line_forms.get(&slot).map(|&(known, _)| known);
        assert_eq!(saved, Some(segment(&mut vm)), "the entry follows the value (moving: {moving})");
        assert!(std::sync::Arc::ptr_eq(&first, &segment_handle(&mut vm)), "the form is the cached one (moving: {moving})");
    }
}

/// A segment's form goes when the value it was converted from dies: the
/// slot can never hold that value again, so its entry is neither kept nor
/// matched by a new object at the same index.
#[test]
fn a_dead_segment_value_takes_its_form() {
    let mut vm = Vm::new();
    set_segment(&mut vm);
    segment_handle(&mut vm);
    let slot = vm.slot("romacs-mode-line-format");
    assert!(vm.mode_line_forms.contains_key(&slot), "converted, so cached");
    vm.eval_source("(setq romacs-mode-line-format (list \"c\"))").unwrap();
    vm.collect_garbage();
    assert!(!vm.mode_line_forms.contains_key(&slot), "the old value died, so its entry did");
}

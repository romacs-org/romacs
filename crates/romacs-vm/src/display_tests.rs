use super::*;
#[test]
fn snapshot_survives_source_string_and_property_compaction() {
    let mut vm = Vm::new();
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

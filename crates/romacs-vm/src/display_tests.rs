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

use romacs_vm::Vm;

#[test]
fn point_markers_share_anchor_motion_narrowing_and_lifetime() {
    let source = r#"(with-temp-buffer
      (insert "aλЖz") (narrow-to-region 2 4) (goto-char 3)
      (let ((a (point-min-marker)) (b (point-marker)) (c (point-max-marker)))
        (list (mapcar #'marker-position (list a b c))
              (mapcar #'marker-insertion-type (list a b c))
              (progn (insert "xx") (garbage-collect)
                     (mapcar #'marker-position (list a b c)))
              (progn (widen) (goto-char 1) (insert "y")
                     (mapcar #'marker-position (list a b c))))))"#;
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(),
               "((2 3 4) (nil nil nil) (2 3 6) (3 4 7))");
    assert_eq!(vm.eval_source("(let ((marker (with-temp-buffer (point-max-marker)))) (marker-buffer marker))").unwrap().display().unwrap(), "nil");
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "((2 3 4) (nil nil nil) (2 3 6) (3 4 7))");
}

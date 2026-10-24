use romacs_vm::Vm;

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn filesystem_usage_and_named_attribute_ids_follow_native_contracts() {
    let root = std::env::temp_dir().join(format!("romacs-filesystem-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("owned");
    std::fs::write(&file, "native").unwrap();
    let mut vm = Vm::new();
    vm.eval_source(&format!("(setq fixture {})", serde_json::to_string(&file).unwrap())).unwrap();
    assert_eq!(printed(&mut vm, "(let ((a (file-attributes fixture 'string)) (b (file-attributes fixture 'integer))) (list (stringp (nth 2 a)) (stringp (nth 3 a)) (integerp (nth 2 b)) (integerp (nth 3 b)) (equal (nth 2 a) (nth 2 (file-attributes fixture t)))))"), "(t t t t t)", "FILE_ATTRIBUTE_ID_FORMAT");
    for (index, option) in [(2, "-un"), (3, "-gn")] {
        let output = std::process::Command::new("id").arg(option).output().unwrap();
        assert!(output.status.success());
        let expected = std::str::from_utf8(&output.stdout).unwrap().trim();
        assert_eq!(printed(&mut vm, &format!("(equal (nth {index} (file-attributes fixture 'string)) {})", serde_json::to_string(expected).unwrap())), "t", "FILE_ATTRIBUTE_SYSTEM_ID_NAME");
    }
    assert_eq!(printed(&mut vm, "(let ((info (file-system-info fixture))) (list (= (length info) 3) (integerp (nth 0 info)) (> (nth 0 info) 0) (>= (nth 1 info) (nth 2 info))))"), "(t t t t)", "FILESYSTEM_BYTE_COUNTS");
    assert_eq!(printed(&mut vm, "(file-system-info (concat fixture \"/missing\"))"), "nil", "FILESYSTEM_MISSING_RETURNS_NIL");
    vm.eval_source("(defun fs-handler (&rest args) args) (put 'fs-handler 'operations '(file-system-info))").unwrap();
    assert_eq!(printed(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . fs-handler)))) (file-system-info \"/remote\"))"), "(file-system-info \"/remote\")", "FILESYSTEM_HANDLER");
    vm.eval_source("(defun fs-handler (&rest args) 42)").unwrap();
    assert_eq!(printed(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . fs-handler)))) (condition-case e (file-system-info \"/remote\") (error 'refused)))"), "refused", "FILESYSTEM_BAD_HANDLER_REFUSED");
    std::fs::remove_dir_all(root).unwrap();
}

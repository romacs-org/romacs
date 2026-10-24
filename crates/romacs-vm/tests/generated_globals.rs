use romacs_vm::Vm;

fn leaked_vm() -> Vm {
    let mut vm = Vm::new();
    for source in [
        "(defvar leaked nil)",
        "(defmacro car (x) (setq leaked x) (list 'cdr x))",
        "(let ((v (cons 1 2))) (cl-incf (car v) 1))",
        "(defmacro use-leak () leaked)",
    ] {
        vm.eval_source(source).unwrap();
    }
    vm
}

#[test]
fn escaped_generated_variable_is_only_unbound_at_runtime() {
    let mut vm = leaked_vm();
    let result = vm.eval_source("(if nil (use-leak) 'fine)");
    assert!(result.is_ok(), "dead branch: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "fine");
    let entry = vm
        .compile_source("(use-leak)")
        .expect("free generated read must compile");
    assert!(
        vm.execute(entry).is_err(),
        "a live unbound read must still signal"
    );
}

#[test]
fn escaped_generated_variable_can_be_set_without_interning_its_spelling() {
    let mut vm = leaked_vm();
    vm.eval_source("(defmacro set-leak () (list 'setq leaked 3))")
        .unwrap();
    let result = vm.eval_source("(set-leak)");
    assert!(result.is_ok(), "free generated write: {result:?}");
    assert_eq!(result.unwrap().as_integer().unwrap(), 3);
    assert_eq!(
        vm.eval_source("(use-leak)").unwrap().as_integer().unwrap(),
        3
    );
    let entry = vm.compile_source("(use-leak)").unwrap();
    let store = std::env::temp_dir().join(format!("s40-generated-{}", std::process::id()));
    std::fs::create_dir(&store).unwrap();
    let key = vm.publish_code_image(&store, entry, [40; 32]).unwrap();
    let image = unsafe { romacs_vm::ImageRegion::open(&store, &key) }.unwrap();
    let (mut restored, entry) = Vm::from_code_image(image, [40; 32]).unwrap();
    assert_eq!(restored.execute(entry).unwrap().as_integer().unwrap(), 3);
    assert_eq!(
        restored
            .eval_source("(use-leak)")
            .unwrap()
            .as_integer()
            .unwrap(),
        3
    );
    // `intern` lazily creates a mutable obarray, which code images cannot
    // serialize. Check symbol identity only after publishing the image.
    for vm in [&mut vm, &mut restored] {
        let spelling = vm.eval_source("leaked").unwrap().display().unwrap();
        let same = vm.eval_source(&format!("(eq leaked (intern {:?}))", spelling)).unwrap();
        assert!(same.is_nil(), "generated symbol must not be the interned {spelling}");
    }
    std::fs::remove_dir_all(store).unwrap();
}

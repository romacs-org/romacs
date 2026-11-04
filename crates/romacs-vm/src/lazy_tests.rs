//! R215 collector proof at the representation level: a lazily held body is a
//! heap Value owned by its function, marked through it and forwarded by the
//! compaction. Complements tests/lazy_bodies.rs, which measures live objects
//! and bytes through the public API.
use super::*;

fn lazy_body_of(vm: &Vm, symbol: &str) -> Option<Value> {
    let slot = vm.names[symbol];
    let closure = vm.slots[slot].function.index_if(4)?;
    let function = vm.closures[closure].function;
    vm.functions[function].lazy.as_ref().map(|lazy| lazy.body)
}

fn load(vm: &mut Vm, name: &str, text: &str) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/lazy-unit");
    let dir = std::env::var_os("CARGO_TARGET_DIR").map_or(dir, |t| std::path::PathBuf::from(t).join("lazy-unit"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    vm.load_file_raw(&path).unwrap();
}

#[test]
fn lazily_held_body_moves_with_compaction_and_is_dropped_on_expansion() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    // Garbage BEFORE the definition, so compaction must move the body.
    vm.eval_source_raw("(let ((i 0) (g nil)) (while (< i 500) (setq g (cons i g)) (setq i (1+ i))) nil)")
        .unwrap();
    load(
        &mut vm,
        "moving.el",
        "(defalias 'lz-move (function (lambda (n) (list n 'lz-a \"lz-s\" (+ n 1)))))\n",
    );
    let before = lazy_body_of(&vm, "lz-move").expect("defined in the non-eager regime: held lazily");
    let form_before = vm.source_form(before, 0).unwrap();
    let rendered_before = vm.display(before).unwrap();
    // The only path to the body is symbol -> closure -> function -> body.
    vm.collect(&mut []);
    let after = lazy_body_of(&vm, "lz-move").expect("still lazy: nothing has called it");
    assert!(
        after.payload() < before.payload(),
        "compaction must have moved the body: {before:?} -> {after:?}"
    );
    assert_eq!(vm.display(after).unwrap(), rendered_before);
    assert_eq!(format!("{:?}", vm.source_form(after, 0).unwrap()), format!("{form_before:?}"));
    let value = vm.eval_source_raw("(lz-move 41)").unwrap();
    assert_eq!(vm.display(value).unwrap(), "(41 lz-a \"lz-s\" 42)");
    assert!(lazy_body_of(&vm, "lz-move").is_none(), "the first call compiles it in place");
}

#[test]
fn deferred_form_stays_lazy_after_a_failed_reach_and_retries() {
    // GNU expands a form each time it is reached: a failure leaves nothing
    // cached, and a later definition of the macro is used on the next reach.
    let mut vm = Vm::new();
    load(
        &mut vm,
        "retry.el",
        "(defalias 'lz-retry (function (lambda (go) (if go (lz-later 7) 'skipped))))\n\
         (defalias 'lz-later (cons 'macro (function (lambda (x) (if (boundp 'lz-ready) (list 'quote (list 'ok x)) (error \"not yet\"))))))\n",
    );
    let skipped = vm.eval_source_raw("(lz-retry nil)").unwrap();
    assert_eq!(vm.display(skipped).unwrap(), "skipped");
    let failed = vm.eval_source_raw("(condition-case e (lz-retry t) (error e))").unwrap();
    assert_eq!(vm.display(failed).unwrap(), "(error \"not yet\")");
    vm.eval_source_raw("(defvar lz-ready t)").unwrap();
    vm.collect(&mut []);
    let value = vm.eval_source_raw("(lz-retry t)").unwrap();
    assert_eq!(vm.display(value).unwrap(), "(ok 7)");
}

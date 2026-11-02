//! A unit's encoding round trip: each form compiled, encoded, decoded
//! into the same VM and run gives what the form gives.
use super::*;

/// FORM's value through a unit, displayed.
fn through_unit(vm: &mut Vm, form: &str) -> String {
    let datum = vm.eval_source_raw(form).unwrap();
    let form = vm.expansion_form(datum, 0).unwrap();
    let mut names = unit::Names::default();
    let bytes = vm.compile_unit(&form, NameSet::default(), &mut names).unwrap();
    let slots: Vec<usize> = names.names.iter().map(|n| vm.slot(n)).collect();
    let mut reader = codec::Reader::new(&bytes);
    let entry = unit::Install { slots: &slots }.unit(vm, &mut reader).unwrap();
    assert!(reader.done());
    let value = vm.execute_raw(entry).unwrap();
    vm.release_request_entry(entry);
    vm.display(value).unwrap()
}

#[test]
fn units_round_trip() {
    let mut vm = Vm::new();
    vm.eval_source("(setq unit-test-fn (let ((k 1)) (lambda (x) (+ x k))))").unwrap();
    for (form, expected) in [
        // A closure over a mutable variable: a captured cell.
        ("'(let ((c (let ((n 0)) (lambda () (setq n (1+ n)))))) (funcall c) (funcall c))", "2"),
        // Equal constants of one function are coalesced, as bytecomp does.
        ("'(let ((a '(x)) (b '(x))) (eq a b))", "t"),
        // Shared and circular structure survives.
        ("'(let ((l '#1=(a . #1#))) (eq l (cdr l)))", "t"),
        ("'(let ((l '(#1=#:g #1#))) (and (eq (car l) (car (cdr l))) (not (eq (car l) (intern \"g\")))))", "t"),
        ("'(get-text-property 1 'face #(\"abc\" 0 3 (face bold)))", "bold"),
        ("'(let ((v [1 2.5 \"s\"])) (and (= (aref v 1) 2.5) (equal (aref v 2) \"s\")))", "t"),
        // A function constant, as an inlined defsubst's frozen copy is.
        ("(list 'funcall (list 'quote unit-test-fn) 41)", "42"),
    ] {
        assert_eq!(through_unit(&mut vm, form), expected, "{form}");
    }
}

#[test]
fn damaged_artifacts_are_deleted_on_lookup() {
    let base = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"), std::path::PathBuf::from);
    let store = base.join(format!("compiled-lisp-unit-{}", std::process::id()));
    std::fs::create_dir_all(&store).unwrap();
    let key = [7; 32];
    let (bytes, inputs) = store::encode(&store::Status::Compiled, Path::new("/x.el"), &[], &[], &[]);
    let path = store::publish(&store, &key, &inputs, &bytes).unwrap();
    assert!(store::lookup(&store, &key, |_| true).is_some_and(|(found, _)| found == path));
    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    std::fs::write(&path, damaged).unwrap();
    assert!(store::lookup(&store, &key, |_| true).is_none());
    assert!(!path.exists());
    std::fs::remove_dir_all(&store).unwrap();
}

//! The snapshot-safety contract (`world::safe`): what a world image may
//! hold is checked by the compiler, field by field, and a VM holding a
//! resource of its process is refused at capture.
use romacs_vm::Vm;

#[test]
fn snapshot_safety_is_enforced_by_the_compiler() {
    let t = trybuild::TestCases::new();
    t.pass("tests/snapshot_safe/pass/*.rs");
    t.compile_fail("tests/snapshot_safe/fail/*.rs");
}

fn refusal(vm: &mut Vm) -> Option<String> {
    vm.world_normalize();
    vm.world_quiescent().err()
}

#[test]
fn a_live_sqlite_object_is_refused_and_a_closed_one_is_data() {
    let mut vm = Vm::new();
    assert_eq!(refusal(&mut vm), None);
    vm.eval_source("(defvar snapshot-db (sqlite-open))").unwrap();
    let refused = refusal(&mut vm).expect("a live connection is refused");
    assert!(refused.contains("a live sqlite object"), "{refused}");
    vm.eval_source("(sqlite-close snapshot-db)").unwrap();
    assert_eq!(refusal(&mut vm), None);
}

#[test]
fn a_live_statement_is_refused() {
    let mut vm = Vm::new();
    vm.eval_source("(defvar snapshot-db (sqlite-open)) (sqlite-execute snapshot-db \"create table t (x)\") \
                    (defvar snapshot-set (sqlite-select snapshot-db \"select * from t\" nil 'set))").unwrap();
    vm.eval_source("(sqlite-close snapshot-db)").ok();
    let refused = refusal(&mut vm);
    assert!(refused.as_deref().is_some_and(|r| r.contains("a live sqlite object")), "{refused:?}");
    vm.eval_source("(sqlite-finalize snapshot-set)").unwrap();
    vm.eval_source("(sqlite-close snapshot-db)").ok();
    assert_eq!(refusal(&mut vm), None);
}

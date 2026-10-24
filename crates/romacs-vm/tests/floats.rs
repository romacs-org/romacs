use romacs_vm::{Fixed, Vm};

#[test]
fn float_source_arithmetic_and_printing() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(abs -3.14)", "3.14"),
        ("(+ 1 2.5)", "3.5"),
        ("(- 2.5)", "-2.5"),
        ("(* 2 1.25)", "2.5"),
        ("(/ 7 2)", "3"),
        ("(/ 7 2.0)", "3.5"),
        ("(1+ 2.5)", "3.5"),
        ("(1- 2.5)", "1.5"),
        ("(< 2 2.5 3)", "t"),
        ("(= 2 2.0)", "t"),
        ("1e2", "100.0"),
        (".5", "0.5"),
        ("-0.0", "-0.0"),
        ("'[1.0 (2.5 . 3)]", "[1.0 (2.5 . 3)]"),
    ] {
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual, Ok(expected.into()), "{source}");
    }
}

#[test]
fn mixed_comparisons_do_not_round_integers_before_comparing() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(= 9007199254740993 9007199254740992.0)", Fixed::NIL),
        ("(< 9007199254740992.0 9007199254740993)", Fixed::T),
        ("(< -9007199254740993 -9007199254740992.0)", Fixed::T),
        ("(< 1 1.25)", Fixed::T),
        ("(< -1.25 -1)", Fixed::T),
        ("(= 0.0e+NaN 0)", Fixed::NIL),
    ] {
        assert_eq!(vm.eval_source(source).unwrap(), expected, "{source}");
    }
}

#[test]
fn boxed_floats_move_and_survive_executor_collection() {
    let mut vm = Vm::new();
    vm.read_source("[0.1 0.2]").unwrap();
    let v = vm.read_source("[1.25 (2.5 . 3.75)]").unwrap().unwrap().0;
    let before = format!("{v:?}");
    let v = v.root();
    vm.collect_garbage();
    let after = v.get(&vm).unwrap();
    assert_ne!(format!("{after:?}"), before, "actual compaction must move the root");
    assert_eq!(after.display().unwrap(), "[1.25 (2.5 . 3.75)]");
    vm.set_collection_floor(1);
    let v = vm.eval_source("(defun float-constant () 1.25) (let ((i 0) (x 0.0)) (while (< i 50000) (setq x (+ x 0.5) i (1+ i))) (+ x (float-constant)))").unwrap().display();
    assert!(vm.heap_stats().collections > 1);
    assert_eq!(v.unwrap(), "25001.25");
}

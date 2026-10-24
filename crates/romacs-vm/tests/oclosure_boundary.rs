//! Bounded oclosure frontier evidence. Ordinary closure introspection is parked;
//! explicit interpreted constructors own their metadata independently.
use romacs_vm::Vm;

fn value(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn ordinary_oclosure_prototype_stops_at_the_owned_array_boundary() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(value(&mut vm, "(setq root-prototype (let ((index 0)) (lambda (x) (:documentation 'oclosure-accessor) (nth index x))))"), "#<interpreted-function>");
    assert_eq!(value(&mut vm, "(condition-case e (aref root-prototype 1) (wrong-type-argument (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) root-prototype))))"), "(wrong-type-argument arrayp t)");
    assert_eq!(value(&mut vm, "(funcall root-prototype '(7))"), "7");
    assert_eq!(value(&mut vm, "(condition-case e (aref root-prototype 1) (wrong-type-argument (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) root-prototype))))"), "(wrong-type-argument arrayp t)");
}

#[test]
fn explicit_interpreted_prototype_can_be_reconstructed_without_ordinary_metadata() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(value(&mut vm, "(setq explicit-prototype (make-interpreted-closure '(x) '((nth index x)) '((index . 0) t) 'oclosure-accessor nil))"), "#<interpreted-function>");
    assert_eq!(value(&mut vm, "(let ((f explicit-prototype)) (list (consp (aref f 1)) (aref f 4) (funcall f '(7))))"), "(t oclosure-accessor 7)");
    assert_eq!(value(&mut vm, "(let* ((f explicit-prototype) (copy (make-interpreted-closure (aref f 0) (aref f 1) '((index . 1) t) (aref f 4) nil))) (list (funcall f '(7 9)) (funcall copy '(7 9)) (aref copy 4)))"), "(7 9 oclosure-accessor)");
}

#[test]
fn stale_arithmetic_gain_rows_match_exact_pinned_gnu_answers() {
    // Independently rerun on GNU 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea;
    // original case/source/answer/provenance rows remain untouched.
    for (source, expected) in [
        ("(prin1-to-string (condition-case e (list 'ok (make-vector (1+ most-positive-fixnum) nil)) (error (list 'sig e))))", "(sig (wrong-type-argument wholenump 2305843009213693952))"),
        ("(prin1-to-string (condition-case e (list 'ok (make-vector (- most-negative-fixnum 1) nil)) (error (list 'sig e))))", "(sig (wrong-type-argument wholenump -2305843009213693953))"),
        ("(prin1-to-string (condition-case e (list 'ok (condition-case e (set-window-new-total nil (1+ most-positive-fixnum)) (error (car e)))) (error (list 'sig e))))", "(ok wrong-type-argument)"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let actual = value(&mut vm, source);
        assert_eq!(serde_json::from_str::<String>(&actual).unwrap(), expected, "{source}");
    }
}

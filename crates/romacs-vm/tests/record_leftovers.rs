//! Compact GNU-native prerequisites exposed by the record corpus.
use romacs_vm::Vm;

fn check(source: &str, expected: &str, label: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let actual = vm.eval_source(source).map(|v| v.display().unwrap());
    assert_eq!(actual.as_deref().map_err(|e| e.to_string()), Ok(expected), "{label}: {source}");
}

#[test]
fn arrayp_distinguishes_arrays_from_other_vectorlikes() {
    check("(list (arrayp nil) (arrayp '(1)) (arrayp 1) (arrayp [1]) (arrayp \"x\") (arrayp (unibyte-string 255)) (arrayp (make-char-table 'test)) (arrayp (record 'tag 1)) (arrayp (make-hash-table)) (arrayp (obarray-make)) (arrayp #'car) (arrayp (lambda () nil)))", "(nil nil nil t t t t nil nil nil nil nil)", "ARRAYP_REPRESENTATION");
}

#[test]
fn arrayp_is_callable_and_checks_arity() {
    check("(list (fboundp 'arrayp) (functionp #'arrayp) (funcall #'arrayp [1]) (condition-case e (apply #'arrayp nil) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'arrayp '(nil nil)) (error (list (car e) (nth 2 e)))))", "(t t t (wrong-number-of-arguments 0) (wrong-number-of-arguments 2))", "ARRAYP_CALLABLE");
}

#[test]
fn make_list_preserves_length_identity_and_fresh_spines() {
    check("(let* ((x (list 7)) (a (make-list 3 x)) (b (make-list 3 x))) (list (make-list 0 x) (make-list 1 'fill) (length a) (eq (car a) x) (eq (car a) (car (cdr a))) (eq a b) (eq a (cdr a)) (progn (setcar x 9) a) (progn (setcar a 'changed) (car b))))", "(nil (fill) 3 t t nil nil (changed (9) (9)) (9))", "MAKE_LIST_IDENTITY");
}

#[test]
fn make_list_rejects_non_wholenums_and_is_callable() {
    check("(list (fboundp 'make-list) (functionp #'make-list) (funcall #'make-list 2 'x) (condition-case e (make-list -1 nil) (error e)) (condition-case e (make-list 1.0 nil) (error e)) (condition-case e (make-list 'bad nil) (error e)) (condition-case e (apply #'make-list '(1)) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'make-list '(1 nil extra)) (error (list (car e) (nth 2 e)))))", "(t t (x x) (wrong-type-argument wholenump -1) (wrong-type-argument wholenump 1.0) (wrong-type-argument wholenump bad) (wrong-number-of-arguments 1) (wrong-number-of-arguments 3))", "MAKE_LIST_CONTRACT");
}

#[test]
fn make_list_reaches_record_slot_boundary() {
    check("(let ((r (apply #'record 'tag (make-list 4094 'fill)))) (list (length r) (aref r 0) (aref r 4094)))", "(4095 tag fill)", "MAKE_LIST_RECORD_BOUNDARY");
}

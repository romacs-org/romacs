use romacs_vm::Vm;

// The public source evaluator is the eval_str path (no bytecode fixture).
trait EvalStr {
    fn eval_str(&mut self, source: &str) -> romacs_vm::Result<romacs_vm::Local<'_>>;
}
impl EvalStr for Vm {
    fn eval_str(&mut self, source: &str) -> romacs_vm::Result<romacs_vm::Local<'_>> {
        self.eval_source(source)
    }
}
fn answer(vm: &mut Vm, source: &str) -> String {
    match vm.eval_str(source) {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("VM error: {e}"),
    }
}
#[test]
fn equal_is_deep_and_type_sensitive() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(
            &mut vm,
            "(list (equal '(a (2 . 3)) '(a (2 . 3))) (equal [a (2)] [a (2)]) (equal (copy-sequence \"text\") \"text\") (equal '(1) '(2)) (equal 1 1.0) (equal 1.5 1.5))"
        ),
        "(t t t nil nil t)"
    );
    assert!(vm.eval_str("(equal 1)").is_err());
}
#[test]
fn equal_handles_cycles_and_ignores_string_properties() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(
            &mut vm,
            "(let ((a (list 1)) (b (list 1)) (s (copy-sequence \"a\"))) (setcdr a a) (setcdr b b) (put-text-property 0 1 'face 'bold s) (list (equal a b) (equal s \"a\")))"
        ),
        "(t t)"
    );
}
#[test]
fn min_preserves_winner_type_and_exact_integer_order() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(
            &mut vm,
            "(list (min 7) (min 4 2 8) (min 1 2.5) (min 4 1.5) (min 9007199254740993 9007199254740992) (min 1.0 1) (min 1 1.0))"
        ),
        "(7 2 1 1.5 9007199254740992 1.0 1)"
    );
    assert!(vm.eval_str("(min)").is_err());
    assert!(vm.eval_str("(min 1 'bad)").is_err());
}
#[test]
fn append_copies_prefix_and_shares_final_tail() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(
            &mut vm,
            "(let ((a (list 1 2)) (b (list 3))) (let ((r (append a b))) (list r (eq r a) (eq (cdr (cdr r)) b) (append '(4) 'tail) (append) (eq (append b) b))))"
        ),
        "((1 2 3) nil t (4 . tail) nil t)"
    );
    assert!(vm.eval_str("(append '(1 . 2) nil)").is_err());
}
#[test]
fn append_accepts_vectors_unicode_and_unibyte_strings() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(&mut vm, "(append [1 2] \"λ\" (unibyte-string 255) '(end))"),
        "(1 2 955 255 end)"
    );
}
#[test]
fn prin1_quotes_nested_strings_and_honors_noescape() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(&mut vm, "(prin1-to-string '(a [\"quoted\"] . tail))"),
        "\"(a [\\\"quoted\\\"] . tail)\""
    );
    assert_eq!(
        answer(&mut vm, "(prin1-to-string '(\"a\" . \"b\") t)"),
        "\"(a . b)\""
    );
    assert!(vm.eval_str("(prin1-to-string)").is_err());
}
#[test]
fn prin1_honors_escape_newlines_and_circle_mode() {
    let mut vm = Vm::new();
    assert_eq!(
        answer(
            &mut vm,
            "(let ((print-escape-newlines nil)) (prin1-to-string \"a\\nb\"))"
        ),
        "\"\\\"a\\nb\\\"\""
    );
    assert_eq!(
        answer(
            &mut vm,
            "(let ((print-escape-newlines t)) (prin1-to-string \"a\\nb\"))"
        ),
        "\"\\\"a\\\\nb\\\"\""
    );
    assert_eq!(
        answer(&mut vm, "(let ((print-circle t)) (prin1-to-string '(a)))"),
        "\"(a)\""
    );
}
#[test]
fn data_results_survive_counted_moving_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX);
    assert_eq!(
        answer(
            &mut vm,
            "(dotimes (i 80) (copy-sequence \"dead\")) (setq s124-data-tail (list (copy-sequence \"tail\")) s124-data-list (append (list (copy-sequence \"head\")) s124-data-tail) s124-data-print (prin1-to-string s124-data-list)) (car s124-data-list)"
        ),
        "\"head\""
    );
    let old = format!("{:?}", vm.eval_str("s124-data-list").unwrap());
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before);
    let new = format!("{:?}", vm.eval_str("s124-data-list").unwrap());
    assert_ne!(old, new, "fixture must actually move the appended conses");
    assert_eq!(
        answer(
            &mut vm,
            "(list s124-data-list (eq (cdr s124-data-list) s124-data-tail) (equal s124-data-print (prin1-to-string s124-data-list)) (min 3.0 2))"
        ),
        "((\"head\" \"tail\") t t 2)"
    );
}

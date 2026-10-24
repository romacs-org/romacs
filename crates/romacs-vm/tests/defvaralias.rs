//! `defvaralias` semantics beyond the primh sweep: the alias redirect must be
//! honoured by every reader and writer VM2 has, and it must survive a moving
//! collection (it holds a slot ID, so the VALUE moves inside the base slot).
use romacs_vm::Vm;

fn show(vm: &mut Vm, source: &str) -> String {
    let wrapped = format!("(condition-case e (list 'ok {source}) (error (list 'sig e)))");
    match vm.eval_source(&wrapped) {
        Ok(value) => value.display().unwrap_or_else(|e| format!("VM-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[test]
fn alias_reads_writes_and_binds_the_base_variable() {
    let mut vm = Vm::new();
    assert_eq!(show(&mut vm, "(progn (defvar va-base 5) (defvaralias 'va-alias 'va-base))"), "(ok va-base)");
    // Compiled global read and write of the ALIAS go to the base's cell.
    assert_eq!(show(&mut vm, "va-alias"), "(ok 5)");
    assert_eq!(show(&mut vm, "(progn (setq va-alias 6) va-base)"), "(ok 6)");
    assert_eq!(show(&mut vm, "(progn (setq va-base 7) (list va-alias (symbol-value 'va-alias) (boundp 'va-alias)))"), "(ok (7 7 t))");
    // A dynamic binding of the alias binds the base, and is undone on exit.
    assert_eq!(show(&mut vm, "(list (let ((va-alias 8)) (list va-base va-alias)) va-base)"), "(ok ((8 8) 7))");
    // GNU data.c:782: makunbound of an alias UNDOES the alias; the base keeps its value.
    assert_eq!(show(&mut vm, "(progn (makunbound 'va-alias) (list (boundp 'va-base) (boundp 'va-alias)))"), "(ok (t nil))");
    assert_eq!(show(&mut vm, "va-alias"), "(sig (void-variable va-alias))");
    assert_eq!(show(&mut vm, "(list (special-variable-p 'va-alias) (special-variable-p 'va-base))"), "(ok (t t))");
}

#[test]
fn defvaralias_checks_run_in_gnu_order() {
    let mut vm = Vm::new();
    assert_eq!(show(&mut vm, "(progn (defvaralias 'va-c1 'va-c2) (defvaralias 'va-c2 'va-c3) (defvaralias 'va-c3 'va-c1))"),
               "(sig (cyclic-variable-indirection va-c1))");
    assert_eq!(show(&mut vm, "(defvaralias :va-kw 'va-x)"), "(sig (error \"Cannot make a constant an alias: :va-kw\"))");
    assert_eq!(show(&mut vm, "(defvaralias 1 'va-x)"), "(sig (wrong-type-argument symbolp 1))");
    // NEW's value seeds an unbound BASE, then the let-binding check refuses:
    // the copy is a side effect GNU leaves behind (eval.c:682 before 702).
    assert_eq!(show(&mut vm, "(defvar va-l 3)"), "(ok va-l)");
    assert_eq!(show(&mut vm, "(let ((va-l 4)) (defvaralias 'va-l 'va-lbase))"),
               "(sig (error \"Don\u{2019}t know how to make a let-bound variable an alias: va-l\"))");
    assert_eq!(show(&mut vm, "(symbol-value 'va-lbase)"), "(ok 4)");
    assert_eq!(show(&mut vm, "(progn (defvar va-bl 1) (make-variable-buffer-local 'va-bl) (defvaralias 'va-bl 'va-y))"),
               "(sig (error \"Don\u{2019}t know how to make a buffer-local variable an alias: va-bl\"))");
    assert_eq!(show(&mut vm, "(progn (put 'va-d 'variable-documentation \"Old\") (defvaralias 'va-d 'va-db) (get 'va-d 'variable-documentation))"), "(ok nil)");
    assert_eq!(show(&mut vm, "(progn (defvaralias 'va-n nil) (list va-n (condition-case e (setq va-n 1) (error e)) va-n))"),
               "(ok (nil (setting-constant va-n) nil))");
}

#[test]
fn alias_of_a_buffer_local_base_follows_the_current_buffer() {
    let mut vm = Vm::new();
    let got = show(&mut vm, "(progn (defvar va-loc 1) (make-variable-buffer-local 'va-loc) (defvaralias 'va-la 'va-loc)
        (list (with-temp-buffer (setq va-la 5) (list va-loc (local-variable-if-set-p 'va-la))) va-loc va-la))");
    assert_eq!(got, "(ok ((5 t) 1 1))");
}

#[test]
fn let_binding_an_alias_of_a_buffer_local_shadows_its_default() {
    // GNU let_shadows_buffer_binding_p: a `let` of the default value makes a
    // setq inside it assign that binding instead of creating a buffer-local
    // value. Binding through the ALIAS must carry the base's localization
    // (tools/primh/logs/defvaralias/bind-oracle.log: both rows (9 1)).
    let mut vm = Vm::new();
    assert_eq!(show(&mut vm, "(progn (defvar va-bb 1) (make-variable-buffer-local 'va-bb) (defvaralias 'va-ba 'va-bb))"), "(ok va-bb)");
    assert_eq!(show(&mut vm, "(with-temp-buffer (list (let ((va-bb 8)) (setq va-bb 9) va-bb) va-bb))"), "(ok (9 1))");
    assert_eq!(show(&mut vm, "(with-temp-buffer (list (let ((va-ba 8)) (setq va-ba 9) va-bb) va-bb))"), "(ok (9 1))");
}

#[test]
fn alias_value_survives_a_moving_collection() {
    // Default pacing: the garbage below must still be live heap at collect().
    let mut vm = Vm::new();
    assert_eq!(show(&mut vm, "(progn (defvar va-gc-base nil) (defvaralias 'va-gc 'va-gc-base))"), "(ok va-gc-base)");
    // Garbage first, so the payload allocated after it must move.
    vm.eval_source("(let ((i 0)) (while (< i 2000) (list i (copy-sequence \"garbage\")) (setq i (1+ i))))").unwrap();
    vm.eval_source("(setq va-gc (list 1 (list 2 3) (copy-sequence \"payload bytes\")))").unwrap();
    let before = vm.heap_stats().collections;
    let objects = vm.heap_objects();
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before, "the collection must run");
    assert!(vm.heap_objects() < objects, "the collection must reclaim garbage, so survivors move");
    assert_eq!(show(&mut vm, "va-gc"), "(ok (1 (2 3) \"payload bytes\"))");
    assert_eq!(show(&mut vm, "(eq va-gc va-gc-base)"), "(ok t)");
}

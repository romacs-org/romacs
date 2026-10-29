//! data.c:993 automatic advice activation: a symbol's
//! `defalias-fset-function' stores the definition in place of Ffset; its
//! signal is defalias's. nadvice relies on it to keep advice on a function
//! that is still an autoload. Expected answers are pinned GNU 32.0.50's.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap_or_else(|e| format!("VM-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[test]
fn defalias_calls_the_symbols_fset_function_instead_of_fset() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, expected) in [
        ("(defvar dh-log nil)", "dh-log"),
        ("(put 'dh-f 'defalias-fset-function (lambda (sym def) (setq dh-log (cons (list sym (functionp def)) dh-log)) (fset sym (lambda () 'hooked))))", "#<interpreted-function>"),
        ("(list (defalias 'dh-f (lambda () 'plain) \"Doc.\") (dh-f) dh-log (get 'dh-f 'function-documentation))", "(dh-f hooked ((dh-f t)) \"Doc.\")"),
        ("(put 'dh-g 'defalias-fset-function (lambda (_sym _def) (signal 'wrong-type-argument '(dh 1))))", "#<interpreted-function>"),
        ("(condition-case e (defalias 'dh-g #'car) (error (list e (fboundp 'dh-g))))", "((wrong-type-argument dh 1) nil)"),
    ] {
        assert_eq!(eval(&mut vm, source), expected, "{source}");
    }
}

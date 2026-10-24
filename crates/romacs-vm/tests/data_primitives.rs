use romacs_vm::Vm;

#[test]
fn data_primitive_inventory() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for name in ["eq", "null", "type-of", "cl-type-of", "consp", "atom", "listp", "nlistp", "bare-symbol-p", "symbol-with-pos-p", "symbolp", "keywordp", "vectorp", "recordp", "stringp", "multibyte-string-p", "char-table-p", "vector-or-char-table-p", "bool-vector-p", "arrayp", "sequencep", "bufferp", "markerp", "user-ptrp", "subrp", "closurep", "byte-code-function-p", "interpreted-function-p", "module-function-p", "char-or-string-p", "integerp", "integer-or-marker-p", "natnump", "numberp", "number-or-marker-p", "floatp", "threadp", "mutexp", "condition-variable-p", "car", "car-safe", "cdr", "cdr-safe", "setcar", "setcdr", "boundp", "fboundp", "makunbound", "fmakunbound", "symbol-function", "symbol-plist", "symbol-name", "bare-symbol", "symbol-with-pos-pos", "remove-pos-from-symbol", "position-symbol", "fset", "defalias", "setplist", "subr-arity", "subr-name", "native-comp-function-p", "subr-native-lambda-list", "subr-type", "subr-native-comp-unit", "native-comp-unit-file", "native-comp-unit-set-file", "interactive-form", "command-modes", "indirect-variable", "symbol-value", "set", "add-variable-watcher", "remove-variable-watcher", "get-variable-watchers", "default-boundp", "default-value", "set-default", "make-variable-buffer-local", "make-local-variable", "kill-local-variable", "local-variable-p", "local-variable-if-set-p", "variable-binding-locus", "indirect-function", "aref", "aset", "=", "<", ">", "<=", ">=", "/=", "number-to-string", "string-to-number", "+", "-", "*", "/", "%", "mod", "max", "min", "logand", "logior", "logxor", "logcount", "ash", "1+", "1-", "lognot", "byteorder", "bool-vector-exclusive-or", "bool-vector-union", "bool-vector-intersection", "bool-vector-set-difference", "bool-vector-subsetp", "bool-vector-not", "bool-vector-count-population", "bool-vector-count-consecutive"] {
        let value = vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap();
        println!("{name}: {value}");
    }
}

#[test]
fn existing_callable_controls_are_not_missing_primitives() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source("(funcall #'car '(7))").unwrap().display().unwrap(), "7");
    assert_eq!(vm.eval_source("(mapcar (lambda (x) x) '(1 2))").unwrap().display().unwrap(), "(1 2)");
    assert_eq!(vm.eval_source("(fboundp 'mapcar)").unwrap().display().unwrap(), "t");
}

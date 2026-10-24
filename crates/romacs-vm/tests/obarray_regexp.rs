use romacs_vm::Vm;

#[test]
fn inventory() {
    let mut vm = Vm::new();
    for name in ["obarrayp", "obarray-make", "regexp-quote", "elt", "funcall", "mapcar", "nthcdr", "aref", "string-search", "safe-length", "length<", "length=", "length>"] {
        println!("{name}: {}", vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap());
    }
}

#[test]
fn regexp_quote_contract() {
    let mut vm = Vm::new();
    let result = vm.eval_source("(regexp-quote \"a[.*\\\\?+^$]λ🙂\")").map(|v| v.display().unwrap());
    assert_eq!(result.as_deref().ok(), Some("\"a\\\\[\\\\.\\\\*\\\\\\\\\\\\?\\\\+\\\\^\\\\$]λ🙂\""), "regexp-quote quotes exactly GNU metacharacters");
}

#[test]
fn target_inventory() {
 let mut vm = Vm::new();
 for name in ["append", "assoc", "assq", "compare-strings", "concat", "copy-sequence", "delete", "delq", "elt", "eql", "equal", "equal-including-properties", "featurep", "get", "get-load-suffixes", "gethash", "hash-table-p", "intern", "intern-soft", "length", "length=", "length>", "line-number-at-pos", "load", "locate-file-internal", "looking-at", "make-hash-table", "mapatoms", "mapc", "mapcan", "mapcar", "mapconcat", "match-beginning", "match-data", "match-end", "member", "memq", "nconc", "nreverse", "nth", "nthcdr", "obarrayp", "plist-get", "plist-put", "provide", "put", "puthash", "rassq", "re-search-backward", "re-search-forward", "read", "regexp-quote", "replace-match", "require", "reverse", "search-backward", "search-forward", "secure-hash", "secure-hash-algorithms", "sort", "string-equal", "string-match", "string-search", "substring", "yes-or-no-p"] {
 let result = vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap();
 if result == "nil" {println!("MISSING {name}");}
 }
}

#[test]
fn eql_contract() {
    let mut vm = Vm::new();
    let result = vm.eval_source("(list (eql 1 1) (eql 1 1.0) (eql 1.0 1.0) (eql (list 1) (list 1)))").map(|v| v.display().unwrap());
    assert_eq!(result.as_deref().ok(), Some("(t nil t nil)"), "eql value/type/identity contract");
}

#[test]
fn target_frontiers() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for target in ["files.el", "simple.el", "minibuffer.el", "window.el"] {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
        for prerequisite in ["emacs-lisp/byte-run.el", "subr.el", "emacs-lisp/macroexp.el", "emacs-lisp/pcase.el", "custom.el"] {
            vm.load_file(&root.join("gnu-lisp").join(prerequisite)).unwrap();
        }
        let path = root.join("gnu-lisp").join(target);
        let result = vm.load_file(&path).map(drop);
        let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let report = vm.load_report();
        let steps: Vec<_> = report["steps"].as_array().unwrap().iter().filter(|s| s["file"] == canonical).collect();
        let completed = steps.iter().filter(|s| s["status"] == "ok").count();
        println!("FRONTIER {target}: completed={completed}; result={result:?}");
        for step in report["steps"].as_array().unwrap().iter().filter(|s| s["status"] != "ok") {
            println!("CAUSE {} {} {} {}", step["file"], step["ordinal"], step["line"], step["error"]);
        }
    }
}

#[test]
fn evaluator_and_prelude_builtin_inventory() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for (source, expected) in [
        ("(funcall (lambda (x) x) 3)", "3"),
        ("(mapcar (lambda (x) (1+ x)) '(1 2))", "(2 3)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected);
    }
}

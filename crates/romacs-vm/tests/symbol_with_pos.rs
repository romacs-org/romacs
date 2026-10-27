use romacs_vm::Vm;
use std::path::PathBuf;
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use source_only_bootstrap::SourceOnlyEmacsLisp;

#[test]
fn predicate_rejects_every_representable_kind() {
    let mut vm = Vm::new();
    for form in [
        "(symbol-with-pos-p nil)", "(symbol-with-pos-p t)",
        "(symbol-with-pos-p 'ordinary)", "(symbol-with-pos-p :keyword)",
        "(symbol-with-pos-p (make-symbol \"fresh\"))",
        "(symbol-with-pos-p 42)", "(symbol-with-pos-p 1.5)",
        "(symbol-with-pos-p (car (read-from-string \"#x10000000000000000000000000000\")))",
        "(symbol-with-pos-p '(a . b))", "(symbol-with-pos-p [a 1])",
        "(symbol-with-pos-p \"text\")", "(symbol-with-pos-p (unibyte-string 255))",
        "(symbol-with-pos-p (make-hash-table))",
        "(symbol-with-pos-p (make-bool-vector 9 t))",
        "(symbol-with-pos-p (bool-vector nil t))",
        "(symbol-with-pos-p (standard-syntax-table))",
        "(symbol-with-pos-p (car (read-from-string \"#^^[1 0 nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil]\")))",
        "(symbol-with-pos-p (make-marker))", "(symbol-with-pos-p (current-buffer))",
        "(symbol-with-pos-p (selected-window))", "(symbol-with-pos-p (make-overlay 1 1))",
        "(symbol-with-pos-p (lambda (x) x))", "(symbol-with-pos-p #'car)",
        "(funcall #'symbol-with-pos-p 'ordinary)",
        "(let ((symbols-with-pos-enabled t)) (symbol-with-pos-p 'ordinary))",
    ] {
        let result = vm.eval_source(form).and_then(|v| v.display());
        assert_eq!(result, Ok("nil".into()), "predicate must reject ordinary object: {form}");
    }
    assert_eq!(vm.eval_source("(list (fboundp 'symbol-with-pos-p) (functionp #'symbol-with-pos-p))").unwrap().display().unwrap(), "(t t)");
}

#[test]
fn no_positioned_symbol_producer_is_available() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source("(list (fboundp 'position-symbol) (fboundp 'read-positioning-symbols))").unwrap().display().unwrap(), "(nil nil)", "adding a positioned-symbol producer requires implementing its object and predicate");
    assert_eq!(vm.eval_source("(let ((symbols-with-pos-enabled t)) (eq (car (read-from-string \"same\")) (car (read-from-string \"  same\"))))").unwrap().display().unwrap(), "t", "ordinary reader returns interned bare symbols, not positioned occurrences");
    assert_heap_kind_inventory(include_str!("../src/data.rs"));
}

fn assert_heap_kind_inventory(source: &str) {
    let object = source.split("pub(super) enum Object {").nth(1).unwrap().split("\n}").next().unwrap();
    let variants: Vec<_> = object.lines().map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap())
        .collect();
    // Native subtable slots and traversal cursors contain ordinary values;
    // neither constructs positioned-symbol objects. The reader check above
    // exercises the new public subtable representation.
    assert_eq!(variants, ["SyntaxTable", "Float", "Bignum", "Window", "Frame", "Terminal", "WindowConfiguration", "String", "RawString", "Vector", "BoolVector", "Record", "CharTable", "SubCharTable", "MapKeymapCursor", "Buffer", "Marker", "Overlay", "Bytes", "ByteBuffer", "HashTable", "Obarray"], "new heap kinds require re-auditing the no-positioned-symbol premise");
}

#[test]
#[should_panic(expected = "new heap kinds require re-auditing the no-positioned-symbol premise")]
fn heap_kind_inventory_rejects_unregistered_kind() {
    let source = include_str!("../src/data.rs").replacen(
        "pub(super) enum Object {",
        "pub(super) enum Object {\n    UnregisteredPositionedSymbol,",
        1,
    );
    assert_heap_kind_inventory(&source);
}

#[test]
fn records_read_print_and_trace_bare_symbols_without_positioning() {
    let mut vm = Vm::new();
    let text = "#s(sympos-tag sympos-leaf (sympos-leaf) [sympos-leaf] #s(inner sympos-leaf))";
    let source = format!("(let ((symbols-with-pos-enabled t)) (car (read-from-string {text:?})))");
    let root = vm.eval_source(&source).unwrap().root();
    vm.declare_special_rooted("sympos-record-root", &root).unwrap();
    let check = "(let ((r sympos-record-root)) (and (recordp r) (not (symbol-with-pos-p r)) (eq (aref r 0) 'sympos-tag) (eq (aref r 1) 'sympos-leaf) (eq (car (aref r 2)) 'sympos-leaf) (eq (aref (aref r 3) 0) 'sympos-leaf) (eq (aref (aref r 4) 1) 'sympos-leaf) (bare-symbol-p (aref r 1)) (not (symbol-with-pos-p (aref r 1)))))";
    assert_eq!(vm.eval_source(check).unwrap().display().unwrap(), "t", "RECORD_BARE_SYMBOL_READER");
    let printed = vm.eval_source("(prin1-to-string sympos-record-root)").unwrap().display().unwrap();
    assert_eq!(serde_json::from_str::<String>(&printed).unwrap(), text, "RECORD_BARE_SYMBOL_PRINTER");
    let reread = vm.eval_source("(let ((symbols-with-pos-enabled t)) (car (read-from-string (prin1-to-string sympos-record-root))))").unwrap().root();
    vm.declare_special_rooted("sympos-record-root", &reread).unwrap();
    let collections = vm.heap_stats().collections;
    vm.collect_garbage();
    assert_eq!(vm.heap_stats().collections, collections + 1);
    assert_eq!(vm.eval_source(check).unwrap().display().unwrap(), "t", "RECORD_BARE_SYMBOL_ROUNDTRIP_GC");
}

#[test]
fn record_constructors_mutation_and_copy_preserve_uninterned_bare_symbols() {
    let mut vm = Vm::new();
    let root = vm.eval_source("(let* ((s (make-symbol \"sympos-fresh\")) (r (make-record 'tag 2 s)) (q (copy-sequence r))) (set s (list 7)) (put s 'payload (list 9)) (aset r 0 s) (record 'outer s r q))").unwrap().root();
    vm.declare_special_rooted("sympos-constructor-root", &root).unwrap();
    let check = "(let* ((outer sympos-constructor-root) (s (aref outer 1)) (r (aref outer 2)) (q (aref outer 3))) (and (bare-symbol-p s) (not (symbol-with-pos-p s)) (not (symbol-with-pos-p r)) (not (eq s (intern \"sympos-fresh\"))) (not (eq r q)) (eq (aref r 0) s) (eq (aref r 1) s) (eq (aref r 2) s) (eq (aref q 1) s) (eq (aref q 2) s) (equal (symbol-value s) '(7)) (equal (get s 'payload) '(9))))";
    assert_eq!(vm.eval_source(check).unwrap().display().unwrap(), "t", "RECORD_BARE_SYMBOL_CONSTRUCTORS");
    let collections = vm.heap_stats().collections;
    vm.collect_garbage();
    assert_eq!(vm.heap_stats().collections, collections + 1);
    assert_eq!(vm.eval_source(check).unwrap().display().unwrap(), "t", "RECORD_BARE_SYMBOL_CONSTRUCTOR_GC");
}

#[test]
fn positioned_symbol_chain_frontier() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), source_only.path().to_owned()]).unwrap();
    for file in ["debug-early.el", "byte-run.el", "backquote.el"] {
        vm.load_file(&root.join("gnu-lisp/emacs-lisp").join(file)).unwrap();
    }
    for file in ["subr.el", "keymap.el", "emacs-lisp/pcase.el", "emacs-lisp/gv.el", "emacs-lisp/inline.el"] {
        vm.load_file(&root.join("gnu-lisp").join(file)).unwrap();
    }
    let result = vm.load_file(&root.join("gnu-lisp/emacs-lisp/ert.el")).map(|_| ());
    assert!(result.is_err(), "frontier must be remeasured if ERT loads");
    let report = vm.load_report();
    let cl_macs: Vec<_> = report["steps"].as_array().unwrap().iter()
        .filter(|step| step["file"].as_str().unwrap().ends_with("/cl-macs.el"))
        .collect();
    assert_eq!(cl_macs.len(), 178, "cl-macs now completes after subrp");
    assert!(cl_macs.iter().all(|s| s["status"] == "ok"));
    for (name, completed, ordinal, line) in [
        ("cl-lib.el", 94, 95, 599),
        ("ert.el", 0, 1, 57),
    ] {
        let steps: Vec<_> = report["steps"].as_array().unwrap().iter()
            .filter(|step| step["file"].as_str().unwrap().ends_with(&format!("/{name}")))
            .collect();
        assert_eq!(steps.len(), completed + 1, "attempted frontier for {name}");
        assert_eq!(steps.iter().filter(|step| step["status"] == "ok").count(), completed, "completed frontier for {name}");
        let failed = steps.iter().find(|step| step["status"] == "error").unwrap();
        assert_eq!(failed["ordinal"], ordinal, "frontier ordinal for {name}");
        assert_eq!(failed["line"], line, "frontier line for {name}");
        assert!(failed["error"].as_str().unwrap().ends_with("(void-function define-minor-mode)"), "exact next blocker for {name}: {failed}");
        println!("{name}: completed={completed}, attempted={}, form={ordinal}, line={line}, next=(void-function define-minor-mode)", steps.len());
    }
}

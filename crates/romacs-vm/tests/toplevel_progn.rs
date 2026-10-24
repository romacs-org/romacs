use romacs_vm::Vm;
use std::path::PathBuf;

fn source_file(name: &str, source: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("toplevel-progn");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    path
}

fn eager_vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
    vm
}

#[test]
fn toplevel_progn_installs_macro_before_expanding_sibling() {
    let mut vm = eager_vm();
    let path = source_file("macro.el", include_str!("../evidence/toplevel-progn/macro.el"));
    let result = vm.load_file(&path);
    assert!(result.is_ok(), "TOPLEVEL_MACRO_SEQUENCE: {result:?}");
    assert_eq!(vm.eval_source("tp-result").unwrap().display().unwrap(), "42", "TOPLEVEL_MACRO_VALUE");
}

#[test]
fn macro_generated_progn_installs_function_before_compile_time_sibling() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gnu-lisp/emacs-lisp/debug-early.el")).unwrap();
    vm.load_file(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gnu-lisp/emacs-lisp/byte-run.el")).unwrap();
    vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
    vm.eval_source("(defmacro tp-generated () '(progn (defun tp-constructor () 42) (eval-and-compile (tp-constructor))))").unwrap();
    let path = source_file("generated.el", include_str!("../evidence/toplevel-progn/generated.el"));
    let result = vm.load_file(&path);
    assert!(result.is_ok(), "TOPLEVEL_GENERATED_SEQUENCE: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "42", "TOPLEVEL_GENERATED_VALUE");
}

#[test]
fn eager_prog1_and_prog2_preserve_gnu_expansion_boundaries() {
    for (name, source, expected) in [
        ("prog1", include_str!("../evidence/toplevel-progn/prog1.el"), "1"),
        ("prog2-first", include_str!("../evidence/toplevel-progn/prog2-first.el"), "2"),
        ("prog2-second", include_str!("../evidence/toplevel-progn/prog2-second.el"), "1"),
        ("lexical", include_str!("../evidence/toplevel-progn/lexical.el"), "19"),
    ] {
        let mut vm = eager_vm();
        vm.eval_source("(defmacro tp-m () 1)").unwrap();
        let path = source_file(name, source);
        let outcome = vm.load_file(&path);
        assert!(outcome.is_ok(), "TOPLEVEL_BOUNDARY_{name}: {outcome:?}");
        assert_eq!(vm.eval_source("tp-answer").unwrap().display().unwrap(), expected, "TOPLEVEL_BOUNDARY_VALUE_{name}");
    }
}

#[test]
fn nested_load_preserves_suspended_macro_cache_dependencies() {
    let mut vm = Vm::new();
    let path = source_file("nested-cache.el", "(setq tp-answer 19)");
    let path = serde_json::to_string(&path.to_string_lossy()).unwrap();
    vm.eval_source("(setq tp-cache-state 1)").unwrap();
    vm.eval_source(&format!("(defmacro tp-cache-m () (load {path} nil t t) tp-cache-state)")).unwrap();
    let request = "(setq tp-cache-result (list (tp-cache-m)))";
    assert_eq!(vm.eval_source(request).unwrap().display().unwrap(), "(1)");
    vm.eval_source("(setq tp-cache-state 2)").unwrap();
    assert_eq!(vm.eval_source(request).unwrap().display().unwrap(), "(2)", "TOPLEVEL_NESTED_CACHE_DEPENDENCIES");
}

#[test]
fn toplevel_macro_wrappers_are_reclaimed_after_each_load() {
    let mut vm = eager_vm();
    vm.eval_source("(defmacro tp-string (&rest ignored) '(quote \"ephemeral\"))").unwrap();
    let path = source_file("retention.el", "(tp-string \"argument-owned-by-wrapper\")");
    let mut observed = Vec::new();
    for _ in 0..32 {
        assert_eq!(vm.load_file(&path).unwrap().display().unwrap(), "\"ephemeral\"");
        vm.clear_source_cache();
        vm.collect_garbage();
        let stats = vm.heap_stats();
        observed.push((stats.live_objects, stats.live_bytes));
    }
    assert!(observed.iter().all(|s| *s == observed[0]), "TOPLEVEL_WRAPPER_RETENTION: {observed:?}");
}

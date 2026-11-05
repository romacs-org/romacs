//! A one-argument `(defvar SYMBOL)` at the top level of a loaded file makes
//! SYMBOL special for the REST OF THAT FILE (GNU `Fdefvar`, src/eval.c:990-996:
//! it conses SYMBOL onto `internal-interpreter-environment`, which
//! `readevalloop` binds once for the whole file, src/lread.c). Forms before
//! the declaration, and other files, are unaffected. cl-macs.el depends on
//! this: `(defvar cl--loop-args)` at line 939, then `cl-loop`'s
//! `(let ((cl--loop-args ...)) ...)` must bind dynamically for
//! `cl--parse-loop-clause` to read it. Every expected value is GNU 32.0.50's
//! (`evidence/file-local-special/gnu-transcript.txt`).
use romacs_vm::Vm;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(std::env::var("HOME").unwrap()).join(format!(
        ".hermes/cache/scratch/romacs/vm2/file-special-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const FLA: &str = ";;; fla.el --- -*- lexical-binding: t -*-
(defun fla-get () (if (boundp 'fla-v) fla-v 'unbound))
(defun fla-early () (let ((fla-v 1)) (fla-get)))
(defvar fla-v)
(load \"fln\")
(setq flz-2 (let ((fla-v 8)) (flz-get)))
(defun fla-late () (let ((fla-v 2)) (fla-get)))
(defmacro fla-mac () (let ((fla-v 3)) (list 'quote (fla-get))))
(setq fla-same (let ((fla-v 7)) (fla-get)))
(setq fla-r (list (fla-early) (fla-late) (fla-mac) (funcall (lambda () (let ((fla-v 4)) (fla-get)))) (let ((fla-v 5)) (fla-get)) (special-variable-p 'fla-v)))
";

/// Loaded first: the same `flz-*` form text, compiled where nothing declared
/// `fla-v`, must not be reused by fla.el after its declaration.
const FLZ: &str = ";;; flz.el --- -*- lexical-binding: t -*-
(defun flz-get () (if (boundp 'fla-v) fla-v 'unbound))
(setq flz-1 (let ((fla-v 8)) (flz-get)))
(setq flz-2 (let ((fla-v 8)) (flz-get)))
";

/// Loaded by fla.el after its declaration: a nested load must hand the
/// declaration back to the rest of fla.el.
const FLN: &str = ";;; fln.el --- -*- lexical-binding: t -*-
(setq fln-done t)
";

const FLB: &str = ";;; flb.el --- -*- lexical-binding: t -*-
(defun flb-late () (let ((fla-v 6)) (fla-get)))
(setq flb-a fla-same)
(setq fla-same (let ((fla-v 7)) (fla-get)))
(setq flb-r (list (flb-late) flb-a fla-same))
";

fn run(name: &str, eager: bool) -> String {
    let dir = scratch(name);
    std::fs::write(dir.join("fla.el"), FLA).unwrap();
    std::fs::write(dir.join("flb.el"), FLB).unwrap();
    std::fs::write(dir.join("flz.el"), FLZ).unwrap();
    std::fs::write(dir.join("fln.el"), FLN).unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    if eager {
        // Select the loader's eager (source-cached) regime, as GNU does once
        // macroexp.el is loaded (src/lread.c readevalloop).
        vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
    }
    let source = "(condition-case e (progn (load \"flz\") (load \"fla\") (load \"flb\") (list fla-r flb-r flz-1 flz-2)) (error (list 'signal e)))";
    let value = vm.eval_source(source).unwrap();
    value.display().unwrap()
}

/// GNU: ((unbound 2 3 4 5 nil) (unbound 7 unbound) unbound 8).
#[test]
fn a_top_level_defvar_without_value_is_special_for_the_rest_of_its_file() {
    assert_eq!(run("lazy", false), "((unbound 2 3 4 5 nil) (unbound 7 unbound) unbound 8)");
}

/// The same, in the eager regime whose compiled forms are cached by source
/// text: `fla-same`'s identical form must not reuse fla.el's dynamic binding
/// in flb.el, where nothing declared `fla-v`, and `flz-2`'s must not reuse
/// flz.el's lexical one in fla.el, where it is declared.
#[test]
fn a_file_local_special_does_not_leak_through_the_source_cache() {
    assert_eq!(run("eager", true), "((unbound 2 3 4 5 nil) (unbound 7 unbound) unbound 8)");
}

const FLP: &str = ";;; flp.el --- -*- lexical-binding: t -*-
(defun flp-get () (if (boundp 'flp-x) flp-x 'unbound))
(defvar flp-x)
(setq flp-1 (funcall (lambda (flp-x) (flp-get)) 42))
(setq flp-2 (let ((flp-x 43)) (flp-get)))
";

/// Same form text as FLP's two `setq`s, compiled where `flp-x` is GLOBALLY
/// special: a different compilation context from FLP's file-local one.
const FLQ: &str = ";;; flq.el --- -*- lexical-binding: t -*-
(defvar flp-x 0)
(setq flp-1 (funcall (lambda (flp-x) (flp-get)) 42))
(setq flp-2 (let ((flp-x 43)) (flp-get)))
";

/// Eager (source-cached) VM with FLP/FLQ on its load path, optionally with a
/// persistent store.
fn cached_vm(dir: &std::path::Path, store: Option<&std::path::Path>) -> Vm {
    let mut vm = Vm::new();
    if let Some(store) = store {
        vm.set_source_cache_store(store.to_path_buf()).unwrap();
    }
    vm.set_load_path(&[dir.to_path_buf()]).unwrap();
    vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
    vm
}

fn eval_display(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap();
    value.display().unwrap()
}

/// Review finding (Sol, af1209d0): the persistent cache recompiled a
/// file-local special as a GLOBAL declaration, so the warm load bound the
/// lambda parameter dynamically (42) where the cold load bound it lexically
/// (`unbound`, GNU 32.0.50 interpreted and byte-compiled alike, VM2-VD1).
/// A cache must reproduce the compilation it replaces.
#[test]
fn the_persistent_cache_keeps_a_file_local_special_file_local() {
    let dir = scratch("persist");
    std::fs::write(dir.join("flp.el"), FLP).unwrap();
    let store = dir.join("store");
    let cold = eval_display(&mut cached_vm(&dir, Some(&store)), "(progn (load \"flp\") (list flp-1 flp-2))");
    let mut warm_vm = cached_vm(&dir, Some(&store));
    let warm = eval_display(&mut warm_vm, "(progn (load \"flp\") (list flp-1 flp-2))");
    assert!(warm_vm.source_cache_stats().disk_hits > 0, "the warm load must actually use the store");
    assert_eq!(cold, "(unbound 43)", "GNU's answer, cold");
    assert_eq!(warm, cold, "a persisted compilation changed binding semantics");
}

/// The in-memory cache must not reuse FLP's file-local compilation of a form
/// for the identical text in FLQ, where the symbol is globally special (or the
/// reverse): its validity key has to tell the two kinds of declaration apart.
#[test]
fn the_memory_cache_tells_a_file_local_special_from_a_global_one() {
    let dir = scratch("kinds");
    std::fs::write(dir.join("flp.el"), FLP).unwrap();
    std::fs::write(dir.join("flq.el"), FLQ).unwrap();
    let mut shared = cached_vm(&dir, None);
    let first = eval_display(&mut shared, "(progn (load \"flp\") (list flp-1 flp-2))");
    let after = eval_display(&mut shared, "(progn (load \"flq\") (list flp-1 flp-2))");
    // The same flq.el compiled with nothing cached from flp.el: flp-get comes
    // from a differently spelled definition, so no flq form text is shared.
    let mut fresh = cached_vm(&dir, None);
    eval_display(&mut fresh, "(defun flp-get () (if (boundp (quote flp-x)) flp-x (quote unbound)))");
    let alone = eval_display(&mut fresh, "(progn (load \"flq\") (list flp-1 flp-2))");
    assert_eq!(first, "(unbound 43)");
    assert_eq!(after, alone, "flq.el reused flp.el's file-local compilation");
}

/// time.el declares its `display-time-string-forms' variables inside a
/// top-level call, `(with-no-warnings (defvar now) ...)': GNU's Fdefvar
/// adds the name to the environment it is evaluated in, the file's while
/// the file's own forms run, so the declaration lasts for the rest of the
/// file and `display-time-update''s `let*' binds `now' dynamically for the
/// forms it `eval's.  What counts is what evaluation reaches: a call's
/// arguments and a macro's expansion declare, an untaken branch and a
/// `let' with a lexical binding do not.
const FNW: &str = ";;; fnw.el --- -*- lexical-binding: t -*-
(defun fnw-get (s) (if (boundp s) (symbol-value s) 'unbound))
(defmacro fnw-declare (name) (list 'with-no-warnings (list 'defvar name)))
(with-no-warnings (defvar fnw-a) (defvar fnw-b))
(let ((x 1)) (defvar fnw-c) x)
(if nil (defvar fnw-d))
(ignore (defvar fnw-e))
(fnw-declare fnw-f)
(defun fnw-late ()
  (list (let ((fnw-a 1)) (fnw-get 'fnw-a)) (let ((fnw-b 2)) (fnw-get 'fnw-b)) (let ((fnw-c 3)) (fnw-get 'fnw-c))
        (let ((fnw-d 4)) (fnw-get 'fnw-d)) (let ((fnw-e 5)) (fnw-get 'fnw-e)) (let ((fnw-f 6)) (fnw-get 'fnw-f))))
(setq fnw-r (list (fnw-late) (special-variable-p 'fnw-a)))
";

/// Loaded after FNW: the same declaring form's text, which the eager
/// regime's source cache hands back compiled, declares in this file too.
const FNX: &str = ";;; fnx.el --- -*- lexical-binding: t -*-
(ignore (defvar fnw-e))
(defun fnx-late () (let ((fnw-e 7)) (fnw-get 'fnw-e)))
(defun fnx-early-a () (let ((fnw-a 8)) (fnw-get 'fnw-a)))
(setq fnx-r (list (fnx-late) (fnx-early-a)))
";

/// GNU 32.0.50: ((1 2 unbound unbound 5 6) nil) and (7 unbound), lazy and
/// eager alike.
#[test]
fn a_defvar_evaluated_at_a_files_level_is_special_for_the_rest_of_it() {
    for (name, eager) in [("nested-lazy", false), ("nested-eager", true)] {
        let dir = scratch(name);
        std::fs::write(dir.join("fnw.el"), FNW).unwrap();
        std::fs::write(dir.join("fnx.el"), FNX).unwrap();
        let mut vm = Vm::new();
        vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
        // byte-run.el's and subr.el's, which a bare VM lacks: functions, so
        // their arguments are evaluated.
        vm.eval_source("(defalias 'with-no-warnings (lambda (&rest _body) nil))").unwrap();
        vm.eval_source("(defalias 'ignore (lambda (&rest _args) nil))").unwrap();
        if eager {
            vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
        }
        assert_eq!(eval_display(&mut vm, "(progn (load \"fnw\") (load \"fnx\") (list fnw-r fnx-r))"),
                   "(((1 2 unbound unbound 5 6) nil) (7 unbound))", "{name}");
    }
}

/// The same text as a file's form and as an ordinary evaluation compiles
/// apart: evaluated first, it must not hand the file a compilation that
/// declares nothing; loaded first, it must not hand the evaluation one that
/// declares for a file.  GNU: 9, then nil.
#[test]
fn a_file_level_declaration_and_an_evaluation_do_not_share_a_compilation() {
    let dir = scratch("cache-apart");
    std::fs::write(dir.join("fnv.el"), ";;; fnv.el --- -*- lexical-binding: t -*-
(ignore (defvar fnv-x))
(defun fnv-late () (let ((fnv-x 9)) (if (boundp 'fnv-x) fnv-x 'unbound)))
(setq fnv-r (fnv-late))
").unwrap();
    let fresh = || {
        let mut vm = Vm::new();
        vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
        vm.eval_source("(defalias 'ignore (lambda (&rest _args) nil))").unwrap();
        vm.eval_source("(defalias 'internal-macroexpand-for-load (lambda (form _full) form))").unwrap();
        vm
    };
    // Evaluated first, outside any file.
    let mut vm = fresh();
    vm.eval_source("(ignore (defvar fnv-x))\n").unwrap();
    assert_eq!(eval_display(&mut vm, "(progn (load \"fnv\") fnv-r)"), "9", "evaluated, then loaded");
    // Loaded first, in a VM that has evaluated nothing: the evaluation
    // declares nothing for a file.
    let mut vm = fresh();
    assert_eq!(eval_display(&mut vm, "(progn (load \"fnv\") fnv-r)"), "9");
    vm.eval_source("(setq macroexp--dynvars nil)").unwrap();
    vm.eval_source("(ignore (defvar fnv-x))\n").unwrap();
    assert_eq!(eval_display(&mut vm, "macroexp--dynvars"), "nil", "loaded, then evaluated");
}

/// A load form whose macro fails to expand until the form is reached (the
/// lazy regime defers it, as GNU's interpreter expands at its reach) still
/// declares for the file what its expansion declares.  GNU, without
/// `internal-macroexpand-for-load': 9.
#[test]
fn a_deferred_load_form_declares_for_the_rest_of_its_file() {
    let dir = scratch("deferred");
    std::fs::write(dir.join("fny.el"), ";;; fny.el --- -*- lexical-binding: t -*-
(defun fny-get (s) (if (boundp s) (symbol-value s) 'unbound))
(defvar fny-ready nil)
(defmacro fny-m () (if fny-ready '(defvar fny-z) (error \"not yet\")))
(if t (progn (setq fny-ready t) (fny-m)))
(defun fny-late () (let ((fny-z 9)) (fny-get 'fny-z)))
(setq fny-r (fny-late))
").unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    assert_eq!(eval_display(&mut vm, "(progn (load \"fny\") fny-r)"), "9");
}

//! Lazy function bodies in GNU's non-eager load regime (ruling R215).
//!
//! GNU `readevalloop' (src/lread.c:2183-2188) macroexpands each top-level
//! form eagerly only once `internal-macroexpand-for-load' is fbound, i.e.
//! after macroexp.el is loaded. Before that -- exactly where subr.el is
//! loaded during bootstrap -- forms go through `eval_sub', which leaves an
//! interpreted closure's body unexpanded and expands each form only when
//! evaluation reaches it. Every expected value below was produced by GNU
//! Emacs 32.0.50 (`/usr/bin/emacs -Q --batch`); the transcripts are in
//! crates/romacs-vm/evidence/lazy-bodies-oracle.txt.
use romacs_vm::Vm;
use std::path::PathBuf;

fn source_file(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lazy-bodies");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    path
}

fn shown(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

/// GNU subr.el `add-hook' guards its `setf' forms so that they never run --
/// and therefore are never expanded -- before gv.el is loaded. VM2's native
/// `setf' refuses the `alist-get' place, which is what stopped the bootstrap
/// at subr.el form 228. GNU: (f3 nil) => skipped.
#[test]
fn unreached_unexpandable_form_does_not_fail_definition_or_call() {
    let mut vm = Vm::new();
    let path = source_file(
        "unreached.el",
        "(defalias 'lz-f3 (function (lambda (go) (if go (setf (alist-get 'k lz-place) 2) 'skipped))))\n",
    );
    vm.load_file(&path).expect("GNU defines lz-f3 without expanding its body");
    assert_eq!(shown(&mut vm, "(lz-f3 nil)"), "skipped");
}

/// GNU expands a body form when it is reached, so a macro defined after the
/// function but before its first call is used. GNU: (f1 t) => (1).
#[test]
fn macro_defined_after_the_function_is_expanded_at_the_call() {
    let mut vm = Vm::new();
    let path = source_file(
        "late-macro.el",
        "(defalias 'lz-f1 (function (lambda (x) (if x (lz-zork 1) 'no))))\n\
         (defalias 'lz-zork (cons 'macro (function (lambda (&rest a) (cons 'list a)))))\n",
    );
    vm.load_file(&path).unwrap();
    assert_eq!(shown(&mut vm, "(lz-f1 nil)"), "no");
    assert_eq!(shown(&mut vm, "(lz-f1 t)"), "(1)");
}

/// An expansion-time error surfaces when the form is reached, with the
/// macro's own condition, after earlier forms of the body have run.
/// GNU: (f2 nil) => done, side=ran; (f2 t) => (error "boom-at-expansion"),
/// side=ran.
#[test]
fn expansion_error_is_signalled_at_reach_time_with_gnus_condition() {
    let mut vm = Vm::new();
    let path = source_file(
        "boom.el",
        "(defvar lz-side nil)\n\
         (defalias 'lz-boom (cons 'macro (function (lambda () (error \"boom-at-expansion\")))))\n\
         (defalias 'lz-f2 (function (lambda (go) (setq lz-side 'ran) (if go (lz-boom)) 'done)))\n",
    );
    vm.load_file(&path).expect("GNU defines lz-f2 without expanding its body");
    assert_eq!(shown(&mut vm, "(list (lz-f2 nil) lz-side)"), "(done ran)");
    assert_eq!(
        shown(&mut vm, "(progn (setq lz-side nil) (list (condition-case e (lz-f2 t) (error e)) lz-side))"),
        "((error \"boom-at-expansion\") ran)"
    );
}

/// The reach-time signal is the macro's own condition, not a flattened
/// `error'. GNU: (f4) => (wrong-type-argument listp 7).
#[test]
fn reach_time_expansion_keeps_a_non_error_condition_structured() {
    let mut vm = Vm::new();
    let path = source_file(
        "boom2.el",
        "(defalias 'lz-boom2 (cons 'macro (function (lambda () (signal 'wrong-type-argument '(listp 7))))))\n\
         (defalias 'lz-f4 (function (lambda () (lz-boom2))))\n",
    );
    vm.load_file(&path).expect("GNU defines lz-f4 without expanding its body");
    assert_eq!(
        shown(&mut vm, "(condition-case e (lz-f4) (error e))"),
        "(wrong-type-argument listp 7)"
    );
}

/// GNU data.c:541: t only for an interpreted closure. VM2 loads no .elc, so
/// every closure built from Lisp source is interpreted in GNU's sense; a
/// primitive is not. GNU: car => nil, nil => nil, (lambda (x) x) => t,
/// (interpreted-function-p) => (wrong-number-of-arguments ...).
#[test]
fn interpreted_function_p_reports_gnus_answers() {
    let mut vm = Vm::new();
    assert_eq!(
        shown(&mut vm, "(list (interpreted-function-p (symbol-function 'car)) (interpreted-function-p nil) (interpreted-function-p (lambda (x) x)) (interpreted-function-p 7))"),
        "(nil nil t nil)"
    );
    let path = source_file("ifp.el", "(defalias 'lz-ifp (function (lambda () 1)))\n");
    vm.load_file(&path).unwrap();
    assert_eq!(shown(&mut vm, "(interpreted-function-p (symbol-function 'lz-ifp))"), "t");
}

/// A body discarded by its own expansion. Held lazily it retains every cons
/// of the quoted list and the whole string; once expanded it retains none.
fn discarded_body() -> String {
    let symbols: Vec<String> = (0..300).map(|i| format!("lz-sym{i}")).collect();
    format!(
        "(defalias 'lz-ignore (cons 'macro (function (lambda (&rest _) nil))))\n\
         (defalias 'lz-probe (function (lambda (n) (lz-ignore '({}) \"{}\") (+ n 1))))\n",
        symbols.join(" "),
        "z".repeat(65_536)
    )
}

fn settled(vm: &mut Vm) -> (usize, usize) {
    vm.collect_garbage();
    let stats = vm.heap_stats();
    (stats.live_objects, stats.live_bytes)
}

/// The retained unexpanded body is collector-visible state. Measured against
/// a twin VM loading the identical file in GNU's EAGER regime (a fbound
/// `internal-macroexpand-for-load'), which never holds a body:
/// 1. while lazily held, its 300+ conses and 64 KiB string stay live across
///    a forced collection whose only path to them is the function object;
/// 2. the collection moves it (garbage allocated first is compacted away)
///    and the function is still callable and correct afterwards;
/// 3. once expanded the body is reclaimed: live objects AND bytes return to
///    exactly the eager twin's figures, so nothing leaks.
#[test]
fn lazily_held_body_survives_moving_collection_and_is_reclaimed_after_expansion() {
    let eager_path = source_file("gc-eager.el", &discarded_body());
    let lazy_path = source_file("gc-lazy.el", &discarded_body());
    // Identical heaps except for WHICH symbol holds the one closure: only the
    // eager twin's makes `internal-macroexpand-for-load' fbound.
    let mut eager = Vm::new();
    eager.eval_source("(defalias 'internal-macroexpand-for-load (function (lambda (form _full) form)))").unwrap();
    let mut lazy = Vm::new();
    lazy.eval_source("(defalias 'lz-not-the-switch (function (lambda (form _full) form)))").unwrap();
    // Garbage allocated BEFORE the definition: compacting it away moves
    // every object allocated after it, including a lazily held body.
    for vm in [&mut eager, &mut lazy] {
        vm.eval_source("(let ((i 0) (g nil)) (while (< i 2000) (setq g (cons i g)) (setq i (1+ i))) nil)").unwrap();
    }
    eager.load_file(&eager_path).unwrap();
    lazy.load_file(&lazy_path).unwrap();
    let (eager_objects, eager_bytes) = settled(&mut eager);
    let (held_objects, held_bytes) = settled(&mut lazy);
    eprintln!("eager {eager_objects} objects / {eager_bytes} bytes; lazily held {held_objects} / {held_bytes}");
    assert!(
        held_objects >= eager_objects + 300 && held_bytes >= eager_bytes + 65_536,
        "the unexpanded body must stay live across collection: eager {eager_objects}/{eager_bytes}, lazy {held_objects}/{held_bytes}"
    );
    assert_eq!(shown(&mut eager, "(lz-probe 41)"), "42");
    assert_eq!(shown(&mut lazy, "(lz-probe 41)"), "42", "body must survive the moving collection");
    // `load-history' keeps each loaded file's entry, as GNU's does; it is
    // not the object under test, so drop it before comparing heaps.
    for vm in [&mut eager, &mut lazy] { vm.eval_source("(setq load-history nil)").unwrap(); }
    let (eager_after, eager_after_bytes) = settled(&mut eager);
    let (lazy_after, lazy_after_bytes) = settled(&mut lazy);
    eprintln!("after first call: eager {eager_after} / {eager_after_bytes}; lazy {lazy_after} / {lazy_after_bytes}");
    assert_eq!(
        (lazy_after, lazy_after_bytes),
        (eager_after, eager_after_bytes),
        "an expanded body must be reclaimed, not leaked"
    );
    assert_eq!(shown(&mut lazy, "(lz-probe 1)"), "2");
}

/// A body that is never expanded dies with its function: once nothing
/// reaches the function object, a collection reclaims the body (live
/// objects and bytes return to the pre-definition baseline) instead of
/// forwarding a dangling reference.
#[test]
fn never_called_lazy_body_is_reclaimed_with_its_function() {
    let mut vm = Vm::new();
    vm.eval_source("(progn (defalias 'lz-probe nil) (setq current-load-list nil))").unwrap();
    let (base_objects, base_bytes) = settled(&mut vm);
    let path = source_file("gc-dead.el", &discarded_body());
    vm.load_file(&path).unwrap();
    let (held_objects, held_bytes) = settled(&mut vm);
    assert!(
        held_objects >= base_objects + 300 && held_bytes >= base_bytes + 65_536,
        "held lazily: base {base_objects}/{base_bytes}, held {held_objects}/{held_bytes}"
    );
    vm.eval_source("(progn (defalias 'lz-probe nil) (defalias 'lz-ignore nil))").unwrap();
    // `load-history' keeps each loaded file's entry, and `current-load-list'
    // the (defun . NAME) entries those top-level defaliases attach, as
    // GNU's do; neither is the object under test, so drop both before
    // comparing heaps.
    vm.eval_source("(setq load-history nil current-load-list nil)").unwrap();
    let (dead_objects, dead_bytes) = settled(&mut vm);
    eprintln!("base {base_objects}/{base_bytes} held {held_objects}/{held_bytes} dead {dead_objects}/{dead_bytes}");
    assert_eq!((dead_objects, dead_bytes), (base_objects, base_bytes), "an unreachable lazy body must be reclaimed");
}

/// A lazily held closure shares its captured lexical cells with siblings.
/// GNU: (lz-set 5) (lz-get) => 5.
#[test]
fn lazily_held_closures_share_captured_cells_across_collection() {
    let mut vm = Vm::new();
    let path = source_file(
        "cells.el",
        "(let ((x 1)) (defalias 'lz-get (function (lambda () x))) (defalias 'lz-set (function (lambda (v) (setq x v)))))\n",
    );
    vm.load_file(&path).unwrap();
    vm.eval_source("(let ((i 0) (g nil)) (while (< i 2000) (setq g (cons i g)) (setq i (1+ i))) nil)").unwrap();
    vm.collect_garbage();
    assert_eq!(shown(&mut vm, "(progn (lz-set 5) (lz-get))"), "5");
    vm.collect_garbage();
    assert_eq!(shown(&mut vm, "(progn (lz-set 6) (lz-get))"), "6");
}

/// The eager regime stays eager: once `internal-macroexpand-for-load' is
/// fbound, GNU expands the whole definition at load and an expansion error
/// fails the load ("Eager macro-expansion failure").
#[test]
fn eager_regime_still_fails_the_definition_at_load() {
    let mut vm = Vm::new();
    vm.eval_source("(defalias 'internal-macroexpand-for-load (function (lambda (form _full) form)))").unwrap();
    let path = source_file(
        "eager-boom.el",
        "(defalias 'lz-eboom (cons 'macro (function (lambda () (error \"boom-at-expansion\")))))\n\
         (defalias 'lz-ef (function (lambda (go) (if go (lz-eboom)) 'done)))\n",
    );
    let error = vm.load_file(&path).unwrap_err();
    assert!(error.contains("boom-at-expansion"), "{error}");
}

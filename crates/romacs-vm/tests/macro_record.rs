//! The macro record (redesign/macro-invalidation S1): each compiled
//! function records the macro cells, alias links and compiler-macro
//! handlers it expanded through, `romacs-who-macroexpands` finds it by
//! any of them, and a write to one is counted as a would-be invalidation
//! while the old expansion keeps running.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;
use romacs_vm::Vm;
use std::path::PathBuf;

/// BODY run on GNU batch's VM, on a stack its boot fits.
fn with_gnu(body: impl FnOnce(&mut Vm) + Send + 'static) {
    std::thread::Builder::new().stack_size(256 << 20).spawn(move || body(&mut gnu_lisp(&[])))
        .unwrap().join().unwrap();
}

fn shown(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Load TEXT as the lexical file NAME.
fn load(vm: &mut Vm, name: &str, text: &str) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("macro-record");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    vm.load_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
}

/// Whether `romacs-who-macroexpands` of MACRO names CALLER.
fn finds(vm: &mut Vm, macro_name: &str, caller: &str) -> bool {
    shown(vm, &format!("(and (memq '{caller} (romacs-who-macroexpands '{macro_name})) t)")) == "t"
}

#[test]
fn a_defun_records_its_macro_from_a_load_and_from_eval() {
    with_gnu(|vm| {
        load(vm, "mr-defun.el", "(defmacro mr-m () 1)\n(defun mr-loaded () (mr-m))\n");
        shown(vm, "(eval '(defun mr-evaluated () (mr-m)) t)");
        shown(vm, "(mr-evaluated)");
        assert!(finds(vm, "mr-m", "mr-loaded"));
        assert!(finds(vm, "mr-m", "mr-evaluated"));
        assert!(!finds(vm, "mr-m", "car"), "only the callers");
    });
}

#[test]
fn every_link_of_an_alias_chain_is_recorded() {
    with_gnu(|vm| {
        load(vm, "mr-alias.el", "(defmacro mr-m () 1)\n(defalias 'mr-b 'mr-m)\n(defalias 'mr-a 'mr-b)\n(defun mr-f () (mr-a))\n");
        for link in ["mr-a", "mr-b", "mr-m"] {
            assert!(finds(vm, link, "mr-f"), "{link}");
        }
    });
}

#[test]
fn an_expander_s_own_macroexpansion_is_recorded_but_not_its_local_macros() {
    with_gnu(|vm| {
        load(vm, "mr-macrolet.el", "(require 'cl-lib)
(defmacro mr-m () 1)
(defmacro mr-all (form) (macroexpand-all form))
(defun mr-in-macrolet () (cl-macrolet ((mr-x () '(mr-m))) (mr-x)))
(defun mr-in-expander () (mr-all (mr-m)))\n");
        assert!(finds(vm, "mr-m", "mr-in-macrolet"));
        assert!(finds(vm, "mr-m", "mr-in-expander"));
        assert_eq!(shown(vm, "(romacs-who-macroexpands 'mr-x)"), "nil", "a cl-macrolet name is not recorded");
    });
}

#[test]
fn a_compiler_macro_records_its_head_and_handler() {
    with_gnu(|vm| {
        load(vm, "mr-compiler-macro.el", "(defun mr-target (x) (list x))
(defun mr-handler (_form x) (list 'quote (list x x)))
(put 'mr-target 'compiler-macro #'mr-handler)
(defmacro mr-all (form) (macroexpand-all form))
(defun mr-optimized () (mr-all (mr-target 1)))\n");
        assert_eq!(shown(vm, "(mr-optimized)"), "(1 1)", "the handler applied");
        assert!(finds(vm, "mr-handler", "mr-optimized"));
        assert!(finds(vm, "mr-target", "mr-optimized"));
    });
}

#[test]
fn a_lazy_body_is_recorded_once_it_is_forced() {
    with_gnu(|vm| {
        load(vm, "mr-lazy.el", "(defmacro mr-m () 1)\n(defun mr-outer () (mapcar (lambda (_) (mr-m)) '(1)))\n");
        shown(vm, "(eval '(defalias 'mr-lazy (lambda () (mr-m))) t)");
        assert!(!finds(vm, "mr-m", "mr-lazy"), "held unexpanded until its first call");
        assert_eq!(shown(vm, "(list (mr-lazy) (mr-outer))"), "(1 (1))");
        assert!(finds(vm, "mr-m", "mr-lazy"));
        assert!(finds(vm, "mr-m", "mr-outer"), "through the lambda its code makes");
    });
}

#[test]
fn a_defun_under_a_top_level_macrolet_takes_the_form_s_expansions() {
    with_gnu(|vm| {
        load(vm, "mr-top-macrolet.el", "(require 'cl-lib)
(defmacro mr-m () 1)
(cl-macrolet ((mr-x () '(mr-m)))
  (defun mr-under () (mr-x)))\n");
        assert_eq!(shown(vm, "(mr-under)"), "1");
        assert!(finds(vm, "mr-m", "mr-under"));
        assert_eq!(shown(vm, "(romacs-who-macroexpands 'mr-x)"), "nil");
    });
}

/// A write is counted, with what it would reach through the expanders it
/// makes stale; the old expansion keeps running (S1 changes nothing).
#[test]
fn a_redefinition_counts_its_would_be_invalidations_and_changes_nothing() {
    with_gnu(|vm| {
        load(vm, "mr-counter.el", "(defmacro mr-inner () 1)
(defmacro mr-outer () (list 'quote (mr-inner)))
(defun mr-direct () (mr-inner))
(defun mr-through () (mr-outer))\n");
        let before = shown(vm, "(plist-get (romacs-macro-invalidation-stats) :events)");
        shown(vm, "(defmacro mr-inner () 2)");
        assert_eq!(shown(vm, "(list (mr-direct) (mr-through))"), "(1 1)", "no behaviour change");
        let events = shown(vm, "(plist-get (romacs-macro-invalidation-stats) :events)");
        assert_eq!(events.parse::<u64>().unwrap(), before.parse::<u64>().unwrap() + 1);
        assert_eq!(shown(vm, "(let ((e (car (last (plist-get (romacs-macro-invalidation-stats) :log))))) \
                              (list (nth 0 e) (nth 1 e) (nth 2 e) (nth 4 e)))"),
                   "(mr-inner 3 2 (mr-outer))", "mr-direct, mr-outer's expander, then mr-through");
        assert_eq!(shown(vm, "(romacs-who-macroexpands 'mr-inner t)"), "(mr-direct mr-outer mr-through)");
    });
}

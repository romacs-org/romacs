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

/// TEXT written as the lexical file NAME, for a load or an autoload.
fn file(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("macro-record");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    path
}

/// Load TEXT as the lexical file NAME.
fn load(vm: &mut Vm, name: &str, text: &str) {
    vm.load_file(&file(name, text)).unwrap_or_else(|e| panic!("{name}: {e}"));
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

/// The first expansion through an autoload that installs `a -> b -> m`
/// records all three, through Lisp's `macroexpand-1` and the `macroexpand`
/// primitive alike.
#[test]
fn a_first_expansion_through_an_autoload_records_the_chain_it_installs() {
    with_gnu(|vm| {
        let lisp = file("mr-chain-1.el", "(defmacro mr-m1 () 1)\n(defalias 'mr-b1 'mr-m1)\n(defalias 'mr-a1 'mr-b1)\n");
        let native = file("mr-chain-2.el", "(defmacro mr-m2 () 2)\n(defalias 'mr-b2 'mr-m2)\n(defalias 'mr-a2 'mr-b2)\n");
        load(vm, "mr-chain-users.el", &format!("(autoload 'mr-a1 {lisp:?} nil nil 'macro)
(autoload 'mr-a2 {native:?} nil nil 'macro)
(defmacro mr-all (form) (macroexpand-all form))
(defmacro mr-once (form) (macroexpand form))
(defun mr-via-lisp () (mr-all (mr-a1)))
(defun mr-via-primitive () (mr-once (mr-a2)))\n"));
        assert_eq!(shown(vm, "(list (mr-via-lisp) (mr-via-primitive))"), "(1 2)");
        for link in ["mr-a1", "mr-b1", "mr-m1"] {
            assert!(finds(vm, link, "mr-via-lisp"), "{link}");
        }
        for link in ["mr-a2", "mr-b2", "mr-m2"] {
            assert!(finds(vm, link, "mr-via-primitive"), "{link}");
        }
    });
}

#[test]
fn a_global_alias_to_a_local_macro_records_the_alias_alone() {
    with_gnu(|vm| {
        load(vm, "mr-local.el", "(defalias 'mr-g 'mr-x)
(defmacro mr-local (form) (macroexpand form '((mr-x . (lambda () 7)))))
(defun mr-via-alias () (mr-local (mr-g)))\n");
        assert_eq!(shown(vm, "(mr-via-alias)"), "7");
        assert!(finds(vm, "mr-g", "mr-via-alias"));
        assert_eq!(shown(vm, "(romacs-who-macroexpands 'mr-x)"), "nil", "the local binding is not a cell");
    });
}

/// A compiler macro's handler is recorded through its alias, at the
/// version it had when it ran, even one that replaces itself as it runs.
#[test]
fn a_compiler_macro_records_its_handler_as_it_resolved_it() {
    with_gnu(|vm| {
        load(vm, "mr-handlers.el", "(defun mr-target (x) (list x))
(defun mr-handler (_form x) (list 'quote (list x x)))
(defalias 'mr-h 'mr-handler)
(put 'mr-target 'compiler-macro 'mr-h)
(defun mr-target2 (x) x)
(defun mr-self (_form x) (fset 'mr-self (lambda (form &rest _) form)) (list 'quote (list x)))
(put 'mr-target2 'compiler-macro 'mr-self)
(defmacro mr-all (form) (macroexpand-all form))
(defun mr-aliased () (mr-all (mr-target 1)))
(defun mr-replaced () (mr-all (mr-target2 1)))\n");
        assert_eq!(shown(vm, "(list (mr-aliased) (mr-replaced))"), "((1 1) (1))");
        assert!(finds(vm, "mr-h", "mr-aliased") && finds(vm, "mr-handler", "mr-aliased"));
        assert!(finds(vm, "mr-self", "mr-replaced"));
        // The record holds the handler as it ran, not the one it installed:
        // replacing that one makes nothing stale.
        shown(vm, "(fset 'mr-self (lambda (form &rest _) form))");
        assert_eq!(shown(vm, "(let ((e (car (last (plist-get (romacs-macro-invalidation-stats) :log))))) (list (nth 0 e) (nth 1 e)))"),
                   "(mr-self 0)");
    });
}

/// A handler still autoloaded at its first call is recorded as it runs:
/// the alias its file installs and the function at the alias's end.
#[test]
fn an_autoloaded_compiler_macro_handler_is_recorded_as_it_loads() {
    with_gnu(|vm| {
        let library = file("mr-cm-lib.el", "(defun mr-cm-real (_form x) (list 'quote (list x 'real)))\n(defalias 'mr-cm-h 'mr-cm-real)\n");
        load(vm, "mr-cm-users.el", &format!("(defun mr-target3 (x) x)
(autoload 'mr-cm-h {library:?})
(put 'mr-target3 'compiler-macro 'mr-cm-h)
(defmacro mr-all (form) (macroexpand-all form))
(defun mr-auto-handled () (mr-all (mr-target3 1)))\n"));
        assert_eq!(shown(vm, "(mr-auto-handled)"), "(1 real)", "the loaded handler ran");
        assert!(finds(vm, "mr-cm-h", "mr-auto-handled") && finds(vm, "mr-cm-real", "mr-auto-handled"));
    });
}

/// A handler whose first library aliases it to a second autoload is
/// recorded as it finally ran: every link the two loads installed.
#[test]
fn a_chained_autoload_handler_is_recorded_as_it_finally_runs() {
    with_gnu(|vm| {
        let second = file("mr-chain-cm-2.el", "(defun mr-ch-real (_form x) (list 'quote (list x 'chained)))\n(defalias 'mr-ch2 'mr-ch-real)\n");
        let first = file("mr-chain-cm-1.el", &format!("(autoload 'mr-ch2 {second:?})\n(defalias 'mr-ch 'mr-ch2)\n"));
        load(vm, "mr-chain-cm-users.el", &format!("(defun mr-target4 (x) x)
(autoload 'mr-ch {first:?})
(put 'mr-target4 'compiler-macro 'mr-ch)
(defmacro mr-all (form) (macroexpand-all form))
(defun mr-chain-handled () (mr-all (mr-target4 1)))\n"));
        assert_eq!(shown(vm, "(mr-chain-handled)"), "(1 chained)", "the handler at the chain's end ran");
        for link in ["mr-ch", "mr-ch2", "mr-ch-real"] {
            assert!(finds(vm, link, "mr-chain-handled"), "{link}");
        }
    });
}

/// Recording a compiler macro calls nothing Lisp can see: advice on
/// `autoload-do-load` sees the calls pinned GNU's makes for the probe's
/// own symbols, and no other. (GNU's load of the handler's file expands it
/// eagerly in Lisp, so calls naming what that file uses differ already.)
#[test]
fn recording_a_handler_calls_autoload_do_load_as_gnu_does() {
    let library = file("mr-probe-lib.el", "(defun mr-ph-real (_form x) (list 'quote (list x 'p)))\n(defalias 'mr-ph 'mr-ph-real)\n");
    let case = format!("(progn
  (defvar mr-calls nil)
  (advice-add 'autoload-do-load :before
              (lambda (_def &optional name &rest _)
                (when (memq name '(mr-tp mr-ph mr-ph-real)) (push name mr-calls))))
  (defun mr-tp (x) x)
  (autoload 'mr-ph {library:?})
  (put 'mr-tp 'compiler-macro 'mr-ph)
  (list (macroexpand-all '(mr-tp 1)) (nreverse mr-calls)))");
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {case}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = gnu_lisp(&[]);
        let printed = vm.eval_source(&format!("(prin1-to-string {case})"))?.display()?;
        Ok::<_, String>(serde_json::from_str::<String>(&printed).unwrap_or(printed))
    }).unwrap().join().unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}

/// A load a compile triggers, its after-load forms included, records
/// nothing into that compile.
#[test]
fn a_load_a_compile_triggers_records_nothing_into_it() {
    with_gnu(|vm| {
        let library = file("mr-lib.el", "(defmacro mr-auto () 3)\n(macroexpand '(mr-z))\n");
        load(vm, "mr-load-users.el", &format!("(defmacro mr-z () 4)
(defvar mr-after nil)
(with-eval-after-load \"mr-lib\" (setq mr-after (macroexpand '(mr-z))))
(autoload 'mr-auto {library:?} nil nil 'macro)
(defun mr-triggers () (mr-auto))\n"));
        assert_eq!(shown(vm, "(list (mr-triggers) mr-after)"), "(3 4)");
        assert!(finds(vm, "mr-auto", "mr-triggers"));
        assert!(!finds(vm, "mr-z", "mr-triggers"));
    });
}

/// A logged write names its symbol as it was: one outside the initial
/// obarray, whose slot a collection may give another symbol, by its name.
#[test]
fn the_write_log_keeps_a_symbol_s_name() {
    with_gnu(|vm| {
        let name = shown(vm, "(let* ((ob (obarray-make)) (s (intern \"mr-ob\" ob)))
  (defalias s (cons 'macro (lambda () 1)))
  (eval (list 'defalias ''mr-obcall (list 'function (list 'lambda nil (list s)))) t)
  (mr-obcall)
  (defalias s (cons 'macro (lambda () 2)))
  (car (car (last (plist-get (romacs-macro-invalidation-stats) :log)))))");
        assert_eq!(name, "\"mr-ob\"");
    });
}

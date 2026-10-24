//! VM2-AUTOLOAD1: an autoload loads its file (GNU eval.c `Fautoload_do_load`,
//! `load_with_autoload_queue`, `un_autoload`). Every expected value below is
//! GNU 32.0.50's, recorded by `evidence/autoload-load/oracle.sh` into
//! `evidence/autoload-load/gnu-transcript.txt`.
use romacs_vm::Vm;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(std::env::var("HOME").unwrap()).join(format!(
        ".hermes/cache/scratch/romacs/vm2/autoload-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const FILES: &[(&str, &str)] = &[
    ("al-fn.el", "(defun al-fn (x) (list 'al-fn x))\n(provide 'al-fn)\n"),
    ("al-mac.el", "(defmacro al-mac (x) (list 'quote (list 'al-mac-expanded x)))\n"),
    (
        "al-bad.el",
        "(defun al-helper-fresh () 'fresh)\n(defun al-existing () 'redefined)\n(defalias 'al-aliased (lambda () 'redefined))\n(defun al-bad () 'bad)\n(provide 'al-bad)\n(error \"al-bad deliberately fails\")\n",
    ),
    ("al-nodef.el", "(defun al-other () 1)\n"),
    (
        "al-outer.el",
        "(setq al-inner-result (condition-case e (al-bad) (error (list 'caught (car e)))))\n(defun al-outer () (list 'outer al-inner-result))\n",
    ),
    ("al-match.el", "(string-match \"b+\" \"xxbbbyy\")\n(defun al-match () 'm)\n"),
    (
        "al-req.el",
        "(defun al-existing () 'req-redefined)\n(provide 'al-req)\n(error \"al-req deliberately fails\")\n",
    ),
    ("al-user.el", "(setq al-user-value (al-mac in-file))\n"),
    ("al-test.el", "(defun al-test (a b) (equal a b))\n"),
    ("al-out2.el", "(defun al-x1 () 'outer-redefined)\n(al-in2)\n(error \"outer fails\")\n"),
    ("al-in2.el", "(defun al-x2 () 'inner-redefined)\n(defun al-in2 () 'in)\n"),
    ("al-am.el", "(defmacro al-am (x) (list 'quote (list 'expanded x)))\n"),
    ("al-mnodef.el", "(defun al-other () 1)\n"),
    ("al-redirect.el", "(defmacro al-real-m (x) (list 'quote (list 'redirected x)))\n(defalias 'al-redirect-m 'al-real-m)\n"),
    ("al-later.el", "(defmacro al-later-m (x) (list 'quote (list 'later x)))\n"),
    ("al-cyc.el", "(defalias 'al-cyc-m 'al-cyc-target)\n(defalias 'al-cyc-target 'al-cyc-m)\n"),
];

fn vm(name: &str) -> (Vm, PathBuf) {
    let dir = scratch(name);
    for (file, text) in FILES {
        std::fs::write(dir.join(file), text).unwrap();
    }
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    (vm, dir)
}

/// FORM's value, or `(signal CONDITION)` for an escaping signal, so a
/// missing behaviour fails at the assertion that names it.
fn shown(vm: &mut Vm, form: &str) -> String {
    let source = format!("(condition-case e {form} (error (list 'signal e)))");
    let value = vm.eval_source(&source).unwrap_or_else(|e| panic!("{form}: {e}"));
    value.display().unwrap()
}

#[test]
fn an_autoloaded_function_loads_its_file_when_called_funcalled_or_applied() {
    let (mut vm, _dir) = vm("call");
    vm.eval_source("(autoload 'al-fn \"al-fn\")").unwrap();
    assert_eq!(shown(&mut vm, "(list (al-fn 1) (featurep 'al-fn) (car-safe (symbol-function 'al-fn)))"), "((al-fn 1) t nil)");
    vm.eval_source("(fset 'al-fn nil) (setq features nil) (autoload 'al-fn \"al-fn\")").unwrap();
    assert_eq!(shown(&mut vm, "(funcall 'al-fn 2)"), "(al-fn 2)");
    vm.eval_source("(fset 'al-fn nil) (autoload 'al-fn \"al-fn\")").unwrap();
    assert_eq!(shown(&mut vm, "(apply 'al-fn '(3))"), "(al-fn 3)");
}

/// A primitive's call request (`Step::Call`, here `assoc`'s TESTFN) reaching
/// an autoload loads it too: GNU `funcall_general` (eval.c:3176) is the same
/// path for C callers.
#[test]
fn a_primitive_calling_an_autoloaded_function_loads_it() {
    let (mut vm, _dir) = vm("t2");
    vm.eval_source("(autoload 'al-test \"al-test\")").unwrap();
    assert_eq!(shown(&mut vm, "(list (assoc 2 '((1 . a) (2 . b)) 'al-test) (car-safe (symbol-function 'al-test)))"), "((2 . b) nil)");
}

/// GNU Fmacroexpand follows symbol aliases before testing for a macro or
/// a macro autoload (eval.c:1290-1305,1326). GNU: (expanded foo) for both.
#[test]
fn an_alias_of_a_macro_or_macro_autoload_expands() {
    let (mut vm, _dir) = vm("alias");
    vm.eval_source("(defmacro al-m1 (x) (list 'quote (list 'expanded x))) (defalias 'al-a1 'al-m1)").unwrap();
    assert_eq!(shown(&mut vm, "(al-a1 foo)"), "(expanded foo)");
    vm.eval_source("(autoload 'al-am \"al-am\" nil nil 'macro) (defalias 'al-alias-am 'al-am) (defalias 'al-alias2 'al-alias-am)").unwrap();
    assert_eq!(shown(&mut vm, "(al-alias2 foo)"), "(expanded foo)");
    assert_eq!(shown(&mut vm, "(car-safe (symbol-function 'al-am))"), "macro");
}

/// A macro autoload whose file makes the symbol an ALIAS of a macro expands
/// through it (eval.c:1326-1332). GNU: (redirected foo).
#[test]
fn a_macro_autoload_whose_file_installs_an_alias_expands() {
    let (mut vm, _dir) = vm("redirect");
    vm.eval_source("(autoload 'al-redirect-m \"al-redirect\" nil nil 'macro)").unwrap();
    assert_eq!(shown(&mut vm, "(al-redirect-m foo)"), "(redirected foo)");
}

/// A call compiled through an alias of a FUNCTION is not reused once the
/// alias target, or the alias itself, turns into a macro or macro autoload.
/// GNU: (changed 7) (changed 8) (later 9).
#[test]
fn a_cached_call_through_an_alias_is_recompiled_when_its_target_becomes_a_macro() {
    let (mut vm, _dir) = vm("alias-cache");
    vm.eval_source("(defun al-target-f (x) x) (defalias 'al-alias-f 'al-target-f)").unwrap();
    assert_eq!(shown(&mut vm, "(al-alias-f 7)"), "7");
    assert_eq!(shown(&mut vm, "(al-alias-f 7)"), "7");
    vm.eval_source("(defmacro al-target-f (x) (list 'quote (list 'changed x)))").unwrap();
    assert_eq!(shown(&mut vm, "(al-alias-f 7)"), "(changed 7)");
    assert_eq!(shown(&mut vm, "(al-alias-f 8)"), "(changed 8)");
    vm.eval_source("(defun al-later-m (x) x) (defalias 'al-alias-l 'al-later-m)").unwrap();
    assert_eq!(shown(&mut vm, "(al-alias-l 9)"), "9");
    vm.eval_source("(fset 'al-later-m nil) (autoload 'al-later-m \"al-later\" nil nil 'macro)").unwrap();
    assert_eq!(shown(&mut vm, "(al-alias-l 9)"), "(later 9)");
}

/// A failed macro autoload is a failed request; its condition must not be
/// re-raised by a later, unrelated failing load. GNU: file-missing.
#[test]
fn a_failed_macro_autoload_does_not_replace_a_later_signal() {
    let (mut vm, _dir) = vm("stale");
    vm.eval_source("(autoload 'al-mnodef \"al-mnodef\" nil nil 'macro)").unwrap();
    assert!(vm.eval_source("(al-mnodef)").is_err(), "al-mnodef.el defines no al-mnodef");
    assert_eq!(shown(&mut vm, "(condition-case e (load \"al-no-such-file\") (error (car e)))"), "file-missing");
    assert_eq!(shown(&mut vm, "(condition-case e (require 'al-no-such-feature) (error (car e)))"), "file-missing");
}

#[test]
fn an_autoloaded_macro_loads_before_its_call_is_expanded() {
    let (mut vm, dir) = vm("macro");
    vm.eval_source("(autoload 'al-mac \"al-mac\" nil nil t)").unwrap();
    // GNU: (al-mac foo) => (al-mac-expanded foo); without the load the form
    // compiles as a call and evaluating its argument signals void-variable.
    assert_eq!(shown(&mut vm, "(al-mac foo)"), "(al-mac-expanded foo)");
    vm.eval_source("(fset 'al-mac nil) (autoload 'al-mac \"al-mac\" nil nil 'macro)").unwrap();
    let loaded = vm.load_file(&dir.join("al-user.el")).map(|_| ());
    assert_eq!(shown(&mut vm, "al-user-value"), "(al-mac-expanded in-file)", "{loaded:?}");
}

#[test]
fn autoload_do_load_follows_gnu_for_each_argument_shape() {
    let (mut vm, _dir) = vm("do-load");
    // Not an autoload: returned unchanged, nothing loaded.
    assert_eq!(shown(&mut vm, "(autoload-do-load 42 'whatever)"), "42");
    // MACRO-ONLY on a function autoload: no load, FUNDEF back.
    assert_eq!(
        shown(&mut vm, "(progn (fset 'al-fn2 '(autoload \"al-fn\")) (list (autoload-do-load (symbol-function 'al-fn2) 'al-fn2 'macro) (symbol-function 'al-fn2) (featurep 'al-fn)))"),
        "((autoload \"al-fn\") (autoload \"al-fn\") nil)"
    );
    // No FUNNAME: loads, returns nil.
    assert_eq!(shown(&mut vm, "(progn (autoload 'al-fn \"al-fn\") (list (autoload-do-load (symbol-function 'al-fn)) (featurep 'al-fn)))"), "(nil t)");
    // FUNNAME: the new definition.
    assert_eq!(shown(&mut vm, "(progn (fset 'al-fn nil) (autoload 'al-fn \"al-fn\") (funcall (autoload-do-load (symbol-function 'al-fn) 'al-fn) 7))"), "(al-fn 7)");
    assert_eq!(shown(&mut vm, "(condition-case e (autoload-do-load '(autoload \"al-fn\") 42) (error e))"), "(wrong-type-argument symbolp 42)");
    assert_eq!(shown(&mut vm, "(condition-case e (funcall '(autoload \"al-fn\") 1) (error e))"), "(wrong-type-argument symbolp (autoload \"al-fn\"))");
}

#[test]
fn a_failed_autoload_restores_redefinitions_and_features_but_keeps_new_functions() {
    let (mut vm, _dir) = vm("failed");
    vm.eval_source("(defun al-existing () 'original) (defalias 'al-aliased (lambda () 'original)) (autoload 'al-bad \"al-bad\")").unwrap();
    assert_eq!(shown(&mut vm, "(condition-case e (al-bad) (error (car e)))"), "error");
    // GNU: ((autoload "al-bad" nil nil nil) t original original nil). The
    // cell is the autoload again, not the half-loaded definition; `defun'
    // and `defalias' redefinitions revert; a function with no previous
    // definition stays defined (data.c:984 records only non-nil).
    assert_eq!(
        shown(&mut vm, "(list (symbol-function 'al-bad) (fboundp 'al-helper-fresh) (al-existing) (al-aliased) (featurep 'al-bad))"),
        "((autoload \"al-bad\" nil nil nil) t original original nil)"
    );
    // The restored autoload is still live: calling it tries the load again.
    assert_eq!(shown(&mut vm, "(condition-case e (al-bad) (error (car e)))"), "error");
}

#[test]
fn an_autoload_whose_file_defines_nothing_or_is_missing_signals_and_stays() {
    let (mut vm, dir) = vm("nodef");
    vm.eval_source("(autoload 'al-nodef \"al-nodef\") (autoload 'al-missing \"al-no-such-file\")").unwrap();
    let path = std::fs::canonicalize(dir.join("al-nodef.el")).unwrap();
    assert_eq!(
        shown(&mut vm, "(condition-case e (al-nodef) (error e))"),
        format!("(error \"Autoloading file {} failed to define function al-nodef\")", path.display())
    );
    assert_eq!(shown(&mut vm, "(list (symbol-function 'al-nodef) (fboundp 'al-other))"), "((autoload \"al-nodef\" nil nil nil) t)");
    assert_eq!(
        shown(&mut vm, "(condition-case e (al-missing) (error e))"),
        "(file-missing \"Cannot open load file\" \"No such file or directory\" \"al-no-such-file\")"
    );
    assert_eq!(shown(&mut vm, "(symbol-function 'al-missing)"), "(autoload \"al-no-such-file\" nil nil nil)");
}

/// `defalias` stores through `Ffset` (data.c:997), which refuses a definition
/// whose symbol chain reaches SYMBOL (data.c:910-913) BEFORE writing the cell.
/// A `(macro . SYM)` definition is a cons, so it is not walked. In an autoload
/// file the signal fails the load, which is undone. GNU 32.0.50 answers.
#[test]
fn defalias_refuses_a_cyclic_alias_even_inside_an_autoload() {
    let (mut vm, _dir) = vm("cycle");
    assert_eq!(shown(&mut vm, "(progn (defalias 'al-c-a 'al-c-b) (defalias 'al-c-b 'al-c-a) 'accepted)"), "(signal (cyclic-function-indirection al-c-b))");
    assert_eq!(shown(&mut vm, "(list (symbol-function 'al-c-a) (symbol-function 'al-c-b))"), "(al-c-b nil)");
    assert_eq!(shown(&mut vm, "(defalias 'al-c-s 'al-c-s)"), "(signal (cyclic-function-indirection al-c-s))");
    assert_eq!(shown(&mut vm, "(progn (defalias 'al-c-x 'al-c-y) (defalias 'al-c-y 'al-c-z) (defalias 'al-c-z 'al-c-x))"), "(signal (cyclic-function-indirection al-c-z))");
    assert_eq!(shown(&mut vm, "(defalias 'al-c-mm '(macro . al-c-mm))"), "al-c-mm");
    vm.eval_source("(autoload 'al-cyc-m \"al-cyc\" nil nil 'macro)").unwrap();
    // GNU signals (cyclic-function-indirection al-cyc-target) out of the load;
    // VM2's loader drops a signal's condition and data (VM2-LOAD1), so only
    // "it signals" is compared here. GNU: signalled.
    assert_eq!(shown(&mut vm, "(condition-case nil (progn (autoload-do-load (symbol-function 'al-cyc-m) 'al-cyc-m) 'no-signal) (error 'signalled))"), "signalled");
    assert_eq!(shown(&mut vm, "(list (car-safe (symbol-function 'al-cyc-m)) (symbol-function 'al-cyc-target))"), "(autoload nil)");
}

/// A nested autoload that SUCCEEDS discards its own level's records when it
/// returns (unbind_to restores the caller's queue), so the caller's later
/// failure undoes the caller's redefinitions and keeps the inner file's.
#[test]
fn a_succeeding_nested_autoload_hands_back_the_callers_queue() {
    let (mut vm, _dir) = vm("nested-ok");
    vm.eval_source("(defun al-x1 () 'original) (defun al-x2 () 'original) (autoload 'al-out2 \"al-out2\") (autoload 'al-in2 \"al-in2\")").unwrap();
    assert_eq!(shown(&mut vm, "(condition-case e (al-out2) (error (car e)))"), "error");
    // GNU: (original inner-redefined nil autoload)
    assert_eq!(
        shown(&mut vm, "(list (al-x1) (al-x2) (car-safe (symbol-function 'al-in2)) (car-safe (symbol-function 'al-out2)))"),
        "(original inner-redefined nil autoload)"
    );
}

#[test]
fn a_nested_failed_autoload_is_undone_while_its_caller_succeeds() {
    let (mut vm, _dir) = vm("nested");
    vm.eval_source("(autoload 'al-bad \"al-bad\") (autoload 'al-outer \"al-outer\")").unwrap();
    assert_eq!(shown(&mut vm, "(al-outer)"), "(outer (caught error))");
    assert_eq!(shown(&mut vm, "(list (car-safe (symbol-function 'al-bad)) (car-safe (symbol-function 'al-outer)))"), "(autoload nil)");
}

#[test]
fn an_autoload_preserves_the_callers_match_data() {
    let (mut vm, _dir) = vm("match");
    vm.eval_source("(autoload 'al-match \"al-match\")").unwrap();
    assert_eq!(shown(&mut vm, "(progn (string-match \"c\" \"abcd\") (al-match) (match-data))"), "(2 3)");
}

#[test]
fn a_failed_require_restores_redefinitions_and_features() {
    let (mut vm, _dir) = vm("require");
    vm.eval_source("(defun al-existing () 'original)").unwrap();
    assert_eq!(shown(&mut vm, "(condition-case e (require 'al-req) (error (car e)))"), "error");
    assert_eq!(shown(&mut vm, "(list (al-existing) (featurep 'al-req))"), "(original nil)");
}

/// An autoload retains nothing once it returns: no undo queue, no FUNDEF, no
/// saved match data. Measured through `load_file` (eval_source keeps its
/// results live), after a forced collection, in live objects AND bytes: a
/// cycle that redefines through a successful autoload and runs a failing one
/// must leave the heap exactly where the previous identical cycle left it.
#[test]
fn an_autoload_retains_no_objects_or_bytes_after_collection() {
    let (mut vm, dir) = vm("retain");
    // A successful autoload, then later definitions: if the load left its
    // undo queue active, every later defun/provide would add records.
    std::fs::write(
        dir.join("ok-cycle.el"),
        "(fset 'al-fn nil) (setq features nil)\n(autoload 'al-fn \"al-fn\")\n(setq al-cycle (al-fn 1))\n(defun al-existing () 'original)\n(provide 'al-after)\n(string-match \"b\" \"abc\")\n",
    )
    .unwrap();
    // A failing autoload, undone.
    std::fs::write(
        dir.join("bad-cycle.el"),
        "(defun al-existing () 'original)\n(fset 'al-bad nil) (autoload 'al-bad \"al-bad\")\n(condition-case nil (al-bad) (error nil))\n",
    )
    .unwrap();
    let settle = |vm: &mut Vm, file: &str| {
        vm.load_file(&dir.join(file)).unwrap();
        vm.collect_garbage();
        let stats = vm.heap_stats();
        (stats.live_objects, stats.live_bytes)
    };
    for file in ["ok-cycle.el", "bad-cycle.el"] {
        settle(&mut vm, file);
        let base = settle(&mut vm, file);
        eprintln!("{file}: live_objects={} live_bytes={} after a forced collection", base.0, base.1);
        for round in 0..3 {
            assert_eq!(settle(&mut vm, file), base, "{file} round {round}: an autoload cycle retained heap state");
        }
    }
    assert_eq!(shown(&mut vm, "(list al-cycle (car (symbol-function 'al-bad)) (al-existing))"), "((al-fn 1) autoload original)");
}

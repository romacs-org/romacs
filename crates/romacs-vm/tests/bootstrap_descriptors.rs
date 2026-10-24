use romacs_vm::Vm;

#[test]
fn indirect_function_keeps_macro_descriptors() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defmacro boot-indirect (x) (list '+ x 2))
        (defalias 'boot-indirect-copy (indirect-function 'boot-indirect))
        (boot-indirect-copy 40)").unwrap();
    assert_eq!(value.display().unwrap(), "42");
}

#[test]
fn fset_keeps_macro_descriptors() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defmacro boot-fset (x) (list '+ x 2))
        (fset 'boot-fset-copy (symbol-function 'boot-fset))
        (condition-case e (boot-fset-copy 40) (error 'lost-macro))").unwrap();
    assert_eq!(value.display().unwrap(), "42");
    vm.collect_garbage();
    let value = vm.eval_source("(fset 'boot-fset-copy (lambda (x) (- x 2))) (boot-fset-copy 40)").unwrap();
    assert_eq!(value.display().unwrap(), "38");
}

#[test]
fn vectorp_recognizes_only_ordinary_vectors() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(list (vectorp []) (vectorp [1 nil]) (vectorp nil) (vectorp '(1)) (vectorp \"a\") (vectorp 1))").unwrap();
    assert_eq!(value.display().unwrap(), "(t t nil nil nil nil)");
}

#[test]
fn symbol_function_keeps_macro_aliases_expandable() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defmacro boot-macro (x) (list '+ x 2))
        (defalias 'boot-macro-copy (symbol-function 'boot-macro))
        (boot-macro-copy 40)").unwrap();
    assert_eq!(value.display().unwrap(), "42");
    let value = vm.eval_source("(car (symbol-function 'boot-macro))").unwrap();
    assert_eq!(value.display().unwrap(), "macro");
}

#[test]
fn symbol_function_reads_function_cells_without_following_aliases() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defalias 'boot-alias 'car)
        (list (symbol-function 'boot-alias) (symbol-function 'boot-undefined)
              (eq (symbol-function 'car) (symbol-function 'car)))").unwrap();
    assert_eq!(value.display().unwrap(), "(car nil t)");
}

#[test]
fn use_global_map_publishes_bindings() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(let ((map (make-sparse-keymap)))
        (define-key map [97] 'boot-command)
        (list (use-global-map map) (key-binding [97])))").unwrap();
    assert_eq!(value.display().unwrap(), "(nil boot-command)");
    let value = vm.eval_source("(let ((map (make-sparse-keymap)))
        (define-key map [98] 'boot-frozen-command)
        (defalias 'boot-map map) (use-global-map 'boot-map)
        (defalias 'boot-map (make-sparse-keymap)) (key-binding [98]))").unwrap();
    assert_eq!(value.display().unwrap(), "boot-frozen-command");
    vm.collect_garbage();
    let value = vm.eval_source("(key-binding [98])").unwrap();
    assert_eq!(value.display().unwrap(), "boot-frozen-command");
}

#[test]
fn make_symbol_preserves_name_and_fresh_identity() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(let ((a (make-symbol \"boot-fresh\")) (b (make-symbol \"boot-fresh\")))
        (list (equal (symbol-name a) \"boot-fresh\") (eq a b) (eq a 'boot-fresh) (boundp a) (fboundp a)))").unwrap();
    assert_eq!(value.display().unwrap(), "(t nil nil nil nil)");
}

#[test]
fn numberp_recognizes_numbers_without_coercing_other_values() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(list (numberp 1) (numberp 1.5) (numberp nil) (numberp \"1\") (numberp '(1)))").unwrap();
    assert_eq!(value.display().unwrap(), "(t t nil nil nil)");
}

#[test]
fn macroexp_dynamic_context_starts_bound_and_empty() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(list (boundp 'macroexp--dynvars) macroexp--dynvars)").unwrap();
    assert_eq!(value.display().unwrap(), "(t nil)");
}

#[test]
fn listp_accepts_nil_and_cons_including_improper_lists() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(list (listp nil) (listp '(1)) (listp '(1 . 2)) (listp []) (listp 1))").unwrap();
    assert_eq!(value.display().unwrap(), "(t t t nil nil)");
}

#[test]
fn byte_run_loads_with_existing_t3_prelude() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    vm.load_file(&root.join("gnu-lisp/emacs-lisp/byte-run.el")).unwrap();
}

#[test]
fn eval_and_compile_runs_source_body_once() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("gnu-lisp/emacs-lisp/byte-run.el")).unwrap();
    let value = vm.eval_source("(defvar bootstrap-count 0) (eval-and-compile (setq bootstrap-count (1+ bootstrap-count))) bootstrap-count").unwrap();
    assert_eq!(value.display().unwrap(), "1");
}

#[test]
// GNU's own byte-run.el (restored to the pin in Phase 0): `eval-and-compile`
// evaluates BODY while it is EXPANDED and returns the value quoted
// (lisp/emacs-lisp/byte-run.el, `(list 'quote (eval (cons 'progn body) ...))`).
// GNU 32.0.50: `(macroexpand-all '(eval-and-compile (setq bt (1+ bt))))` with bt
// 0 gives `'1`, bt is 1 after expansion, evaluating the expansion gives 1 and bt
// stays 1 (transcript in reference/lisp-oracle/VM-GAPS.md, VM2-BOOT1 withdrawn).
// This used to assert the opposite (deferral to execution), which held only
// because lisp/emacs-lisp/byte-run.el had been edited in place.
fn eval_and_compile_evaluates_its_body_at_expansion_as_gnu_does() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("gnu-lisp/emacs-lisp/byte-run.el")).unwrap();
    vm.eval_source("(defvar bootstrap-timing 0)").unwrap();
    let id = vm.compile_source("(eval-and-compile (setq bootstrap-timing (1+ bootstrap-timing)))").unwrap();
    let before = vm.eval_source("bootstrap-timing").unwrap();
    assert_eq!(before.as_integer().unwrap(), 1, "GNU runs the body during expansion");
    let value = vm.execute(id).unwrap();
    assert_eq!(value.as_integer().unwrap(), 1);
    let after = vm.eval_source("bootstrap-timing").unwrap();
    assert_eq!(after.as_integer().unwrap(), 1, "GNU's expansion is a quoted value: no second run");
}

#[test]
// RATCHET (Phase 0), not a goal. GNU's `buffer-local-restore-state`
// (lisp/subr.el:256) destructures with `pcase-dolist`, which GNU autoloads from
// pcase.el. This test used to pass against an in-place rewrite of subr.el
// (plain `dolist` + `car-safe`, ledger row VM2-BOOT2, withdrawn). GNU 32.0.50
// answers `(nil 9 (nil))` and `nil` for the two calls below (transcript in
// reference/lisp-oracle/VM-GAPS.md, gap G4). VM2 cannot reach pcase yet:
// subr.el stops at form 139 (radix reader, G1), so pcase-dolist is unbound,
// and `eval_source` (which compiles the body eagerly, unlike GNU's lazy
// `defun`) already signals on the definition. Measured 2026-10-20 on the
// restored tree: `(void-function pcase-dolist)` after head-order repair. When this fails because the call now
// SUCCEEDS, assert GNU's answers above instead; never relax the pin.
fn restore_state_needs_pcase_dolist_which_vm2_cannot_reach_yet() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join("gnu-lisp/subr.el")).unwrap();
    let definition = source.split("(defun buffer-local-restore-state").nth(1).unwrap()
        .split("(defmacro push").next().unwrap();
    assert!(definition.contains("(pcase-dolist (`(,variable ,local ,value) states)"),
        "gnu-lisp/subr.el is GNU's again; this pins GNU's definition: {definition}");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    // `.map(|_| ())` ends the result's borrow of `vm` (master's heap boundary:
    // a returned value borrows the VM) before the next request.
    let defined = vm.eval_source(&format!("(defun buffer-local-restore-state{definition}")).map(|_| ());
    let called = defined.and_then(|_| vm.eval_source("(defvar restore-a 5) (defvar restore-b 6)
        (defvar restore-kills nil)
        (defun kill-local-variable (symbol) (setq restore-kills (cons symbol restore-kills)))
        (buffer-local-restore-state '((restore-a t . tail) atom (restore-b t 9 extra)))
        (list restore-a restore-b restore-kills)").map(|v| v.display().unwrap()));
    assert_eq!(called, Err("(void-function pcase-dolist)".to_owned()),
        "frontier moved: if this is GNU's (nil 9 (nil)), turn the ratchet into the GNU assertion");
}

#[test]
// RATCHET, not a goal. `subr.el` does not load completely yet, so asserting
// that it does would encode a TODO as a permanent failure and carry no
// regression information. This pins the EXACT measured frontier instead, so it
// fails if the bootstrap regresses AND if it advances -- either way forcing a
// re-measurement rather than a silent drift. Update the numbers WITH a fresh
// measurement when the frontier moves; never relax them to quiet a failure.
// Re-measured after dense make-keymap and GNU subr.el restoration: unchanged,
// 188 subr forms complete, form 189 (line 2236) faults on defvaralias.
// Re-measured after the defvaralias primitive (eval.c:631): 188 -> 225 subr
// forms complete (243 -> 280 total), form 226 (line 2309) faults on autoload.
// Re-measured after the autoload primitive (eval.c:2411): 225 -> 227 subr
// forms complete (280 -> 282 total). Form 228 (line 2315, add-hook) fails to
// COMPILE: VM2's native setf has no `get'/`alist-get' place. In GNU those
// places come from gv.el (Lisp), so this frontier is not a C primitive.
// Re-measured after lazy function bodies (R215, GNU's non-eager regime before
// macroexp.el): 227 -> 284 subr forms complete (282 -> 339 total). add-hook is
// now defined unexpanded and CALLED by subr.el's own top-level forms, exactly
// as in GNU, without reaching its guarded `setf' forms. Form 285 (line 3556,
// a top-level add-hook call) faults on default-boundp, a C primitive
// (data.c:2006).
// Re-measured after the default-boundp primitive (data.c:2006): unchanged,
// 284 subr forms complete (339 total); the same form 285 (line 3556, add-hook)
// now passes default-boundp and faults on default-value (data.c:2019).
// Re-measured after the default-value primitive (data.c:2019): unchanged,
// 284 subr forms complete (339 total); form 285 (line 3556) now faults on
// functionp (eval.c:3087).
// Re-measured after the functionp primitive (eval.c:3087): unchanged, 284
// subr forms complete (339 total); form 285 (line 3556) now faults on
// set-default (data.c:2130).
// Re-measured after the set-default primitive (data.c:2130): 284 -> 302 subr
// forms complete (339 -> 357 total); add-hook's top-level calls now run.
// Form 303 (line 3929, (defvar read-char-from-minibuffer-map ...)) faults on
// (void-variable minibuffer-local-map): a variable GNU defines and fills in
// C (DEFVAR_LISP plus keymap initialisation), not a C primitive (DEFUN).
// Re-measured after forwarded built-in variables (R216, forwarded.rs):
// unchanged, 302 subr forms complete (357 total); the same form 303 now
// reads minibuffer-local-map and faults on set-keymap-parent (keymap.c:273).
// Re-measured after keymap-parent and set-keymap-parent (keymap.c:253/273):
// 302 -> 489 subr forms complete (357 -> 544 total). Form 490 (line 7625,
// (defvar package--builtin-versions ...)) reads the built-in emacs-version
// and faults inside version-to-list's save-match-data on set-match-data
// (search.c:2949).
// Re-measured after set-match-data (search.c:2949), string-to-number
// (data.c:3092), and read-from-string (lread.c:2511): 489 -> 496 subr
// forms complete (544 -> 551 total). Form 497, line 7701, now reaches
// the real reader and fails on the unsupported #s(hash-table ...) reader
// substrate. Do not replace GNU's hash-table readability probe with a shim.
// Re-measured after `#s(hash-table ...)` read syntax (lread.c:3153, :3852;
// vm2/hashread): 496 -> 523 subr forms complete (551 -> 578 total), i.e.
// ALL 523 top-level forms of this tree's subr.el (GNU's reader counts 523
// in it); `load` returns normally. The frontier pin therefore becomes a
// completion pin: it fails if any subr form stops completing, and if the
// form count changes, forcing a re-measurement either way.
// Goal tracked as NOW.md S1, not as a red test.
// GNU radix syntax now passes form 139 and use-global-map form 140.
// Remeasured after the reader repair: 140 subr forms, 196 including byte-run;
// the next real failure is form 141, missing logior (line 1801).
// The earlier 523-form completion figure came from edited Lisp, not this pin.
// logior (data.c:3506) advances 140 -> 522/522; add-hook is now defined.
// Sequential macroexp and pcase then complete; files needs custom's defgroup.
fn subr_cold_frontier_is_pinned() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    vm.load_file(&root.join("gnu-lisp/emacs-lisp/byte-run.el")).unwrap();
    // The returned `Local` borrows `vm`; drop it before `load_report`.
    let outcome = vm.load_file(&root.join("gnu-lisp/subr.el")).map(drop);
    let report = vm.load_report();
    let subr = root.join("gnu-lisp/subr.el").canonicalize().unwrap();
    let subr = subr.to_string_lossy();

    // Count first, so that a frontier move in EITHER direction (including subr.el
    // loading completely again) fails at the count, the assertion this pin exists for.
    let completed = report["steps"].as_array().unwrap().iter()
        .filter(|s| s["file"] == subr.as_ref() && s["status"] == "ok")
        .count();
    let failure = match &outcome {
        Ok(_) => "subr.el LOADED COMPLETELY".to_owned(),
        Err(e) => e.to_string(),
    };
    eprintln!("subr frontier measured: completed={completed}, total={}, failure={failure}", report["completed_forms"]);
    assert_eq!(completed, 522, "subr frontier moved; re-measure, do not edit blindly: {failure}");
    assert!(outcome.is_ok(), "{failure}");
    assert_eq!(report["completed_forms"], 578, "{failure}");
    assert_eq!(vm.eval_source("(fboundp 'add-hook)").unwrap().display().unwrap(), "t", "SUBR_ADD_HOOK_MISSING");
    for (relative, expected) in [("gnu-lisp/emacs-lisp/macroexp.el", 50), ("gnu-lisp/emacs-lisp/pcase.el", 53), ("gnu-lisp/files.el", 49)] {
        let path = root.join(relative);
        let result = vm.load_file(&path).map(drop);
        let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let report = vm.load_report();
        let completed = report["steps"].as_array().unwrap().iter().filter(|s| s["file"] == canonical && s["status"] == "ok").count();
        eprintln!("BOOTSTRAP_CHAIN {relative}: completed={completed}; result={result:?}");
        assert_eq!(completed, expected, "BOOTSTRAP_CHAIN_MOVED {relative}: {result:?}");
        if relative == "gnu-lisp/files.el" {
            let error = result.unwrap_err();
            assert!(error.contains("line 494 form 50:") && error.ends_with("(void-function bound-and-true-p)"), "FILES_MACRO_FRONTIER_MOVED {error}");
            // Native defgroup is a compiler form, not a callable function cell.
            assert_eq!(vm.eval_source("(fboundp 'defgroup)").unwrap().display().unwrap(), "nil", "FILES_DEFGROUP_ATTRIBUTION_CHANGED");
        } else { assert!(result.is_ok(), "{result:?}"); }
    }
}

#[test]
fn defalias_macro_descriptor_expands_and_redefinition_removes_macro_status() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defalias 'bootstrap-m (cons 'macro (lambda (x) (list '+ x 2)))) (bootstrap-m 40)").unwrap();
    assert_eq!(value.display().unwrap(), "42");
    let value = vm.eval_source("(defalias 'bootstrap-m (lambda (x) (- x 2))) (bootstrap-m 40)").unwrap();
    assert_eq!(value.display().unwrap(), "38");
}

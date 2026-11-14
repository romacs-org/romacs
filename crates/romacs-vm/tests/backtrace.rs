use romacs_vm::Vm;

fn compare(name: &str, source: &str) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let oracle = format!(
        "(progn (setq lexical-binding t) (defalias 'bt-compile #'byte-compile) (prin1 {source}))"
    );
    let output = std::process::Command::new(binary)
        .args(["-Q", "--batch", "--eval", &oracle])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(defun bt-compile (function) function)")
        .unwrap();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{name}: {source}");
}

#[test]
fn compiled_call_identity_arguments_and_bases_match_gnu() {
    let cases = [
        (
            "entry_args",
            r#"(progn
          (fset 'bt-outer (bt-compile (lambda (first &optional second &rest tail)
            (setq first 99 second 88 tail '(77))
            (let (seen)
              (mapbacktrace (lambda (evaluated function arguments flags)
                (if (memq function '(bt-outer mapbacktrace))
                    (setq seen (cons (list evaluated function arguments flags) seen)))))
              (nreverse seen)))))
          (let ((result (bt-outer 1 2 3 4)))
            (list (car (car result)) (car (cdr (car result)))
                  (car (cdr result)))))"#,
        ),
        (
            "alias_base_offset",
            r#"(progn
          (fset 'bt-outer (bt-compile (lambda ()
            (let (seen)
              (mapbacktrace (lambda (e f a flags)
                (setq seen (cons f seen)) (if (eq f 'bt-alias) (throw 'bt-done (nreverse seen))))
                '(1 . bt-outer))))))
          (fset 'bt-inner (bt-compile (lambda () (catch 'bt-done (bt-outer)))))
          (defalias 'bt-alias 'bt-inner)
          (bt-alias))"#,
        ),
        (
            "missing_base",
            "(let ((called nil)) (list (mapbacktrace (lambda (e f a flags) (setq called t)) 'bt-absent) called))",
        ),
        (
            "callback_arity",
            "(condition-case e (mapbacktrace (lambda (e f a flags) (list e f a flags)) 'mapbacktrace) (error (car e)))",
        ),
        (
            "locals_errors",
            "(list (condition-case e (backtrace--locals 0) (error (car e))) (condition-case e (backtrace--locals -1) (error (car e))) (condition-case e (backtrace--locals 'wrong) (error (car e))) (condition-case e (backtrace--locals 9999) (error (car e))))",
        ),
        (
            "argument_object_mutation",
            r#"(progn
          (fset 'bt-outer (bt-compile (lambda (value)
            (setcar value 'changed) (setq value 'replaced)
            (catch 'bt-done
              (mapbacktrace (lambda (e f a flags) (throw 'bt-done a)) 'bt-outer)))))
          (bt-outer (list 'original)))"#,
        ),
    ];
    for (name, source) in cases {
        compare(name, source);
    }
}

#[test]
fn dynamic_locals_are_read_without_rewinding_bindings() {
    compare(
        "nested_dynamic",
        r#"(progn
      (defvar bt-special 'outside)
      (fset 'bt-query (bt-compile (lambda ()
        (list (backtrace--locals 1 'bt-query) (backtrace--locals 2 'bt-query) bt-special))))
      (fset 'bt-inner (bt-compile (lambda () (let ((bt-special 'inner)) (bt-query)))))
      (fset 'bt-outer (bt-compile (lambda () (let ((bt-special 'outer)) (bt-inner)))))
      (bt-outer))"#,
    );
    compare(
        "alias_dynamic",
        r#"(progn
      (defvar bt-special 'outside) (defvar bt-alias) (defvaralias 'bt-alias 'bt-special)
      (fset 'bt-query (bt-compile (lambda () (backtrace--locals 1 'bt-query))))
      (fset 'bt-outer (bt-compile (lambda () (let ((bt-alias 'inside)) (bt-query)))))
      (bt-outer))"#,
    );
}

#[test]
fn handler_callback_base_retains_failing_body_and_native_error() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = r#"(let ((debugfun nil) (frames nil))
      (fset 'bt-failure (lambda (payload) (apply 'apply '(error ("failure")))))
      (setq debugfun (lambda (condition)
        (garbage-collect)
        (mapbacktrace (lambda (e f a flags)
          (garbage-collect) (setq frames (cons (list f a) frames))) debugfun)))
      (condition-case e
        (handler-bind-1 (lambda () (bt-failure 42)) 'error debugfun)
        (error nil))
      (setq frames (nreverse frames))
      (list (eq (car (car frames)) debugfun)
            (car (cdr (assq 'bt-failure frames)))
            (car (cdr (assq 'error frames)))))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(t (42) (\"failure\"))"
    );
}

#[test]
fn t2_primitive_origin_and_callback_are_real_frames() {
    compare(
        "t2_origin",
        r#"(progn
      (fset 'bt-callback (bt-compile (lambda (value)
        (let (seen)
          (mapbacktrace (lambda (evaluated function arguments flags)
            (if (memq function '(bt-callback mapcan))
                (setq seen (cons (list evaluated function arguments flags) seen)))))
          (list (nreverse seen))))))
      (mapcan 'bt-callback '(42)))"#,
    );
}

/// A leaf runs with no activation, and has one the moment it fails: what a
/// signal hook sees is GNU's frame, through a compiled call, `funcall' and
/// `apply'.
#[test]
fn a_failing_leaf_shows_its_frame() {
    compare(
        "failing_leaf",
        r#"(progn
      (defvar bt-seen nil)
      (fset 'bt-record (bt-compile (lambda (&rest _)
        (let (found)
          (mapbacktrace (lambda (evaluated function arguments flags)
            (if (memq function '(symbol-name bt-leaf bt-through bt-spread))
                (setq found (cons (list evaluated function arguments flags) found)))))
          (setq bt-seen (cons (nreverse found) bt-seen))))))
      (fset 'bt-leaf (bt-compile (lambda (x) (symbol-name x))))
      (fset 'bt-through (bt-compile (lambda (x) (funcall 'symbol-name x))))
      (fset 'bt-spread (bt-compile (lambda (x) (apply 'symbol-name x))))
      (fset 'bt-drive (bt-compile (lambda (f x)
        (condition-case nil
            (let ((signal-hook-function 'bt-record)) (funcall f x))
          (error nil)))))
      (bt-drive 'bt-leaf 1)
      (bt-drive 'bt-through 1)
      (bt-drive 'bt-spread '(1))
      (list bt-seen (length (bt-leaf 'ok)) (length (bt-through 'ok)) (length (bt-spread '(ok)))))"#,
    );
}

/// A primitive that reads the stack sees its own frame, and a reader's
/// callback sees `read'.
#[test]
fn stack_readers_and_reader_callbacks_keep_their_frames() {
    compare(
        "stack_readers",
        r#"(progn
      (defvar bt-chars (append "(a)" nil))
      (defvar bt-seen nil)
      (fset 'bt-frame (bt-compile (lambda ()
        (let (found)
          (mapbacktrace (lambda (evaluated function arguments flags)
            (if (memq function '(read bt-stream bt-frame))
                (setq found (cons (list evaluated function) found)))))
          found))))
      (fset 'bt-stream (bt-compile (lambda (&optional unread)
        (if unread
            (setq bt-chars (cons unread bt-chars))
          (or bt-seen (setq bt-seen (bt-frame)))
          (prog1 (car bt-chars) (setq bt-chars (cdr bt-chars)))))))
      (list (read 'bt-stream) bt-seen))"#,
    );
}

#[test]
fn eval_and_load_reentry_retain_parent_activations() {
    compare(
        "eval_parent",
        r#"(progn
      (fset 'bt-child (bt-compile (lambda ()
        (let (seen)
          (catch 'bt-frame-end
            (mapbacktrace (lambda (e f a flags)
              (if (memq f '(bt-child eval bt-parent)) (setq seen (cons f seen)))
              (if (eq f 'bt-parent) (throw 'bt-frame-end nil)))) )
          (nreverse seen)))))
      (fset 'bt-parent (bt-compile (lambda () (eval '(bt-child)))))
      (bt-parent))"#,
    );
    let path = std::env::temp_dir().join(format!("backtrace-load-{}.el", std::process::id()));
    std::fs::write(&path, "(setq bt-loaded (bt-child))").unwrap();
    let source = format!(
        r#"(progn
      (defvar bt-loaded nil)
      (fset 'bt-child (bt-compile (lambda ()
        (let (seen)
          (catch 'bt-frame-end
            (mapbacktrace (lambda (e f a flags)
              (if (memq f '(bt-child load bt-parent)) (setq seen (cons f seen)))
              (if (eq f 'bt-parent) (throw 'bt-frame-end nil)))) )
          (nreverse seen)))))
      (fset 'bt-parent (bt-compile (lambda () (load {:?} nil t t) bt-loaded)))
      (bt-parent))"#,
        path.to_str().unwrap()
    );
    compare("load_parent", &source);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn buffer_local_dynamic_values_use_each_bindings_storage() {
    compare(
        "distinct_buffer_bindings",
        r#"(progn
      (fset 'bt-query (bt-compile (lambda ()
        (list (backtrace--locals 1 'bt-query) (backtrace--locals 2 'bt-query) fill-column))))
      (fset 'bt-inner (bt-compile (lambda () (let ((fill-column 82)) (bt-query)))))
      (fset 'bt-outer (bt-compile (lambda ()
        (let ((fill-column 81)) (set-buffer " *bt-buffer-two*") (bt-inner)))))
      (let ((one (get-buffer-create " *bt-buffer-one*"))
            (two (get-buffer-create " *bt-buffer-two*")))
        (unwind-protect
          (progn (set-buffer two) (make-local-variable 'fill-column) (setq fill-column 72)
                 (set-buffer one) (make-local-variable 'fill-column) (setq fill-column 71)
                 (bt-outer))
          (kill-buffer one) (kill-buffer two))))"#,
    );
    compare(
        "ordinary_buffer_local",
        r#"(progn
      (defvar bt-special 'outside)
      (fset 'bt-query (bt-compile (lambda ()
        (list (backtrace--locals 1 'bt-query) (backtrace--locals 2 'bt-query) bt-special))))
      (fset 'bt-inner (bt-compile (lambda () (let ((bt-special 'inner)) (bt-query)))))
      (fset 'bt-outer (bt-compile (lambda () (let ((bt-special 'outer)) (bt-inner)))))
      (with-temp-buffer (make-local-variable 'bt-special) (setq bt-special 'local) (bt-outer)))"#,
    );
    compare(
        "forwarded_global",
        r#"(progn
      (fset 'bt-query (bt-compile (lambda ()
        (list (backtrace--locals 1 'bt-query) (backtrace--locals 2 'bt-query) max-lisp-eval-depth))))
      (fset 'bt-inner (bt-compile (lambda () (let ((max-lisp-eval-depth 900002)) (bt-query)))))
      (fset 'bt-outer (bt-compile (lambda () (let ((max-lisp-eval-depth 900001)) (bt-inner)))))
      (bt-outer))"#,
    );
}

#[test]
fn collection_roots_original_arguments_and_nested_iteration() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = r#"(progn
      (fset 'bt-hold (lambda (argument)
        (setq argument nil)
        (garbage-collect)
        (let ((seen nil) (nested nil))
          (mapbacktrace (lambda (e f a flags)
            (garbage-collect)
            (if (eq f 'bt-hold)
              (progn
                (setq seen a)
                (mapbacktrace (lambda (e2 f2 a2 flags2)
                  (garbage-collect)
                  (if (eq f2 'bt-hold) (setq nested a2))) 'bt-hold)))) 'bt-hold)
          (list seen nested))))
      (bt-hold (list 42)))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(((42)) ((42)))"
    );
    assert!(vm.heap_stats().collections > 0);
    assert!(vm.heap_stats().reclaimed > 0);
}

#[test]
fn callback_throws_and_errors_remove_all_escaped_activations() {
    compare(
        "nonlocal_cleanup",
        r#"(progn
      (fset 'bt-throw (bt-compile (lambda ()
        (mapbacktrace (lambda (e f a flags) (throw 'bt-done 'thrown))))))
      (fset 'bt-error (bt-compile (lambda ()
        (mapbacktrace (lambda (e f a flags) (error "callback"))))))
      (fset 'bt-run (bt-compile (lambda ()
        (let ((first (catch 'bt-done (bt-throw)))
              (second (condition-case e (bt-error) (error (car e))))
              (leaked nil))
          (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'bt-throw)
          (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'bt-error)
          (list first second leaked)))))
      (bt-run))"#,
    );
}

#[test]
fn review_regressions_match_pinned_compiled_gnu() {
    compare(
        "compiled_apply_nested",
        r#"(progn (fset 'bt-hold (bt-compile (lambda (x) (let ((out nil)) (mapbacktrace (lambda (e f a flags) (if (memq f '(bt-hold bt-run)) (setq out (cons (list f a) out))))) (nreverse out))))) (fset 'bt-run (bt-compile (lambda () (apply 'apply '(bt-hold (42)))))) (bt-run))"#,
    );
    compare(
        "compiled_apply_primitive",
        r#"(progn (fset 'bt-run (bt-compile (lambda () (let ((out nil)) (condition-case e (handler-bind-1 (lambda () (apply 'apply '(error ("failure")))) 'error (lambda (e) (mapbacktrace (lambda (e f a flags) (if (memq f '(error bt-run)) (setq out (cons (list f a) out))))))) (error (nreverse out))))))) (bt-run))"#,
    );
    compare(
        "nil_cons_base",
        r#"(let ((seen nil)) (mapbacktrace (lambda (e f a flags) (setq seen t)) '(0)) seen)"#,
    );
    compare(
        "kill_local",
        r#"(progn (fset 'bt-query (bt-compile (lambda () (backtrace--locals 1 'bt-query)))) (fset 'bt-hold (bt-compile (lambda () (let ((fill-column 82)) (kill-local-variable 'fill-column) (bt-query))))) (with-temp-buffer (make-local-variable 'fill-column) (bt-hold)))"#,
    );
    compare(
        "kill_buffer_local",
        r#"(progn (fset 'bt-query (bt-compile (lambda () (backtrace--locals 1 'bt-query)))) (fset 'bt-hold (bt-compile (lambda () (let ((fill-column 82)) (kill-buffer (current-buffer)) (bt-query))))) (with-temp-buffer (make-local-variable 'fill-column) (bt-hold)))"#,
    );
    compare(
        "kill_ordinary_local",
        r#"(progn (defvar bt-local 'default) (fset 'bt-query (bt-compile (lambda () (backtrace--locals 1 'bt-query)))) (fset 'bt-hold (bt-compile (lambda () (let ((bt-local 82)) (kill-local-variable 'bt-local) (bt-query))))) (with-temp-buffer (make-local-variable 'bt-local) (bt-hold)))"#,
    );
    compare(
        "killed_ordinary_owner",
        r#"(progn
      (defvar bt-local 'default)
      (fset 'bt-query (bt-compile (lambda () (backtrace--locals 1 'bt-query))))
      (fset 'bt-hold (bt-compile (lambda () (let ((bt-local 82)) (kill-buffer (current-buffer)) (bt-query)))))
      (with-temp-buffer (make-local-variable 'bt-local) (bt-hold)))"#,
    );
    for base in ["nil", "'(0)", "'(1)", "'(-1)"] {
        compare(
            base,
            &format!(
                "(let ((seen nil)) (mapbacktrace (lambda (e f a flags) (setq seen t)) {base}) seen)"
            ),
        );
        compare(
            base,
            &format!(
                "(progn (fset 'bt-run (bt-compile (lambda () (condition-case e (backtrace--locals 1 {base}) (error (car e)))))) (bt-run))"
            ),
        );
    }
}

#[test]
fn dynamic_apply_preserves_real_operation_and_final_callee_identity() {
    compare(
        "dynamic_apply",
        r#"(progn
      (fset 'bt-hold (bt-compile (lambda (x)
        (let ((out nil))
          (mapbacktrace (lambda (e f a flags)
            (if (memq f '(bt-hold apply bt-run)) (setq out (cons f out)))))
          (nreverse out)))))
      (fset 'bt-run (bt-compile (lambda (f args) (apply f args))))
      (bt-run 'bt-hold '(42)))"#,
    );
    let mut vm = Vm::new();
    // GNU can compile constant apply away. Romacs retains the operations it
    // executes and snapshots their original evaluated arguments.
    let source = r#"(progn
      (fset 'bt-hold (lambda (x)
        (let ((out nil))
          (mapbacktrace (lambda (e f a flags)
            (if (memq f '(bt-hold apply bt-run)) (setq out (cons (list f a) out)))))
          (nreverse out))))
      (fset 'bt-run (lambda () (apply 'apply '(bt-hold (42)))))
      (bt-run))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "((bt-hold (42)) (apply (bt-hold (42))) (apply (apply (bt-hold (42)))) (bt-run nil))"
    );
    compare(
        "nested_apply_base",
        r#"(progn
      (fset 'bt-hold (bt-compile (lambda (x)
        (catch 'bt-done
          (mapbacktrace (lambda (e f a flags) (throw 'bt-done (list f a))) 'bt-hold)))))
      (apply 'apply '(bt-hold (42))))"#,
    );
}

#[test]
fn apply_chains_from_t2_and_nonlocal_exits_release_their_frames() {
    compare(
        "t2_apply_chain",
        r#"(progn
      (fset 'bt-hold (bt-compile (lambda (x)
        (let ((out nil))
          (mapbacktrace (lambda (e f a flags)
            (if (memq f '(bt-hold apply mapcan)) (setq out (cons f out)))))
          (list (nreverse out))))))
      (fset 'bt-spread (bt-compile (lambda (call) (apply (car call) (car (cdr call))))))
      (mapcan 'bt-spread '((bt-hold (42)))))"#,
    );
    compare(
        "apply_chain_cleanup",
        r#"(progn
      (fset 'bt-escape (bt-compile (lambda () (error "stop"))))
      (fset 'bt-run (bt-compile (lambda ()
        (let ((caught (condition-case e (apply 'apply '(bt-escape nil)) (error (car e))))
              (leaked nil))
          (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'bt-escape)
          (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'apply)
          (list caught leaked)))))
      (bt-run))"#,
    );
}

#[test]
fn refused_closure_calls_retain_attempted_frame_until_escape_cleanup() {
    for (name, parameters, call) in [
        ("ordinary_arity", "argument", "(bt-arity)"),
        ("t2_arity", "", "(assoc 2 '((1 . value)) 'bt-arity)"),
    ] {
        let source = format!(
            r#"(progn
          (defvar bt-entered nil) (defvar bt-body-local 'outside)
          (defvar bt-out nil) (defvar bt-base nil) (defvar bt-locals 'unset)
          (defvar bt-condition nil) (defvar bt-count nil)
          (fset 'bt-arity (bt-compile (lambda ({parameters})
            (let ((bt-body-local 'unentered)) (setq bt-entered t)))))
          (fset 'bt-debug (bt-compile (lambda (condition)
            (garbage-collect)
            (setq bt-locals (backtrace--locals 1 'bt-debug))
            (mapbacktrace (lambda (e f a flags)
              (if (memq f '(bt-arity bt-run)) (setq bt-out (cons (list f a) bt-out)))))
            (setq bt-base (catch 'bt-frame
              (mapbacktrace (lambda (e f a flags) (throw 'bt-frame (list f a))) 'bt-arity))))))
          (fset 'bt-run (bt-compile (lambda ()
            (condition-case e
              (handler-bind-1 (lambda () {call}) 'error 'bt-debug)
              (error (setq bt-condition (car e) bt-count (car (cdr (cdr e))))))
            (let ((leaked nil))
              (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'bt-arity)
              (list bt-condition bt-count (nreverse bt-out) bt-base bt-locals bt-entered leaked)))))
          (bt-run))"#
        );
        compare(name, &source);
    }
}

fn compare_autoload_attempt(case: usize, failing: bool) {
    let callback = case == 2;
    let path = std::env::temp_dir().join(format!(
        "backtrace-autoload-{case}-{failing}-{}.el",
        std::process::id()
    ));
    let body = if failing {
        "(unwind-protect (signal 'arith-error '(autoload-error)) (setq bt-cleaned t))".to_owned()
    } else {
        let result = if callback { "(list x)" } else { "x" };
        format!("(bt-snapshot) (defun bt-auto (x) (setq bt-entered t) (bt-snapshot) {result})")
    };
    std::fs::write(&path, body).unwrap();
    let call = match case {
        0 => "(funcall 'bt-auto 7)",
        1 => "(apply 'bt-auto '(7))",
        2 => "(mapcan 'bt-auto '(7))",
        _ => "(bt-auto 7)",
    };
    // Named calls load at ResolveHead before their arguments are evaluated.
    // Verify their error/cleanup behavior without inventing an evaluated frame.
    let report = if case == 3 {
        "(list result bt-count bt-cleaned-at-handler bt-cleaned bt-entered leaked)"
    } else {
        "(list result (nreverse bt-before) (nreverse bt-after) bt-count bt-cleaned-at-handler bt-cleaned bt-entered leaked)"
    };
    let source = format!(
        r#"(progn
      (defvar bt-before nil) (defvar bt-after nil) (defvar bt-entered nil)
      (defvar bt-cleaned nil) (defvar bt-cleaned-at-handler 'unset)
      (defvar bt-count 0) (defvar bt-seen nil)
      (fset 'bt-snapshot (bt-compile (lambda ()
        (mapbacktrace (lambda (e f a flags)
          (if (memq f '(bt-auto bt-run))
            (if bt-entered (setq bt-after (cons (list f a) bt-after))
              (setq bt-before (cons (list f a) bt-before))))) 'bt-auto))))
      (autoload 'bt-auto {:?} nil nil)
      (fset 'bt-run (bt-compile (lambda ()
        (let ((result
          (condition-case e
            (handler-bind-1 (lambda () {call}) 'error
              (lambda (condition)
                (setq bt-count (1+ bt-count) bt-cleaned-at-handler bt-cleaned)
                (if (null bt-seen) (setq bt-seen condition))
                (garbage-collect) (bt-snapshot)))
            (error (list (car e) (cdr e) (eq e bt-seen))))))
          (let ((leaked nil))
            (mapbacktrace (lambda (e f a flags) (setq leaked t)) 'bt-auto)
            {report})))))
      (bt-run))"#,
        path.to_str().unwrap()
    );
    compare(
        if failing {
            "autoload_failure"
        } else {
            "autoload_success"
        },
        &source,
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn autoload_success_exposes_one_pending_activation_before_definition() {
    for case in 0..3 {
        compare_autoload_attempt(case, false);
    }
}

#[test]
fn autoload_error_preserves_one_original_escape_and_cleanup() {
    for case in 0..4 {
        compare_autoload_attempt(case, true);
    }
}

// The backtrace shows a call's arguments as the caller passed them (GNU's
// `record_in_backtrace' keeps a pointer to the caller's pushed values): not
// a parameter the callee assigns or boxes for a capturing closure; the same
// across a nested `eval' (the caller's registers set aside), a recursion
// deep enough to widen the registers, `apply' (whose spread window a nested
// `apply' reuses) and `&rest' with extra arguments.
#[test]
fn arguments_are_shown_as_passed_from_the_callers_registers() {
    compare(
        "args-as-passed",
        r#"(progn
  (defun bt-args (name) (let (out) (mapbacktrace (lambda (_e f a _flags) (if (eq f name) (setq out (cons a out)))) name) out))
  (fset 'bt-setq (bt-compile (lambda (x) (setq x 9) (bt-args 'bt-setq))))
  (fset 'bt-cell (bt-compile (lambda (x) (let ((c (lambda () (setq x 9)))) (funcall c) (bt-args 'bt-cell)))))
  (fset 'bt-inner (bt-compile (lambda () (bt-args 'bt-outer))))
  (fset 'bt-outer (bt-compile (lambda (x) (eval '(bt-inner) t))))
  (fset 'bt-deep (bt-compile (lambda (n) (if (= n 0) (length (bt-args 'bt-deep)) (bt-deep (1- n))))))
  (fset 'bt-b (bt-compile (lambda (&rest r) (bt-args 'bt-a))))
  (fset 'bt-a (bt-compile (lambda (x y) (apply 'bt-b (list x 7 8 9)))))
  (fset 'bt-rest (bt-compile (lambda (x &rest r) (setq r nil) (bt-args 'bt-rest))))
  (list (bt-setq 1) (bt-cell 1) (bt-outer 5) (bt-deep 300) (apply 'bt-a '(1 2)) (bt-rest 1 2 3) (funcall 'bt-rest 1)))"#,
    );
}

// `backtrace-debug' sets a frame's debug-on-exit flag, which `mapbacktrace'
// passes as FLAGS (`(:debug-on-exit t)'); the flag is cleared before the
// frame returns, so GNU calls no debugger. A frame beyond the stack is no
// error, as get_backtrace_frame's nil.
#[test]
fn backtrace_debug_flags_match_gnu() {
    compare(
        "debug-flags",
        r#"(progn
  (defun bt-flags (name) (let (out) (mapbacktrace (lambda (_e f _a flags) (if (eq f name) (setq out (cons flags out)))) name) out))
  (fset 'bt-flagged (bt-compile (lambda ()
    (list (backtrace-debug 0 t 'bt-flagged) (bt-flags 'bt-flagged)
          (backtrace-debug 0 nil 'bt-flagged) (bt-flags 'bt-flagged)
          (backtrace-debug 999 t 'bt-flagged)
          (condition-case e (backtrace-debug -1 t) (error (car e)))))))
  (bt-flagged))"#,
    );
}

// `backtrace-eval' sees the dynamic environment at the frame's entry (its
// own bindings undone, eval.c backtrace_eval_unrewind), lands a `setq' in
// the binding that frame saw, and swaps everything back however EXP exits.
#[test]
fn backtrace_eval_bindings_match_gnu() {
    compare(
        "eval-bindings",
        r#"(progn
  (defvar bt-dyn 'outer)
  (fset 'bt-inner (bt-compile (lambda ()
    (list (backtrace-eval 'bt-dyn 0 'bt-inner) (backtrace-eval 'bt-dyn 1 'bt-inner) (backtrace-eval 'bt-dyn 2 'bt-inner)
          (catch 'bt-out (backtrace-eval '(throw 'bt-out bt-dyn) 2 'bt-inner))
          (condition-case e (backtrace-eval '(car bt-dyn) 1 'bt-inner) (error (car e)))
          (condition-case e (backtrace-eval 'bt-dyn 99 'bt-inner) (error e))
          (backtrace-eval '(setq bt-dyn 'changed) 1 'bt-inner)
          bt-dyn))))
  (fset 'bt-mid (bt-compile (lambda () (let ((bt-dyn 'mid)) (bt-inner)))))
  (fset 'bt-top (bt-compile (lambda () (let ((bt-dyn 'top)) (list (bt-mid) bt-dyn)))))
  (list (bt-top) bt-dyn))"#,
    );
}

// The current buffer and `save-excursion' are part of the unrewind: an
// excursion begun in a caller before the call stays for the callee's entry
// (the inner frame sees B at 5), one begun inside the frame is undone (the
// outer frame sees A at 1), and B's point is 5 again afterwards.
#[test]
fn backtrace_eval_buffers_and_excursions_match_gnu() {
    compare(
        "eval-excursions",
        r#"(let ((a (get-buffer-create " bt-a")) (b (get-buffer-create " bt-b")))
  (with-current-buffer a (erase-buffer) (insert "aaaaaaaaaa") (goto-char 1))
  (with-current-buffer b (erase-buffer) (insert "bbbbbbbbbb") (goto-char 1))
  (fset 'bt-ex-inner (bt-compile (lambda ()
    (let ((here '(list (buffer-name) (point))))
      (list (backtrace-eval here 0 'bt-ex-inner) (backtrace-eval here 1 'bt-ex-inner)
            (backtrace-eval '(progn (goto-char 3) (buffer-name)) 1 'bt-ex-inner)
            (eval here t))))))
  (fset 'bt-ex-outer (bt-compile (lambda () (save-excursion (set-buffer b) (goto-char 5) (bt-ex-inner)))))
  (fset 'bt-cb-inner (bt-compile (lambda () (list (backtrace-eval '(buffer-name) 1 'bt-cb-inner) (buffer-name)))))
  (fset 'bt-cb-outer (bt-compile (lambda () (with-current-buffer b (bt-cb-inner)))))
  (with-current-buffer a
    (list (bt-ex-outer) (buffer-name) (point) (with-current-buffer b (point)) (bt-cb-outer))))"#,
    );
}

// `backtrace--frames-and-locals' is `mapbacktrace' plus `backtrace--locals'
// for every frame but the first, from one walk; `backtrace-get-frames'
// (lisp/romacs/romacs-backtrace.el) is built on it.
#[test]
fn frames_and_locals_equal_the_per_frame_primitives() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = r#"(progn
  (defvar bt-v1 nil) (defvar bt-v2 nil)
  (fset 'bt-leaf (lambda ()
    (let ((bulk (backtrace--frames-and-locals 'bt-leaf)) (each nil) (i 0))
      (mapbacktrace (lambda (e f a flags)
                      (setq each (cons (list e f a flags (if (= i 0) nil (backtrace--locals i 'bt-leaf))) each)
                            i (1+ i)))
                    'bt-leaf)
      (list (equal bulk (nreverse each)) (nth 4 (nth 1 bulk)) (length bulk)))))
  (fset 'bt-binder (lambda (n) (let ((bt-v1 n) (bt-v2 (* n 2))) (if (> n 0) (bt-binder (1- n)) (bt-leaf)))))
  (bt-binder 3))"#;
    let value = vm.eval_source(source).and_then(|v| v.display());
    let value = value.unwrap();
    assert!(value.starts_with("(t ((bt-v1 . 0) (bt-v2 . 0)) "), "{value}");
}

// A buffer's local binding is swapped while the buffer has one, even one
// made again after the `let' (GNU `local-variable-p'); a binding whose local
// is gone becomes GNU's SPECPDL_NOP, so a local made during the evaluation
// stays, and the `let''s end leaves it too. Ordinary and forwarded
// (`fill-column') locals.
#[test]
fn backtrace_eval_buffer_locals_match_gnu() {
    compare(
        "eval-locals",
        r#"(progn
  (defvar bt-loc 'global)
  (fset 'bt-re-inner (bt-compile (lambda ()
    (kill-local-variable 'bt-loc) (set (make-local-variable 'bt-loc) 'recreated)
    (list (backtrace-eval 'bt-loc 1 'bt-re-inner) bt-loc))))
  (fset 'bt-re-outer (bt-compile (lambda () (let ((bt-loc 'entry)) (bt-re-inner)))))
  (fset 'bt-gone-inner (bt-compile (lambda (var value)
    (kill-local-variable var)
    (backtrace-eval (list 'set (list 'make-local-variable (list 'quote var)) value) 1 'bt-gone-inner)
    (list (symbol-value var) (local-variable-p var)))))
  (fset 'bt-gone-outer (bt-compile (lambda (var value) (eval (list 'let (list (list var 82)) (list 'bt-gone-inner (list 'quote var) value)) t))))
  (list (with-temp-buffer (set (make-local-variable 'bt-loc) 'before) (list (bt-re-outer) bt-loc))
        (with-temp-buffer (set (make-local-variable 'bt-loc) 71) (list (bt-gone-outer 'bt-loc 93) bt-loc (local-variable-p 'bt-loc)))
        (with-temp-buffer (set (make-local-variable 'fill-column) 71) (list (bt-gone-outer 'fill-column 93) fill-column (local-variable-p 'fill-column)))))"#,
    );
}

// An excursion's swap restores the saved window's point when another
// window (here the minibuffer's) is selected and it still shows the buffer
// (GNU save_excursion_restore), from the point as restored: clipped to a
// narrowing made inside the excursion.
#[test]
fn backtrace_eval_excursion_window_point_matches_gnu() {
    compare(
        "eval-window-point",
        r#"(progn
  (defvar bt-w1 nil) (defvar bt-buf nil)
  (setq bt-buf (get-buffer-create " bt-w") bt-w1 (selected-window))
  (with-current-buffer bt-buf (erase-buffer) (insert "0123456789"))
  (set-window-buffer bt-w1 bt-buf)
  (set-buffer bt-buf) (goto-char 2)
  (let ((w2 (minibuffer-window)))
    (fset 'bt-w-inner (bt-compile (lambda ()
      (list (backtrace-eval '(list (buffer-name) (point) (window-point bt-w1)) 1 'bt-w-inner)
            (buffer-name) (window-point bt-w1)))))
    (fset 'bt-w-outer (bt-compile (lambda (w2) (save-excursion (goto-char 5) (select-window w2) (bt-w-inner)))))
    (fset 'bt-n-outer (bt-compile (lambda (w2) (save-excursion (narrow-to-region 4 9) (select-window w2) (bt-w-inner)))))
    (list (prog1 (bt-w-outer w2) (select-window bt-w1))
          (prog1 (bt-n-outer w2) (select-window bt-w1) (with-current-buffer bt-buf (widen))))))"#,
    );
}

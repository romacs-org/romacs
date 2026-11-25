//! O7m C2 places in-memory fixtures across two independent VMs.
#[macro_use]
#[path = "support/o7m_codec.rs"]
mod support;

use romacs_vm::Vm;
use std::path::Path;

use self::support::{configure_store, eval, gnu_lisp};

const TABLE: &str = "'(:incarnation (123 . 1) :world \"o7m-test-world\" :files nil :fbound nil :autoloads nil :features nil)";

fn vm() -> Vm {
    let mut vm = Vm::new();
    install_fixture_lisp(&mut vm);
    vm
}

fn install_fixture_lisp(vm: &mut Vm) {
    vm.install_lisp_prelude().unwrap();
    eval(
        vm,
        "(defun caar (x) (car (car x))) (defun cadr (x) (car (cdr x)))
              (defalias 'function-documentation 'romacs--closure-document)",
    );
}

fn encode(vm: &mut Vm, roots: &str, limit: usize) -> Vec<u8> {
    let form = format!("(romacs--seed-build '{roots} {TABLE} {limit})");
    let request = vm.compile_source(&form).unwrap();
    // The counter includes newly compiled request code, so sample after compilation.
    let before = vm.image_functions_materialized();
    let (backing, refusals) = vm.execute(request).unwrap().cons_pair().unwrap();
    assert!(refusals.is_nil(), "{}", refusals.display().unwrap());
    let bytes = backing.seed_backing_bytes().unwrap();
    assert_eq!(vm.image_functions_materialized(), before);
    assert!(bytes.starts_with(b"VM2FRZ02"));
    bytes
}

fn place(receiver: &mut Vm, bytes: &[u8]) {
    let seed_root = receiver.place_seed(bytes).unwrap().root();
    receiver
        .declare_special_rooted("o7m-decoded", &seed_root)
        .unwrap();
    eval(
        receiver,
        r"
      (setq o7m-root (nth 1 (assq 'o7m-root (mapcar 'cdr (nth 2 o7m-decoded))))
            o7m-decoded nil)",
    );
}

fn copy_root(source: &mut Vm, receiver: &mut Vm) {
    place(receiver, &encode(source, "((variable o7m-root))", 100_000));
}

fn body_kind(vm: &mut Vm, expr: &str) -> &'static str {
    vm.eval_source(expr).unwrap().function_body_kind().unwrap()
}

// S1. The same-VM freeze path remains closed to direct code, including
// tag-6 subrs, even when the unsupported value is deeper than the root.
#[test]
fn same_vm_freeze_refuses_code() {
    let mut vm = vm();
    assert_eq!(
        eval(
            &mut vm,
            r#"
        (let ((closure (lambda () 'callable))
              (subr (symbol-function 'car)))
          (list (subrp subr)
                (condition-case err (freeze closure)
                  (error (and (equal (nth 1 err) "Cannot freeze")
                              (eq closure (nth 2 err)))))
                (condition-case err (freeze (list (vector subr)))
                  (error (and (equal (nth 1 err) "Cannot freeze")
                              (eq subr (nth 2 err))
                              (consp (nth 3 err)))))
                (frozen-p (freeze '(car)))))"#
        ),
        "(t t t t)"
    );
}

// R5, T5 and T10. A dynamic late cell shadows an outer lexical one. Another
// late cell has no shadow and must look up the receiver's dynamic value.
// The third is still lexical. The first call alone runs the macro.
#[test]
fn lazy_receive_gc_inspect_first_call_late_shadow() {
    in_private_store!(lazy_receive_gc_inspect_first_call_late_shadow, |dir| {
        let (mut source, mut receiver) = (vm(), vm());
        make_lazy_shadow(&mut source, dir);
        for vm in [&mut source, &mut receiver] {
            eval(
                vm,
                "(setq o7m-expansions 0) (defmacro o7m-expand (x) (setq o7m-expansions (1+ o7m-expansions)) x)",
            );
        }
        eval(
            &mut receiver,
            "(defvar o7m-forward 72) (defvar o7m-shadow 73)",
        );
        let inspect = "(list (aref o7m-root 1) (func-arity o7m-root) (documentation o7m-root t) (interactive-form o7m-root) (commandp o7m-root) o7m-expansions)";
        let expected = eval(&mut source, inspect);
        assert!(
            expected.ends_with(" (1 . 1) \"O7m doc.\" (interactive \"p\") t 0)"),
            "{expected}"
        );
        let features = eval(&mut source, "features");
        assert_eq!(body_kind(&mut source, "o7m-root"), "lazy");
        copy_root(&mut source, &mut receiver);
        assert_eq!(body_kind(&mut source, "o7m-root"), "lazy");
        assert_eq!(eval(&mut source, inspect), expected);
        assert_eq!(eval(&mut source, "features"), features);
        receiver.collect_compacting();
        assert_eq!(body_kind(&mut receiver, "o7m-root"), "lazy");
        assert_eq!(eval(&mut receiver, inspect), expected);
        assert_eq!(
            eval(
                &mut receiver,
                "(list (funcall o7m-root 5) (funcall o7m-root 6) o7m-expansions)"
            ),
            "((5 7 72 11) (6 7 72 11) 1)"
        );
    });
}

fn make_lazy_shadow(source: &mut Vm, path: &Path) {
    let file = path.join("lazy-shadow.el");
    std::fs::write(
        &file,
        r#";; -*- lexical-binding: t; -*-
        (setq o7m-root
          (let ((o7m-shadow 7))
            (eval '(defvar o7m-shadow 41) nil)
            (eval '(defvar o7m-forward 42) nil)
            (let ((o7m-shadow 9) (o7m-forward 10) (o7m-unforward 11))
              (lambda (x)
                "O7m doc."
                (interactive "p")
                (list (o7m-expand x) o7m-shadow o7m-forward o7m-unforward)))))
        "#,
    )
    .unwrap();
    eval(source, "(fmakunbound 'internal-macroexpand-for-load)");
    source.load_file(&file).unwrap();
}

const COMMAND: &str = ";; -*- lexical-binding: t; -*-\n\
    (defun o7m-command (x &optional y) \"O7m command.\" (interactive \"p\") (list x y (o7m-callee (vector \"literal\"))))\n";

// The legacy image publisher accepts string and cons literals only.
const IMAGE_COMMAND: &str = r#"(defun o7m-command (x &optional y)
    "O7m mapped command."
    (list x y (o7m-callee (vector "literal"))))"#;

fn command(vm: &mut Vm) {
    eval(
        vm,
        "(defun o7m-callee (x) x) (setq o7m-root (symbol-function 'o7m-command))",
    );
}

fn assert_eager_copy(source: &mut Vm, kind: &str) {
    let mut receiver = vm();
    eval(
        &mut receiver,
        "(defun o7m-foreign (x) (o7m-other x)) (defun o7m-callee (x) x)",
    );
    assert_eq!(body_kind(source, "o7m-root"), kind);
    let info = "(list (func-arity o7m-root) (documentation o7m-root t) (interactive-form o7m-root) (commandp o7m-root))";
    let expected = match kind {
        "eager" => r#"((1 . 2) "O7m command." (interactive "p") t)"#,
        "pending" => r#"((1 . 2) "O7m command.\n\n(fn X &optional Y)" (interactive "p") t)"#,
        "mapped" => r#"((1 . 2) "O7m mapped command." nil nil)"#,
        _ => panic!("unexpected sender body kind: {kind}"),
    };
    assert_eq!(eval(source, info), expected);
    copy_root(source, &mut receiver);
    assert_eq!(
        body_kind(source, "o7m-root"),
        kind,
        "sender representation changed"
    );
    receiver.collect_compacting();
    assert_eq!(body_kind(&mut receiver, "o7m-root"), "pending");
    assert_eq!(eval(&mut receiver, info), expected);
    assert_eq!(
        eval(&mut receiver, "(funcall o7m-root 3)"),
        r#"(3 nil ["literal"])"#
    );
    assert_eq!(body_kind(&mut receiver, "o7m-root"), "eager");
    eval(
        &mut receiver,
        "(fset 'o7m-callee (lambda (x) (cons 'new x)))",
    );
    assert_eq!(
        eval(&mut receiver, "(funcall o7m-root 3 4)"),
        r#"(3 4 (new . ["literal"]))"#
    );
}

// R5 and T5. Eager, pending artifact and cold image bodies all arrive
// pending. An uncalled lazy closure is checked separately above.
#[test]
fn pending_only_for_eager() {
    in_private_store!(pending_only_for_eager, |dir| {
        let mut eager = vm();
        eval(&mut eager, COMMAND);
        command(&mut eager);
        assert_eq!(
            eval(&mut eager, "(funcall o7m-root 3)"),
            r#"(3 nil ["literal"])"#
        );
        assert_eager_copy(&mut eager, "eager");

        let mut compiler = gnu_lisp(&[]);
        eval(&mut compiler, "(require 'romacs-compile)");
        configure_store(&dir.join("rlc"));
        let path = dir.join("pending.el");
        std::fs::write(&path, COMMAND).unwrap();
        compiler.compile_lisp_file(&path).unwrap();
        let mut pending = gnu_lisp(&[]);
        let name = serde_json::to_string(&path.with_extension("")).unwrap();
        eval(&mut pending, &format!("(load {name} nil t)"));
        command(&mut pending);
        assert_eager_copy(&mut pending, "pending");

        let image_dir = dir.join("image");
        let mut image_source = Vm::new();
        eval(&mut image_source, IMAGE_COMMAND);
        command(&mut image_source);
        assert_eq!(body_kind(&mut image_source, "o7m-root"), "eager");
        let entry = image_source.compile_source("nil").unwrap();
        let key = image_source
            .publish_code_image(&image_dir, entry, [0; 32])
            .unwrap();
        let image = romacs_vm::ImageRegion::open(&image_dir, &key).unwrap();
        let (mut mapped, _) = Vm::from_code_image(image, [0; 32]).unwrap();
        install_fixture_lisp(&mut mapped);
        assert_eager_copy(&mut mapped, "mapped");
    });
}

// T5. Neither the lazy macro autoload nor a mutating failed expansion
// runs during seed build or receive. Both belong to the receiver's first call.
#[test]
fn lazy_macro_autoload_and_failed_expansion_stay_in_receiver() {
    in_private_store!(
        lazy_macro_autoload_and_failed_expansion_stay_in_receiver,
        |dir| {
            let macro_file = dir.join("o7m-macro.el");
            std::fs::write(
                &macro_file,
                r#"
            (setq o7m-loads (1+ o7m-loads))
            (defmacro o7m-late-macro ()
              (setq o7m-expands (1+ o7m-expands))
              (error "o7m expansion failed"))
            (provide 'o7m-macro)"#,
            )
            .unwrap();
            let (mut source, mut receiver) = (vm(), vm());
            let filename = serde_json::to_string(&macro_file).unwrap();
            let setup = format!(
                "(setq o7m-loads 0 o7m-expands 0) (autoload 'o7m-late-macro {filename} nil nil 'macro)"
            );
            for vm in [&mut source, &mut receiver] {
                eval(vm, &setup);
            }
            eval(
                &mut source,
                "(setq o7m-root (eval '(lambda () (o7m-late-macro)) t))",
            );
            let state = "(list o7m-loads (> o7m-expands 0) (featurep 'o7m-macro))";
            assert_eq!(body_kind(&mut source, "o7m-root"), "lazy");
            copy_root(&mut source, &mut receiver);
            assert_eq!(eval(&mut source, state), "(0 nil nil)");
            assert_eq!(eval(&mut receiver, state), "(0 nil nil)");
            assert_eq!(
                eval(
                    &mut receiver,
                    "(condition-case err (funcall o7m-root) (error (cadr err)))"
                ),
                "\"o7m expansion failed\""
            );
            assert_eq!(eval(&mut receiver, state), "(1 t t)");
            assert_eq!(eval(&mut source, state), "(0 nil nil)");
            assert_eq!(body_kind(&mut source, "o7m-root"), "lazy");
        }
    );
}

// S7, T12. Embed an actual live object in eager code, not a variable
// referring to it. Probe the retained edge before asking the seed walk.
#[test]
fn constant_live_object_refused() {
    let mut vm = vm();
    eval(
        &mut vm,
        r"
       (setq o7m-root (eval (list 'lambda nil (list 'quote (vector (current-buffer)))) t))",
    );
    assert_eq!(
        eval(&mut vm, "(eq (aref (funcall o7m-root) 0) (current-buffer))"),
        "t"
    );
    assert_eq!(body_kind(&mut vm, "o7m-root"), "eager");
    let refusal = eval(
        &mut vm,
        &format!("(cdr (romacs--seed-build '((variable o7m-root)) {TABLE} 100000))"),
    );
    assert!(
        refusal.starts_with("((variable o7m-root cannot-freeze buffer "),
        "{refusal}"
    );
    assert!(refusal.contains("(constant "), "{refusal}");
}

// S7, T2. Refuse three different live handle types at the capture edge
// and roll their entries back while keeping a later legal closure.
#[test]
fn capture_handle_refusal_keeps_good_entry() {
    let (mut source, mut receiver) = (vm(), vm());
    eval(
        &mut source,
        r#"
       (setq o7m-process (make-pipe-process :name "o7m-pipe" :noquery t)
             o7m-buffer-root (let ((x (current-buffer))) (lambda () x))
             o7m-marker-root (let ((x (point-marker))) (lambda () x))
             o7m-process-root (let ((x o7m-process)) (lambda () x))
             o7m-root (let ((x 17)) (lambda () x)))"#,
    );
    let result = format!(
        "(romacs--seed-build '((variable o7m-buffer-root) (variable o7m-marker-root) (variable o7m-process-root) (variable o7m-root)) {TABLE} 100000)"
    );
    let (seed, refusals) = source.eval_source(&result).unwrap().cons_pair().unwrap();
    let refusals = refusals.display().unwrap();
    for (name, kind) in [
        ("buffer", "buffer"),
        ("marker", "marker"),
        ("process", "process"),
    ] {
        assert!(
            refusals.contains(&format!(
                "variable o7m-{name}-root cannot-freeze {kind} ((capture 0)"
            )),
            "{refusals}"
        );
    }
    place(&mut receiver, &seed.seed_backing_bytes().unwrap());
    assert_eq!(eval(&mut receiver, "(funcall o7m-root)"), "17");
    eval(&mut source, "(delete-process o7m-process)");
}

// R4. Each refusal fits in the budget alone. Together their work crosses
// the limit during discovery and even a cheap trailing root gets too-large.
#[test]
fn one_budget_repeated_refusals() {
    let mut vm = vm();
    eval(
        &mut vm,
        r"
      (defun o7m-expensive ()
        (let ((v (make-vector 201 nil)))
          (dotimes (i 200) (aset v i (cons i i)))
          (aset v 200 (current-buffer))
          v))
      (setq o7m-a (o7m-expensive) o7m-b (o7m-expensive) o7m-c 6)",
    );
    let build = |vm: &mut Vm, roots, limit| {
        eval(
            vm,
            &format!("(cdr (romacs--seed-build '{roots} {TABLE} {limit}))"),
        )
    };
    let (mut low, mut high) = (0, 65_536);
    while high - low > 1 {
        let middle = i32::midpoint(low, high);
        if build(&mut vm, "((variable o7m-a))", middle).contains("cannot-freeze") {
            high = middle;
        } else {
            low = middle;
        }
    }
    let limit = high + 16;
    assert!(build(&mut vm, "((variable o7m-a))", limit).contains("cannot-freeze"));
    assert!(build(&mut vm, "((variable o7m-b))", limit).contains("cannot-freeze"));
    assert_eq!(build(&mut vm, "((variable o7m-c))", limit), "nil");
    let combined = build(
        &mut vm,
        "((variable o7m-a) (variable o7m-b) (variable o7m-c))",
        limit,
    );
    for (name, reason) in [
        ("a", "cannot-freeze"),
        ("b", "too-large"),
        ("c", "too-large"),
    ] {
        assert!(
            combined.contains(&format!("(variable o7m-{name} {reason} ")),
            "{combined}"
        );
    }
}

// S1, T1 and T7. Retain a closure cycle, a duplicate reference, a shared
// cell and its exposed lexical pair. A second placement starts afresh.
#[test]
fn cycle_and_duplicate_closure_refs() {
    let (mut source, mut first, mut second) = (vm(), vm(), vm());
    eval(
        &mut source,
        r"
      (setq o7m-root
        (let ((n 0) (cycle (list nil)))
          (let* ((inc (lambda () (setq n (1+ n))))
                 (read-n (lambda () n))
                 (read-cycle (lambda () cycle))
                 (pair (assq 'n (aref inc 2))))
            (setcar cycle read-cycle)
            (vector inc read-n read-cycle inc pair))))",
    );
    let bytes = encode(&mut source, "((variable o7m-root))", 100_000);
    place(&mut first, &bytes);
    place(&mut second, &bytes);
    first.collect_compacting();
    assert_eq!(
        eval(
            &mut first,
            r"
       (let ((inc (aref o7m-root 0)) (read-n (aref o7m-root 1))
             (cycle (aref o7m-root 2)) (pair (aref o7m-root 4)))
         (list (frozen-p o7m-root) (eq inc (aref o7m-root 3))
               (eq cycle (car (funcall cycle))) (eq pair (assq 'n (aref read-n 2)))
               (funcall inc) (funcall read-n)
               (progn (setcdr pair 14) (funcall read-n))))"
        ),
        "(nil t t t 1 1 14)"
    );
    let read = "(funcall (aref o7m-root 1))";
    assert_eq!(eval(&mut second, read), "0");
    assert_eq!(eval(&mut source, read), "0");
    place(&mut first, &bytes);
    assert_eq!(eval(&mut first, read), "0");
}

// T8, T10 and T12. Subrs are receiver-local, the uninterned capture name
// is shared inside one copy, and one legal constant is shared by code and another graph edge.
#[test]
fn subrs_uninterned_capture_and_shared_constants_round_trip() {
    let (mut source, mut receiver) = (vm(), vm());
    eval(
        &mut source,
        r#"
        (setq o7m-name (make-symbol "o7m-private")
              o7m-body (vector 'literal))
        (setq o7m-root
          (let ((code (list 'lambda nil (list 'quote o7m-body))))
            (vector (make-interpreted-closure
                       nil (list o7m-name) (list (cons o7m-name 41)))
                    o7m-name
                    (let ((a (symbol-function 'car)) (b (symbol-function 'string-equal)))
                      (lambda () (list a b)))
                    (eval code t) nil)))"#,
    );
    // Avoid a lazy source-form copy in this particular constant identity test.
    eval(&mut source, "(aset o7m-root 4 (funcall (aref o7m-root 3)))");
    assert_eq!(
        eval(&mut source, "(mapcar 'subrp (funcall (aref o7m-root 2)))"),
        "(t t)"
    );
    let bytes = encode(&mut source, "((variable o7m-root))", 100_000);
    place(&mut receiver, &bytes);
    receiver.collect_compacting();
    assert_eq!(
        eval(
            &mut receiver,
            r#"
      (let* ((f (aref o7m-root 0)) (name (aref o7m-root 1)) (subrs (funcall (aref o7m-root 2))))
        (list (funcall f) (eq name (caar (aref f 2))) (eq name (intern "o7m-private"))
              (eq (car subrs) (symbol-function 'car)) (funcall (car subrs) '(7 8))
              (eq (cadr subrs) (symbol-function 'string-equal))
              (eq (funcall (aref o7m-root 3)) (aref o7m-root 4))))"#
        ),
        "(41 t nil t 7 t t)"
    );
    let mut unknown = bytes;
    let name = b"string-equal";
    let mut replaced = false;
    for offset in (0..=unknown.len() - name.len()).rev() {
        if &unknown[offset..offset + name.len()] == name {
            unknown[offset..offset + name.len()].copy_from_slice(b"o7m-missing!");
            replaced = true;
        }
    }
    assert!(replaced, "subr name absent from wire data");
    let error = receiver
        .place_seed(&unknown)
        .expect_err("unknown subr was accepted");
    assert!(error.contains("unknown-subr"), "{error}");
}

// T3. Both sides load the class library. A mutable slot was already
// exposed through an accessor in the sender and remains mutable in the copy.
#[test]
fn exposed_oclosure_slot_round_trips() {
    in_private_store!(exposed_oclosure_slot_round_trips, |dir| {
        let (mut source, mut receiver) = (gnu_lisp(&[]), gnu_lisp(&[]));
        let file = dir.join("o7m-class.el");
        std::fs::write(
            &file,
            ";; -*- lexical-binding: t; -*-\n(oclosure-define o7m-probe (slot :mutable t))\n",
        )
        .unwrap();
        for vm in [&mut source, &mut receiver] {
            eval(vm, "(require 'oclosure)");
            vm.load_file(&file).unwrap();
        }
        eval(
            &mut source,
            "(setq o7m-root (oclosure-lambda (o7m-probe (slot 7)) () slot))",
        );
        assert_eq!(eval(&mut source, "(o7m-probe--slot o7m-root)"), "7");
        copy_root(&mut source, &mut receiver);
        receiver.collect_compacting();
        assert_eq!(
            eval(
                &mut receiver,
                r"
          (list (oclosure-type o7m-root) (o7m-probe--slot o7m-root)
                (progn (setf (o7m-probe--slot o7m-root) 19) (funcall o7m-root)))"
            ),
            "(o7m-probe 7 19)"
        );
        assert_eq!(eval(&mut source, "(funcall o7m-root)"), "7");
    });
}

// T4. Both registration orders make a realized wrapper. Keep the
// advice closure's identity with the variable that contains the wrapper.
#[test]
fn realized_advice_round_trips_before_and_after_library_load() {
    in_private_store!(
        realized_advice_round_trips_before_and_after_library_load,
        |dir| {
            let file = dir.join("o7m-lib.el");
            std::fs::write(
                &file,
                ";; -*- lexical-binding: t; -*-\n(defun o7m-target (x) x)\n",
            )
            .unwrap();
            for before_load in [true, false] {
                let (mut source, mut receiver) = (gnu_lisp(&[]), gnu_lisp(&[]));
                if !before_load {
                    source.load_file(&file).unwrap();
                }
                eval(
                    &mut source,
                    r"
              (require 'nadvice)
              (defun o7m-around (old x) (1+ (funcall old x)))
              (advice-add 'o7m-target :around (symbol-function 'o7m-around))",
                );
                if before_load {
                    source.load_file(&file).unwrap();
                }
                eval(
                    &mut source,
                    "(setq o7m-root (vector (symbol-function 'o7m-target) (symbol-function 'o7m-around)))",
                );
                assert_eq!(eval(&mut source, "(o7m-target 1)"), "2");
                eval(&mut receiver, "(require 'nadvice)");
                copy_root(&mut source, &mut receiver);
                receiver.collect_compacting();
                assert_eq!(
                    eval(
                        &mut receiver,
                        r"
              (progn
                (fset 'o7m-target (aref o7m-root 0))
                (list (and (advice-member-p (aref o7m-root 1) 'o7m-target) t)
                      (o7m-target 1)
                      (progn (advice-remove 'o7m-target (aref o7m-root 1)) (o7m-target 1))))"
                    ),
                    "(t 2 1)"
                );
                assert_eq!(eval(&mut source, "(o7m-target 1)"), "2");
            }
        }
    );
}

// T7. Both init function cells are reached through one root, and direct
// calls recur through the receiver's symbol cells with its cache indices.
#[test]
fn mutually_recursive_init_functions_round_trip() {
    let (mut source, mut receiver) = (vm(), vm());
    eval(
        &mut source,
        r"
      (defun o7m-even (n) (if (= n 0) t (o7m-odd (1- n))))
      (defun o7m-odd (n) (if (= n 0) nil (o7m-even (1- n))))
      (setq o7m-root (list 'o7m-even))",
    );
    let bytes = encode(&mut source, "((variable o7m-root))", 100_000);
    let seed_root = receiver.place_seed(&bytes).unwrap().root();
    receiver
        .declare_special_rooted("o7m-decoded", &seed_root)
        .unwrap();
    assert_eq!(
        eval(
            &mut receiver,
            r"
      (progn
        (dolist (entry (nth 2 o7m-decoded))
          (when (eq (car entry) 'function) (fset (nth 1 entry) (nth 2 entry))))
        (list (o7m-even 10) (o7m-odd 10) (o7m-even 11) (o7m-odd 11)))"
        ),
        "(t nil nil t)"
    );
}

fn variable_refusal(vm: &mut Vm, root: &str, limit: usize) -> String {
    eval(
        vm,
        &format!("(nth 2 (car (cdr (romacs--seed-build '((variable {root})) {TABLE} {limit}))))"),
    )
}

// Sol S3. Every edge of a wide vector costs work, even repeated fixnums.
#[test]
fn review_vector_elements_consume_seed_budget() {
    let mut vm = vm();
    eval(&mut vm, "(setq o7m-wide (make-vector 8 23)) nil");
    assert_eq!(variable_refusal(&mut vm, "o7m-wide", 128), "nil");
    eval(&mut vm, "(setq o7m-wide (make-vector 4096 23)) nil");
    assert_eq!(variable_refusal(&mut vm, "o7m-wide", 32_768), "nil");
    assert_eq!(variable_refusal(&mut vm, "o7m-wide", 128), "too-large");
}

// Sol S3. A cyclic root spine must produce a refusal within a small budget.
// Limit the child before giving it the cycle so a stalled walk cannot exhaust
// the test host's memory.
#[test]
fn review_circular_roots_refused_within_budget() {
    const CHILD: &str = "ROMACS_O7M_CIRCULAR_ROOTS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert_circular_roots_refused();
        return;
    }
    let name = support::test_name(
        module_path!(),
        "review_circular_roots_refused_within_budget",
    );
    let output = std::process::Command::new("/usr/bin/timeout")
        .args([
            "--kill-after=1s",
            "5s",
            "/usr/bin/prlimit",
            "--as=536870912",
            "--",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
    assert!(
        output.status.success(),
        "cyclic roots did not produce a bounded refusal: {}\n{stdout}\n{stderr}",
        output.status
    );
}

fn assert_circular_roots_refused() {
    let mut vm = vm();
    eval(
        &mut vm,
        "(setq o7m-value 17 o7m-roots (list '(variable o7m-value))) nil",
    );
    assert_eq!(variable_refusal(&mut vm, "o7m-value", 64), "nil");
    eval(&mut vm, "(setcdr o7m-roots o7m-roots) nil");
    let actual = vm
        .eval_source(&format!(
            "(nth 2 (car (cdr (romacs--seed-build o7m-roots {TABLE} 64))))"
        ))
        .and_then(romacs_vm::Local::display);
    match actual {
        Ok(reason) => assert_eq!(reason, "too-large"),
        Err(error) => assert!(
            ["cycle", "circular", "too-large", "budget", "limit", "listp"]
                .iter()
                .any(|reason| error.contains(reason)),
            "{error}"
        ),
    }
}

// Opus S1. The two eager definitions mention the same symbols and differ
// mainly in opcode count. A successful large-budget build controls the fixture.
#[test]
fn review_eager_opcodes_consume_seed_budget() {
    let mut vm = vm();
    eval(
        &mut vm,
        r"(defvar o7m-x 0)
           (eval '(defun o7m-small () (setq o7m-x (1+ o7m-x))) t)
           (eval (cons 'defun (cons 'o7m-big
                       (cons nil (make-list 20000 '(setq o7m-x (1+ o7m-x)))))) t)
           (o7m-big) (o7m-small)",
    );
    for symbol in ["o7m-small", "o7m-big"] {
        assert_eq!(
            body_kind(&mut vm, &format!("(symbol-function '{symbol})")),
            "eager"
        );
    }
    let refusal = |vm: &mut Vm, name, limit| {
        eval(
            vm,
            &format!(
                "(nth 2 (car (cdr (romacs--seed-build '((function {name})) {TABLE} {limit}))))"
            ),
        )
    };
    assert_eq!(refusal(&mut vm, "o7m-small", 256), "nil");
    assert_eq!(refusal(&mut vm, "o7m-big", 1_000_000), "nil");
    assert_eq!(refusal(&mut vm, "o7m-big", 256), "too-large");
}

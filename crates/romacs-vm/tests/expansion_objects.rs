//! GNU's evaluator works on Lisp objects, not on printed source: a macro may
//! expand to a form carrying any object (a closure, a buffer), and `eval' may
//! be handed such a form (src/eval.c Feval, eval_sub). Self-evaluating
//! objects return themselves, quoted ones are the identical object, and a
//! function object in head position is called.
//!
//! The case that blocked GNU's own Lisp: macroexp.el:955
//!     (eval-when-compile (add-hook 'emacs-startup-hook (lambda () ...)))
//! `eval-when-compile' (byte-run.el:674) expands to `(quote VALUE)' where
//! VALUE is the hook list add-hook returns -- a list holding a closure.
//! GNU transcript (emacs 32.0.50 -Q --batch):
//!   (eval (list (lambda (x) (* x 2)) 21) t)                     => 42
//!   (let ((c (lambda () 1)))
//!     (eq c (car (eval (list 'progn (list 'quote (list c))) t)))) => t
//!   (let ((b (current-buffer))) (eq b (eval (list 'progn b) t))) => t
//!   (defmacro pm () (list 'quote (list (lambda () 7))))
//!   (funcall (car (eval '(pm) t)))                                => 7
//!   (funcall (car (eval-when-compile (list (lambda () 9)))))      => 9
use romacs_vm::Vm;
use std::path::PathBuf;

fn shown(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

/// byte-run.el's own definition, verbatim in effect.
const EVAL_WHEN_COMPILE: &str = "(defalias 'eo-ewc (cons 'macro (function (lambda (&rest body) (list 'quote (eval (cons 'progn body) t))))))";

#[test]
fn macro_may_expand_to_a_quoted_closure() {
    let mut vm = Vm::new();
    vm.eval_source("(defalias 'eo-m (cons 'macro (function (lambda () (list 'quote (list (function (lambda () 7))))))))")
        .unwrap();
    assert_eq!(shown(&mut vm, "(funcall (car (eo-m)))"), "7");
}

#[test]
fn eval_when_compile_returning_a_hook_list_holding_a_closure() {
    // The exact shape of macroexp.el:955.
    let mut vm = Vm::new();
    vm.eval_source(EVAL_WHEN_COMPILE).unwrap();
    vm.eval_source("(defvar eo-hook nil)").unwrap();
    assert_eq!(
        shown(&mut vm, "(progn (eo-ewc (setq eo-hook (cons (function (lambda () 1)) eo-hook))) (funcall (car eo-hook)))"),
        "1"
    );
    // The expansion carries the identical closure the body built. (Its list
    // cells are copied, unlike GNU: DIVERGENCES.md VM2-EVAL2.)
    assert_eq!(shown(&mut vm, "(eq (car eo-hook) (car (eo-ewc eo-hook)))"), "t");
}

#[test]
fn eval_preserves_identity_of_objects_in_the_datum() {
    let mut vm = Vm::new();
    assert_eq!(
        shown(&mut vm, "(let ((c (function (lambda () 1)))) (eq c (car (eval (list 'progn (list 'quote (list c))) t))))"),
        "t"
    );
    assert_eq!(shown(&mut vm, "(let ((b (current-buffer))) (eq b (eval (list 'progn b) t)))"), "t");
    assert_eq!(shown(&mut vm, "(eval (list (function (lambda (x) (* x 2))) 21) t)"), "42");
    assert_eq!(shown(&mut vm, "(eval (list 'funcall (function (lambda (x) (* x 2))) 21) t)"), "42");
}

#[test]
fn a_non_function_object_in_head_position_is_invalid_function() {
    // GNU: (condition-case e (eval (list (current-buffer)) t) (error (car e)))
    //      => invalid-function
    let mut vm = Vm::new();
    assert_eq!(
        shown(&mut vm, "(let ((b (current-buffer))) (condition-case e (eval (list b) t) (error (list (car e) (eq (car (cdr e)) b)))))"),
        "(invalid-function t)"
    );
}

fn source_file(name: &str, text: &str) -> PathBuf {
    let dir = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"), PathBuf::from)
        .join("expansion-objects");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!(";;; -*- lexical-binding: t -*-\n{text}")).unwrap();
    path
}

#[test]
fn expansion_objects_survive_a_moving_collection_and_are_reclaimed() {
    // The closure, and the 64 KiB string it captures, are created by the
    // macro's expansion: no source text spells them, so once compiled they
    // are reachable solely through the compiled function's constant.
    // Garbage allocated before them forces the compaction to move them.
    // (Loaded from a file: `eval_source' requests retain their own results
    // past a collection, which would mask reclamation either way.)
    let settled = |vm: &mut Vm| {
        vm.collect_garbage();
        let stats = vm.heap_stats();
        (stats.live_objects, stats.live_bytes)
    };
    let mut vm = Vm::new();
    vm.eval_source(EVAL_WHEN_COMPILE).unwrap();
    vm.eval_source("(defalias 'eo-keep nil)").unwrap();
    let base = settled(&mut vm);
    vm.eval_source("(let ((i 0) (g nil)) (while (< i 2000) (setq g (cons i g)) (setq i (1+ i))) nil)").unwrap();
    let define = source_file(
        "keep.el",
        "(defalias 'eo-keep (function (lambda () (eo-ewc (let ((s (make-string 65536 ?q))) (list (function (lambda () s))))))))\n\
         (defvar eo-forced (length (funcall (car (eo-keep)))))\n",
    );
    vm.load_file(&define).unwrap();
    let held = settled(&mut vm);
    eprintln!("base {base:?} held {held:?}");
    assert!(held.1 >= base.1 + 65_536, "the expansion's string must stay live across collection: base {base:?}, held {held:?}");
    let check = source_file(
        "check.el",
        "(defvar eo-check (list (eq (car (eo-keep)) (car (eo-keep))) (length (funcall (car (eo-keep)))) (aref (funcall (car (eo-keep))) 65535) eo-forced))\n",
    );
    vm.load_file(&check).unwrap();
    // After the moving collection: one object, not copies, with its bytes.
    assert_eq!(shown(&mut vm, "eo-check"), "(t 65536 113 65536)");
    let drop = source_file("drop.el", "(defalias 'eo-keep nil)\n(setq eo-check nil)\n");
    vm.load_file(&drop).unwrap();
    let dead = settled(&mut vm);
    eprintln!("dead {dead:?}");
    assert_eq!(dead, base, "an unreachable expansion object must be reclaimed exactly: base {base:?}, held {held:?}");
}

#[test]
fn a_later_expansion_cannot_collect_an_earlier_expansion_object() {
    // An expansion's object waits in the compiler's builder, unrooted, while
    // later macros in the same form expand. Those expansions run Lisp that
    // allocates; with collection at every opportunity (floor 1) none of them
    // may collect, or the waiting object would be reclaimed or moved under
    // the builder. 200000 iterations cross the collection threshold inside
    // the expansion (5000 did not, and then this test could not fail).
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(EVAL_WHEN_COMPILE).unwrap();
    // Garbage made BEFORE the object, inside the same expansion, so a
    // collection during the later expansions would have to move it.
    vm.eval_source(
        "(defalias 'eo-junk (function (lambda () (let ((i 0) (g nil)) (while (< i 3000) (setq g (cons (make-string 8 ?j) g)) (setq i (1+ i))) g))))",
    )
    .unwrap();
    vm.eval_source(
        "(defalias 'eo-churn (cons 'macro (function (lambda () (let ((i 0) (g nil)) (while (< i 200000) (setq g (cons (make-string 8 ?g) g)) (setq i (1+ i)))) 1))))",
    )
    .unwrap();
    assert_eq!(
        shown(
            &mut vm,
            "(let ((r (list (eo-ewc (let ((junk (eo-junk))) (setq junk nil) (let ((s (make-string 4096 ?z))) (function (lambda () s))))) (eo-churn) (eo-churn)))) (list (length (funcall (car r))) (aref (funcall (car r)) 4095) (car (cdr r))))"
        ),
        "(4096 122 1)"
    );
}

#[test]
fn a_lazily_held_body_may_carry_an_expansion_object() {
    // In the non-eager regime (no `internal-macroexpand-for-load') a function
    // body is held unexpanded and compiled when first called (R215). A body
    // produced by a top-level macro may already hold an object; forcing it
    // must carry that object, as GNU's interpreter would evaluate it.
    let mut vm = Vm::new();
    let path = source_file(
        "lazy.el",
        "(defalias 'eo-mkdef (cons 'macro (function (lambda () (list 'defalias ''eo-lazy (list 'function (list 'lambda nil (list 'funcall (list 'quote (let ((n 40)) (function (lambda () (+ n 2)))))))))))))\n\
         (eo-mkdef)\n",
    );
    vm.load_file(&path).unwrap();
    assert_eq!(shown(&mut vm, "(interpreted-function-p (symbol-function 'eo-lazy))"), "t", "held lazily");
    assert_eq!(shown(&mut vm, "(eo-lazy)"), "42");
}

#[test]
fn a_dotted_call_on_a_function_object_signals_on_its_tail() {
    // GNU eval_sub: a function-object head is a function, so the dotted
    // argument list is what fails:
    //   (eval (cons (lambda (x) x) (cons 1 2)) t)            => (wrong-type-argument listp 2)
    //   (eval (cons (symbol-function 'car) (cons 1 2)) t)    => (wrong-type-argument listp 2)
    //   (eval (cons (current-buffer) (cons 1 2)) t)          => (invalid-function #<buffer>)
    let mut vm = Vm::new();
    assert_eq!(
        shown(&mut vm, "(condition-case e (eval (cons (function (lambda (x) x)) (cons 1 2)) t) (error (list (car e) (if (eq (car e) 'wrong-type-argument) (cdr e) 'other))))"),
        "(wrong-type-argument (listp 2))"
    );
    assert_eq!(
        shown(&mut vm, "(condition-case e (eval (cons (symbol-function 'car) (cons 1 2)) t) (error (list (car e) (if (eq (car e) 'wrong-type-argument) (cdr e) 'other))))"),
        "(wrong-type-argument (listp 2))"
    );
    assert_eq!(
        shown(&mut vm, "(let ((b (current-buffer))) (condition-case e (eval (cons b (cons 1 2)) t) (error (list (car e) (eq (car (cdr e)) b)))))"),
        "(invalid-function t)"
    );
}

#[test]
fn macroexp_el_loads_past_its_eval_when_compile_add_hook_form() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp"), root.join("lisp/emacs-lisp")]).unwrap();
    vm.load_file(&root.join("lisp/emacs-lisp/byte-run.el")).unwrap();
    // subr.el stops at its own frontier (pinned in bootstrap_descriptors.rs).
    // Since Phase 0 restored GNU's subr.el that frontier (form 139, the radix
    // reader gap G1) comes BEFORE `add-hook` (subr.el:2295), which is all
    // macroexp.el's eval-when-compile form needs. This test protects the
    // closure-in-quoted-expansion representation, not subr's frontier, so it
    // loads GNU's own `add-hook` defun, verbatim from lisp/subr.el, through the
    // loader (lazy body, as GNU defines it) instead of depending on the frontier.
    let _ = vm.load_file(&root.join("lisp/subr.el"));
    let subr = std::fs::read_to_string(root.join("lisp/subr.el")).unwrap();
    let start = subr.find("\n(defun add-hook (hook function &optional depth local)\n").expect("GNU add-hook") + 1;
    let end = start + subr[start..].find("\n(defun remove-hook ").expect("GNU remove-hook follows add-hook");
    let add_hook = std::env::temp_dir().join(format!("gnu-add-hook-{}.el", std::process::id()));
    std::fs::write(&add_hook, format!(";;; -*- lexical-binding: t -*-\n{}\n", &subr[start..end])).unwrap();
    let loaded = vm.load_file(&add_hook);
    let _ = std::fs::remove_file(&add_hook);
    loaded.unwrap();
    let path = root.join("lisp/emacs-lisp/macroexp.el").canonicalize().unwrap();
    let result = vm.load_file(&path).map(|_| ());
    let report = vm.load_report();
    let step = report["steps"].as_array().unwrap().iter()
        .find(|s| s["file"] == path.to_string_lossy().as_ref() && s["line"] == 956)
        .cloned();
    let step = step.expect("macroexp.el:956 must be reached");
    assert_eq!(step["status"], "ok", "macroexp.el:956: {step}");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(shown(&mut vm, "(featurep 'macroexp)"), "t");
    // The startup hook now holds GNU's closure, callable as a function.
    assert_eq!(shown(&mut vm, "(functionp (car emacs-startup-hook))"), "t");
}

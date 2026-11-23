//! Interpreted closure environments and the nadvice consumers of slot 2,
//! compared with the pinned GNU interpreter on the loadup chain.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;

const EXPRESSIONS: &[&str] = &[
    "(aref (lambda (x) x) 2)",
    "(eval '(aref (lambda (x) x) 2) nil)",
    "(progn (defun ce-f (x) (interactive (list 1)) x) (aref (symbol-function 'ce-f) 2))",
    "(progn (defun ce-cmd (x) (interactive (list (+ 1 2))) x) (advice-add 'ce-cmd :around (lambda (f &rest a) (apply f a))) (list (commandp 'ce-cmd) (call-interactively 'ce-cmd) (car (interactive-form 'ce-cmd))))",
    r#"(progn
      (defun ce-captured (x) (interactive "p") x)
      (advice-add 'ce-captured :around
        (let ((k 1)) (lambda (o &rest a) (interactive (list k)) (apply o a)))
        '((depth . -99)))
      (list (commandp 'ce-captured) (interactive-form 'ce-captured)
            (call-interactively 'ce-captured)))"#,
    r"(progn
      (defun ce-iform-function (x) (interactive (list 2)) x)
      (advice-add 'ce-iform-function :around
        (let ((k 40))
          (lambda (o &rest a)
            (interactive (lambda (spec) (list (+ k (car (advice-eval-interactive-spec spec))))))
            (apply o a))))
      (list (commandp 'ce-iform-function) (call-interactively 'ce-iform-function)))",
    r"(progn
      (defun ce-once (x) (interactive (list 2)) x)
      (let ((k 40) (offset 3))
        (letrec ((wrapper (lambda (o x)
                            (interactive (list k))
                            (advice-remove 'ce-once wrapper)
                            (funcall o (+ offset x)))))
          (advice-add 'ce-once :around wrapper)))
      (list (commandp 'ce-once) (call-interactively 'ce-once) (call-interactively 'ce-once)))",
    "(let ((k 1) (unused 9)) (aref (lambda (x) (interactive (list k)) x) 2))",
    "(let ((y 5) (unused 9)) (aref (lambda (x) (+ x y)) 2))",
    "(let ((y 5)) (aref (lambda (x) x) 2))",
    "(let ((y 5)) (let* ((f (lambda () (setq y (1+ y)))) (env (aref f 2))) (setcdr (assq 'y env) 40) (list (funcall f) y)))",
    r#"(list (length (lambda (x) x))
             (length (lambda (x) "d" x))
             (length (lambda (x) (interactive "p") x))
             (length (lambda (x) (interactive) x)))"#,
    r#"(let ((f (lambda (x) "doc" (interactive "p" foo-mode) x)))
         (list (aref f 0) (aref f 3) (aref f 4) (aref f 5)
               (arrayp f) (sequencep f) (vectorp f)))"#,
    "(condition-case e (aref (lambda (x) x) 3) (args-out-of-range (list (car e) (nth 2 e))))",
    "(condition-case e (aset (lambda (x) x) 0 nil) (wrong-type-argument (list (car e) (nth 1 e))))",
    r"(eval '(progn
           (eval '(defvar ra-active 4) nil)
           (list (let ((ra-active 9))
                   (let* ((f (lambda () ra-active))
                          (p (assq 'ra-active (aref f 2))))
                     (when p (setcdr p 40))
                     (funcall f)))
                 (symbol-value 'ra-active)))
        t)",
    r"(eval '(progn
           (eval '(defvar ra-forward 4) nil)
           (let ((ra-forward 9))
             (setq ra-forward-f (lambda () ra-forward)))
           (let* ((env (aref ra-forward-f 2))
                  (pair (assq 'ra-forward env))
                  (before (funcall ra-forward-f)))
             (when pair (setcdr pair 40))
             (list before (funcall ra-forward-f) (and pair t))))
        t)",
    r#"(progn
  (set 'ra-local-special 1)
  (defun ra-cmd (x) (interactive "p") x)
  (let ((captured 40))
    (defvar ra-local-special)
    (let ((wrapper (lambda (orig &rest args)
                    (interactive
                     (let ((ra-local-special captured))
                       (list (symbol-value 'ra-local-special))))
                    (apply orig args))))
      (advice-add 'ra-cmd :around wrapper)
      (list (aref wrapper 2) (commandp 'ra-cmd) (call-interactively 'ra-cmd)))))"#,
    r#"(progn
  (defun ra-counted (x) (interactive "p") x)
  (let ((n 0))
    (let ((wrapper (lambda (orig &rest args)
                     (interactive (list (setq n (1+ n))))
                     (apply orig args))))
      (advice-add 'ra-counted :around wrapper)
      (list (call-interactively 'ra-counted) (call-interactively 'ra-counted) n))))"#,
    // A dynamic `let' hides its name: slot 2 shows the outer lexical
    // binding it shadows, and a write through that pair reaches both.
    r"(eval '(let ((g (let ((ra-shadowed 1))
                       (lambda ()
                         (list ra-shadowed
                               (let ((ra-shadowed 9))
                                 (let ((f (lambda () ra-shadowed)))
                                   (setcdr (assq 'ra-shadowed (aref f 2)) 40)
                                   (list (funcall f) ra-shadowed (symbol-value 'ra-shadowed))))
                               ra-shadowed)))))
         (eval '(defvar ra-shadowed 4) nil)
         (list (funcall g) (symbol-value 'ra-shadowed)))
      t)",
    // No lexical variable in view: GNU keeps the whole context, the local
    // special declaration and the sentinel, so the spec's `let' binds it
    // dynamically for the function it calls.
    r#"(progn
  (defun ra-bare-read () ra-bare-special)
  (defun ra-bare-cmd (n) (interactive "p") n)
  (defvar ra-bare-special)
  (let ((wrapper (lambda (orig &rest args)
                   (interactive (let ((ra-bare-special 42)) (list (ra-bare-read))))
                   (apply orig args))))
    (advice-add 'ra-bare-cmd :around wrapper)
    (list (aref wrapper 2) (commandp 'ra-bare-cmd) (call-interactively 'ra-bare-cmd))))"#,
    // The metadata is parsed once: a string after the interactive form is
    // body, not a docstring.
    r#"(eval '(list (aref (lambda () (interactive "p") "value" 42) 1)
                  (aref (lambda () "doc" (interactive "p") "value" 42) 1)
                  (aref (lambda () (interactive "p")) 1))
            t)"#,
    r#"(let ((f (lambda () (interactive "p") "value" 42))) (list (documentation f) (length f) (funcall f)))"#,
];

// VM2-CLOSEQ1: neither mutation nor pair identity reaches an unboxed capture.
const UNBOXED_CAPTURE: &str = r"(let ((y 5))
  (let* ((f (lambda () (1+ y))) (p (assq 'y (aref f 2))))
    (setcdr p 40)
    (list (funcall f) y (eq p (assq 'y (aref f 2))))))";

#[test]
fn closure_slots_and_advice_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = gnu_lisp(&[]);
    let mut failures = Vec::new();
    for expression in EXPRESSIONS.iter().copied().chain([UNBOXED_CAPTURE]) {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if expression == UNBOXED_CAPTURE {
            assert_eq!(expected, "(41 40 t)", "GNU's live pair");
            assert_eq!(actual, "(6 5 nil)", "VM2-CLOSEQ1");
            continue;
        }
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

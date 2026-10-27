//! C primitives and variables emacs-hermes reaches, against pinned GNU:
//! line-number-at-pos, text-property-any, get-buffer-process (no process
//! exists), frame-first-window, timer-idle-list, and the subrs in the
//! special forms' function cells, and print-gensym as gnosis's serializer
//! binds it.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(with-temp-buffer (insert "a\nb\nc\nd") (list (line-number-at-pos) (line-number-at-pos 1) (line-number-at-pos 3) (line-number-at-pos 4) (progn (narrow-to-region 3 6) (list (line-number-at-pos) (line-number-at-pos 3) (line-number-at-pos 1) (line-number-at-pos 1 t) (line-number-at-pos 7) (line-number-at-pos nil t))) (condition-case e (line-number-at-pos 99) (error e)) (condition-case e (line-number-at-pos 0) (error e)) (condition-case e (line-number-at-pos 'x) (error e)) (line-number-at-pos (copy-marker 5))))
(with-temp-buffer (insert "λ\nμ\nν") (list (line-number-at-pos 4) (line-number-at-pos 5)))
(with-temp-buffer (insert "a\rb\nc") (list (line-number-at-pos) (progn (setq selective-display t) (line-number-at-pos)) (progn (setq selective-display 3) (line-number-at-pos))))
(with-temp-buffer (insert "abcdef") (put-text-property 3 5 'p 'v) (list (text-property-any 1 7 'p 'v) (text-property-any 1 7 'p nil) (text-property-any 4 7 'p nil) (text-property-any 7 1 'p 'v) (text-property-any 3 3 'p nil) (text-property-any 1 7 'q 'v) (text-property-any 1 7 'q nil) (condition-case e (text-property-any 0 3 'p 'v) (error e))))
(with-temp-buffer (insert "abc") (list (text-property-any 1 4 'p nil) (text-property-any 1 4 'p 'v) (text-property-any 2 1 'p nil)))
(let ((s (propertize "abc" 'p 1))) (list (text-property-any 0 3 'p 1 s) (text-property-any 0 3 'p 2 s) (text-property-any 1 3 'p 1 "xyz") (text-property-any 0 3 'p nil "xyz")))
(list (get-buffer-process nil) (get-buffer-process "nonexistent") (get-buffer-process (current-buffer)) (get-buffer-process (buffer-name)) (condition-case e (get-buffer-process 1) (error e)))
(list (eq (frame-first-window) (selected-window)) (eq (frame-first-window (selected-frame)) (selected-window)) (eq (frame-first-window (selected-window)) (selected-window)) (condition-case e (frame-first-window 1) (error e)))
(list (boundp 'timer-idle-list) (special-variable-p 'timer-idle-list))
(list (subr-arity (symbol-function 'and)) (subr-arity (symbol-function 'catch)) (subr-arity (symbol-function 'cond)) (subr-arity (symbol-function 'condition-case)) (subr-arity (symbol-function 'defconst)) (subr-arity (symbol-function 'defvar)) (subr-arity (symbol-function 'function)) (subr-arity (symbol-function 'if)) (subr-arity (symbol-function 'interactive)) (subr-arity (symbol-function 'let)) (subr-arity (symbol-function 'let*)) (subr-arity (symbol-function 'or)) (subr-arity (symbol-function 'prog1)) (subr-arity (symbol-function 'progn)) (subr-arity (symbol-function 'quote)) (subr-arity (symbol-function 'save-current-buffer)) (subr-arity (symbol-function 'save-excursion)) (subr-arity (symbol-function 'save-restriction)) (subr-arity (symbol-function 'setq)) (subr-arity (symbol-function 'unwind-protect)) (subr-arity (symbol-function 'while)))
(list (fboundp 'progn) (subrp (symbol-function 'if)) (functionp 'if) (functionp (symbol-function 'progn)) (subr-name (symbol-function 'save-excursion)) (subr-name (symbol-function 'car)) (condition-case e (subr-name 'car) (error e)))
(list (condition-case e (funcall 'if t 1) (error (list (car e) (eq (car (cdr e)) (symbol-function 'if))))) (condition-case e (apply #'progn '(1 2)) (error (car e))))
(list (condition-case e (eval '(catch) t) (error e)) (condition-case e (eval '(if) t) (error e)) (condition-case e (eval '(quote) t) (error e)) (condition-case e (eval '(quote 1 2) t) (error e)) (condition-case e (eval '(function) t) (error e)) (condition-case e (eval '(let) t) (error e)) (condition-case e (eval '(while) t) (error e)) (condition-case e (eval '(defvar) t) (error e)) (condition-case e (eval '(prog1) t) (error e)) (condition-case e (eval '(condition-case x) t) (error e)) (condition-case e (eval '(unwind-protect) t) (error e)) (condition-case e (eval '(defconst x) t) (error e)))
(list (if t 1 2) (let ((a 1)) (let* ((b (1+ a))) (and a b (or nil b)))) (catch 'k (throw 'k 5)) (condition-case nil (car 1) (error 'caught)) (prog1 1 2) (cond ((= 1 2) 'a) (t 'b)) (save-excursion 7) (save-restriction 8) (save-current-buffer 9) (unwind-protect 10 11) (quote q) (function car))
(list (type-of (symbol-function 'if)) (cl-type-of (symbol-function 'if)) (type-of (symbol-function 'car)) (cl-type-of (symbol-function 'car)) (condition-case e (eval '(defvar x 1 nil 4) t) (error e)) (condition-case e (eval '(defconst x 1 nil 4) t) (error e)) (condition-case e (eval '(defvar 1 2 nil 4) t) (error e)) (condition-case e (eval '(defconst 1 2 nil 4) t) (error e)))
(let ((s (make-symbol "g")) (e (make-symbol "")) (print-gensym t)) (list (prin1-to-string (list s s 'a :k e nil t)) (let ((print-circle t)) (prin1-to-string (list s s e e 'zz))) (prin1-to-string (list s) nil '((gensym . nil))) (let ((print-gensym nil)) (prin1-to-string s nil '((gensym . t)))) (let ((print-charset-text-property t)) (prin1-to-string (propertize "ab" 'face 'bold))) (let ((print-circle t)) (prin1-to-string (list (intern "x" (obarray-make)) 'y)))))
(let ((print-gensym t) (print-circle t) (x (intern "x" (obarray-make)))) (list (prin1-to-string (list x x)) (prin1-to-string (list (make-symbol "quote") (make-symbol "x"))) (prin1-to-string '(quote a)) (prin1-to-string '(function f))))
"###;

#[test]
fn package_primitives_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = Vm::new().eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

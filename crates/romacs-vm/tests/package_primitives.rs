//! C primitives and variables emacs-hermes reaches, against pinned GNU:
//! line-number-at-pos, text-property-any, get-buffer-process (no process
//! exists), frame-first-window, timer-idle-list, and the subrs in the
//! special forms' function cells, print-gensym as gnosis's serializer
//! binds it, `random' seeded as GNU seeds glibc's generator,
//! overlay-properties, byte-to-string, the base64 region coders,
//! command-modes and local-minor-modes.
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
(progn (random "abc") (list (random) (random 10) (random 1000000) (random (expt 2 70)) (random (expt 2 64)) (random most-positive-fixnum) (progn (random "") (random 100)) (progn (random "a longer seed string") (list (random 7) (random 7))) (condition-case e (random 0) (error e)) (condition-case e (random -5) (error e)) (condition-case e (random (- (expt 2 70))) (error e)) (fixnump (random nil)) (fixnump (random 'x)) (progn (random t) (< -1 (random 5) 5))))
(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 1 3))) (list (overlay-properties o) (progn (overlay-put o 'face 'bold) (overlay-put o 'a 1) (overlay-put o 'face 'italic) (overlay-put o 'b nil) (overlay-properties o)) (let ((p (overlay-properties o))) (setcar (cdr p) 'zz) (overlay-get o 'b)) (eq (overlay-properties o) (overlay-properties o)) (progn (delete-overlay o) (overlay-properties o)))))
(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 2))) (overlay-put o 'evaporate t) (overlay-put o 'face 'bold) (list (overlay-properties o) (condition-case e (overlay-properties 1) (error e)) (condition-case e (overlay-properties nil) (error e)) (condition-case e (overlay-properties (current-buffer)) (error e)))))
(list (byte-to-string 0) (byte-to-string 65) (byte-to-string 255) (multibyte-string-p (byte-to-string 200)) (string-bytes (byte-to-string 200)) (aref (byte-to-string 200) 0) (condition-case e (byte-to-string 256) (error e)) (condition-case e (byte-to-string -1) (error e)) (condition-case e (byte-to-string ?é) (error e)) (condition-case e (byte-to-string 1.0) (error e)) (condition-case e (byte-to-string (expt 2 70)) (error e)) (condition-case e (byte-to-string nil) (error e)) (concat "a" (byte-to-string 255)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "hello\377\0world") (list (base64-encode-region 1 (point-max)) (buffer-string) (point) (multibyte-string-p (buffer-string))))
(with-temp-buffer (insert "abcdefgh") (goto-char 4) (let ((m (copy-marker 2)) (n (copy-marker 6)) (o (copy-marker 7 t)) (p (copy-marker 4))) (list (base64-encode-region 2 6) (buffer-string) (point) (marker-position m) (marker-position n) (marker-position o) (marker-position p))))
(with-temp-buffer (insert "abcdefgh") (list (progn (goto-char 8) (base64-encode-region 6 2)) (point) (buffer-string) (progn (goto-char 1) (base64url-encode-region 1 3)) (point) (buffer-string) (base64url-encode-region 1 3 t) (buffer-string)))
(with-temp-buffer (insert (make-string 100 ?x)) (list (base64-encode-region 1 101) (buffer-string) (progn (erase-buffer) (insert (make-string 100 ?y)) (base64-encode-region 1 101 t)) (buffer-string) (progn (erase-buffer) (insert (make-string 100 ?z)) (base64url-encode-region 1 101)) (buffer-string)))
(with-temp-buffer (insert "aé") (list (condition-case e (base64-encode-region 1 3) (error e)) (buffer-string) (condition-case e (base64-encode-region 0 2) (error e)) (condition-case e (base64-encode-region 1 'x) (error e))))
(with-temp-buffer (insert "xx" "aGVsbG8=" "yy") (goto-char 11) (let ((m (copy-marker 5))) (list (base64-decode-region 3 11) (buffer-string) (point) (marker-position m) (condition-case e (base64-decode-region 1 3) (error e)) (buffer-string) (progn (erase-buffer) (insert "a\nGV s!bG8") (condition-case e (base64-decode-region 1 (point-max)) (error e))) (buffer-string) (base64-decode-region 1 (point-max) nil t) (buffer-string))))
(with-temp-buffer (insert "_-8") (list (condition-case e (base64-decode-region 1 4) (error e)) (buffer-string) (progn (erase-buffer) (insert "YWI") (base64-decode-region 1 4 t)) (buffer-string) (progn (erase-buffer) (insert "YWI") (condition-case e (base64-decode-region 1 4) (error e))) (buffer-string)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "/w==") (list (base64-decode-region 1 5) (append (buffer-string) nil) (multibyte-string-p (buffer-string))))
(list (commandp 'base64-encode-region) (commandp 'base64url-encode-region) (commandp 'base64-decode-region) (interactive-form 'base64-decode-region))
(list (command-modes (lambda () (interactive nil foo-mode) 1)) (command-modes (lambda () (interactive "p" a-mode b-mode) 1)) (command-modes (lambda () (interactive "p") 1)) (command-modes (lambda () 1)) (command-modes 'car) (command-modes 'cm-undefined) (command-modes nil) (command-modes 1) (command-modes "x") (command-modes (lambda () (interactive "p" . m-mode) 1)))
(progn (defalias 'cm-a (lambda () (interactive nil x-mode) 1)) (defalias 'cm-b 'cm-a) (put 'cm-b 'command-modes '(y-mode)) (list (command-modes 'cm-a) (command-modes 'cm-b) (progn (put 'cm-a 'command-modes '(z-mode)) (command-modes 'cm-a)) (interactive-form 'cm-a) (commandp 'cm-a)))
(let ((f (lambda (x) (interactive (list 7) q-mode) x)) (g (lambda (x) (interactive "p") x))) (list (interactive-form f) (command-modes f) (call-interactively f) (command-modes g)))
(progn (autoload 'cm-auto "cm-file" nil '(w-mode v-mode)) (autoload 'cm-auto2 "cm-file" nil t) (list (command-modes 'cm-auto) (command-modes 'cm-auto2) (command-modes '(lambda () (interactive "p" l-mode) 1)) (command-modes '(lambda () (interactive "p") 1)) (command-modes '(lambda () 1)) (command-modes '(macro . car))))
(let ((f (make-interpreted-closure nil '(1) '(t) nil '(interactive nil r-mode s-mode)))) (command-modes f))
(let ((f (make-interpreted-closure nil '(1) '(t) nil '(interactive nil r-mode s-mode)))) (interactive-form f))
(let ((f (make-interpreted-closure nil '(1) '(t) nil '(interactive nil r-mode s-mode)))) (aref f 5))
(let ((f (make-interpreted-closure nil '(1) '(t) nil '(interactive nil r-mode s-mode)))) (funcall f))
(let ((g (make-interpreted-closure nil '(1) '(t) nil '(interactive "p")))) (list (command-modes g) (aref g 5)))
(with-temp-buffer (list (boundp 'local-minor-modes) local-minor-modes (local-variable-p 'local-minor-modes) (special-variable-p 'local-minor-modes) (default-value 'local-minor-modes) (progn (setq local-minor-modes '(foo-mode)) (kill-all-local-variables) (list local-minor-modes (with-temp-buffer local-minor-modes) (local-variable-p 'local-minor-modes) (default-value 'local-minor-modes) (buffer-local-value 'local-minor-modes (current-buffer)))) (assq 'local-minor-modes (buffer-local-variables)) (let ((local-minor-modes '(x))) (list local-minor-modes (default-value 'local-minor-modes))) (progn (set-default 'local-minor-modes '(d)) (prog1 (list (default-value 'local-minor-modes) local-minor-modes (with-temp-buffer local-minor-modes)) (set-default 'local-minor-modes nil)))))
"###;

/// Cases whose change hooks run Lisp, which needs the Lisp prelude.
const PRELUDE_CASES: &str = r###"
(with-temp-buffer (buffer-enable-undo) (insert "abc") (undo-boundary) (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l) log))))) (list (base64-encode-region 1 4) (buffer-string) (reverse log) buffer-undo-list (progn (setq log nil) (base64-decode-region 1 5)) (reverse log) (buffer-string))))
(progn (defun cm-d () "doc" (interactive nil d-mode e-mode) 1) (defun cm-e (x) (interactive "p") x) (list (command-modes 'cm-d) (interactive-form 'cm-d) (commandp 'cm-d) (command-modes 'cm-e)))
"###;

#[test]
fn package_primitives_match_pinned_gnu() {
    matches_gnu(CASES, Vm::new);
}

#[test]
fn package_primitives_with_prelude_match_pinned_gnu() {
    matches_gnu(PRELUDE_CASES, || {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm
    });
}

fn matches_gnu(cases: &str, vm: fn() -> Vm) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm().eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A VM2 multibyte buffer holds Unicode text, so the raw bytes GNU decodes
/// into one are refused before the buffer changes (VM2-BASE64-REGION1).
#[test]
fn base64_decode_region_refuses_raw_bytes_in_a_multibyte_buffer() {
    let source = "(with-temp-buffer (insert \"x_-8\") (goto-char 2) (list (condition-case e (base64-decode-region 2 5 t) (error e)) (buffer-string) (point) buffer-undo-list))";
    let actual = Vm::new().eval_source(source).and_then(|v| v.display()).unwrap();
    assert_eq!(actual, "((error \"VM2 base64-decode-region of raw bytes into a multibyte buffer is unsupported\") \"x_-8\" 2 t)");
}

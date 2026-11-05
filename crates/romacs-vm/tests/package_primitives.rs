//! C primitives and variables emacs-hermes reaches, against pinned GNU:
//! line-number-at-pos, text-property-any, get-buffer-process (no process
//! exists), frame-first-window, timer-idle-list, and the subrs in the
//! special forms' function cells, print-gensym as gnosis's serializer
//! binds it, `random' seeded as GNU seeds glibc's generator,
//! overlay-properties, byte-to-string, the base64 region coders,
//! command-modes, local-minor-modes, replace-region-contents and
//! insert-before-markers into a unibyte buffer.
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
(with-temp-buffer (insert "hello world") (let ((m1 (copy-marker 3)) (m2 (copy-marker 8)) (m3 (copy-marker 12)) (m4 (copy-marker 10 t))) (goto-char 9) (list (replace-region-contents 1 12 "jello wordl!") (buffer-string) (point) (marker-position m1) (marker-position m2) (marker-position m3) (marker-position m4))))
(with-temp-buffer (insert (propertize "abc" 'face 'bold) "def") (list (replace-region-contents 1 7 (propertize "abXdef" 'q 1)) (buffer-string) (progn (goto-char 4) (replace-region-contents 2 6 "zzzz")) (buffer-string) (point)))
(with-temp-buffer (insert "αβγδ") (goto-char 3) (list (replace-region-contents 1 5 "αXγδε") (buffer-string) (point) (replace-region-contents 2 3 "") (buffer-string) (replace-region-contents 2 2 "βb") (buffer-string) (point) (replace-region-contents 1 1 "") (buffer-string)))
(with-temp-buffer (insert "abcdef") (goto-char 4) (list (replace-region-contents 2 5 "XYZW" 0) (buffer-string) (point) (replace-region-contents 2 6 "bcd" nil 0) (buffer-string) (point) (replace-region-contents 1 (point-max) "abcdef" 1.5 100) (buffer-string)))
(with-temp-buffer (insert "one two three") (let ((src (get-buffer-create " rrc-src"))) (with-current-buffer src (insert "xx one 2 three yy") (narrow-to-region 4 16)) (list (replace-region-contents 1 14 src) (buffer-string) (replace-region-contents 1 (point-max) (vector src 5 9)) (buffer-string) (condition-case e (replace-region-contents 1 2 (vector src 1 3)) (error e)) (condition-case e (replace-region-contents 1 2 (vector src 5 'x)) (error e)) (condition-case e (replace-region-contents 1 2 (vector src 4)) (error e)) (condition-case e (replace-region-contents 1 2 (vector 'x 1 2)) (error e)) (condition-case e (replace-region-contents 1 2 'x) (error e)) (condition-case e (replace-region-contents 1 2 7) (error e)) (condition-case e (replace-region-contents 1 2 (current-buffer)) (error e)) (condition-case e (replace-region-contents 1 2 (vector (current-buffer) 1 2)) (error e)) (progn (kill-buffer src) (condition-case e (replace-region-contents 1 2 src) (error e))) (condition-case e (replace-region-contents 0 2 "a") (error e)) (condition-case e (replace-region-contents 1 2 "ab" nil 'x) (error e)) (condition-case e (replace-region-contents 1 2 "ab" 'x) (error e)) (buffer-string))))
(with-temp-buffer (insert "abcdefgh") (list (replace-region-contents 3 6 (lambda () (list (point-min) (point-max)) (upcase (buffer-string)))) (buffer-string) (point-min) (point-max) (replace-region-contents 3 6 (lambda () (current-buffer))) (buffer-string)))
(with-temp-buffer (insert "aaa bbb ccc") (goto-char 1) (re-search-forward "c+") (let ((before (list (match-beginning 0) (match-end 0)))) (replace-region-contents 1 4 (propertize "zzzzz" 'p 1) 0 nil t) (list before (match-beginning 0) (match-end 0) (buffer-string) (progn (re-search-backward "b+") (replace-region-contents 1 8 "q" 0) (list (match-beginning 0) (match-end 0))))))
(with-temp-buffer (insert (propertize "a" 'face 'bold) "bc" (propertize "d" 'face 'italic)) (list (replace-region-contents 2 4 "XYZ" 0 nil 'inherit) (buffer-string) (replace-region-contents 2 5 "QR" nil nil 'inherit) (buffer-string)))
(with-temp-buffer (buffer-enable-undo) (insert "abcdef") (undo-boundary) (goto-char 3) (let ((tick (buffer-modified-tick))) (list (replace-region-contents 1 7 "aXcdYYf") buffer-undo-list (- (buffer-modified-tick) tick) (buffer-chars-modified-tick))))
(with-temp-buffer (buffer-enable-undo) (insert "abcdef") (undo-boundary) (goto-char 7) (let ((tick (buffer-modified-tick))) (list (replace-region-contents 2 5 "QQQQQ" 0) buffer-undo-list (- (buffer-modified-tick) tick) (point) (replace-region-contents 3 3 "zz" 0) (car (cdr buffer-undo-list)) (car buffer-undo-list))))
(with-temp-buffer (insert "0123456789") (let ((a (make-overlay 2 4)) (b (make-overlay 5 6)) (c (make-overlay 3 9)) (d (make-overlay 9 9)) (e (make-overlay 4 5))) (overlay-put e 'evaporate t) (list (replace-region-contents 1 11 "01ab45x789") (buffer-string) (list (overlay-start a) (overlay-end a) (overlay-start b) (overlay-end b) (overlay-start c) (overlay-end c) (overlay-start d) (overlay-end d) (overlay-start e) (overlay-end e)))))
(list (commandp 'replace-region-contents) (car (cdr (interactive-form 'replace-region-contents))))
(with-temp-buffer (insert "0123456789") (let ((f (make-overlay 4 6)) (g (make-overlay 4 6))) (overlay-put f 'evaporate t) (list (replace-region-contents 1 11 "012xy6789") (buffer-string) (overlay-buffer f) (overlay-start f) (overlay-end f) (overlay-start g) (overlay-end g) (replace-region-contents 1 10 "012x6789") (overlay-buffer f) (overlay-start g) (overlay-end g) (replace-region-contents 3 5 "" 0) (overlay-buffer f) (overlay-start g))))
(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (let ((f (oclosure--fix-type (ignore) (lambda () (interactive "p" a-mode) 1))) (g (make-interpreted-closure nil '(1) '(t) nil '(interactive "p" b-mode)))) (list (command-modes f) (progn (aset (aref f 5) 1 '(z-mode)) (aref f 5)) (command-modes f) (interactive-form f) (progn (aset (aref f 5) 0 "P") (interactive-form f)) (progn (aset (aref g 5) 1 '(y-mode)) (command-modes g)))))
(with-temp-buffer (set-buffer-multibyte nil) (buffer-enable-undo) (insert "abc") (undo-boundary) (goto-char 3) (list (replace-region-contents 1 4 "XYZ") (buffer-string) (point) (multibyte-string-p (buffer-string)) buffer-undo-list (save-excursion (goto-char 4) (replace-region-contents 2 4 "MNO" 0) (point)) (buffer-string) (point) (save-restriction (narrow-to-region 2 5) (replace-region-contents 3 5 (string-to-unibyte "\377") 0) (list (point-min) (point-max))) (append (buffer-string) nil) (point-max)))
(with-temp-buffer (insert "ab") (goto-char 1) (re-search-forward "b") (list (replace-region-contents 2 2 "X" 0 nil t) (list (match-beginning 0) (match-end 0)) (progn (goto-char 1) (re-search-forward "\\(a\\)\\(X\\)\\(b\\)") (replace-region-contents 2 3 "YZ" 0 nil t) (list (match-beginning 0) (match-end 0) (match-beginning 2) (match-end 2) (match-beginning 3) (match-end 3))) (progn (goto-char 1) (re-search-forward "\\(a\\)\\(Y\\)") (replace-region-contents 3 3 "W" 0 nil t) (list (match-beginning 0) (match-end 0) (match-beginning 1) (match-end 1) (match-beginning 2) (match-end 2))) (buffer-string)))
(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (let* ((make (lambda () (oclosure--fix-type (ignore) (lambda () (interactive "p" a-mode) 1)))) (f (funcall make)) (g (funcall make))) (list (eq (aref f 5) (aref g 5)) (progn (aset (aref f 5) 1 '(z-mode)) (command-modes f)) (command-modes g) (aref g 5) (progn (aset (aref f 5) 0 "P") (interactive-form g)))))
(with-temp-buffer (set-buffer-multibyte nil) (buffer-enable-undo) (insert "abc") (undo-boundary) (goto-char 2) (list (save-excursion (insert-before-markers "XY" ?Z) (point)) (point) (buffer-string) buffer-undo-list (multibyte-string-p (buffer-string)) (progn (goto-char (point-max)) (insert-before-markers "e" (string-to-unibyte "\377")) (point)) (append (buffer-string) nil) buffer-undo-list))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abcdef") (narrow-to-region 2 4) (goto-char 2) (list (save-restriction (insert-before-markers "q") (list (point-min) (point-max) (point))) (point-min) (point-max) (point) (progn (goto-char (point-max)) (save-restriction (insert-before-markers "r")) (list (point-min) (point-max) (point))) (progn (widen) (buffer-string))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc") (goto-char 2) (list (insert-before-markers-and-inherit "X") (point) (buffer-string) (progn (goto-char 1) (insert-before-markers) (point)) (buffer-modified-tick)))
"###;

/// Cases whose change hooks run Lisp, which needs the Lisp prelude.
const PRELUDE_CASES: &str = r###"
(with-temp-buffer (buffer-enable-undo) (insert "abc") (undo-boundary) (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l) log))))) (list (base64-encode-region 1 4) (buffer-string) (reverse log) buffer-undo-list (progn (setq log nil) (base64-decode-region 1 5)) (reverse log) (buffer-string))))
(progn (defun cm-d () "doc" (interactive nil d-mode e-mode) 1) (defun cm-e (x) (interactive "p") x) (list (command-modes 'cm-d) (interactive-form 'cm-d) (commandp 'cm-d) (command-modes 'cm-e)))
(with-temp-buffer (insert "hello world") (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e (buffer-string)) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l (buffer-string)) log))))) (list (replace-region-contents 1 12 "jello wurld") (reverse log) (progn (setq log nil) (replace-region-contents 2 4 "XYZ" 0)) (reverse log) (progn (setq log nil) (let ((inhibit-modification-hooks t)) (replace-region-contents 1 4 "abc"))) log)))
(with-temp-buffer (insert "x x") (goto-char 3) (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l (buffer-string)) log))))) (list (base64-decode-region 2 3) (reverse log) (buffer-string) (point) (progn (setq log nil) (base64-decode-region 2 2)) (reverse log) (buffer-string))))
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

/// The octets GNU decodes into a multibyte buffer go in as raw-byte
/// characters (VM2-RAWTEXT1). Pinned GNU: (2 "x\377\357" 2 t).
#[test]
fn base64_decode_region_puts_raw_bytes_in_a_multibyte_buffer() {
    let source = "(prin1-to-string (with-temp-buffer (insert \"x_-8\") (goto-char 2) (list (condition-case e (base64-decode-region 2 5 t) (error e)) (buffer-string) (point) buffer-undo-list)))";
    let actual = Vm::new().eval_source(source).and_then(|v| v.display()).unwrap();
    let printed: String = serde_json::from_str(&actual).unwrap();
    assert_eq!(printed, "(2 \"x\\377\\357\" 2 t)");
}


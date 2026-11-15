;;; romacs-lint-tests.el --- The lint holds Lisp to the machine  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each test lints a file written for it and reads the findings: what the
;; lint must say, and what it must leave alone.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-lint)

(defun romacs-lint-tests--lint (&rest lines)
  "Lint a file of LINES; return its findings as \"LEVEL: TEXT\" strings."
  (let ((file (make-temp-file "romacs-lint-tests-" nil ".el" (concat (string-join lines "\n") "\n"))))
    (unwind-protect
        (mapcar (lambda (finding) (format "%s: %s" (nth 1 finding) (nth 2 finding)))
                (cdar (romacs-lint-files (list file))))
      (delete-file file))))

(defun romacs-lint-tests--some (regexp findings)
  "Non-nil when one of FINDINGS matches REGEXP."
  (cl-some (lambda (finding) (string-match-p regexp finding)) findings))

(defconst romacs-lint-tests--cookie ";;; x.el --- x  -*- lexical-binding: t -*-")

(ert-deftest romacs-lint-tests-unknown-function ()
  "A call nothing defines is an error that names the nearest function."
  (let ((findings (romacs-lint-tests--lint
                   romacs-lint-tests--cookie
                   "(defun romacs-lint-tests--a (x) (strng-trim x))")))
    (should (romacs-lint-tests--some "^error: in romacs-lint-tests--a: `strng-trim' is not defined.*did you mean `string-trim'" findings))
    (should (= (length findings) 1))))

(ert-deftest romacs-lint-tests-known-functions ()
  "The file's own functions, guarded calls and quiet bodies are not findings."
  (should-not (romacs-lint-tests--lint
               romacs-lint-tests--cookie
               "(defun romacs-lint-tests--b (x) (romacs-lint-tests--c x))"
               "(defun romacs-lint-tests--c (x)"
               "  (when (fboundp 'romacs-lint-tests--nowhere) (romacs-lint-tests--nowhere x))"
               "  (with-no-warnings (romacs-lint-tests--nowhere-else x)))")))

(ert-deftest romacs-lint-tests-guards ()
  "A guard holds where its condition decides the branch: the else of a
negation, a loop's body, the rest of an `or'.  It names a function or a
variable, not both, and excuses only that the function is not defined."
  (should-not (romacs-lint-tests--lint
               romacs-lint-tests--cookie
               "(defun romacs-lint-tests--n (x)"
               "  (if (not (fboundp 'romacs-lint-tests--nowhere)) nil (romacs-lint-tests--nowhere x))"
               "  (while (fboundp 'romacs-lint-tests--nowhere) (romacs-lint-tests--nowhere x))"
               "  (or (not (fboundp 'romacs-lint-tests--nowhere)) (romacs-lint-tests--nowhere x))"
               "  (cond ((not (boundp 'romacs-lint-tests--unset)) nil) (t romacs-lint-tests--unset)))"))
  (let ((findings (romacs-lint-tests--lint
                   romacs-lint-tests--cookie
                   "(defun romacs-lint-tests--o (x)"
                   "  (when (boundp 'romacs-lint-tests--nowhere) (romacs-lint-tests--nowhere x))"
                   "  (when (fboundp 'romacs-lint-tests--unset) romacs-lint-tests--unset)"
                   "  (when (fboundp 'car) (car x x)))")))
    (should (romacs-lint-tests--some "`romacs-lint-tests--nowhere' is not defined" findings))
    (should (romacs-lint-tests--some "`romacs-lint-tests--unset' is a free variable" findings))
    (should (romacs-lint-tests--some "`car' called with 2 arguments" findings))
    (should (= (length findings) 3)))
  ;; A guard is around one call, not every call of the function.
  (should (equal (length (romacs-lint-tests--lint
                          romacs-lint-tests--cookie
                          "(defun romacs-lint-tests--s (x)"
                          "  (when (fboundp 'romacs-lint-tests--nowhere) (romacs-lint-tests--nowhere x))"
                          "  (romacs-lint-tests--nowhere x))"))
                 1)))

(ert-deftest romacs-lint-tests-a-variable-is-no-function ()
  "A variable the file defines does not define a function of its name."
  (should (equal (romacs-lint-tests--lint
                  romacs-lint-tests--cookie
                  "(defvar romacs-lint-tests--only-a-variable nil)"
                  "(defun romacs-lint-tests--p () (romacs-lint-tests--only-a-variable))")
                 '("error: in romacs-lint-tests--p: `romacs-lint-tests--only-a-variable' is not defined: not by this machine, this file, or what it requires; fix: require its library, or correct the name"))))

(ert-deftest romacs-lint-tests-interactive-arguments ()
  "The code that reads a command's arguments is held as any other."
  (should (equal (romacs-lint-tests--lint
                  romacs-lint-tests--cookie
                  "(defun romacs-lint-tests--q (x) (interactive (list (romacs-lint-tests--no-reader))) x)"
                  "(defun romacs-lint-tests--r (x) (interactive \"p\") x)")
                 '("error: in romacs-lint-tests--q: `romacs-lint-tests--no-reader' is not defined: not by this machine, this file, or what it requires; fix: require its library, or correct the name"))))

(ert-deftest romacs-lint-tests-required-as-it-starts ()
  "A function that requires its library in its body may call into it."
  (should-not (romacs-lint-tests--lint
               romacs-lint-tests--cookie
               "(defun romacs-lint-tests--d (x) (require 'ring) (ring-length x))")))

(ert-deftest romacs-lint-tests-argument-count ()
  "A call with a count the function does not take is an error."
  (let ((findings (romacs-lint-tests--lint
                   romacs-lint-tests--cookie
                   "(defun romacs-lint-tests--e (x) (car x x))"
                   "(defun romacs-lint-tests--f (x) (romacs-lint-tests--e))")))
    (should (romacs-lint-tests--some "^error: in romacs-lint-tests--e: `car' called with 2 arguments, takes (1 \\. 1)" findings))
    (should (romacs-lint-tests--some "^error: in romacs-lint-tests--f: `romacs-lint-tests--e' called with 0 arguments" findings))
    (should (= (length findings) 2))))

(ert-deftest romacs-lint-tests-declare-function ()
  "A declaration must be true: the file, and the argument list."
  (let ((findings (romacs-lint-tests--lint
                   romacs-lint-tests--cookie
                   "(declare-function ring-length \"ring\" (ring))"
                   "(declare-function car \"ring\" (list))"
                   "(declare-function ring-size \"ring\" (ring extra))"
                   "(declare-function romacs-lint-tests--absent \"ring\")"
                   "(declare-function romacs-lint-tests--lost \"romacs-lint-tests-no-library\")"
                   "(declare-function ring-empty-p \"romacs-lint-tests-no-library\" (ring))"
                   "(declare-function ring-insert \"subr-x\" (ring item))"
                   "(declare-function romacs-lint-tests--outside \"ext:somewhere\")")))
    (should (romacs-lint-tests--some "^error: declare-function `car' names \"ring\", but it is a primitive" findings))
    (should (romacs-lint-tests--some "^error: declare-function `ring-size' gives arguments (ring extra), but it takes (1 \\. 1)" findings))
    (should (romacs-lint-tests--some "^error: declare-function `romacs-lint-tests--absent': loading \"ring\" does not define it" findings))
    (should (romacs-lint-tests--some "^error: declare-function `romacs-lint-tests--lost' names \"romacs-lint-tests-no-library\", which is not on the load path" findings))
    (should (romacs-lint-tests--some "^error: declare-function `ring-empty-p' names \"romacs-lint-tests-no-library\", but ring defines it" findings))
    (should (romacs-lint-tests--some "^error: declare-function `ring-insert' names \"subr-x\", but ring defines it" findings))
    (should (= (length findings) 6))))

(ert-deftest romacs-lint-tests-variables ()
  "A free variable and an unused lexical one are warnings; a declared
variable and an underscored one are not."
  (let ((findings (romacs-lint-tests--lint
                   romacs-lint-tests--cookie
                   "(defvar romacs-lint-tests--dynamic)"
                   "(defun romacs-lint-tests--g (x)"
                   "  (let ((unused 1) (_ignored 2) (romacs-lint-tests--dynamic 3))"
                   "    (list x romacs-lint-tests--free)))")))
    (should (romacs-lint-tests--some "^warning: in romacs-lint-tests--g: `romacs-lint-tests--free' is a free variable" findings))
    (should (romacs-lint-tests--some "^warning: in romacs-lint-tests--g: lexical variable `unused' is never used.*(defvar unused)" findings))
    (should (= (length findings) 2))))

(ert-deftest romacs-lint-tests-cookie ()
  "A file that does not say how it binds is a warning: the machines differ."
  (should (romacs-lint-tests--some "^warning: no lexical-binding cookie" (romacs-lint-tests--lint ";;; x.el --- x" "(defvar romacs-lint-tests--h nil)")))
  (should (romacs-lint-tests--some "^warning: lexical-binding: nil" (romacs-lint-tests--lint ";;; x.el --- x  -*- lexical-binding: nil -*-"))))

(ert-deftest romacs-lint-tests-docstring-width ()
  "A docstring line that could be wrapped and is over 80 columns is a
warning; one long word, and the argument line, are not."
  (let ((wide (make-string 41 ?a)))
    (should (equal (romacs-lint-tests--lint
                    romacs-lint-tests--cookie
                    (format "(defvar romacs-lint-tests--k nil \"Short.\n%s %s\")" wide wide)
                    (format "(defvar romacs-lint-tests--l nil \"Short.\n%s%s\")" wide wide)
                    (format "(defun romacs-lint-tests--m (x) \"Short.\n\n(fn %s %s)\" x)" wide wide))
                   '("warning: in romacs-lint-tests--k: docstring has a line wider than 80 columns; fix: wrap it")))))

(ert-deftest romacs-lint-tests-unreadable ()
  "A file that stops reading says where."
  (should (equal (romacs-lint-tests--lint romacs-lint-tests--cookie "(defvar romacs-lint-tests--i nil)" "(defun romacs-lint-tests--i (x)")
                 '("error: does not read: the form at line 3: the file ends inside it; fix: balance the form"))))

(ert-deftest romacs-lint-tests-declared-hint ()
  "A hint its owner put on a function reaches each caller in another file."
  (defalias 'romacs-lint-tests--slow #'identity)
  (put 'romacs-lint-tests--slow 'romacs-lint-hint "the machine serves this another way")
  (unwind-protect
      (should (equal (romacs-lint-tests--lint
                      romacs-lint-tests--cookie
                      "(defun romacs-lint-tests--j (x) (romacs-lint-tests--slow x))")
                     '("hint: in romacs-lint-tests--j: `romacs-lint-tests--slow': the machine serves this another way")))
    (put 'romacs-lint-tests--slow 'romacs-lint-hint nil)
    (fmakunbound 'romacs-lint-tests--slow)))

(ert-deftest romacs-lint-tests-stateless ()
  "Work that reaches only pure primitives is stateless; a read of the
machine's state under it is not."
  (skip-unless (fboundp 'romacs--subr-effect))
  (should (eq (romacs--subr-effect (symbol-function 'subr-arity)) 'pure))
  (should (eq (romacs--subr-effect (symbol-function 'romacs--rust-definition)) 'reads))
  (should (romacs-lint--stateless-p 'outer '((outer inner subr-arity) (inner subr-name outer))))
  (should-not (romacs-lint--stateless-p 'outer '((outer inner) (inner romacs--rust-definition))))
  (should-not (romacs-lint--stateless-p 'outer '((outer inner subr-arity) (inner romacs-lint--state)))))

(ert-deftest romacs-lint-tests-state-in-a-body ()
  "A body that reads, writes or binds a variable not its own, or saves the
buffer's state, reaches the machine's state; one that keeps to its own
variables does not."
  (cl-flet ((callees (source)
              (let (events)
                (romacs-lint--walk (romacs-lint--expand (read source)) nil nil (make-hash-table) (lambda (event) (push event events)))
                (romacs-lint--inner-callees (nreverse events)))))
    (should (equal (callees "(defun f (x) (let ((y (car x))) (setq y (cdr y)) y))") '(car cdr)))
    (should (memq 'romacs-lint--state (callees "(defun f (x) (car (list x fill-column)))")))
    (should (memq 'romacs-lint--state (callees "(defun f (x) (setq fill-column x))")))
    (should (memq 'romacs-lint--state (callees "(defun f (x) (let ((fill-column x)) (car x)))")))
    (should (memq 'romacs-lint--state (callees "(defun f (x) (save-excursion (car x)))")))
    (should (memq 'romacs-lint--state (callees "(defun f (fill-column) (car fill-column))")))))

(ert-deftest romacs-lint-tests-struct-layout ()
  "A struct laid out other than its base is a hint; the same layout is not."
  (let ((base (car (romacs-lint--read "(cl-defstruct (thing (:copier nil)) \"A thing.\" a (b 1) c)")))
        (same (car (romacs-lint--read "(cl-defstruct thing a (b 2 :read-only t) c)")))
        (moved (car (romacs-lint--read "(cl-defstruct thing a new b c)"))))
    (should-not (romacs-lint--layout-changes base same))
    (should (string-match-p "cl-defstruct `thing' changes layout (was (a b c), now (a new b c))"
                            (nth 2 (car (romacs-lint--layout-changes base moved)))))))

(ert-deftest romacs-lint-tests-line-pins ()
  "A string naming a line of a file of the tree is a warning; of any other
file, or a line of nothing, is not."
  (let ((root (file-name-as-directory (make-temp-file "romacs-lint-tests-root-" t))))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "crates/a/src" root) t)
          (write-region "" nil (expand-file-name "crates/a/src/lib.rs" root) nil 'silent)
          (let ((pins (romacs-lint--line-pins "x\n(\"crates/a/src/lib.rs:12\" \"src/lib.rs:3\" \"crates/a/src/gone.rs:4\")\n" root)))
            (should (equal (mapcar #'car pins) '(2)))
            (should (string-match-p "pins a line of crates/a/src/lib.rs" (nth 2 (car pins))))))
      (delete-directory root t))))

(ert-deftest romacs-lint-tests-the-loadup-bootstrap ()
  "The lint gives its findings on the file that boots the machine, which
it loads into a machine already booted.  It runs in a machine of its own:
loading that file leaves the one that loads it changed."
  (let* ((root (locate-dominating-file (or load-file-name buffer-file-name default-directory) "Cargo.lock"))
         (host (expand-file-name invocation-name invocation-directory))
         (file (expand-file-name "lisp/session/romacs-loadup-bootstrap.el" root))
         (status nil))
    (skip-unless (and root (file-exists-p file)))
    (with-temp-buffer
      (setq status (call-process host nil t nil "-Q" "--batch" "-L" (expand-file-name "lisp/romacs" root)
                                 "-l" "romacs-lint" "-f" "romacs-lint-batch" file))
      (should (memq status '(0 1)))
      (goto-char (point-min))
      (should (re-search-forward "^romacs-lint: 1 file, " nil t)))))

(ert-deftest romacs-lint-tests-each-file-alone ()
  "tools/romacs-lint judges each file in a machine of its own, working in a
private home: a file that empties `load-path' leaves the next file's
judgement alone, and a file that writes where it runs writes nothing into
the tree."
  (let* ((root (locate-dominating-file (or load-file-name buffer-file-name default-directory) "Cargo.lock"))
         (host (expand-file-name invocation-name invocation-directory))
         (dir (file-name-as-directory (make-temp-file "romacs-lint-tests-alone-" t)))
         (poison (expand-file-name "a-poison.el" dir))
         (plain (expand-file-name "b-plain.el" dir))
         (status nil))
    (skip-unless (and root (executable-find "python3")))
    (unwind-protect
        (progn
          (with-temp-file poison
            (insert romacs-lint-tests--cookie "\n(setq load-path nil)\n(write-region \"\" nil \"romacs-lint-tests-written\")\n"))
          (with-temp-file plain
            (insert romacs-lint-tests--cookie "\n(require 'ring)\n(defun romacs-lint-tests--n (x) (ring-length x))\n"))
          (with-temp-buffer
            (let ((default-directory root))
              (setq status (call-process "python3" nil t nil "-B" "tools/romacs-lint" "--binary" host "--all" poison plain)))
            (should (eql status 0))
            (should (string-match-p "^romacs-lint: 2 files, 0 errors" (buffer-string))))
          (should-not (file-exists-p (expand-file-name "romacs-lint-tests-written" root))))
      (delete-directory dir t))))

(ert-deftest romacs-lint-tests-arglist-arity ()
  "An argument list's arity, as `func-arity' would give its function's."
  (should (equal (romacs-lint--arglist-arity '(a b)) '(2 . 2)))
  (should (equal (romacs-lint--arglist-arity '(a &optional b c)) '(1 . 3)))
  (should (equal (romacs-lint--arglist-arity '(&rest all)) '(0 . many)))
  (should (equal (romacs-lint--arglist-arity '(a &key b)) '(1 . many))))

(provide 'romacs-lint-tests)
;;; romacs-lint-tests.el ends here

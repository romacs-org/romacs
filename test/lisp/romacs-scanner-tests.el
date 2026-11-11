;;; romacs-scanner-tests.el --- Scanners tabulated from a step function -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'ert)
(require 'romacs-scanner)

(defun romacs-scanner-tests--step (state char)
  "A small C lexer: mark the semicolons that are code."
  (pcase state
    ('code (pcase char (?\; '(code . t)) (?/ '(slash)) (?\" '(string)) (_ '(code))))
    ('slash (pcase char (?/ '(line)) (?* '(block)) (?\; '(code . t)) (?\" '(string)) (_ '(code))))
    ('line (if (eq char ?\n) '(code) '(line)))
    ('block (if (eq char ?*) '(star) '(block)))
    ('star (pcase char (?/ '(code)) (?* '(star)) (_ '(block))))
    ('string (pcase char (?\\ '(escape)) (?\" '(code)) (?\n '(code)) (_ '(string))))
    (_ '(string))))

(define-scanner romacs-scanner-tests--semicolons #'romacs-scanner-tests--step 'code
  "Semicolons outside strings and comments.")

(defun romacs-scanner-tests--by-hand (start end state)
  "What the step function itself answers from START to END, one call a character."
  (let ((marked nil))
    (while (< start end)
      (let* ((char (char-after start))
             (answer (romacs-scanner-tests--step state (if (< char 128) char romacs-scanner-other))))
        (when (cdr answer) (push start marked))
        (setq state (car answer) start (1+ start))))
    (cons state (nreverse marked))))

(defconst romacs-scanner-tests--text
  "int a; /* b; \"c;\" */ char *s = \"d;\\\"e;\"; // f;\nint g; /* h\n*/ i;\n")

(ert-deftest romacs-scanner-tabulates-the-reachable-states ()
  (should (romacs-scanner-p romacs-scanner-tests--semicolons))
  (should (equal (romacs-scanner-states romacs-scanner-tests--semicolons)
                 '(code string slash escape block line star))))

(ert-deftest romacs-scanner-answers-as-its-step-function ()
  (with-temp-buffer
    (insert romacs-scanner-tests--text)
    (let ((by-hand (romacs-scanner-tests--by-hand (point-min) (point-max) 'code)))
      (should (equal (cdr by-hand) '(6 40 53 64)))
      (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons) by-hand))
      (should (equal (romacs-scanner-count romacs-scanner-tests--semicolons) '(code . 4)))
      ;; The table walked in Lisp answers as the primitive does.
      (should (equal (romacs-scanner--walk (aref romacs-scanner-tests--semicolons 1) 0 (point-min) (point-max) nil)
                     (cons 0 (cdr by-hand)))))))

(ert-deftest romacs-scanner-starts-anywhere-in-any-state ()
  (with-temp-buffer
    (insert romacs-scanner-tests--text)
    ;; From inside the block comment, and ending inside the string.
    (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons 10 30 'block)
                   (romacs-scanner-tests--by-hand 10 30 'block)))
    (should (eq (car (romacs-scanner-scan romacs-scanner-tests--semicolons 1 33)) 'string))
    (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons 5 5) '(code)))
    (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons (copy-marker 1) (copy-marker 7))
                   '(code 6)))
    (should-error (romacs-scanner-scan romacs-scanner-tests--semicolons 1 2 'no-such-state)
                  :type 'wrong-type-argument)))

(ert-deftest romacs-scanner-respects-narrowing ()
  (with-temp-buffer
    (insert romacs-scanner-tests--text)
    (narrow-to-region 8 41)
    (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons nil nil 'block)
                   (romacs-scanner-tests--by-hand 8 41 'block)))
    (should-error (romacs-scanner-scan romacs-scanner-tests--semicolons 1 41) :type 'args-out-of-range)
    (should-error (romacs-scanner-scan romacs-scanner-tests--semicolons 8 42) :type 'args-out-of-range)
    (should-error (romacs-scanner-scan romacs-scanner-tests--semicolons 20 10) :type 'args-out-of-range)))

(ert-deftest romacs-scanner-counts-characters-not-bytes ()
  ;; Positions after non-ASCII characters, which are several bytes each
  ;; and all one symbol to the step function.
  (with-temp-buffer
    (insert "é; \"ü;\" λλ; /* 日本; */ ;")
    (let ((by-hand (romacs-scanner-tests--by-hand (point-min) (point-max) 'code)))
      (should (equal (cdr by-hand) '(2 11 23)))
      (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons) by-hand))
      (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons 9 12) '(code 11))))))

(ert-deftest romacs-scanner-reads-unibyte-text ()
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert "a;\300;\200\"\377;\";")
    (should (equal (romacs-scanner-scan romacs-scanner-tests--semicolons)
                   (romacs-scanner-tests--by-hand (point-min) (point-max) 'code)))))

(ert-deftest romacs-scanner-marks-the-other-character ()
  (let ((non-ascii (romacs-scanner-make (lambda (_state char) (cons nil (eq char romacs-scanner-other))) nil)))
    (with-temp-buffer
      (insert "aéb日c")
      (should (equal (romacs-scanner-scan non-ascii) '(nil 2 4))))))

(ert-deftest romacs-scanner-refuses-what-is-not-a-finite-pure-function ()
  ;; A state for every character seen: not finite.
  (should-error (romacs-scanner-make (lambda (state _char) (list (cons 'x state))) nil))
  ;; An answer that depends on something besides its arguments.
  (let ((calls 0))
    (should-error (romacs-scanner-make (lambda (_state _char) (list (setq calls (1+ calls)))) 0)))
  ;; An answer that is not (STATE . MARK).
  (should-error (romacs-scanner-make (lambda (_state _char) 'code) 'code)))

(provide 'romacs-scanner-tests)
;;; romacs-scanner-tests.el ends here

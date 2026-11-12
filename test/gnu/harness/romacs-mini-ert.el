;;; romacs-mini-ert.el --- REDUCED test harness; this is NOT GNU's ert.el  -*- lexical-binding: t -*-

;; WHAT THIS IS.  A deliberately small stand-in for the part of GNU's
;; `lisp/emacs-lisp/ert.el' that a test FILE touches: `ert-deftest' and the
;; assertion macros.  It exists only because real ERT cannot load on romacs
;; yet: ert.el requires cl-lib, debug, backtrace, ewoc, find-func, pp, map and
;; help-mode, and cl-lib is still being unblocked.  A test runner is harness,
;; not a Lisp-visible feature, so a reduced harness is legitimate; it is never
;; shipped to users and never loaded outside `gnu-tests/run.py'.
;;
;; WHAT IT IS NOT.  Results produced under this harness are reported with
;; "harness": "romacs-mini-ert" in every row and in the summary, and must
;; never be quoted as "passes GNU's ERT".  Loading GNU's real ert.el is the
;; target; `run.py --harness real' does that and reports that it cannot load.
;;
;; FIDELITY.  `run.py --harness-fidelity' runs THIS FILE inside real GNU Emacs
;; against the same test files and compares every verdict with real ERT's.
;; Any difference is a harness defect and is reported, never absorbed.
;;
;; Semantics copied from GNU ert.el at the pin (45b0f7699df):
;;   ert-deftest   docstring, then :expected-result / :tags / :documentation
;;                 keyword pairs, then BODY; :expected-result is evaluated at
;;                 definition time (ert.el `ert-deftest').  Redefinition
;;                 replaces the test in place.
;;   should        signals `ert-test-failed' when FORM is nil; returns FORM.
;;   should-not    signals `ert-test-failed' when FORM is non-nil.
;;   should-error  signals `ert-test-failed' unless FORM signals; with :type
;;                 the signal's `error-conditions' must intersect TYPE; with
;;                 :exclude-subtypes the condition itself must be in TYPE.
;;                 Returns the condition (ERROR-SYMBOL . DATA).
;;   skip-unless / skip-when / ert-skip   signal `ert-test-skipped'.
;;   ert-fail      signals `ert-test-failed'.
;; Anything else from ert.el/ert-x.el is ABSENT on purpose; a test that uses
;; it errors with void-function and the runner reports it as harness-limited
;; (not-yet-runnable), never as a romacs failure or a pass.

(defvar romacs-gnu-tests-harness "romacs-mini-ert")
(defvar romacs-gnu-tests--registry nil
  "Registered tests, oldest first: each (NAME FUNCTION EXPECTED-RESULT).")
(defvar romacs-gnu-tests--last nil
  "(OUTCOME . DATA) of the most recent `romacs-gnu-tests--run'.")

;; GNU: (define-error 'ert-test-failed "Test failed") and likewise skipped.
(put 'ert-test-failed 'error-conditions '(ert-test-failed error))
(put 'ert-test-failed 'error-message "Test failed")
(put 'ert-test-skipped 'error-conditions '(ert-test-skipped error))
(put 'ert-test-skipped 'error-message "Test skipped")

(defun romacs-gnu-tests--register (name function expected)
  (let ((cell (assq name romacs-gnu-tests--registry)))
    (if cell
        (setcdr cell (list function expected))
      (setq romacs-gnu-tests--registry
            (append romacs-gnu-tests--registry
                    (list (list name function expected))))))
  name)

;; A private test, not a definition of `keywordp': the harness must never
;; supply a GNU function romacs lacks, or it would hide that gap from the
;; very tests that measure it.
(defun romacs-gnu-tests--keyword-p (x)
  (and (symbolp x) x (not (eq x t))
       (let ((name (symbol-name x)))
         (and (> (length name) 0) (eq (aref name 0) ?:)))))

(defmacro ert-deftest (name _arglist &rest docstring-keys-and-body)
  (let ((body docstring-keys-and-body)
        (expected :passed))
    (if (and (stringp (car body)) (cdr body))
        (setq body (cdr body)))
    (while (romacs-gnu-tests--keyword-p (car body))
      (if (eq (car body) :expected-result)
          (setq expected (car (cdr body))))
      (setq body (cdr (cdr body))))
    (list 'romacs-gnu-tests--register
          (list 'quote name)
          (cons 'lambda (cons nil body))
          expected)))

(defun romacs-gnu-tests--fail (data)
  (signal 'ert-test-failed (list data)))

(defmacro should (form)
  (list 'romacs-gnu-tests--should (list 'quote form) form))
(defun romacs-gnu-tests--should (form value)
  (if value value (romacs-gnu-tests--fail (list 'should form :value value))))

(defmacro should-not (form)
  (list 'romacs-gnu-tests--should-not (list 'quote form) form))
(defun romacs-gnu-tests--should-not (form value)
  (if value (romacs-gnu-tests--fail (list 'should-not form :value value)) nil))

(defmacro should-error (form &rest keys)
  (let ((type nil) (exclude nil))
    (while keys
      (cond ((eq (car keys) :type) (setq type (car (cdr keys))))
            ((eq (car keys) :exclude-subtypes) (setq exclude (car (cdr keys)))))
      (setq keys (cdr (cdr keys))))
    (list 'romacs-gnu-tests--should-error
          (list 'quote form) (list 'lambda nil form) type exclude)))

(defun romacs-gnu-tests--should-error (form thunk type exclude-subtypes)
  (let ((caught nil) (signalled nil))
    (condition-case err
        (funcall thunk)
      (error (setq caught err signalled t)))
    (if (not signalled)
        (romacs-gnu-tests--fail
         (list 'should-error form :value nil
               :fail-reason "did not signal an error")))
    (if type
        (let ((types (if (listp type) type (list type)))
              (conditions (get (car caught) 'error-conditions))
              (hit nil))
          (while (and types (not hit))
            (if (memq (car types) conditions) (setq hit t))
            (setq types (cdr types)))
          (if (not hit)
              (romacs-gnu-tests--fail
               (list 'should-error form :condition caught
                     :fail-reason "the error signaled did not have the expected type")))
          (if (and exclude-subtypes
                   (not (memq (car caught) (if (listp type) type (list type)))))
              (romacs-gnu-tests--fail
               (list 'should-error form :condition caught
                     :fail-reason "the error signaled was a subtype of the expected type")))))
    caught))

(defun ert-skip (data) (signal 'ert-test-skipped (list data)))
;; GNU ert.el:740 expands to (let ((ert--infos (cons (cons PREFIX-FORM
;; MESSAGE-FORM) ert--infos))) BODY...): both forms are EVALUATED, prefix
;; first, before BODY, and only their values are kept for a failure report.
;; Evaluate them the same way and discard the values; dropping them would let
;; a romacs failure inside MESSAGE-FORM pass unseen.
(defmacro ert-info (message-and-keys &rest body)
  (let ((message-form (car message-and-keys))
        (prefix-form "Info: ")
        (keys (cdr message-and-keys)))
    (while keys
      (if (eq (car keys) :prefix)
          (setq prefix-form (car (cdr keys))))
      (setq keys (cdr (cdr keys))))
    (cons 'progn (cons prefix-form (cons message-form body)))))
(defun ert-fail (data) (signal 'ert-test-failed (list data)))
(defmacro skip-unless (form)
  (list 'if form nil (list 'ert-skip (list 'quote (list 'skip-unless form)))))
(defmacro skip-when (form)
  (list 'if form (list 'ert-skip (list 'quote (list 'skip-when form)))))

;;; Runner interface, called by the romacs-gnu-ert adapter.

(defun romacs-gnu-tests--count () (length romacs-gnu-tests--registry))
(defun romacs-gnu-tests--name (i)
  (symbol-name (car (nth i romacs-gnu-tests--registry))))
(defun romacs-gnu-tests--expected (i)
  (let ((e (nth 2 (nth i romacs-gnu-tests--registry))))
    (cond ((eq e :passed) ":passed")
          ((eq e :failed) ":failed")
          ((eq e t) "t")
          (t "unsupported"))))

(defun romacs-gnu-tests--run (i)
  (setq romacs-gnu-tests--last
        (condition-case err
            (progn (funcall (nth 1 (nth i romacs-gnu-tests--registry)))
                   (list 'passed))
          (ert-test-failed (cons 'failed (cdr err)))
          (ert-test-skipped (cons 'skipped (cdr err)))
          (error (cons 'errored err))))
  nil)

;; Test files say (require 'ert).  Satisfy it with THIS harness, so romacs
;; does not try to load real ert.el, which cannot load yet.  The substitution
;; is recorded in every result row (see the header).
(provide 'ert)

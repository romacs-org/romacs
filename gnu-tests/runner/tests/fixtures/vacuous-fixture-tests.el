;;; vacuous-fixture-tests.el --- fixtures for the should-error audit  -*- lexical-binding: t -*-

;; NOT a GNU test.  `gts-fixture-void' is nil unless a test of the runner sets
;; it (romacs only, through --probe): then `gts-fixture-signal' is never
;; defined, and `fixture-should-error-real' is satisfied by void-function in
;; romacs while GNU caught a real `error' -- a vacuous pass the runner must
;; not count.

(require 'ert)

(defvar gts-fixture-void nil)

;; `signal', not `error': romacs's `error' needs `format-message', which is
;; absent today, and the audit rightly reports THAT as a vacuous catch.
(unless gts-fixture-void
  (defun gts-fixture-signal () (signal 'error (list "A real error"))))

(ert-deftest fixture-should-error-real ()
  (should-error (gts-fixture-signal)))

(ert-deftest fixture-should-error-typed-void ()
  ;; Asks for void-function: the catch is the subject, not a gap.
  (should-error (gts-fixture-absent-everywhere) :type 'void-function))

(ert-deftest fixture-no-should-error ()
  (should (equal (car '(1)) 1)))

;;; vacuous-fixture-tests.el ends here

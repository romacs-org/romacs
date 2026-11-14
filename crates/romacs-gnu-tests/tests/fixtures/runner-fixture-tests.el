;;; runner-fixture-tests.el --- fixtures for the runner's own tests  -*- lexical-binding: t -*-

;; NOT a GNU test.  Exercises each verdict the runner must report, in GNU and
;; in romacs alike.  `gts-fixture-mutant' is nil unless a test of the runner
;; deliberately sets it, which turns `fixture-genuine' into a real failure.

(require 'ert)

(defvar gts-fixture-mutant nil)

(ert-deftest fixture-passes ()
  (should (equal (car '(1 2)) 1))
  (should-not (cdr '(1)))
  (should-error (car 1) :type 'wrong-type-argument))

(ert-deftest fixture-genuine ()
  (should-not gts-fixture-mutant))

(ert-deftest fixture-fails ()
  (should (equal (car '(1 2)) 2)))

(ert-deftest fixture-errors ()
  (signal 'wrong-type-argument (list 'fixture 1)))

(ert-deftest fixture-skips ()
  (skip-unless nil))

(ert-deftest fixture-expected-failure ()
  :expected-result :failed
  (should nil))

(ert-deftest fixture-wrong-should-error-type ()
  (should-error (car 1) :type 'void-variable))

(ert-deftest fixture-ert-info-evaluates-its-message ()
  ;; GNU evaluates MESSAGE-FORM (ert.el:740); an error there is the test's.
  (ert-info ((signal 'wrong-type-argument (list 'fixture-info 1)) :prefix "Info: ")
    (should t)))

;;; unreadable-fixture-tests.el --- GNU-positive reader controls  -*- lexical-binding: t -*-

;; NOT a GNU test. GNU and Romacs define and pass both valid record tests.
;; The runner's negative reader test corrupts the adapter-only extracted
;; definitions to #s() after GNU recording; it must report NOT-YET-RUNNABLE,
;; never passing. The unchanged fixture is also a positive capability control.

(require 'ert)

(ert-deftest fixture-unreadable-1 ()
  (should (recordp #s(fixture-record 1))))

(ert-deftest fixture-unreadable-2 ()
  (should (equal (aref #s(fixture-record 1 2) 2) 2)))

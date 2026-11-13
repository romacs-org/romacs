;;; hang-fixture-tests.el --- a test that never returns  -*- lexical-binding: t -*-

;; NOT a GNU test.  The runner must report `fixture-hangs' as a timeout and
;; still run `fixture-after-hang'.

(require 'ert)

(ert-deftest fixture-hangs ()
  (while t))

(ert-deftest fixture-after-hang ()
  (should (eq 'a 'a)))

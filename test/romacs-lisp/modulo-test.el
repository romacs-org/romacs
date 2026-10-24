;;; modulo-test.el --- Arithmetic prerequisite for GNU kill-ring rotation -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'romacs-test)

(romacs-deftest modulo-gnu-signs-and-numeric-types ()
  (romacs-should
   (equal (list (mod 5 3) (mod -5 3) (mod 5 -3) (mod -5 -3)
                (mod 5.5 3) (mod -5.5 3.0)
                (mod (* 123456789012345 1000000000000000) 97))
          '(2 1 -1 -2 2.5 0.5 55))))

(romacs-deftest modulo-markers-and-errors ()
  (with-temp-buffer
    (insert "abc")
    (romacs-should (= (mod (copy-marker 3) 2) 1)))
  (romacs-should-error (mod 1 0) :type 'arith-error)
  (romacs-should-error (mod "bad" 2) :type 'wrong-type-argument))

(provide 'modulo-test)

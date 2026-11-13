;;; value-order-test.el --- GNU standard value ordering -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(romacs-deftest value-order-default-sort-uses-standard-comparison ()
  (romacs-should
   (equal (condition-case err
              (sort (list "b" "a"))
            (error err))
          '("a" "b"))))

(romacs-deftest value-order-default-sort-handles-nested-values ()
  (romacs-should
   (equal (sort (list '(1 b) '(1 a) nil))
          '(nil (1 a) (1 b)))))

(romacs-deftest value-order-default-sort-preserves-type-errors ()
  (romacs-should
   (equal (condition-case err
              (sort (list "b" 'x))
            (error err))
          '(type-mismatch x "b"))))

(provide 'value-order-test)

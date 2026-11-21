;;; romacs-setopt-tests.el --- setopt is preloaded, its setter autoloaded -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)

(ert-deftest romacs-setopt-is-a-preloaded-macro ()
  ;; A file that uses `setopt', loaded from source, expands it with
  ;; nothing loaded: cus-edit.el, pp.el and cus-load.el only come with
  ;; setting an option (romacs#403's agents row loaded them at times).
  (should (eq (car-safe (symbol-function 'setopt)) 'macro))
  (should (equal (file-name-base (symbol-file 'setopt)) "custom"))
  (should (equal (macroexpand '(setopt a 1 b 2)) '(progn (setopt--set 'a 1) (setopt--set 'b 2))))
  (should-error (macroexpand '(setopt a)) :type 'wrong-number-of-arguments))

(defcustom romacs-setopt-tests--option 1
  "An option whose setter doubles what it is given."
  :type 'integer
  :set (lambda (symbol value) (set-default symbol (* 2 value)))
  :group 'lisp)

(ert-deftest romacs-setopt-still-sets-through-the-option-s-setter ()
  (let ((romacs-setopt-tests--option romacs-setopt-tests--option))
    (setopt romacs-setopt-tests--option 5)
    (should (= romacs-setopt-tests--option 10))))

(provide 'romacs-setopt-tests)
;;; romacs-setopt-tests.el ends here

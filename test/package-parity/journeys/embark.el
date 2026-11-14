;;; embark.el --- Journey: target finding  -*- lexical-binding: t -*-
(pp-step "require" (require 'embark) (featurep 'embark))
(defun pp-embark-targets (text pos)
  (with-temp-buffer
    (emacs-lisp-mode)
    (insert text)
    (goto-char pos)
    (mapcar (lambda (target) (list (plist-get target :type) (plist-get target :target)))
            (embark--targets))))
(pp-step "url" (pp-embark-targets "see https://example.com/x here" 10))
(pp-step "symbol" (pp-embark-targets "(car list)" 3))
(pp-step "sexp" (pp-embark-targets "(foo (bar baz))" 6))
(pp-step "keymap" (keymapp (embark--action-keymap 'symbol nil)))

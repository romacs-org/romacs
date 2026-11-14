;;; multiple-cursors.el --- Journey: mark all like this, then type  -*- lexical-binding: t -*-
(pp-step "require" (require 'multiple-cursors) (featurep 'multiple-cursors))
(pp-step "edit"
  (with-current-buffer (get-buffer-create "mc")
    (switch-to-buffer (current-buffer))
    (insert "foo bar foo baz foo\n")
    (goto-char (point-min))
    (transient-mark-mode 1)
    (keymap-global-set "C-c m" #'mc/mark-all-like-this)
    (pp-keys "C-SPC M-f C-c m X")
    (list (buffer-string) (mc/num-cursors))))
(pp-step "quit" (pp-keys "<return>") (list (buffer-string) (mc/num-cursors)))

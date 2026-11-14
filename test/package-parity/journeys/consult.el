;;; consult.el --- Journey: consult-line and consult-buffer  -*- lexical-binding: t -*-
(pp-step "require" (require 'consult) (featurep 'consult))
(pp-step "line"
  (with-current-buffer (get-buffer-create "lines")
    (insert "alpha\nbeta\ngamma foo\ndelta\nfoo bar\n")
    (goto-char (point-min))
    (switch-to-buffer (current-buffer))
    (keymap-global-set "C-c l" #'consult-line)
    (let ((consult-preview-key nil)) (pp-keys "C-c l f o o C-n RET"))
    (list (line-number-at-pos) (point))))
(pp-step "buffer"
  (get-buffer-create "pp-other")
  (keymap-global-set "C-c b" #'consult-buffer)
  (let ((consult-preview-key nil)) (pp-keys "C-c b p p - o t h e r RET"))
  (buffer-name (window-buffer)))

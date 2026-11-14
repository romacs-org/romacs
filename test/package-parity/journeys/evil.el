;;; evil.el --- Journey: normal-state edits  -*- lexical-binding: t -*-
(pp-step "require" (require 'evil) (featurep 'evil))
(pp-step "edit"
  (with-current-buffer (get-buffer-create "evil")
    (switch-to-buffer (current-buffer))
    (insert "one two three\nfour five six\n")
    (goto-char (point-min))
    (evil-local-mode 1)
    (let (states)
      (dolist (keys '("w" "dw" "x" "j" "0" "cw" "FOUR <escape>" "u" "dd" "p"))
        (pp-keys keys)
        (push (list keys (buffer-substring-no-properties (point-min) (point-max)) (point) evil-state)
              states))
      (nreverse states))))

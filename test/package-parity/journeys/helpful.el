;;; helpful.el --- Journey: describe a function, variable and command  -*- lexical-binding: t -*-
(pp-step "require" (require 'helpful) (featurep 'helpful))
(pp-step "function" (helpful-function #'mapcar) (list (buffer-name) (pp-buffer-text nil 6)))
(pp-step "variable" (helpful-variable 'fill-column) (list (buffer-name) (pp-buffer-text nil 6)))
(pp-step "command" (helpful-command #'forward-char) (list (buffer-name) (pp-buffer-text nil 6)))

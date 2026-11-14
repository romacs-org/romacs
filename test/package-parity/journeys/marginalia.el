;;; marginalia.el --- Journey: annotations  -*- lexical-binding: t -*-
(pp-step "require" (require 'marginalia) (marginalia-mode 1) marginalia-mode)
(pp-step "classify" (let ((minibuffer-completion-table obarray)) (marginalia-classify-by-prompt)))
(pp-step "symbol" (substring-no-properties (or (marginalia-annotate-symbol "car") "")))
(pp-step "command" (substring-no-properties (or (marginalia-annotate-command "forward-char") "")))
(pp-step "variable" (substring-no-properties (or (marginalia-annotate-variable "fill-column") "")))
(pp-step "face" (substring-no-properties (or (marginalia-annotate-face "bold") "")))

;;; romacs-completion-defaults.el --- Romacs's minibuffer completion defaults  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; --loadup's minibuffer completion is GNU's: `completing-read',
;; `read-file-name' and `read-buffer', displayed by icomplete's
;; `fido-vertical-mode'.  M-x, C-x C-f and C-x b reach it through GNU's
;; `execute-extended-command' and the two readers.
;;
;; This file makes Romacs's choices of the completion options their
;; standard values.  --loadup loads it after romacs-gui-loadup.el and
;; before the user's init, so init overrides any of them as it would a
;; GNU default (with `setopt', Custom, or a plain `setq').  Each value
;; becomes the option's `standard-value', so Custom shows it as the
;; default and `custom-reevaluate-setting' returns to it, and is set
;; through the option's own :set function, as `setopt' does, so a mode
;; option such as `fido-vertical-mode' turns its mode on.
;;
;; The workspace regime has no GNU Lisp and keeps its own readers
;; (romacs-workspace-completion.el).

;;; Code:

(require 'icomplete)

(defconst romacs-completion-defaults
  '((completion-styles (basic flex))
    (completion-auto-select t)
    (completion-auto-help visible)
    (completions-format one-column)
    (completions-sort historical)
    (completions-max-height 20)
    (completion-ignore-case t)
    (icomplete-compute-delay 0.15)
    (icomplete-delay-completions-threshold 400)
    (completion-category-overrides ((file (styles partial-completion))))
    (fido-vertical-mode t))
  "Romacs's standard value for each completion option, in the order set.")

(defun romacs-completion-defaults-apply (defaults)
  "Make each (OPTION VALUE) of DEFAULTS OPTION's standard value, and set it.
The value is set through OPTION's :set function, as `setopt' does."
  (dolist (default defaults)
    (let ((option (car default)) (value (cadr default)))
      (put option 'standard-value (list (custom-quote value)))
      (funcall (or (get option 'custom-set) #'set-default) option value))))

(romacs-completion-defaults-apply romacs-completion-defaults)

;; TAB takes the selected candidate, as RET would before exiting.
(keymap-set icomplete-fido-mode-map "TAB" #'icomplete-force-complete)

(provide 'romacs-completion-defaults)
;;; romacs-completion-defaults.el ends here

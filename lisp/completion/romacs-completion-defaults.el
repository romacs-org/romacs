;;; romacs-completion-defaults.el --- Romacs's minibuffer completion defaults  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; --loadup's minibuffer completion is GNU's: `completing-read',
;; `read-file-name' and `read-buffer', displayed by `romacs-complete-mode'
;; (lisp/completion/romacs-complete.el): the minibuffer and its candidates in a
;; layer over the frame, so no window or terminal resizes for a prompt
;; (ledger/divergences VM2-COMPLETE-DEFAULT).  Without a window system, or
;; where that mode is absent (GNU loading this file for a parity harness),
;; icomplete's
;; `fido-vertical-mode' displays it.  M-x, C-x C-f and C-x b reach it
;; through GNU's `execute-extended-command' and the two readers.
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
;; Where available, `fzf' is the default style for every category.
;; Fido uses it too when the global styles begin with fzf.
;; See ledger/divergences VM2-FZF-DEFAULT.
;;
;; The workspace regime has no GNU Lisp and keeps its own readers
;; (romacs-workspace-completion.el).

;;; Code:

(require 'icomplete)

(defconst romacs-completion-defaults
  `(,@(when (assq 'fzf completion-styles-alist)
        '((completion-styles (fzf))
          (completion-category-defaults nil)))
    (completion-auto-select t)
    (completion-auto-help visible)
    (completions-format one-column)
    (completions-sort historical)
    (completions-max-height 20)
    (completion-ignore-case t)
    (icomplete-compute-delay 0.15)
    (icomplete-delay-completions-threshold 400)
    ;; A layer is a child frame, which only a window system makes.
    ,(if (and window-system (fboundp 'romacs-complete-mode)) '(romacs-complete-mode t) '(fido-vertical-mode t))
    ;; Evaluation results in a chip after the sexp, where messages are
    ;; toasts (romacs-eval-chip.el, ledger/divergences VM2-ECHO-LAYER).
    ,@(and window-system (fboundp 'romacs-complete-mode) (fboundp 'romacs-eval-chip-mode)
           '((romacs-eval-chip-mode t)))
    ;; The keys of an active transient map (repeat-mode's) as hints.
    ,@(and window-system (fboundp 'romacs-popup-hints-mode) '((romacs-popup-hints-mode t)))
    ;; Romacs's mode line (romacs-mode-line.el, ledger/divergences VM2-MODE-LINE).
    ,@(and window-system (fboundp 'romacs-mode-line-mode) '((romacs-mode-line-mode t))))
  "Romacs's standard value for each completion option, in the order set.")

(defun romacs-completion-defaults-apply (defaults)
  "Make each (OPTION VALUE) of DEFAULTS OPTION's standard value, and set it.
The value is set through OPTION's :set function, as `setopt' does."
  (dolist (default defaults)
    (let ((option (car default)) (value (cadr default)))
      (put option 'standard-value (list (custom-quote value)))
      (funcall (or (get option 'custom-set) #'set-default) option value))))

(romacs-completion-defaults-apply romacs-completion-defaults)

(defun romacs-completion--fido-fzf ()
  "Complete with fzf in a `fido-mode' minibuffer where fido put `flex'.
Restore the global styles and category defaults while fzf comes first."
  (when (and (local-variable-p 'completion-styles)
             (equal completion-styles '(flex))
             (eq (car (default-value 'completion-styles)) 'fzf))
    (setq-local completion-styles (default-value 'completion-styles)
                completion-category-defaults (default-value 'completion-category-defaults))))

;; After `icomplete--fido-mode-setup', which fido adds at the default depth.
(add-hook 'minibuffer-setup-hook #'romacs-completion--fido-fzf 90)

;; TAB takes the selected candidate, as RET would before exiting.
(keymap-set icomplete-fido-mode-map "TAB" #'icomplete-force-complete)

(provide 'romacs-completion-defaults)
;;; romacs-completion-defaults.el ends here

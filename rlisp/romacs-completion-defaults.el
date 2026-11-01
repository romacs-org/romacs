;;; romacs-completion-defaults.el --- Romacs's minibuffer completion defaults  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; --loadup's minibuffer completion is GNU's: `completing-read',
;; `read-file-name' and `read-buffer', displayed by `romacs-complete-mode'
;; (rlisp/romacs-complete.el): the minibuffer and its candidates in a
;; layer over the frame, so no window or terminal resizes for a prompt
;; (DIVERGENCES.md VM2-COMPLETE-DEFAULT).  Without a window system, or
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
;; The `fzf' style (rlisp/romacs-fzf.el) comes first in
;; `completion-styles', before GNU's own default styles, which answer
;; where fzf finds nothing (`/u/l/b' by partial completion, a prefix by
;; basic); file names and project files try fzf first too.  Fido, which
;; completes with `flex' in its minibuffers whatever `completion-styles'
;; says, completes with fzf instead while `completion-styles' begins
;; with it (`romacs-completion--fido-fzf').  Where the fzf style is
;; absent (GNU loading this file for a parity harness) the values are
;; GNU's styles alone.  DIVERGENCES.md VM2-FZF-DEFAULT.
;;
;; The workspace regime has no GNU Lisp and keeps its own readers
;; (romacs-workspace-completion.el).

;;; Code:

(require 'icomplete)

(defun romacs-completion--fzf-first (styles)
  "STYLES with `fzf' first, where this runtime has the fzf style."
  (if (assq 'fzf completion-styles-alist) (cons 'fzf styles) styles))

(defconst romacs-completion-defaults
  `((completion-styles ,(romacs-completion--fzf-first '(basic partial-completion emacs22)))
    (completion-auto-select t)
    (completion-auto-help visible)
    (completions-format one-column)
    (completions-sort historical)
    (completions-max-height 20)
    (completion-ignore-case t)
    (icomplete-compute-delay 0.15)
    (icomplete-delay-completions-threshold 400)
    (completion-category-overrides
     ((file (styles ,@(romacs-completion--fzf-first '(partial-completion))))
      ,@(and (assq 'fzf completion-styles-alist)
             ;; GNU's `completion-category-defaults' entry, fzf first.
             '((project-file (styles fzf substring))))))
    ;; A layer is a child frame, which only a window system makes.
    ,(if (and window-system (fboundp 'romacs-complete-mode)) '(romacs-complete-mode t) '(fido-vertical-mode t)))
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
Fido makes `completion-styles' (flex) there and puts flex first in each
category's default styles.  While the global `completion-styles' begins
with fzf, as its standard value does, the minibuffer completes with
that value instead, and with fzf where fido put flex; with any other
styles it keeps fido's."
  (when (and (local-variable-p 'completion-styles)
             (equal completion-styles '(flex))
             (eq (car (default-value 'completion-styles)) 'fzf))
    (setq-local completion-styles (default-value 'completion-styles)
                completion-category-defaults
                (mapcar (lambda (entry)
                          (cons (car entry)
                                (mapcar (lambda (prop)
                                          (if (eq (car-safe prop) 'styles)
                                              (cons 'styles (mapcar (lambda (s) (if (eq s 'flex) 'fzf s))
                                                                    (cdr prop)))
                                            prop))
                                        (cdr entry))))
                        completion-category-defaults))))

;; After `icomplete--fido-mode-setup', which fido adds at the default depth.
(add-hook 'minibuffer-setup-hook #'romacs-completion--fido-fzf 90)

;; TAB takes the selected candidate, as RET would before exiting.
(keymap-set icomplete-fido-mode-map "TAB" #'icomplete-force-complete)

(provide 'romacs-completion-defaults)
;;; romacs-completion-defaults.el ends here

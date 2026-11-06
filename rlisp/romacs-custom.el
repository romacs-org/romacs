;;; romacs-custom.el --- Enabling and disabling themes -*- lexical-binding: t -*-
;; Copyright (C) 1996-1997, 1999, 2001-2026 Free Software Foundation,
;; Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU's `enable-theme' and `disable-theme' with their loops over a theme's
;; face settings in Rust (DIVERGENCES.md VM2-FACE-ENGINE).  A run of face
;; settings is one primitive call that stops at the next setting of any
;; other kind, so faces and variables still interleave in the order of
;; `theme-settings', and what variables, hooks and the `user' theme do is
;; GNU's own text.  Loaded after custom.el.
;;
;; Adapted from GNU custom.el.

;;; Code:

(defun enable-theme (theme)
  "Reenable all variable and face settings defined by THEME.
THEME should be either `user', or a theme loaded via `load-theme'.

After this function completes, THEME will have the highest
precedence (after `user') among enabled themes.

Note that any already-enabled themes remain enabled after this
function runs.  To disable other themes, use `disable-theme'.

After THEME has been enabled, runs `enable-theme-functions'."
  (interactive (list (intern
		      (completing-read
		       "Enable custom theme: "
		       obarray (lambda (sym) (get sym 'theme-settings)) t))))
  (unless (custom-theme-p theme)
    (error "Undefined Custom theme %s" theme))
  (let ((tail (get theme 'theme-settings)) ; '(prop symbol theme value)
        ;; We are enabling the theme, so don't inhibit enabling it.  (Bug#34027)
        (custom--inhibit-theme-enable nil))
    ;; Loop through theme settings, recalculating vars/faces.
    (while (setq tail (romacs--theme-enable-faces theme tail))
      (let* ((s (car tail))
             (prop (car s))
             (symbol (cadr s))
             (spec-list (get symbol prop))
             (val (and (boundp symbol) (symbol-value symbol))))
        ;; We can't call `custom-push-theme' when enabling the theme: it's not
        ;; that the theme settings have changed, it's just that we want to
        ;; enable those settings.  But we might need to save a user setting
        ;; outside of Customize, in order to get back to it when disabling
        ;; the theme, just like in `custom-push-theme'.
        (when (and (custom--should-apply-setting theme)
                   (eq prop 'theme-value)
                   (boundp symbol)
                   (not (or spec-list
                            ;; Only if the current value is different from
                            ;; the standard value.
                            (custom--standard-value-p symbol val)
                            ;; And only if the changed value is different
                            ;; from the new value under the user theme.
                            (and (eq theme 'user)
                                 (equal (custom-quote val) (nth 3 s))))))
          (setq spec-list `((changed ,(custom-quote val)))))
        (put symbol prop (cons (cddr s) (assq-delete-all theme spec-list)))
        (when (eq prop 'theme-value)
          ;; Ignore `custom-enabled-themes' and `custom-safe-themes'.
          (unless (memq symbol '(custom-enabled-themes custom-safe-themes))
            (custom-theme-recalc-variable symbol))))
      (setq tail (cdr tail))))
  (unless (eq theme 'user)
    (setq custom-enabled-themes
	  (cons theme (remq theme custom-enabled-themes)))
    ;; Give the `user' theme the highest priority.
    (enable-theme 'user))
  ;; Allow callers to react to the enabling.
  (run-hook-with-args 'enable-theme-functions theme))

(defun disable-theme (theme)
  "Disable all variable and face settings defined by THEME.
See `custom-enabled-themes' for a list of enabled themes.

After THEME has been disabled, runs `disable-theme-functions'."
  (interactive (list (intern
		      (completing-read
		       "Disable custom theme: "
                       (mapcar #'symbol-name custom-enabled-themes)
		       nil t))))
  (when (custom-theme-enabled-p theme)
    (let ((tail (get theme 'theme-settings)))
      (while (setq tail (romacs--theme-disable-faces theme tail))
	(let* ((s (car tail))
	       (prop   (car s))
	       (symbol (cadr s))
	       (val (assq-delete-all theme (get symbol prop))))
          (put symbol prop val)
	  (when (eq prop 'theme-value)
            (custom-theme-recalc-variable symbol)
            ;; We might have to reset the stashed value of the variable, if
            ;; no other theme is customizing it.  Without this, loading a theme
            ;; that has a setting for an unbound user option and then disabling
            ;; it will leave this lingering setting for the option, and if then
            ;; Emacs evaluates the defcustom the saved-value might be used to
            ;; set the variable.  (Bug#20766)
            (unless (get symbol 'theme-value)
              (put symbol 'saved-value nil))))
        (setq tail (cdr tail))))
    ;; Recompute faces on all frames.
    (dolist (frame (frame-list))
      ;; We must reset the fg and bg color frame parameters, or
      ;; `face-set-after-frame-default' will use the existing
      ;; parameters, which could be from the disabled theme.
      (set-frame-parameter frame 'background-color
                           (custom--frame-color-default
                            frame :background "background" "Background"
                            "unspecified-bg" "white"))
      (set-frame-parameter frame 'foreground-color
                           (custom--frame-color-default
                            frame :foreground "foreground" "Foreground"
                            "unspecified-fg" "black"))
      (face-set-after-frame-default frame))
    (setq custom-enabled-themes
          (delq theme custom-enabled-themes))
    ;; Allow callers to react to the disabling.
    (run-hook-with-args 'disable-theme-functions theme)))

;;;; Previewing a theme

(defcustom romacs-theme-preview t
  "Non-nil to show each theme `load-theme' offers as it is selected.
Only a theme `load-theme' would load without asking is shown: moving
through a list must not run code nobody agreed to."
  :type 'boolean
  :group 'customize)

(defvar romacs-theme--stock)

(defvar romacs-theme-preview--base nil
  "The themes enabled when the preview began, highest priority first.")

(defvar romacs-theme-preview--stock nil
  "`romacs-theme--stock' when the preview began.")

(defvar romacs-theme-preview--names nil
  "The names of the themes offered.")

(defvar romacs-theme-preview--selected :opening
  "The theme selected when the selection last changed, nil for none;
`:opening' until the minibuffer has shown its first.")

(defvar romacs-theme-preview--moved nil
  "Non-nil once the selection left the one the minibuffer opened on.")

(defun romacs-theme-preview-safe-p (theme)
  "Whether `load-theme' would load THEME without asking."
  (or (get theme 'theme-settings)
      (when-let* ((file (locate-file (concat (symbol-name theme) "-theme.el")
                                     (custom-theme--load-path) '("" "c"))))
        (or (eq custom-safe-themes t)
            (and (memq 'default custom-safe-themes)
                 (equal (file-name-directory file)
                        (expand-file-name "themes/" data-directory)))
            (member (with-temp-buffer
                      (insert-file-contents file)
                      (secure-hash 'sha256 (current-buffer)))
                    custom-safe-themes)))))

(defun romacs-theme-preview--show (theme)
  "Show THEME as `load-theme' would over the themes the preview began
with; with THEME nil, or one that may not be read, only those."
  (let ((safe (and theme (romacs-theme-preview-safe-p theme))))
    (when (and theme (not safe))
      (minibuffer-message "Not shown: `load-theme' would ask first"))
    (with-demoted-errors "Theme preview: %S"
      (when (and safe (not (get theme 'theme-settings)))
        (load-theme theme t t))
      (mapc #'disable-theme custom-enabled-themes)
      (mapc #'enable-theme (reverse romacs-theme-preview--base))
      (setq romacs-theme--stock romacs-theme-preview--stock)
      (when safe (enable-theme theme)))))

(defun romacs-theme-preview--current ()
  "The theme selected in the minibuffer, nil for none."
  (let ((name (if (bound-and-true-p romacs-complete--candidates)
                  (romacs-complete--candidate)
                (minibuffer-contents-no-properties))))
    (and (member name romacs-theme-preview--names) (intern name))))

(defun romacs-theme-preview--follow ()
  "Show the theme selected now, where that is another.  What the
minibuffer opens on is nobody's choice yet, and shows nothing."
  (let ((theme (romacs-theme-preview--current))
        (opening (eq romacs-theme-preview--selected :opening)))
    (unless (eq theme romacs-theme-preview--selected)
      (setq romacs-theme-preview--selected theme)
      (unless opening
        (setq romacs-theme-preview--moved t)
        (romacs-theme-preview--show theme)))))

(defun romacs-theme-preview--setup ()
  "Follow the selection in this minibuffer."
  (add-hook 'post-command-hook #'romacs-theme-preview--follow 90 t))

(defun romacs-theme-preview-read ()
  "Read a theme as `load-theme' does, showing what is selected.
The themes enabled before are back when this returns, however it does:
what the reader chose, `load-theme' goes on to load."
  (let ((names (mapcar #'symbol-name (custom-available-themes)))
        (prompt "Load custom theme: "))
    (list
     (intern
      (if (not romacs-theme-preview)
          (completing-read prompt names)
        (let* ((romacs-theme-preview--base custom-enabled-themes)
               (romacs-theme-preview--names names)
               (romacs-theme-preview--selected :opening)
               (romacs-theme-preview--moved nil)
               (romacs-theme--stock (bound-and-true-p romacs-theme--stock))
               (romacs-theme-preview--stock romacs-theme--stock))
          (unwind-protect
              (minibuffer-with-setup-hook (:append #'romacs-theme-preview--setup)
                (completing-read prompt names))
            (when romacs-theme-preview--moved
              (romacs-theme-preview--show nil))))))
     nil nil)))

(put 'load-theme 'interactive-form '(interactive (romacs-theme-preview-read)))

(provide 'romacs-custom)
;;; romacs-custom.el ends here

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

(provide 'romacs-custom)
;;; romacs-custom.el ends here

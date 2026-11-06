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

(defvar romacs-theme-preview--candidate nil
  "The theme shown or being shown: its settings may be in place though
a failed `enable-theme' never recorded it as enabled.")

(defvar romacs-theme-preview--faces nil
  "Every face as it stood before the first candidate was applied, nil
until one was: each (FRAME (FACE . VECTOR)...), FRAME nil for new frames'
defaults.  A frame showing another's faces has none of its own.")

(defvar romacs-theme-preview--names nil
  "A table of the names offered.")

(defvar romacs-theme-preview--selected :opening
  "The name selected when the selection last changed; `:opening' until
the minibuffer has shown its first.")

(defun romacs-theme-preview--saved-faces ()
  "Every face as it stands, `default' first: its background can flip the
background mode, which recalculates the others."
  (let ((faces (cons 'default (remq 'default (face-list)))))
    (mapcar (lambda (frame)
              (cons frame (mapcar (lambda (face)
                                    (cons face (copy-sequence (internal-lisp-face-p face frame))))
                                  faces)))
            (cons nil (seq-remove #'romacs--frame-shares-faces-p (frame-list))))))

(defun romacs-theme-preview--drop (theme)
  "Take THEME's settings back, whatever state a failure left them in:
one setting that cannot be recalculated leaves the others to be."
  (unless (custom-theme-enabled-p theme) (push theme custom-enabled-themes))
  (condition-case failure
      (disable-theme theme)
    (error
     (message "Theme preview: %S" failure)
     (dolist (setting (get theme 'theme-settings))
       (let ((prop (car setting)) (symbol (nth 1 setting)))
         (put symbol prop (assq-delete-all theme (get symbol prop)))
         (with-demoted-errors "Theme preview: %S"
           (if (eq prop 'theme-value)
               (custom-theme-recalc-variable symbol)
             (custom-theme-recalc-face symbol)))))
     (setq custom-enabled-themes (delq theme custom-enabled-themes)))))

(defun romacs-theme-preview--settle ()
  "Enable only the themes the preview began with.  What a candidate left
alone stays: the work follows the candidate, not the stack under it."
  (when-let* ((theme romacs-theme-preview--candidate))
    (setq romacs-theme-preview--candidate nil)
    (romacs-theme-preview--drop theme))
  (unless (equal custom-enabled-themes romacs-theme-preview--base)
    (mapc #'romacs-theme-preview--drop (copy-sequence custom-enabled-themes))
    (dolist (theme (reverse romacs-theme-preview--base))
      (with-demoted-errors "Theme preview: %S" (enable-theme theme))))
  (setq romacs-theme--stock romacs-theme-preview--stock))

(defun romacs-theme-preview--load (theme)
  "Load THEME as `load-theme' would, refusing where it would ask.  Its
own check, so what it approves is what it reads."
  (let ((confirm (symbol-function 'custom-theme-load-confirm)))
    (unwind-protect
        (progn
          (fset 'custom-theme-load-confirm #'ignore)
          (load-theme theme nil t))
      (fset 'custom-theme-load-confirm confirm))))

(defun romacs-theme-preview--show (theme)
  "Show THEME as `load-theme' would over the themes the preview began
with; with THEME nil, or one that may not be read, only those.  A theme
loads over those themes, which it may read."
  (romacs-theme-preview--settle)
  (when theme
    (condition-case failure
        (progn
          (unless (get theme 'theme-settings) (romacs-theme-preview--load theme))
          (unless romacs-theme-preview--faces
            (setq romacs-theme-preview--faces (romacs-theme-preview--saved-faces)))
          (setq romacs-theme-preview--candidate theme)
          (enable-theme theme))
      (error (minibuffer-message "Not shown: %s" (error-message-string failure))))))

(defun romacs-theme-preview--restore-faces ()
  "Every face as it stood, whose local edits no spec holds."
  (pcase-dolist (`(,frame . ,faces) romacs-theme-preview--faces)
    (when (or (null frame) (frame-live-p frame))
      (pcase-dolist (`(,face . ,vector) faces)
        (with-demoted-errors "Theme preview: %S"
          (romacs--face-restore face frame vector))))))

(defun romacs-theme-preview--revert ()
  "Put back what the preview began with: the themes, then every face,
then the enable hooks told of those faces, which `disable-theme' alone
would not do.  They are told as enabling `user' tells them, without
recalculating what it sets, and what they write goes back too."
  (romacs-theme-preview--settle)
  (romacs-theme-preview--restore-faces)
  (with-demoted-errors "Theme preview: %S"
    (run-hook-with-args 'enable-theme-functions 'user))
  (romacs-theme-preview--restore-faces))

(defmacro romacs-theme-preview--session (&rest body)
  "Run BODY, whose `romacs-theme-preview--show's are undone at its end,
however it ends."
  (declare (indent 0))
  `(let* ((romacs-theme-preview--base custom-enabled-themes)
          (romacs-theme-preview--candidate nil)
          (romacs-theme-preview--faces nil)
          (romacs-theme--stock (bound-and-true-p romacs-theme--stock))
          (romacs-theme-preview--stock romacs-theme--stock))
     (unwind-protect (progn ,@body)
       (when romacs-theme-preview--faces
         (let ((inhibit-quit t)) (romacs-theme-preview--revert))))))

(defun romacs-theme-preview--current ()
  "The name selected in the minibuffer."
  (if (bound-and-true-p romacs-complete--candidates)
      (romacs-complete--candidate)
    (minibuffer-contents-no-properties)))

(defun romacs-theme-preview--follow ()
  "Show the theme selected now, where that is another.  What the
minibuffer opens on is nobody's choice yet, and shows nothing."
  (let ((name (romacs-theme-preview--current))
        (opening (eq romacs-theme-preview--selected :opening)))
    (unless (equal name romacs-theme-preview--selected)
      (setq romacs-theme-preview--selected name)
      (unless opening
        (romacs-theme-preview--show (and (gethash name romacs-theme-preview--names) (intern name)))))))

(defun romacs-theme-preview--setup ()
  "Follow the selection in this minibuffer."
  (add-hook 'post-command-hook #'romacs-theme-preview--follow 90 t))

(defun romacs-theme-preview--table (names)
  "A table of NAMES."
  (let ((table (make-hash-table :test #'equal :size (length names))))
    (dolist (name names table) (puthash name t table))))

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
        (romacs-theme-preview--session
          (let ((romacs-theme-preview--names (romacs-theme-preview--table names))
                (romacs-theme-preview--selected :opening))
            (minibuffer-with-setup-hook (:append #'romacs-theme-preview--setup)
              (completing-read prompt names))))))
     nil nil)))

(put 'load-theme 'interactive-form '(interactive (romacs-theme-preview-read)))

(provide 'romacs-custom)
;;; romacs-custom.el ends here

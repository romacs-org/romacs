;;; dump.el --- GNU oracle for the romacs-face model  -*- lexical-binding: t -*-
;;
;; Regenerate with tests/gnu/regenerate.sh, which runs one isolated
;; `emacs -Q --batch' per theme and merges the defface tables.
;;
;; REGIME.  -Q --batch gives GNU a terminal frame, so `face-spec-choose'
;; would pick tty/mono branches.  The romacs model collapses specs for a
;; graphical truecolour pgtk display, so for a like-for-like comparison we
;; tell GNU's *display predicates* the same thing and nothing else: the
;; window system, the colour count, the display class, `supports', exact
;; colour values and the background mode.  Spec selection, theme application
;; (enable-theme -> face-spec-recalc -> set-face-attribute ->
;; internal-set-lisp-face-attribute) and inheritance resolution
;; (`face-attribute' with INHERIT `default') are GNU's own code, unmodified.
;;
;; WHAT THIS CAN AND CANNOT PROVE.  A batch frame has no font and no colours:
;; before a theme sets them the default face holds "unspecified-fg",
;; "unspecified-bg", family "default" and height 1.  Those frame values are
;; dumped as :base and given to romacs as its base, so a match on them
;; proves only plumbing.  The romacs test reports comparisons whose GNU
;; value is such a frame placeholder separately from those whose value a
;; theme or defface supplied, and asserts the latter are the majority.

(require 'cl-lib)

(defvar s86-packages
  '(diff-mode org-faces compile whitespace info dired flymake ediff-init
    message shr eww cus-edit ansi-color ibuffer)
  "Packages whose faces widen the comparison corpus.")

(advice-add 'window-system :override (lambda (&optional _frame) 'pgtk))
(advice-add 'display-color-cells :override (lambda (&optional _d) 16777216))
(advice-add 'display-graphic-p :override (lambda (&optional _d) t))
;; A truecolour display returns exact values; with `window-system' answering
;; pgtk, the real `color-values' would ask the absent window system.
(advice-add 'color-values :override
            (lambda (color &optional _frame)
              (or (color-values-from-color-spec color)
                  (tty-color-standard-values (downcase color)))))
(advice-add 'display-supports-face-attributes-p :override
            (lambda (&rest _) t))

;; GNU's own `frame-set-background-mode' (run from set-face-attribute on the
;; default face) queries the real terminal.  It is inhibited; the mode is
;; recomputed below with GNU's `frame-background-mode' / `color-dark-p'
;; logic, then every face is recalculated, which is what
;; frame-set-background-mode does on a graphical frame.
(setq inhibit-frame-set-background-mode t)

(defun s86--set-mode (mode)
  (modify-frame-parameters nil `((display-type . color)
                                 (background-mode . ,mode))))

(defun s86--plist (face inherit &optional skip)
  "FACE's attributes as a plist.
SKIP `all' keeps every value; a plist omits values `equal' to its own;
nil omits `unspecified' values."
  (let (out)
    (dolist (pair face-attribute-name-alist)
      (let* ((attr (car pair))
             (v (condition-case err (face-attribute face attr nil inherit)
                  (error (list 's86-error err)))))
        (unless (cond ((eq skip 'all) nil)
                      (skip (equal v (plist-get skip attr)))
                      (t (eq v 'unspecified)))
          (setq out (cons v (cons attr out))))))
    (nreverse out)))

(defun s86--bg-mode ()
  (or frame-background-mode
      (let* ((bg (face-attribute 'default :background nil 'default))
             (rgb (and (stringp bg) (color-values bg))))
        (cond ((null rgb) 'light)
              ((color-dark-p (mapcar (lambda (c) (/ c 65535.0)) rgb)) 'dark)
              (t 'light)))))

(defun s86--palette (theme)
  (when (fboundp 'modus-themes--get-theme-palette-subr)
    (let ((props (get theme 'theme-properties)))
      (when (plist-get props :modus-core-palette)
        (let ((all (modus-themes--get-theme-palette-subr theme t t)))
          (list :overrides (append (symbol-value
                                    (plist-get props :modus-overrides-palette))
                                   modus-themes-common-palette-overrides)
                :user (symbol-value (plist-get props :modus-user-palette))
                :core (symbol-value (plist-get props :modus-core-palette))
                :resolved (mapcar (lambda (e)
                                    (list (car e)
                                          (modus-themes--retrieve-palette-value
                                           (car e) all)))
                                  all)))))))

(defun s86-dump (theme themedir out)
  (s86--set-mode 'light)
  (dolist (p s86-packages) (require p nil t))
  (let ((base (s86--plist 'default nil 'all))
        (print-length nil) (print-level nil) (print-circle nil)
        (print-escape-newlines t) (print-escape-nonascii nil)
        mode default-row)
    (when theme
      ;; modus-* and rum `require-theme' modus-themes, which falls back to a
      ;; plain `require' on load-path, so the theme dir must be on both.
      (add-to-list 'load-path themedir)
      (setq custom-theme-load-path (list themedir))
      (load-theme theme t))
    (setq mode (s86--bg-mode))
    ;; Always recalculate: faces realised during startup and package loading
    ;; may predate the simulated display (`face-spec-recalc' is what
    ;; frame-set-background-mode runs for every face).
    (s86--set-mode mode)
    (dolist (f (face-list)) (face-spec-recalc f (selected-frame)))
    (setq default-row (s86--plist 'default 'default 'all))
    (with-temp-file (concat out ".deffaces")
      (dolist (face (sort (face-list) #'string<))
        (prin1 (list face (face-default-spec face)) (current-buffer))
        (insert "\n")))
    (with-temp-file out
      (insert "(:theme ") (prin1 theme (current-buffer))
      (insert "\n :mode ") (prin1 mode (current-buffer))
      (insert "\n :emacs-version ") (prin1 emacs-version (current-buffer))
      (insert "\n :base ") (prin1 base (current-buffer))
      (insert "\n :default ") (prin1 default-row (current-buffer))
      (insert "\n :palette ") (prin1 (and theme (s86--palette theme)) (current-buffer))
      (insert "\n :faces (")
      (dolist (face (sort (face-list) #'string<))
        (let ((entry (and theme (assq theme (get face 'theme-face)))))
          (insert "\n  ")
          (prin1 (list face
                       (and entry (list (cadr entry)))
                       (s86--plist face nil)
                       (s86--plist face 'default default-row))
                 (current-buffer))))
      (insert "))\n"))))

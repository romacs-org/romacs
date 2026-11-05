;;; theme_oracle_steps.el --- One script for pinned GNU and Romacs -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; theme_oracle_live.py loads this in both engines, on a graphical frame,
;; and compares what each step leaves: every test face's attributes on
;; every frame, `theme-face', frame parameters, the enabled themes and an
;; event log.  Real `frame-set-background-mode' re-entry and real
;; `display-supports-face-attributes-p', which tests/gnu/dump.el inhibits.

;;; Code:

(defvar ot-out (getenv "OT_OUT"))
(defvar ot-log nil)
(defvar ot-frames nil "Each (NAME . FRAME) the dump covers.")
(defvar ot-faces nil)
(defconst ot-attrs '(:family :foundry :width :height :weight :slant :underline :overline :extend
                     :strike-through :box :inverse-video :foreground :background :stipple :inherit))
(defconst ot-params '(background-mode display-type foreground-color background-color cursor-color
                      border-color mouse-color scroll-bar-foreground scroll-bar-background))

(defun ot-note (&rest items) (push (format "%S" items) ot-log))

(defun ot-dump (step)
  (with-temp-buffer
    (pcase-dolist (`(,name . ,frame) ot-frames)
      (when (frame-live-p frame)
        (dolist (p ot-params)
          (insert (format "%s\t%s\tparam\t%s\t%S\n" step name p (frame-parameter frame p))))
        (dolist (face ot-faces)
          (when (facep face)
            (dolist (a ot-attrs)
              (insert (format "%s\t%s\t%s\t%s\t%S\n" step name face a
                              (condition-case err (face-attribute face a frame) (error (car err))))))))))
    (dolist (face ot-faces)
      (insert (format "%s\t-\t%s\ttheme-face\t%S\n" step face (mapcar #'car (get face 'theme-face)))))
    (insert (format "%s\t-\t-\tenabled\t%S\n" step custom-enabled-themes))
    (insert (format "%s\t-\t-\tlog\t%S\n" step (reverse ot-log)))
    (setq ot-log nil)
    (write-region (point-min) (point-max) ot-out t 'quiet)))

(defmacro ot-step (name &rest body)
  (declare (indent 1))
  `(progn
     (condition-case err (progn ,@body)
       (error (ot-note 'signal (car err) (error-message-string err))))
     (ot-dump ',name)))

(defun ot-double (height) (* 2 height))

(defmacro ot-face (name &rest spec)
  (declare (indent 1))
  `(progn (add-to-list 'ot-faces ',name t)
          (custom-declare-face ',name ',spec "Oracle face." :group 'ot)))

(defgroup ot nil "Oracle." :group 'faces)
(defcustom ot-var 'stock "Oracle variable; its setter reads a face."
  :type 'symbol :group 'ot :initialize #'custom-initialize-default
  :set (lambda (symbol value)
         (ot-note 'set symbol value (face-attribute 'ot-plain :foreground))
         (set-default symbol value)))

(defun ot-hook (kind) (lambda (theme) (ot-note kind theme)))

(defun ot-define-faces ()
  (ot-face ot-plain (t :foreground "red"))
  (ot-face ot-mode (((background dark)) :foreground "#aaaaaa") (t :foreground "#222222"))
  (ot-face ot-box (((background dark)) :box t :foreground "cyan") (t :box t :foreground "navy"))
  (ot-face ot-supports (((supports :weight bold)) :foreground "green") (t :foreground "magenta"))
  (ot-face ot-supports-color (((supports :foreground "red")) :underline t) (t :inverse-video t))
  (ot-face ot-parent (t :foreground "blue"))
  (ot-face ot-child (t :inherit ot-parent :extend t))
  (ot-face ot-extend-parent (t))
  (ot-face ot-extend-child (t :extend t))
  (ot-face ot-bold (t :bold t))
  (ot-face ot-italic (t :italic nil))
  (ot-face ot-family (t :family "Foo-Bar"))
  (ot-face ot-family-digits (t :family "Foo-12"))
  (ot-face ot-height (t :height ot-double))
  (ot-face ot-relative (t :height 1.5))
  (ot-face ot-defaults (default :underline t) (((background dark)) :foreground "pink") (t :foreground "brown"))
  (ot-face ot-old-style (t (:foreground "olive")))
  (ot-face ot-class (((class color) (min-colors 88)) :foreground "seagreen") (((class grayscale)) :foreground "gray") (t :inverse-video t))
  (ot-face ot-type (((type graphic)) :foreground "teal") (((type tty)) :foreground "red") (t :foreground "blue"))
  (ot-face ot-local (t :foreground "white"))
  (ot-face ot-global (t :foreground "black"))
  (ot-face ot-over (t :foreground "gold"))
  (ot-face ot-cycle-a (t :inherit ot-cycle-b))
  (ot-face ot-cycle-b (t))
  (ot-face ot-bad (t :foreground "plum"))
  (dolist (face '(default shadow region link mode-line-inactive fringe border cursor mouse scroll-bar))
    (add-to-list 'ot-faces face t)))

(defun ot-define-themes ()
  (deftheme ot-dark "Oracle: dark.")
  (custom-theme-set-faces 'ot-dark
    '(default ((t :background "#101010" :foreground "#e0e0e0")))
    '(ot-plain ((t :foreground "orange")))
    '(ot-local ((t :foreground "khaki")))
    '(ot-child ((t :inherit ot-parent)))
    '(ot-extend-child ((t :inherit ot-extend-parent)))
    '(border ((t :background "purple")))
    '(cursor ((t :background "yellow"))))
  (custom-theme-set-variables 'ot-dark '(ot-var 'dark) '(frame-background-mode 'dark))
  (deftheme ot-light "Oracle: light, its `default' weight read by `supports' under re-entry.")
  (custom-theme-set-faces 'ot-light
    '(default ((t :background "#fafafa" :foreground "#101010" :weight bold)))
    '(ot-plain ((t :foreground "navy" :underline t)))
    '(cursor ((t :background "black"))))
  (custom-theme-set-variables 'ot-light '(ot-var 'light))
  (deftheme ot-stack "Oracle: stacks over another, setting few faces.")
  (custom-theme-set-faces 'ot-stack
    '(ot-plain ((t :slant italic)))
    '(ot-global ((t :foreground "sienna"))))
  (deftheme ot-broken "Oracle: an invalid attribute in the middle.")
  (custom-theme-set-faces 'ot-broken
    '(ot-plain ((t :foreground "lime")))
    '(ot-bad ((t :weight nonsense :foreground "tan")))
    '(ot-local ((t :foreground "peru"))))
  (deftheme ot-lower "Oracle: writes an `:inherit' a theme above replaces.")
  (custom-theme-set-faces 'ot-lower '(ot-cycle-b ((t :inherit ot-cycle-a))))
  (deftheme ot-upper "Oracle: over `ot-lower'.")
  (custom-theme-set-faces 'ot-upper '(ot-cycle-b ((t :inherit nil)))))

(defun ot-reset ()
  (mapc #'disable-theme custom-enabled-themes))

(defun ot-run ()
  (when (boundp 'romacs-theme-fallback) (setq romacs-theme-fallback nil))
  (ot-reset)
  (setq ot-frames (list (cons 'root (selected-frame))))
  (ot-define-faces)
  (ot-define-themes)
  (add-hook 'enable-theme-functions (ot-hook 'enabled))
  (add-hook 'disable-theme-functions (ot-hook 'disabled))
  (ot-step baseline)
  (ot-step enable-dark (enable-theme 'ot-dark))
  (ot-step disable-dark (disable-theme 'ot-dark))
  (ot-step disable-not-enabled (disable-theme 'ot-dark))
  (ot-step enable-light-flip (enable-theme 'ot-light))
  (ot-step light-to-dark (disable-theme 'ot-light) (enable-theme 'ot-dark))
  (ot-step stack (enable-theme 'ot-stack))
  (ot-step unstack-lower (disable-theme 'ot-dark))
  (ot-step reorder (enable-theme 'ot-dark) (enable-theme 'ot-stack))
  (ot-step clear (ot-reset))
  (ot-step local-edit
    (enable-theme 'ot-dark)
    (set-face-attribute 'ot-local (selected-frame) :background "gray20")
    (set-face-attribute 'ot-over (selected-frame) :background "gray30"))
  (ot-step enable-wipes-only-theme-faces (enable-theme 'ot-dark))
  (ot-step disable-wipes-all (disable-theme 'ot-dark))
  (ot-step global-edit
    (set-face-attribute 'ot-global nil :background "gray40")
    (set-face-attribute 'ot-over t :underline t))
  (ot-step global-through-enable (enable-theme 'ot-stack))
  (ot-step global-through-disable (disable-theme 'ot-stack))
  (ot-step user-theme
    (custom-set-faces '(ot-plain ((t :overline t)))))
  (ot-step user-over-enable (enable-theme 'ot-dark))
  (ot-step user-after-disable (disable-theme 'ot-dark))
  (ot-step late-defface
    (enable-theme 'ot-stack)
    (custom-theme-set-faces 'ot-stack '(ot-late ((t :foreground "coral"))))
    (ot-face ot-late (t :background "ivory")))
  (ot-step override-spec
    (face-spec-set 'ot-over '((t :foreground "salmon"))))
  (ot-step override-put-no-recalc
    (put 'ot-over 'face-override-spec '((t :foreground "wheat"))))
  (ot-step override-seen-by-pass (disable-theme 'ot-stack))
  (ot-step box-across-flip (enable-theme 'ot-dark))
  (ot-step box-flip-back (disable-theme 'ot-dark))
  (ot-step supports-under-reentry (enable-theme 'ot-light))
  (ot-step dark-after-supports (disable-theme 'ot-light) (enable-theme 'ot-dark))
  (ot-step extend-fallback (set-face-attribute 'ot-extend-parent (selected-frame) :extend nil)
    (enable-theme 'ot-dark))
  (ot-step extend-unset (set-face-attribute 'ot-extend-parent (selected-frame) :extend 'unspecified)
    (enable-theme 'ot-dark))
  (ot-step clear-2 (ot-reset))
  (ot-step invalid-mid-theme (enable-theme 'ot-broken))
  (ot-step after-invalid (ignore-errors (disable-theme 'ot-broken)) (ot-reset))
  ;; `enable-theme' stopped before recording the theme as enabled, so
  ;; `disable-theme' cannot remove what it pushed.
  (ot-step broken-removed
    (dolist (face '(ot-plain ot-bad ot-local))
      (put face 'theme-face (assq-delete-all 'ot-broken (get face 'theme-face)))
      (face-spec-recalc face (selected-frame))))
  (ot-step malformed-defface-under-theme
    (put 'ot-plain 'face-defface-spec '((((nonsense 1)) :foreground "red")))
    (enable-theme 'ot-dark))
  (ot-step restored-defface
    (put 'ot-plain 'face-defface-spec '((t :foreground "red")))
    (ot-reset))
  (ot-step border-diverged
    (enable-theme 'ot-dark)
    (modify-frame-parameters (selected-frame) '((border-color . "cyan")))
    (ot-note 'border (frame-parameter nil 'border-color))
    (enable-theme 'ot-dark))
  (ot-step clear-3 (ot-reset))
  (ot-step lower-cycle
    (enable-theme 'ot-lower)
    (enable-theme 'ot-upper))
  (ot-step upper-then-cycle-write
    (set-face-attribute 'ot-cycle-a (selected-frame) :inherit 'ot-cycle-b))
  (ot-step clear-4 (ot-reset))
  (ot-step edge-dropped
    (set-face-attribute 'ot-child (selected-frame) :inherit nil)
    (set-face-attribute 'ot-parent (selected-frame) :inherit 'ot-child))
  (ot-step flip-after-cycle-write (enable-theme 'ot-light))
  (ot-step clear-5
    (set-face-attribute 'ot-parent (selected-frame) :inherit 'unspecified)
    (ignore-errors (ot-reset))
    ;; As `broken-removed': a failed enable leaves its stack behind.
    (dolist (face '(ot-cycle-a ot-cycle-b))
      (put face 'theme-face nil)
      (face-spec-recalc face (selected-frame))))
  (ot-step aset-retained
    (aset (gethash 'ot-plain (frame--face-hash-table)) 9 "hotpink"))
  (ot-step aset-wiped (enable-theme 'ot-stack) (disable-theme 'ot-stack))
  (ot-step child-frame
    ;; A frame's drag-and-drop targets are not what this script tests.
    (unless (fboundp 'pgtk-register-dnd-targets) (defalias 'pgtk-register-dnd-targets #'ignore))
    (push (cons 'child (make-frame `((parent-frame . ,(selected-frame)) (visibility . nil) (minibuffer . nil))))
          ot-frames))
  (ot-step child-follows (enable-theme 'ot-dark))
  (ot-step child-after-disable (disable-theme 'ot-dark))
  (ot-step child-owned
    (set-face-attribute 'ot-local (cdr (assq 'child ot-frames)) :foreground "tomato"))
  (ot-step child-owned-enable (enable-theme 'ot-dark))
  (ot-step child-owned-disable (disable-theme 'ot-dark))
  (ot-step done (delete-frame (cdr (assq 'child ot-frames))))
  (write-region "END\n" nil ot-out t 'quiet))

(global-set-key (kbd "<f5>") (lambda () (interactive) (ot-run)))
;;; theme_oracle_steps.el ends here

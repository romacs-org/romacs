;;; rum-configured.el --- GNU oracle: the configured Rum look  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;;
;; The look Romacs's stock theme `default' ports: GNU with modus-themes
;; 5.3.0 (etc/modus/modus-themes.el), the Rum theme built on it
;; (inputs/rum-emacs.el, Omarchy's Rum theme file, loaded as Omarchy
;; loads it), the Modus options and font faces of the reference
;; configuration below, and the notmuch mail reader's faces.  Regenerate
;; with tests/gnu/regenerate-rum.sh, in dump.el's regime: a graphical
;; truecolour display told to GNU's display predicates, everything else
;; GNU's own code.
;;
;; Writes rum-configured.eld: the default face, then every face as
;; (FACE DEFFACE-SPEC ATTRIBUTES), ATTRIBUTES being the realised,
;; inherit-resolved attributes that differ from the default face's.  The
;; Romacs test (romacs-theme-rum-test.el) defines from DEFFACE-SPEC a
;; face of a package it has not loaded.  :init-only names the faces whose
;; look the configuration's font block gives, not the theme.

(defconst rum-configured--colors
  '((omarchy-color-bg . "#000000") (omarchy-color-fg . "#d4c5a0")
    (omarchy-color-bright-black . "#756b59") (omarchy-color-accent . "#d48a20"))
  "The Omarchy colour variables Rum's theme file reads, for Rum.")

(defconst rum-configured--modus "5.3.0" "The modus-themes the look is built on.")
(defconst rum-configured--notmuch "0.40" "The notmuch whose faces the look covers.")

(defconst rum-configured--init-only '(org-modern-symbol variable-pitch)
  "Faces whose look is the font block's, not the theme's.")

(require 'lisp-mnt)

(defconst rum-configured--here (file-name-directory (or load-file-name buffer-file-name))
  "This file's directory, where dump.el is.")

(defun rum-configured-dump (inputs notmuch out)
  (load (expand-file-name "dump.el" rum-configured--here) nil t)
  (s86--set-mode 'dark)
  (add-to-list 'load-path notmuch)
  (dolist (p (append s86-packages
                     '(org org-agenda outline vc vc-dir log-view diff smerge-mode ediff
                       eshell em-prompt em-ls which-key minibuffer icomplete completion-preview
                       gnus gnus-group gnus-sum erc term hl-line paren
                       display-line-numbers tab-bar tab-line isearch replace xref
                       help-mode bookmark calendar man proced package
                       flyspell ispell custom widget button eldoc imenu speedbar
                       notmuch notmuch-show notmuch-tree notmuch-tag notmuch-crypto notmuch-hello
                       notmuch-message notmuch-mua mm-view gnus-art gnus-cite)))
    (require p))
  (unless (equal notmuch-emacs-version rum-configured--notmuch)
    (error "notmuch %s, not %s" notmuch-emacs-version rum-configured--notmuch))
  ;; The font block.
  (custom-set-faces
   '(default ((t (:height 120 :family "Iosevka"))))
   '(org-modern-symbol ((t (:inherit default :family "Iosevka Aile"))))
   '(variable-pitch ((t (:inherit default :family "Iosevka"))))
   '(italic ((t (:family "Iosevka" :slant italic)))))
  ;; The Modus block: its options, and dividers in the default background
  ;; whenever a theme is enabled.
  (add-to-list 'load-path inputs)
  (add-hook 'enable-theme-functions
            (lambda (&rest _)
              (let ((bg (face-background 'default)))
                (custom-set-faces
                 `(fringe ((t :background ,bg :foreground ,bg)))
                 `(window-divider ((t :background ,bg :foreground ,bg)))
                 `(window-divider-first-pixel ((t :background ,bg :foreground ,bg)))
                 `(window-divider-last-pixel ((t :background ,bg :foreground ,bg)))))))
  (require 'modus-themes)
  (unless (equal (lm-version (locate-library "modus-themes.el")) rum-configured--modus)
    (error "modus-themes %s, not %s" (lm-version (locate-library "modus-themes.el")) rum-configured--modus))
  (setq modus-themes-italic-constructs nil
        modus-themes-bold-constructs nil
        modus-themes-mixed-fonts nil
        modus-themes-variable-pitch-ui nil
        modus-themes-disable-other-themes t
        modus-themes-headings '((1 . (ultrabold 1.35)) (2 . (semibold 1.2))
                                (agenda-date . (1.2))
                                (agenda-structure . (variable-pitch light 1.8))
                                (t . (1.15))))
  ;; Omarchy's theme switch: its colours, the default face primed with
  ;; them, then the theme file, enabled alone.
  (dolist (c rum-configured--colors) (set (car c) (cdr c)))
  (set-face-attribute 'default nil :background omarchy-color-bg :foreground omarchy-color-fg)
  (load (expand-file-name "rum-emacs.el" inputs) nil t)
  (enable-theme 'rum)
  (dolist (enabled (copy-sequence custom-enabled-themes))
    (unless (eq enabled 'rum) (disable-theme enabled)))
  (s86--set-mode (s86--bg-mode))
  (dolist (f (face-list)) (face-spec-recalc f (selected-frame)))
  (let ((print-length nil) (print-level nil) (print-circle nil)
        (print-escape-newlines t) (print-escape-nonascii nil)
        (default-row (s86--plist 'default 'default 'all)))
    (with-temp-file out
      (insert ";; GNU oracle: the configured Rum look; tests/gnu/rum-configured.el.\n")
      (insert "(:emacs-version ") (prin1 emacs-version (current-buffer))
      (insert "\n :modus-version ") (prin1 (lm-version (locate-library "modus-themes.el")) (current-buffer))
      (insert "\n :notmuch-version ") (prin1 notmuch-emacs-version (current-buffer))
      (insert "\n :enabled ") (prin1 custom-enabled-themes (current-buffer))
      (insert "\n :init-only ") (prin1 rum-configured--init-only (current-buffer))
      (insert "\n :default ") (prin1 default-row (current-buffer))
      (insert "\n :faces (")
      (dolist (face (sort (face-list) #'string<))
        (insert "\n  ")
        (prin1 (list face (face-default-spec face) (s86--plist face 'default default-row))
               (current-buffer)))
      (insert "))\n"))))

;;; rum-configured.el ends here

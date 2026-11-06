;;; default-theme.el --- Romacs's stock theme: warm parchment on black  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The look a Romacs session starts in.  It starts from Rum
;; (romacs-theme-rum.el, beside this file); `rum' keeps that look exactly, while
;; this theme is where Romacs's own look grows, and holds its text to the
;; contrast floors.
;;
;; Code reads from structure outward: keywords and definitions carry the
;; colour, a variable's uses stay the text colour, and numbers and constants
;; have a hue of their own.  One accent, the amber of the focus edge, marks
;; chrome and layers; the bar is a quiet surface and purple is left to its
;; centre pill.

;;; Code:

(require 'romacs-theme)
;; Rum's definition is data beside the theme files, not on `load-path'
;; (the workspace, which evaluates this file's text, provides it first).
(unless (featurep 'romacs-theme-rum)
  (load (expand-file-name "romacs-theme-rum" (file-name-directory load-file-name)) nil t))

(defconst romacs-theme-default-roles
  '(;; Code.
    (keyword "#c9a2f2") (fnname "#e3b56a") (fnname-call "#d8c79c")
    (type "#5fb0a8") (variable "#a8c6bf") (property "#b7c49a")
    (number "#e8a070") (constant "#e8a070") (preprocessor "#d0a3e8")
    ;; Secondary text, one role, which comments share; line numbers sit below it.
    (fg-dim "#a99c84") (fg-line-number-inactive "#6f6656")
    ;; Prose.
    (fg-heading-1 "#e8866f")
    (bg-prose-block-contents "#18141d") (bg-prose-block-delimiter "#18141d") (fg-prose-block-delimiter "#776d5c")
    ;; The bar.
    (bg-mode-line-active "#1b1814") (fg-mode-line-active "#d4c5a0"))
  "What `default' changes of Rum's roles.")

(defconst romacs-theme-default-slots
  '((bg-mode-line "#1b1814") (comment "#a99c84") (fg-dim "#a99c84")
    (bg-block "#18141d") (bg-block-delimiter "#18141d")
    (keyword "#c9a2f2") (function "#e3b56a") (type "#5fb0a8") (variable "#a8c6bf") (constant "#e8a070"))
  "What `default' changes of Rum's slots.")

(defconst romacs-theme-default-faces
  '((font-lock-variable-use-face :foreground fg)
    ;; The active line alone: `header-line' and others inherit `mode-line'.
    (mode-line-active :overline "#2e2922")
    ;; A pair: under another theme's bar, the pill still reads.
    (romacs-mode-line-pill :background "#50269a" :foreground "#d4c5a0")
    (org-document-title :height 1.4 :weight bold)
    (org-table-header :weight bold)
    (dired-header :weight bold)
    (notmuch-search-date :foreground fg-dim)
    (notmuch-search-count :foreground fg-dim)
    (notmuch-tag-face :foreground fg-dim)
    (notmuch-search-unread-face :weight bold))
  "What `default' sets of single faces.")

(let ((modus (plist-get romacs-theme-rum :modus)))
  (romacs-theme-define
   'default "Romacs's stock look: warm parchment and restrained accents on black."
   :family 'default :style 'modern :mode (plist-get romacs-theme-rum :mode) :floor t
   :modus (plist-put (copy-sequence modus) :overrides
                     (append romacs-theme-default-roles (plist-get modus :overrides)))
   :palette (append romacs-theme-default-slots (plist-get romacs-theme-rum :palette))
   :faces romacs-theme-default-faces))

;;; default-theme.el ends here

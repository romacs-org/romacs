;;; romacs-theme-modus.el --- The Modus theming machinery, for Romacs's themes  -*- lexical-binding: t -*-

;; Copyright (C) 2019-2026  Free Software Foundation, Inc.

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Adapted from modus-themes.el 5.3.0 by Protesilaos Stavrou
;; (https://github.com/protesilaos/modus-themes), as this file loads.

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Modus's three layers under Romacs's names: named colours (the core
;; palettes), the semantic mappings faces reference, and the face table,
;; with the style helpers and options it reads and the palette generator
;; derivative themes start from.  lisp/romacs/romacs-theme.el resolves a
;; theme's palette and builds its faces from these.  Only Modus 5.3.0's
;; live options are here: its prompts and completions options are
;; obsolete and unread upstream.
;;
;; The definitions are Modus's own, taken from etc/modus/modus-themes.el
;; as this file loads: the forms named in `romacs-theme-modus--keep' and
;; every Modus definition they refer to, in the source's order, under
;; Romacs's names, with the reference session's style defaults and a few
;; adaptations (`romacs-theme-modus--adapt').  Nothing is written: the
;; --loadup world loads this file, so a world build adapts once and keys
;; the source it read.  Theme loading, registration and commands are left
;; out; romacs-theme.el defines themes.  A newer Modus is a deliberate
;; replacement of etc/modus/modus-themes.el.

;;; Code:

(eval-when-compile (require 'cl-lib))
(require 'color)

(defgroup romacs-theme-faces nil
  "The faces Romacs's themes build their faces on."
  :group 'faces)

(defconst romacs-theme-modus--renames
  '(("modus-themes-italic-constructs" . "romacs-theme-italic-constructs")
    ("modus-themes-bold-constructs" . "romacs-theme-bold-constructs")
    ("modus-themes-mixed-fonts" . "romacs-theme-mixed-fonts")
    ("modus-themes-variable-pitch-ui" . "romacs-theme-variable-pitch-ui")
    ("modus-themes-headings" . "romacs-theme-headings")
    ("modus-themes-common-palette-user" . "romacs-theme-common-palette-user")
    ("modus-themes-common-palette-overrides" . "romacs-theme-common-palette-overrides")
    ("modus-themes-with-colors" . "romacs-theme-with-colors")
    ("modus-themes-generate-palette" . "romacs-theme-modus-generate-palette")
    ("modus-themes-faces" . "romacs-theme-modus-faces")
    ("modus-themes-custom-variables" . "romacs-theme-modus-custom-variables"))
  "Modus's public names, what users and themes write, and Romacs's.
Any other Modus name takes the prefix `romacs-theme-modus-', or
`romacs-theme-' for a helper face (`romacs-theme-modus--helper-faces')
or a heading.")

(defconst romacs-theme-modus--helper-faces
  '("bold" "slant" "fixed-pitch" "ui-variable-pitch" "reset-soft" "button")
  "Modus's helper faces, which keep their role under Romacs's prefix.")

(defconst romacs-theme-modus--defaults
  '(("modus-themes-italic-constructs" . "nil")
    ("modus-themes-bold-constructs" . "nil")
    ("modus-themes-mixed-fonts" . "nil")
    ("modus-themes-variable-pitch-ui" . "nil")
    ("modus-themes-headings" . "'((1 . (ultrabold 1.35)) (2 . (semibold 1.2)) (agenda-date . (1.2))
    (agenda-structure . (variable-pitch light 1.8)) (t . (1.15)))"))
  "The reference session's values of the five live style options.")

(defconst romacs-theme-modus--definers
  '("defun" "defmacro" "defconst" "defvar" "defcustom" "defface" "defsubst")
  "The forms that define the names a kept form may refer to.")

(defconst romacs-theme-modus--exclude
  '("modus-themes-theme" "modus-themes-declare" "modus-themes-register" "modus-themes-load-theme"
    "modus-themes-activate" "modus-themes-get-current-theme" "modus-themes-get-theme-palette"
    "modus-themes--get-theme-palette-subr" "modus-themes-get-color-value" "modus-themes-registered-items"
    "modus-themes--activated-themes" "modus-themes-disable-other-themes" "modus-themes-to-toggle"
    "modus-themes-to-rotate" "modus-themes-after-load-theme-hook" "modus-themes-prompts"
    "modus-themes-completions" "modus-themes-with-properties" "modus-themes-declare-themes"
    "modus-themes--declared-p" "modus-themes-get-all-known-themes" "modus-themes--modus-theme-p"
    "modus-themes--disable-themes" "modus-themes--get-color-schemes" "modus-themes-sort"
    "modus-themes-known-p" "modus-themes--belongs-to-family-p" "modus-themes--background-p"
    "modus-themes-operandi-palette" "modus-themes-operandi-tinted-palette" "modus-themes-vivendi-tinted-palette"
    "modus-themes-preset-overrides-faint" "modus-themes-preset-overrides-intense"
    "modus-themes-preset-overrides-cooler" "modus-themes-preset-overrides-warmer"
    "modus-themes-operandi-deuteranopia-palette" "modus-themes-operandi-tritanopia-palette"
    "modus-themes-vivendi-deuteranopia-palette" "modus-themes-vivendi-tritanopia-palette"
    ;; Named in docstrings only.
    "modus-themes-items" "modus-themes-faces-deuteranopia" "modus-themes-faces-tritanopia")
  "Modus definitions a kept form refers to that are not taken: theme
loading, registration and commands (romacs-theme.el defines themes),
palettes other than the vivendi core, and options Romacs does not carry.")

(defconst romacs-theme-modus--table-literals
  '(("\"gray50\"" . ",fg-table-dim") ("\"#595959\"" . ",term-dim") ("\"#a6a6a6\"" . ",term-light"))
  "Colours the face table and theme variables write literally, as names:
a theme with roles of its own sets them, romacs-theme.el's
`romacs-theme--modus-constants' keeps Modus's values for the rest.")

(defconst romacs-theme-modus--keep
  `((custom-declare-face-loop)
    ,@(mapcar (lambda (face) (list "defface" (concat "modus-themes-" face)))
              romacs-theme-modus--helper-faces)
    ,@(mapcar (lambda (option) (list "defcustom" (car option))) romacs-theme-modus--defaults)
    ("defcustom" "modus-themes-common-palette-user")
    ("defcustom" "modus-themes-common-palette-overrides")
    ("defconst" "modus-themes-common-palette-mappings")
    ("defconst" "modus-themes-vivendi-palette")
    ("defun" "modus-themes-retrieve-palette-value")
    ("defun" "modus-themes--bold-weight")
    ("defun" "modus-themes--slant")
    ("defun" "modus-themes--fixed-pitch")
    ("defun" "modus-themes--variable-pitch-ui")
    ("defun" "modus-themes--heading")
    ("defun" "modus-themes--box")
    ("defconst" "modus-themes-faces")
    ("defconst" "modus-themes-custom-variables")
    ("defun" "modus-themes-generate-palette"))
  "The forms taken, each by its head and name: the heading faces' loop,
then (HEAD NAME).")

(defun romacs-theme-modus--top-level (text)
  "Each top-level list of TEXT, Modus's source, as (KEY START . FORM-TEXT).
KEY is (HEAD NAME), or (custom-declare-face-loop) for the loop that
declares the heading faces."
  (with-temp-buffer
    (with-syntax-table emacs-lisp-mode-syntax-table
      (insert text)
      (goto-char (point-min))
      (let ((case-fold-search nil) (parse-sexp-ignore-comments t) (found nil))
        (while (progn (forward-comment (buffer-size)) (not (eobp)))
          (if (not (eq (char-after) ?\())
              (forward-char 1)
            (let* ((start (point))
                   (form (buffer-substring start (progn (forward-sexp 1) (point)))))
              (when (string-match "\\`(\\([^ \t\n]+\\)[ \t\n]+'?\\([^ \t\n]+\\)" form)
                (let ((head (match-string 1 form)) (name (match-string 2 form)))
                  (push (cons (if (and (equal head "dotimes") (string-search "custom-declare-face" form)
                                       (string-search "heading" form))
                                  '(custom-declare-face-loop)
                                (list head name))
                              (cons start form))
                        found))))))
        (nreverse found)))))

(defun romacs-theme-modus--taken (forms)
  "The keys of FORMS, (KEY START . TEXT) each, this file takes: those
named in `romacs-theme-modus--keep' and, transitively, each Modus
definition one refers to, but the excluded ones; in the source's order."
  (let ((first (make-hash-table :test #'equal))
        (defs (make-hash-table :test #'equal)))
    (dolist (entry (reverse forms))
      (puthash (car entry) (cdr entry) first))
    ;; A name's last definer among the forms' first occurrences, in the
    ;; source's order.
    (let ((seen (make-hash-table :test #'equal)))
      (dolist (entry forms)
        (unless (gethash (car entry) seen)
          (puthash (car entry) t seen)
          (when (and (cadr (car entry)) (member (caar entry) romacs-theme-modus--definers))
            (puthash (cadr (car entry)) (car entry) defs)))))
    (let ((keys (copy-sequence romacs-theme-modus--keep)) (rest nil))
      (setq rest keys)
      (while rest
        (let ((text (cdr (or (gethash (car rest) first)
                             (error "modus-themes.el: no %s %s" (caar rest) (cadr (car rest))))))
              (start 0) (names nil))
          (while (string-match "modus-themes-[A-Za-z0-9-]*[A-Za-z0-9]" text start)
            (cl-pushnew (match-string 0 text) names :test #'equal)
            (setq start (match-end 0)))
          (dolist (name (nreverse names))
            (let ((key (gethash name defs)))
              (when (and key (not (member key keys)) (not (member name romacs-theme-modus--exclude)))
                (setq keys (nconc keys (list key)))))))
        (setq rest (cdr rest)))
      (sort (mapcar (lambda (key) (cons key (gethash key first))) keys)
            (lambda (a b) (< (cadr a) (cadr b)))))))

(defun romacs-theme-modus--replace (from to what &optional once)
  "Replace each match of the regexp FROM in the buffer with the literal TO,
or with ONCE the first.  WHAT names the adaptation for the error that the
buffer has none."
  (goto-char (point-min))
  (unless (re-search-forward from nil t)
    (error "modus-themes.el: %s moved" what))
  (replace-match to t t)
  (while (and (not once) (re-search-forward from nil t))
    (replace-match to t t)))

(defun romacs-theme-modus--replace-all (from to)
  "Replace each literal FROM in the buffer with the literal TO."
  (goto-char (point-min))
  (while (search-forward from nil t)
    (replace-match to t t)))

(defun romacs-theme-modus--rename ()
  "Rename each Modus name in the buffer (`romacs-theme-modus--renames')."
  (goto-char (point-min))
  (while (re-search-forward "modus-themes-[A-Za-z0-9*%-]+" nil t)
    (let ((name (match-string 0)))
      (replace-match
       (or (cdr (assoc name romacs-theme-modus--renames))
           (let ((rest (substring name (length "modus-themes-"))))
             (if (or (member rest romacs-theme-modus--helper-faces)
                     (string-prefix-p "heading-" rest))
                 (concat "romacs-theme-" rest)
               (concat "romacs-theme-modus-" rest))))
       t t))))

(defun romacs-theme-modus--adapt (key text)
  "The form whose source is TEXT, KEY's, adapted to Romacs."
  (let ((name (cadr key)))
    (with-temp-buffer
      (with-syntax-table emacs-lisp-mode-syntax-table
        (let ((case-fold-search nil) (parse-sexp-ignore-comments t))
          (insert text)
          (when (and (equal (car key) "defcustom") (assoc name romacs-theme-modus--defaults))
            (goto-char (1+ (point-min)))
            (forward-sexp 3)
            (let ((end (point)))
              (backward-sexp 1)
              (delete-region (point) end)
              (insert (cdr (assoc name romacs-theme-modus--defaults)))))
          ;; Custom groups first: `modus-themes-faces' names both a group and
          ;; the face table, which go to different names.
          (romacs-theme-modus--replace-all ":group 'modus-themes-faces" ":group 'romacs-theme-faces")
          (goto-char (point-min))
          (while (re-search-forward ":group 'modus-themes\\_>" nil t)
            (replace-match ":group 'romacs-theme" t t))
          (when (equal name "modus-themes-generate-palette")
            ;; Modus infers one of the cores Romacs does not take: a caller
            ;; names the core.
            (romacs-theme-modus--replace "(core (or core-palette\n\\(?:.\\|\n\\)*?palette))))\n"
                                         "(core (or core-palette (error \"A core palette is required\")))\n"
                                         "the core inference")
            (romacs-theme-modus--replace "With optional CORE-PALETTE use it\\(?:.\\|\n\\)*?With optional MAPPINGS"
                                         "CORE-PALETTE fills in any of the remaining entries.\n\nWith optional MAPPINGS"
                                         "the core inference")
            (romacs-theme-modus--replace-all "like `modus-themes-operandi-palette'" "like `modus-themes-vivendi-palette'"))
          (when (member name '("modus-themes-faces" "modus-themes-custom-variables"))
            (dolist (literal romacs-theme-modus--table-literals)
              (romacs-theme-modus--replace-all (car literal) (cdr literal))))
          (when (equal name "modus-themes-faces")
            ;; The active window's line is `mode-line' itself, so a buffer's
            ;; remapping of `mode-line' (`face-remap-add-relative') reaches it.
            (romacs-theme-modus--replace
             "`(mode-line-active\n\\(?:.\\|\n\\)*?(t :underline ,border-mode-line-active)))"
             "`(mode-line-active ((,c :inherit mode-line)))" "mode-line-active" t))
          (romacs-theme-modus--rename)
          (goto-char (point-min))
          (read (current-buffer)))))))

(defconst romacs-theme-modus-source
  (expand-file-name "../../etc/modus/modus-themes.el"
                    (file-name-directory (file-truename (or load-file-name buffer-file-name))))
  "The Modus source this file adapts, beside lisp/ in the runtime tree.")

(defun romacs-theme-modus-forms (&optional source)
  "The forms this file defines Modus's machinery with, in order, from
SOURCE, modus-themes.el (default `romacs-theme-modus-source').
The rules match text as written, whatever the session's settings:
case-sensitively, and skipping comments as `read' does; each buffer of
the scan binds both where neither is local, as it binds the coding its
read sets, as a world build, which refuses a write to what startup read,
needs."
  (let* ((case-fold-search nil)
         (text (with-temp-buffer
                 (let ((last-coding-system-used nil)
                       (buffer-file-coding-system buffer-file-coding-system))
                   (insert-file-contents (or source romacs-theme-modus-source))
                   (buffer-string)))))
    (mapcar (lambda (taken) (romacs-theme-modus--adapt (car taken) (cddr taken)))
            (romacs-theme-modus--taken (romacs-theme-modus--top-level text)))))

(dolist (form (romacs-theme-modus-forms))
  (eval form t))

(provide 'romacs-theme-modus)
;;; romacs-theme-modus.el ends here

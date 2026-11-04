;;; romacs-icons.el --- Romacs's icons: characters of its own font  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An icon is a character of Romacs Icons (assets/fonts/RomacsIcons.ttf,
;; built from assets/icons by scripts/build-icon-font.py), in the
;; Supplementary Private Use Area-A, U+FE000-U+FE0FF.  The GUI host
;; makes the font a Fontconfig application font at startup, so the
;; character resolves to it in any face, as any character a face's own
;; font lacks resolves to a font that has it.  An icon is text:
;; `format-mode-line' returns it, the glyph atlas caches it.
;;
;; Where the font is not there (`--batch', a stripped install) or
;; `romacs-icons-source' is `text', `romacs-icon' gives the icon's
;; Unicode symbol instead, and with `romacs-icons-ascii' its ASCII
;; character.  DIVERGENCES.md VM2-MODE-LINE.

;;; Code:

(defgroup romacs-icons nil
  "Romacs's icons."
  :group 'faces
  :prefix "romacs-icons-")

(defface romacs-icon '((t))
  "Icons, over the face of the text they are in.")

(defconst romacs-icons-alist
  '((modified #xFE001 "●" "*") (saved #xFE002 "○" "-") (read-only #xFE003 "⊘" "%")
    (remote #xFE004 "⊕" "@") (narrow #xFE005 "⧖" "N") (branch #xFE006 "⎇" "~")
    (tag #xFE007 "⌗" "#") (error #xFE008 "✖" "!") (warning #xFE009 "⚠" "!")
    (note #xFE00A "ⓘ" "i") (check #xFE00B "✔" "+") (lsp #xFE00C "⇄" "L")
    (process #xFE00D "⚙" "&") (agent #xFE00E "◉" "A") (record #xFE00F "⏺" "R")
    (overwrite #xFE010 "⌧" "O") (search #xFE011 "⌕" "/") (region #xFE012 "▭" "=")
    (keyboard #xFE013 "⌨" "K") (clock #xFE014 "◷" "T")
    (lisp #xFE015 nil nil) (rust #xFE016 nil nil) (c #xFE017 nil nil) (python #xFE018 nil nil)
    (shell #xFE019 nil nil) (text #xFE01A nil nil) (org #xFE01B nil nil) (markdown #xFE01C nil nil)
    (dired #xFE01D nil nil) (mail #xFE01E nil nil) (chat #xFE01F nil nil) (term #xFE020 nil nil)
    (web #xFE021 nil nil) (image #xFE022 nil nil) (scratch #xFE023 nil nil) (help #xFE024 nil nil)
    (default #xFE025 nil nil) (missing #xFE026 "⊘" "!") (vc-edited #xFE027 "±" "*") (vc-conflict #xFE028 "⚠" "!")
    (javascript #xFE029 nil nil) (typescript #xFE02A nil nil) (go #xFE02B nil nil) (java #xFE02C nil nil)
    (nix #xFE02D nil nil) (haskell #xFE02E nil nil) (json #xFE02F nil nil) (html #xFE030 nil nil)
    (css #xFE031 nil nil) (lua #xFE032 nil nil) (conf #xFE033 nil nil) (data #xFE034 nil nil)
    (calendar #xFE035 nil nil) (info #xFE036 nil nil) (database #xFE037 nil nil)
    ;; Romacs's own R, for a buffer no mode icon names.
    (romacs #xFE038 nil nil))
  "Each icon: (NAME CODEPOINT TEXT ASCII), the character of Romacs Icons
\(assets/icons/codepoints.txt), the Unicode symbol shown without that
font, and the ASCII character with `romacs-icons-ascii'; a mode's icon
has neither and shows nothing without the font.")

(defcustom romacs-icons-source 'romacs
  "Where icons come from: `romacs', Romacs Icons where the font is there,
else each icon's Unicode symbol; `text', always the symbol."
  :type '(choice (const romacs) (const text)))

(defcustom romacs-icons-ascii nil
  "Non-nil to show each icon as its ASCII character, or nothing."
  :type 'boolean)

(defvar romacs-icons--font 'unknown
  "Whether Romacs Icons is there, once a graphic display answered.")

(defun romacs-icons-font-p ()
  "Whether Romacs Icons can draw icons here."
  (when (eq romacs-icons--font 'unknown)
    (when (display-graphic-p)
      (setq romacs-icons--font (and (find-font (font-spec :family "Romacs Icons")) t))))
  (eq romacs-icons--font t))

(defun romacs-icon (name &optional face)
  "The icon NAME as a string in FACE, or nil where it has none here."
  (when-let* ((entry (assq name romacs-icons-alist))
              (text (cond (romacs-icons-ascii (nth 3 entry))
                          ((and (eq romacs-icons-source 'romacs) (romacs-icons-font-p)) (string (nth 1 entry)))
                          (t (nth 2 entry)))))
    (propertize text 'face (if face (list face 'romacs-icon) 'romacs-icon))))

(provide 'romacs-icons)
;;; romacs-icons.el ends here

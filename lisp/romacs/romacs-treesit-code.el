;;; romacs-treesit-code.el --- Code in any buffer, coloured by its tree-sitter mode -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A buffer that holds code in another language, a Markdown code block or
;; an agent's reply, colours it with `romacs-treesit-fontify-code': the
;; faces that language's tree-sitter mode gives the code, added to the
;; faces already there.  Each block is its own document, so an unclosed
;; brace in one cannot reach the next: it is parsed alone, in a hidden
;; buffer in the language's mode (its hooks held back), one such buffer
;; per buffer and language.  A block that only grew since the last call, as
;; a reply streams in, is parsed again from what it was.
;; `romacs-treesit-fontify-string' does the same for text in no buffer.

;;; Code:

(declare-function treesit-font-lock-fontify-region "treesit" (start end &optional loudly))

(require 'romacs-treesit)    ; `romacs-treesit--mode-languages'

(defvar-local romacs-treesit--code-buffers nil
  "This buffer's hidden code buffers, as (LANGUAGE . BUFFER).
They die with the buffer, or when its major mode changes.")

(defvar sh-set-shell-hook)

(defvar romacs-treesit--string-buffers nil
  "The hidden code buffers `romacs-treesit-fontify-string' uses, as
\(LANGUAGE . BUFFER).")

(defun romacs-treesit--language-mode (language)
  "The tree-sitter mode for LANGUAGE, when its grammar is available."
  (let ((mode (car (rassq language romacs-treesit--mode-languages))))
    (and mode (fboundp mode) (treesit-language-available-p language) mode)))

(defun romacs-treesit-code-language-p (language)
  "Whether LANGUAGE has a grammar and a tree-sitter mode, so that
`romacs-treesit-fontify-code' colours its code."
  (and (romacs-treesit--language-mode language) t))

(defun romacs-treesit--kill-code-buffers ()
  "Kill this buffer's hidden code buffers."
  (dolist (entry romacs-treesit--code-buffers)
    (when (buffer-live-p (cdr entry)) (kill-buffer (cdr entry)))))


(defun romacs-treesit--code-buffer (language mode owners)
  "The live hidden buffer for LANGUAGE in OWNERS's alist, made in MODE.
OWNERS is a symbol whose value is the alist."
  (let ((entry (assq language (symbol-value owners))))
    (if (buffer-live-p (cdr entry))
        (cdr entry)
      (let ((buffer (generate-new-buffer (format " *romacs-code %s*" language) t)))
        ;; No hook of the user's runs for a buffer no one sees: the mode's
        ;; own are held back, and these run as a mode starts.
        (with-current-buffer buffer
          (let ((change-major-mode-hook nil) (after-change-major-mode-hook nil)
                (sh-set-shell-hook nil))
            (delay-mode-hooks (funcall mode))))
        (set owners (cons (cons language buffer) (assq-delete-all language (symbol-value owners))))
        buffer))))

(defun romacs-treesit--code-text (text)
  "Make the current buffer hold TEXT, by appending when it holds the
start of TEXT already."
  (let ((size (buffer-size)))
    (if (and (<= size (length text))
             (string= (buffer-substring-no-properties 1 (1+ size)) (substring text 0 size)))
        (progn (goto-char (point-max)) (insert (substring text size)))
      (erase-buffer)
      (insert text))))

(defun romacs-treesit--fontify-in (code text from to)
  "Fontify FROM..TO of TEXT in the hidden buffer CODE: its face runs,
as (START END FACE), offsets from 0."
  (with-current-buffer code
    (let ((inhibit-read-only t))
      (romacs-treesit--code-text text)
      (let ((start (1+ from)) (end (1+ to)) runs)
        (with-silent-modifications
          (treesit-font-lock-fontify-region start end))
        (while (< start end)
          (let ((next (next-single-property-change start 'face nil end)))
            (push (list (1- start) (1- next) (get-text-property start 'face)) runs)
            (setq start next)))
        (nreverse runs)))))

(defun romacs-treesit-fontify-code (language block-beg block-end &optional start end replace)
  "Colour START..END of the code block BLOCK-BEG..BLOCK-END as LANGUAGE.
START and END default to the block's bounds.  The faces are those
LANGUAGE's tree-sitter mode gives the text in a buffer that holds only
the block, at `treesit-font-lock-level'; each is added in front of the
faces already there, as `add-face-text-property' does, or with REPLACE
non-nil put in their place.  LANGUAGE is a tree-sitter language symbol,
such as `python'; one with no grammar or mode leaves the text as it is.
The block is parsed in a hidden buffer that dies with this one, which a
buffer made with its hooks inhibited (`with-temp-buffer') cannot see to:
there, use `romacs-treesit-fontify-string'.  Returns nil."
  (let ((mode (romacs-treesit--language-mode language))
        (start (or start block-beg)) (end (or end block-end)))
    (when (and mode (< start end))
      (let ((code (romacs-treesit--code-buffer language mode 'romacs-treesit--code-buffers))
            (text (buffer-substring-no-properties block-beg block-end)))
        (add-hook 'kill-buffer-hook #'romacs-treesit--kill-code-buffers nil t)
        (add-hook 'change-major-mode-hook #'romacs-treesit--kill-code-buffers nil t)
        (pcase-dolist (`(,from ,to ,face) (romacs-treesit--fontify-in code text (- start block-beg) (- end block-beg)))
          (let ((from (+ block-beg from)) (to (+ block-beg to)))
            (cond (replace (put-text-property from to 'face face))
                  (face (add-face-text-property from to face)))))))
    nil))

(defun romacs-treesit-fontify-string (language string)
  "A copy of STRING with the faces LANGUAGE's tree-sitter mode gives it.
STRING's own faces stay behind them.  LANGUAGE is as
`romacs-treesit-fontify-code' takes it; with no grammar or mode, the copy
is plain."
  (with-temp-buffer
    (insert string)
    (let ((mode (romacs-treesit--language-mode language)))
      (when (and mode (> (buffer-size) 0))
        (let ((code (romacs-treesit--code-buffer language mode 'romacs-treesit--string-buffers)))
          (pcase-dolist (`(,from ,to ,face) (romacs-treesit--fontify-in code string 0 (length string)))
            (when face (add-face-text-property (1+ from) (1+ to) face))))))
    (buffer-string)))

(provide 'romacs-treesit-code)
;;; romacs-treesit-code.el ends here

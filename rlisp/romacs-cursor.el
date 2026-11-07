;;; romacs-cursor.el --- The cursor's shape by what a buffer is for  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A bar where text goes in, a block where it does not.  While
;; `romacs-cursor-shape-mode' is on, an editable buffer's cursor is
;; `romacs-cursor-edit-type' and a read-only or special buffer's is
;; `romacs-cursor-view-type'.  The mode sets the buffer's `cursor-type'
;; when its major mode or read-only state changes, never per key, and
;; leaves alone a buffer whose `cursor-type' something else set, and every
;; buffer when the user's own default `cursor-type' is not t.  How the
;; shapes are drawn, and the morph between them, is `romacs-cursor-look-style'
;; (romacs-effects.el).

;;; Code:

(defgroup romacs-cursor-shape nil
  "The cursor's shape by what a buffer is for."
  :group 'romacs-cursor
  :prefix "romacs-cursor-")

(defcustom romacs-cursor-edit-type '(bar . 2)
  "The cursor in a buffer that takes text, a `cursor-type' value."
  :type 'sexp)

(defcustom romacs-cursor-view-type 'box
  "The cursor in a read-only or special buffer, a `cursor-type' value."
  :type 'sexp)

(defvar-local romacs-cursor--shaped nil
  "The `cursor-type' `romacs-cursor-shape-mode' last gave this buffer.")

(defun romacs-cursor--type ()
  "The cursor the current buffer should have."
  (if (or buffer-read-only (derived-mode-p 'special-mode))
      romacs-cursor-view-type
    romacs-cursor-edit-type))

(defun romacs-cursor--shape (&rest _)
  "Give the current buffer its cursor, unless something else chose one."
  (when (and (eq (default-value 'cursor-type) t)
             (or (not (local-variable-p 'cursor-type))
                 (equal cursor-type romacs-cursor--shaped)))
    (let ((type (romacs-cursor--type)))
      (setq-local cursor-type type
                  romacs-cursor--shaped type))))

(defun romacs-cursor--unshape ()
  "Give the current buffer back the cursor it had before the mode."
  (when (and romacs-cursor--shaped (equal cursor-type romacs-cursor--shaped))
    (kill-local-variable 'cursor-type))
  (kill-local-variable 'romacs-cursor--shaped))

(define-minor-mode romacs-cursor-shape-mode
  "Show a bar where text goes in and a block where it does not.
An editable buffer's cursor is `romacs-cursor-edit-type', a read-only or
special buffer's `romacs-cursor-view-type'."
  :global t
  :group 'romacs-cursor-shape
  (dolist (hook '(after-change-major-mode-hook read-only-mode-hook))
    (if romacs-cursor-shape-mode
        (add-hook hook #'romacs-cursor--shape)
      (remove-hook hook #'romacs-cursor--shape)))
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (if romacs-cursor-shape-mode (romacs-cursor--shape) (romacs-cursor--unshape)))))

(provide 'romacs-cursor)

;;; romacs-cursor.el ends here

;;; romacs-cursor.el --- The cursor's shape by what a buffer is for  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; One cursor in every buffer: `romacs-cursor-edit-type', a bar.  Setting
;; `romacs-cursor-view-type' (to `box', say) opts read-only and special
;; buffers into a cursor of their own.  `romacs-cursor-shape-mode' sets a
;; buffer's `cursor-type' when its major mode or read-only state changes,
;; never per key, and
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

(defcustom romacs-cursor-view-type nil
  "The cursor in a read-only or special buffer, a `cursor-type' value.
nil, the default, gives those buffers `romacs-cursor-edit-type' too, so
every buffer has the same cursor.  `box' brings back GNU's block where
text does not go in."
  :type '(choice (const :tag "The same cursor as editable buffers" nil)
                 (sexp :tag "A cursor-type value")))

(defvar-local romacs-cursor--shaped nil
  "The `cursor-type' `romacs-cursor-shape-mode' last gave this buffer.")

(defun romacs-cursor--type ()
  "The cursor the current buffer should have."
  (if (and romacs-cursor-view-type
           (or buffer-read-only (derived-mode-p 'special-mode)))
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
  "Give every buffer `romacs-cursor-edit-type', a bar.
A read-only or special buffer gets `romacs-cursor-view-type' instead when
that is set."
  :global t
  :group 'romacs-cursor-shape
  (dolist (hook '(after-change-major-mode-hook read-only-mode-hook view-mode-hook))
    (if romacs-cursor-shape-mode
        (add-hook hook #'romacs-cursor--shape)
      (remove-hook hook #'romacs-cursor--shape)))
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (if romacs-cursor-shape-mode (romacs-cursor--shape) (romacs-cursor--unshape)))))

(provide 'romacs-cursor)

;;; romacs-cursor.el ends here

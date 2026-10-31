;;; romacs-buffer-switch.el --- Buffer selection -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Buffer selection for Romacs, in both regimes.  The workspace reads
;; buffer names with romacs-workspace-completion.el; --loadup with GNU's
;; `read-buffer'.

;;; Code:
(defvar romacs-buffer--previous nil)

(defun romacs-select-buffer (buffer &optional norecord)
  "Display live BUFFER in the selected window, retaining each buffer's point.
Shared by interactive and programmatic file visits and buffer switches."
  (unless (buffer-live-p buffer)
    (signal 'error (list "Cannot select a dead buffer" buffer)))
  (let ((old (window-buffer (selected-window))))
    (unless (or norecord (eq old buffer)) (setq romacs-buffer--previous old))
    (set-window-buffer (selected-window) buffer)
    ;; window.el `switch-to-buffer': selecting the selected window records
    ;; BUFFER first in `buffer-list' (record_buffer).
    (unless norecord (select-window (selected-window)))
    (set-buffer buffer)
    (set-window-point (selected-window) (point))
    buffer))

(defun romacs-display-buffer-paired (buffer anchor paired fraction)
  "Display BUFFER below ANCHOR without selecting its window.
Reuse live ordinary PAIRED when distinct from ANCHOR; the caller validates
its buffer ownership.  Otherwise keep FRACTION of ANCHOR above a new pane,
with at least four text lines and a mode line each, or use ANCHOR itself.
Return the displaying window."
  (unless (buffer-live-p buffer)
    (signal 'error (list "Cannot display a dead buffer" buffer)))
  (unless (and (window-live-p anchor) (not (window-minibuffer-p anchor)))
    (signal 'error (list "Cannot pair an invalid anchor window" anchor)))
  (unless (and (numberp fraction) (> fraction 0) (< fraction 1))
    (signal 'error (list "Pair fraction must be between zero and one" fraction)))
  (let ((target
         (if (and paired (window-live-p paired)
                  (not (window-minibuffer-p paired)) (not (eq paired anchor)))
             paired
           (let* ((height (window-pixel-height anchor))
                  (minimum (* 5 (frame-char-height (window-frame anchor))))
                  (top (max minimum (min (- height minimum)
                                         (round (* height fraction))))))
             (if (< height (* 2 minimum)) anchor
               (let ((window-combination-limit t))
                 (set-window-new-pixel anchor top)
                 (split-window-internal anchor (- height top) nil (- 1.0 fraction))))))))
    (set-window-buffer target buffer)
    target))

(defun romacs-delete-paired-window (window)
  "Delete live ordinary nonsole WINDOW, returning non-nil if deleted.
The caller validates its buffer ownership before calling."
  (when (and window (window-live-p window)
             (not (window-minibuffer-p window)) (window-parent window))
    (romacs-gui-delete window)
    t))

(defun romacs-buffer-default ()
  "Return the last live displayed buffer, or another ordinary live buffer."
  (if (and (buffer-live-p romacs-buffer--previous)
           (not (eq romacs-buffer--previous (current-buffer))))
      romacs-buffer--previous
    (let ((buffers (buffer-list)) (found nil))
      (while (and buffers (not found))
        (let ((buffer (car buffers)))
          (when (and (not (eq buffer (current-buffer)))
                     (not (equal (substring (buffer-name buffer) 0 1) " ")))
            (setq found buffer)))
        (setq buffers (cdr buffers)))
      (or found (current-buffer)))))

(defun switch-to-buffer (buffer-or-name &optional norecord force-same-window)
  "Display BUFFER-OR-NAME in the selected window, creating a named buffer.
Romacs's same-window GNU-conventional switch API.  Nil selects the previous
live buffer.  NORECORD preserves the previous-buffer default.  This slice
always uses the selected window, so FORCE-SAME-WINDOW is accepted."
  (interactive
   (let ((default (buffer-name (romacs-buffer-default))))
     (list (read-buffer "Switch to buffer: " default))))
  (let ((buffer (cond ((null buffer-or-name) (romacs-buffer-default))
                      ((bufferp buffer-or-name) buffer-or-name)
                      ((stringp buffer-or-name)
                       (when (= (length buffer-or-name) 0) (signal 'error '("Empty buffer name")))
                       (get-buffer-create buffer-or-name))
                      (t (signal 'wrong-type-argument (list 'buffer-or-string-p buffer-or-name))))))
    (romacs-select-buffer buffer norecord)))
;;; romacs-buffer-switch.el ends here

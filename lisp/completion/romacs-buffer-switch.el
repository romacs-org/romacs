;;; romacs-buffer-switch.el --- Buffer selection -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Buffer selection for Romacs, in both regimes.  The workspace reads
;; buffer names with romacs-workspace-completion.el; --loadup with GNU's
;; `read-buffer'.  Both retain a previous-buffer default and per-buffer
;; point.  Window selection delegates to window.el when it is available.

;;; Code:
(declare-function window--switch-to-buffer-interactive "window" (&optional default-function))
(declare-function window--switch-to-buffer "window" (buffer-or-name norecord force-same-window))
(declare-function pop-to-buffer-same-window "window" (buffer &optional norecord))
(declare-function romacs-gui-split "romacs-startup" (horizontal &optional window))

(defvar romacs-buffer--previous nil)
(defvar switch-to-buffer-in-dedicated-window nil)

(defun romacs-buffer--select (display target norecord &rest args)
  "Call DISPLAY on TARGET with NORECORD and ARGS.
NORECORD preserves the previous-buffer default.  Keep TARGET's buffer point
if it appears in the selected window."
  (let* ((old (window-buffer))
         (buffer (get-buffer target))
         (position (and buffer (with-current-buffer buffer (point-marker)))))
    (unwind-protect
        (let ((result (apply display target norecord args)))
          (when (eq result (window-buffer))
            (unless (or norecord (eq old result))
              (setq romacs-buffer--previous old))
            (when (and position (eq (marker-buffer position) result))
              (set-window-point (selected-window) position)))
          result)
      (when position
        (set-marker position nil)))))

(defun romacs-buffer--workspace-window (buffer force-same-window)
  "Find a workspace window for BUFFER, splitting only if necessary.
Signal an error when FORCE-SAME-WINDOW requires an unavailable window."
  (let ((window (selected-window)))
    (cond
     ((eq buffer (window-buffer window)) window)
     ((not (or (window-minibuffer-p window) (eq (window-dedicated-p window) t)))
      window)
     (force-same-window
      (signal 'user-error
              (list (if (window-minibuffer-p window)
                        "Cannot switch buffers in minibuffer window"
                      "Cannot switch buffers in a dedicated window"))))
     (t
      (let* ((ordinary (window-list nil 'never))
             (windows ordinary)
            (available nil))
        (while (and windows (not available))
          (let ((candidate (car windows)))
            (unless (window-dedicated-p candidate)
              (setq available candidate)))
          (setq windows (cdr windows)))
        (or (get-buffer-window buffer)
            available
            (romacs-gui-split nil (if (window-minibuffer-p window)
                                      (car ordinary) window))))))))

(defun romacs-buffer--workspace-display (buffer norecord &optional force-same-window)
  "Display BUFFER in the workspace, respecting NORECORD and FORCE-SAME-WINDOW."
  (let ((window (romacs-buffer--workspace-window buffer force-same-window)))
    (unless (eq (window-buffer window) buffer)
      (set-window-buffer window buffer))
    (select-window window norecord)
    (set-buffer buffer)))

(defun romacs-select-buffer (buffer &optional norecord)
  "Display live BUFFER, preferring the selected window and keeping its point.
NORECORD preserves the previous-buffer default and buffer list ordering."
  (unless (buffer-live-p buffer)
    (signal 'error (list "Cannot select a dead buffer" buffer)))
  (romacs-buffer--select
   (if (fboundp 'pop-to-buffer-same-window)
       #'pop-to-buffer-same-window
     #'romacs-buffer--workspace-display)
   buffer norecord))

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
                 (set-window-new-normal anchor (/ top (float height)))
                 (split-window-internal anchor (- height top) nil (/ (- height top) (float height)))))))))
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

(defun romacs-buffer--workspace-switch-force ()
  "Handle interactive dedication before reading the workspace target."
  (cond
   ((window-minibuffer-p) nil)
   ((not (eq (window-dedicated-p) t)) t)
   ((eq switch-to-buffer-in-dedicated-window 'pop) nil)
   ((or (eq switch-to-buffer-in-dedicated-window t)
        (and (eq switch-to-buffer-in-dedicated-window 'prompt)
             (memq (read-char "Undedicate selected window? (y or n) ") '(?y ?Y))))
    (set-window-dedicated-p nil nil)
    t)
   (t (signal 'user-error '("Cannot switch buffers in a dedicated window")))))

(defun romacs-buffer--switch-arguments ()
  "Read arguments for the interactive buffer switch."
  (if (fboundp 'window--switch-to-buffer-interactive)
      (window--switch-to-buffer-interactive #'romacs-buffer-default)
    (let ((force (romacs-buffer--workspace-switch-force)))
      (list (read-buffer "Switch to buffer: " (buffer-name (romacs-buffer-default)))
            nil force))))

(defun switch-to-buffer (buffer-or-name &optional norecord force-same-window)
  "Display BUFFER-OR-NAME, creating a named buffer if needed.
Nil selects the previous live buffer.  NORECORD preserves the previous-buffer
default.  If FORCE-SAME-WINDOW is non-nil, require the selected window unless
display actions apply.  Interactively, respect
`switch-to-buffer-in-dedicated-window'."
  (interactive (romacs-buffer--switch-arguments))
  (let ((target (or buffer-or-name (romacs-buffer-default))))
    (if (fboundp 'window--switch-to-buffer)
        (romacs-buffer--select #'window--switch-to-buffer target
                               norecord force-same-window)
      (let ((buffer (if (bufferp target) target (get-buffer-create target))))
        (unless (buffer-live-p buffer)
          (signal 'error (list "Cannot select a dead buffer" buffer)))
        (romacs-buffer--select #'romacs-buffer--workspace-display buffer
                               norecord force-same-window)))))
;;; romacs-buffer-switch.el ends here

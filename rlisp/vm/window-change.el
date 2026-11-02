(progn
;;; window-change.el --- VM2: window change functions  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU window.c's run_window_change_functions, which redisplay_internal
;; runs before each redisplay: what changed since the last run in each
;; frame's windows (buffers, sizes, the selected window and frame) runs
;; `window-buffer-change-functions', `window-size-change-functions',
;; `window-selection-change-functions' and `window-state-change-functions',
;; buffer-locally with the window and then by default with the frame, then
;; `window-configuration-change-hook' and `window-state-change-hook'; then
;; the state is recorded for the next run, and for `window-old-buffer' and
;; its siblings.  GNU records the state in each window and flags a frame
;; whose windows changed; here the record is a list per frame and a
;; frame's change is its windows differing from that record.  The host
;; calls `romacs--run-window-change-functions' before it redisplays
;; (host_input.rs), with redisplay inhibited, as GNU binds it.

;;; Code:

(defvar romacs--window-change-record nil
  "Per live frame, (FRAME OLD-SELECTED-WINDOW WINDOW-STATES), as GNU's
window_change_record left them; each window state (WINDOW BUFFER
PIXEL-WIDTH PIXEL-HEIGHT BODY-PIXEL-WIDTH BODY-PIXEL-HEIGHT).")

(defvar romacs--old-selected-window nil "window.c old_selected_window.")
(defvar romacs--old-selected-frame nil "window.c old_selected_frame.")

(defun romacs--window-change-state (window)
  (list window (window-buffer window) (window-pixel-width window) (window-pixel-height window)
        (window-body-width window t) (window-body-height window t)))

(defun romacs--window-change-windows (frame)
  "FRAME's live windows in GNU's window_sub_list order, the minibuffer's
last when it is FRAME's own."
  (window-list frame t (frame-first-window frame)))

(defun romacs--window-change-call (function argument)
  "safe_calln: FUNCTION with ARGUMENT, an error reported, not raised."
  (condition-case err
      (funcall function argument)
    (error (message "Error in %S (%S): %S" function argument err))))

(defun romacs--window-change-locally (symbol buffer window)
  "run_window_change_functions_locally: BUFFER's own SYMBOL functions
with WINDOW, BUFFER current."
  (when (and (buffer-live-p buffer) (local-variable-p symbol buffer))
    (with-current-buffer buffer
      (dolist (function (buffer-local-value symbol buffer))
        (unless (eq function t) (romacs--window-change-call function window))))))

(defun romacs--window-change-globally (symbol frame)
  "run_window_change_functions_globally: SYMBOL's default functions with FRAME."
  (dolist (function (default-value symbol))
    (unless (eq function t) (romacs--window-change-call function frame))))

(defun romacs--run-window-configuration-change-hook (frame)
  "window.c run_window_configuration_change_hook: each of FRAME's windows'
buffer-local functions with the window selected, then the default ones."
  (let ((global (default-value 'window-configuration-change-hook)))
    (save-current-buffer
      (set-buffer (window-buffer (selected-window)))
      (let ((old-frame (selected-frame)))
        (unwind-protect
            (progn
              (unless (eq old-frame frame) (select-frame frame t))
              (dolist (window (window-list frame 'never))
                (let ((buffer (window-buffer window)))
                  (when (local-variable-p 'window-configuration-change-hook buffer)
                    (let ((old (selected-window)))
                      (unwind-protect
                          (progn
                            (select-window window t)
                            (dolist (function (buffer-local-value 'window-configuration-change-hook buffer))
                              (unless (eq function t) (romacs--window-change-call-0 function))))
                        (when (window-live-p old) (select-window old t)))))))
              (dolist (function global)
                (unless (eq function t) (romacs--window-change-call-0 function))))
          (when (and (frame-live-p old-frame) (not (eq old-frame (selected-frame))))
            (select-frame old-frame t)))))))

(defun romacs--window-change-call-0 (function)
  (condition-case err
      (funcall function)
    (error (message "Error in %S: %S" function err))))

(defun run-window-configuration-change-hook (&optional frame)
  "Run `window-configuration-change-hook' for FRAME.
If FRAME is omitted or nil, it defaults to the selected frame."
  (romacs--run-window-configuration-change-hook (or frame (selected-frame)))
  nil)

(defun romacs--run-window-change-functions ()
  "window.c run_window_change_functions, then window_change_record."
  (let ((inhibit-redisplay t)
        (selected-frame-change (not (eq (selected-frame) romacs--old-selected-frame)))
        (state-change-hook nil))
    (unwind-protect
        (dolist (frame (frame-list))
          (when (and (frame-live-p frame) (not (frame-parameter frame 'tooltip)))
            (let* ((recorded (cdr (assq frame romacs--window-change-record)))
                   (old-selected (car recorded))
                   (old-states (nth 1 recorded))
                   (windows (romacs--window-change-windows frame))
                   (states (mapcar #'romacs--window-change-state windows))
                   (window-change (or (null recorded) (not (equal states old-states))))
                   (frame-selected-change (and selected-frame-change
                                               (or (eq frame romacs--old-selected-frame)
                                                   (eq frame (selected-frame)))))
                   (selected-window-change (not (eq old-selected (frame-selected-window frame))))
                   (buffer-change nil) (size-change nil))
              (when (or window-change frame-selected-change selected-window-change)
                (dolist (state states)
                  (let* ((window (car state))
                         (buffer (nth 1 state))
                         (old (assq window old-states))
                         (window-buffer-change (and window-change (or (null old) (not (eq buffer (nth 1 old))))))
                         (window-size-change (and window-change
                                                  (or window-buffer-change
                                                      (not (equal (nthcdr 2 state) (nthcdr 2 old))))))
                         (selection (or (and frame-selected-change
                                             (or (eq window romacs--old-selected-window)
                                                 (eq window (selected-window))))
                                        (and selected-window-change
                                             (or (eq window old-selected)
                                                 (eq window (frame-selected-window frame)))))))
                    (setq buffer-change (or buffer-change window-buffer-change)
                          size-change (or size-change window-size-change))
                    (save-current-buffer
                      (when window-buffer-change
                        (when (and old (buffer-live-p (nth 1 old)))
                          (romacs--window-change-locally 'window-buffer-change-functions (nth 1 old) window))
                        (romacs--window-change-locally 'window-buffer-change-functions buffer window))
                      (when (and window-size-change (window-live-p window))
                        (romacs--window-change-locally 'window-size-change-functions buffer window))
                      (when (and selection (window-live-p window))
                        (romacs--window-change-locally 'window-selection-change-functions buffer window))
                      (when (and (or window-buffer-change window-size-change selection) (window-live-p window))
                        (romacs--window-change-locally 'window-state-change-functions buffer window)))))
                (let ((deleted (< (length states) (length old-states))))
                  (when (frame-live-p frame)
                    (when (or buffer-change deleted)
                      (romacs--window-change-globally 'window-buffer-change-functions frame))
                    (when size-change
                      (romacs--window-change-globally 'window-size-change-functions frame))
                    (when (or frame-selected-change selected-window-change)
                      (romacs--window-change-globally 'window-selection-change-functions frame))
                    (when (or frame-selected-change selected-window-change buffer-change deleted size-change)
                      (romacs--window-change-globally 'window-state-change-functions frame)
                      (setq state-change-hook t))
                    (when (or size-change deleted)
                      (romacs--run-window-configuration-change-hook frame))))))))
      (when (and state-change-hook (default-value 'window-state-change-hook))
        (condition-case err (run-hooks 'window-state-change-hook)
          (error (message "Error in window-state-change-hook: %S" err))))
      (romacs--window-change-record))))

(defun romacs--window-change-record ()
  "window.c window_change_record: each live frame's selected window and
its windows' buffers and sizes, and the selected window and frame."
  (setq romacs--window-change-record
        (delq nil (mapcar (lambda (frame)
                            (and (frame-live-p frame)
                                 (list frame (frame-selected-window frame)
                                       (mapcar #'romacs--window-change-state
                                               (romacs--window-change-windows frame)))))
                          (frame-list)))
        romacs--old-selected-window (selected-window)
        romacs--old-selected-frame (selected-frame)))

(defun romacs--window-change-old (window index)
  (let* ((window (or window (selected-window)))
         (state (assq window (nth 2 (assq (window-frame window) romacs--window-change-record)))))
    (and state (nth index state))))

(defun window-old-buffer (&optional window)
  "Return the old buffer displayed by WINDOW.
WINDOW must be a live window and defaults to the selected one."
  (romacs--window-change-old window 1))

(defun window-old-pixel-width (&optional window)
  "Return old total pixel width of WINDOW."
  (or (romacs--window-change-old window 2) 0))

(defun window-old-pixel-height (&optional window)
  "Return old total pixel height of WINDOW."
  (or (romacs--window-change-old window 3) 0))

(defun window-old-body-pixel-width (&optional window)
  "Return old width of WINDOW's text area in pixels."
  (or (romacs--window-change-old window 4) 0))

(defun window-old-body-pixel-height (&optional window)
  "Return old height of WINDOW's text area in pixels."
  (or (romacs--window-change-old window 5) 0))

(defun old-selected-window ()
  "Return the old selected window."
  romacs--old-selected-window)

(defun old-selected-frame ()
  "Return the old selected FRAME."
  romacs--old-selected-frame)

(defun frame-old-selected-window (&optional frame)
  "Return old selected window of FRAME."
  (nth 1 (assq (or frame (selected-frame)) romacs--window-change-record)))
)

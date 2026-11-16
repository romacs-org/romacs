;;; romacs-init-load.el --- The user's init, and what stops it  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The GUI loads the user's init through `romacs-init-load' (its Rust half,
;; crates/romacs-vm-gui/src/user_init.rs), and an error that stops it is
;; said by `romacs-init-failed' (crates/romacs-vm-gui/src/main.rs), as GNU's
;; startup says one: a warning of type `initialization' in *Warnings*.
;; The backtrace, taken as the error happened, is a button away there, and
;; the launch screen's `init' stage ends `failed'.

;;; Code:

(declare-function romacs-startup-progress "animate.rs" (stage &optional label state))

(defvar romacs-init--backtrace nil
  "The backtrace of the error that stopped the user's init, with the loads
it was in, as text; nil when it loaded.")

(declare-function romacs--loads-in-progress "load_progress.rs" ())
(declare-function backtrace-to-string "backtrace" (&optional frames))

(defun romacs-init-load (file)
  "Load the user's init FILE: nil when it loaded, else the error that
stopped it, as the loader words one (`FILE: line N form M: FORM: ERROR'),
its backtrace and the loads it was in kept for
`romacs-init-failed'.  A quit (\[keyboard-quit] in a form that
does not end) stops it as an error does."
  (setq romacs-init--backtrace nil)
  (let ((trace nil) (loads nil))
    (condition-case err
        (progn
          (handler-bind (((error quit)
                          (lambda (_)
                            (unless trace
                              (setq loads (and (fboundp 'romacs--loads-in-progress) (romacs--loads-in-progress))
                                    trace (or (ignore-errors (require 'backtrace) (backtrace-to-string)) ""))))))
            (load file nil t t))
          nil)
      ((error quit)
       (setq romacs-init--backtrace (romacs-init--trace trace loads))
       (romacs-init--error (expand-file-name file) err loads)))))

(defun romacs-init--form-head (file byte)
  "The first line of the form at BYTE of FILE, as the loader names a form."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally file nil byte (+ byte 400))
    (let ((line (decode-coding-string (buffer-substring (point-min) (line-end-position)) 'utf-8)))
      (substring (string-trim line) 0 (min 80 (length (string-trim line)))))))

(defun romacs-init--error (file err loads)
  "ERR, which stopped the init FILE within LOADS
\(`romacs--loads-in-progress'), worded as the loader words it."
  (let ((own nil))
    (dolist (load loads) (when (and (not own) (equal (expand-file-name (car load)) file)) (setq own load)))
    (cond
     ;; The reader's own words name the file, line and form already.
     ((and (eq (car err) 'error) (stringp (cadr err)) (string-prefix-p file (cadr err))) (cadr err))
     (own (format "%s: line %d form %d: %s: %S" file (nth 1 own) (nth 2 own)
                  (or (ignore-errors (romacs-init--form-head file (nth 3 own))) "?") err))
     (t (format "%s: %S" file err)))))

(defun romacs-init--trace (trace loads)
  "TRACE, a backtrace, after the loads it was in, innermost first."
  (concat (mapconcat (lambda (load) (format "In %s, line %d (form %d)\n" (nth 0 load) (nth 1 load) (nth 2 load)))
                     (reverse loads) "")
          (if loads "\n" "") trace))

(defun romacs-init-failed (message)
  "Say the user's init stopped with MESSAGE as GNU's startup says it
\(startup.el `startup--load-user-init-file'): a warning of type
`initialization' in *Warnings*, which the first frame shows, with a
button that shows the backtrace there when one was kept.  The launch
screen's `init' stage ends `failed'."
  (let ((trace romacs-init--backtrace))
    (display-warning
     'initialization
     (concat (format-message "An error occurred while loading your init file:\n\n%s\n\nTo ensure normal operation, you should investigate and remove the cause of the error in your initialization file." message)
             (if trace (concat "\n\n" (buttonize "Show the backtrace" #'romacs-init--show-trace trace)) ""))
     :error))
  (romacs-startup-progress 'init "Your init stopped with an error" (list :status 'failed :error message)))

(defun romacs-init--show-trace (trace)
  "Show TRACE, the init's backtrace, in place of its button in *Warnings*."
  (with-current-buffer (get-buffer-create "*Warnings*")
    (let ((inhibit-read-only t))
      (save-excursion
        (goto-char (point-min))
        (when (search-forward "Show the backtrace" nil t)
          (replace-match "")
          (insert (propertize trace 'face 'shadow)))))))

(provide 'romacs-init-load)
;;; romacs-init-load.el ends here

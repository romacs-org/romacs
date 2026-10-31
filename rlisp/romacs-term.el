;;; romacs-term.el --- Native Alacritty terminal for Romacs -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Native Alacritty terminal for Romacs.

;;; Code:

(defvar romacs-term-shell nil
  "Shell override; nil uses the native host's configured user shell ($SHELL).")
(defvar romacs-term-mode-hook nil)
(defvar romacs-term--closing nil)
(defvar romacs-term-session nil)
(defvar romacs-term-emacs-input nil)
(defvar kill-ring nil)

(defun romacs-term-running-p ()
  "Return non-nil when the current buffer owns a running terminal."
  (and romacs-term-session (romacs--term-running-p romacs-term-session)))

(defun romacs-term-any-running-p ()
  "Return non-nil if a live buffer owns a running terminal, including hidden ones."
  (let ((buffers (buffer-list)) (running nil))
    (while (and buffers (not running))
      (with-current-buffer (car buffers)
        (when (and (boundp 'romacs-term-session) romacs-term-session
                   (romacs--term-running-p romacs-term-session))
          (setq running t)))
      (setq buffers (cdr buffers)))
    running))

(defun romacs-term-keyboard-quit ()
  "Quit editor interaction without sending any input to the terminal."
  (interactive)
  (keyboard-quit))

(defun romacs-term-interrupt ()
  "Send Ctrl-C/ETX to the PTY; its line discipline normally delivers SIGINT."
  (interactive)
  (unless romacs-term-session (signal 'user-error '("Not a terminal buffer")))
  (romacs--term-send romacs-term-session "\003"))

(defun romacs-term-toggle-input ()
  "Toggle terminal input and protected Eshell-style editor navigation."
  (interactive)
  (unless romacs-term-session (signal 'user-error '("Not a terminal buffer")))
  (setq romacs-term-emacs-input (not romacs-term-emacs-input))
  (romacs--term-follow romacs-term-session (not romacs-term-emacs-input))
  (setq mode-name (if romacs-term-emacs-input "Term Emacs" "Term Input"))
  (romacs-gui-echo (if romacs-term-emacs-input
                    "Emacs input: navigate/copy; C-c C-c returns to terminal"
                  "Terminal input: C-x/M-x/C-g stay in editor; C-c C-k interrupts")))

(defun romacs-term-input-mode ()
  "Return explicitly to live terminal input."
  (interactive)
  (when romacs-term-emacs-input (romacs-term-toggle-input)))

(defun romacs-term-emacs-mode ()
  "Navigate protected terminal output with ordinary editor keys."
  (interactive)
  (unless romacs-term-emacs-input (romacs-term-toggle-input)))

(defun romacs-term-set-mark ()
  "Set a marker for copying terminal text without changing the application cursor."
  (interactive)
  (set-mark-command nil))

(defun romacs-term-copy-region ()
  "Copy logical terminal text between point and the terminal navigation mark."
  (interactive)
  (copy-region-as-kill (region-beginning) (region-end)))

(defun romacs-term-search (text)
  "Find TEXT in the protected terminal projection without writing to the PTY."
  (interactive (list (read-string "Search terminal: ")))
  (search-forward text))

(defun romacs-term--kill-query ()
  "Ask before killing a buffer whose terminal still runs, as GNU's
`process-kill-buffer-query-function' asks for a buffer's process."
  (or (not (romacs-term-running-p)) romacs-term--closing
      (yes-or-no-p (format "Buffer %S has a running process; kill it? "
                           (buffer-name (current-buffer))))))

(defun romacs-term--close ()
  "Retire the native session when its terminal buffer is killed."
  (when romacs-term-session
    (romacs--term-close romacs-term-session)
    (setq romacs-term-session nil)))

(defvar romacs-term-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map [3 3] 'romacs-term-toggle-input)
    (define-key map [3 11] 'romacs-term-interrupt)
    (define-key map [7] 'romacs-term-keyboard-quit)
    (define-key map [0] 'romacs-term-set-mark)
    (define-key map (kbd "M-w") 'romacs-term-copy-region)
    (define-key map [19] 'romacs-term-search)
    map))

(defun romacs-term (&optional shell)
  "Open a native Alacritty-backed terminal running SHELL in default-directory.
C-x, M-x and C-g retain editor behavior.  C-c C-k sends Ctrl-C, and C-c C-c toggles
terminal input and Emacs navigation.  Terminal output is always protected."
  (interactive)
  (let* ((directory (or (and (boundp 'default-directory) default-directory) "/"))
         (buffer (get-buffer-create (generate-new-buffer-name "*romacs-term*")))
         (completed nil))
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (make-local-variable 'romacs-term-session)
            (make-local-variable 'romacs-term-emacs-input)
            (make-local-variable 'default-directory)
            (make-local-variable 'mode-line-format)
            (make-local-variable 'kill-buffer-hook)
            (make-local-variable 'kill-buffer-query-functions)
            (setq default-directory directory
                  major-mode 'romacs-term-mode mode-name "Term Input"
                  buffer-file-name nil buffer-read-only t
                  romacs-term-emacs-input nil)
            (use-local-map romacs-term-mode-map))
          ;; Shown first, so the session takes the size of the window that
          ;; shows it (with a one-line mini-window), not the caller's.
          (romacs-select-buffer buffer)
          (with-current-buffer buffer
            (setq romacs-term-session (romacs--term-create (or shell romacs-term-shell "") directory))
            (setq kill-buffer-hook '(romacs-term--close)
                  kill-buffer-query-functions '(romacs-term--kill-query)
                  mode-line-format '("  %b  |  " mode-name "  "
                                     (:eval (if (romacs-term-running-p) "running" "exited"))
                                     "  |  C-c C-c toggle · C-c C-k interrupt"))
            (run-hooks 'romacs-term-mode-hook))
          (setq completed t)
          buffer)
      (unless completed
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (romacs-term--close))
          (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer)))))))

(provide 'romacs-term)

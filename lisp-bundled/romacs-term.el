;;; romacs-term.el --- Native Alacritty terminal for Romacs -*- lexical-binding: t -*-

;; Copyright (C) 2026 Neomacs Contributors
;; License: GPL-3.0-or-later

(defvar romacs-term-shell nil
  "Shell override; nil uses the native host's configured user shell ($SHELL).")
(defvar romacs-term-mode-hook nil)
(defvar romacs-term--closing nil)
(defvar romacs-term-session nil)
(defvar romacs-term-emacs-input nil)
(defvar romacs-term-mark nil)
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
  (setq mark-active nil)
  (signal 'quit nil))

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
  (if (markerp romacs-term-mark) (set-marker romacs-term-mark (point))
    (setq romacs-term-mark (copy-marker (point))))
  (setq mark-active t)
  (message "Mark set"))

(defun romacs-term-copy-region ()
  "Copy logical terminal text between point and the terminal navigation mark."
  (interactive)
  (unless (and (markerp romacs-term-mark) (marker-position romacs-term-mark))
    (signal 'user-error '("Set the terminal mark first")))
  (let* ((mark (marker-position romacs-term-mark))
         (text (buffer-substring-no-properties (min mark (point)) (max mark (point)))))
    (setq kill-ring (cons text kill-ring) mark-active nil)
    (message "Copied terminal text")
    text))

(defun romacs-term-search (text)
  "Find TEXT in the protected terminal projection without writing to the PTY."
  (interactive (list (read-string "Search terminal: ")))
  (search-forward text))

(defun romacs-term--kill-query ()
  "Refuse an unconfirmed programmatic kill while the child is running."
  (if (and (romacs-term-running-p) (not romacs-term--closing))
      (progn (message "Running terminal; use C-x k and confirm closure") nil)
    t))

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
         (columns (max 2 (/ (window-pixel-width) (max 1 (frame-char-width)))))
         (rows (max 1 (- (/ (window-pixel-height) (max 1 (frame-char-height))) 1)))
         (buffer (get-buffer-create (generate-new-buffer-name "*romacs-term*")))
         (completed nil))
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (make-local-variable 'romacs-term-session)
            (make-local-variable 'romacs-term-emacs-input)
            (make-local-variable 'romacs-term-mark)
            (make-local-variable 'default-directory)
            (make-local-variable 'mode-line-format)
            (make-local-variable 'kill-buffer-hook)
            (make-local-variable 'kill-buffer-query-functions)
            (setq default-directory directory
                  major-mode 'romacs-term-mode mode-name "Term Input"
                  buffer-file-name nil buffer-read-only t
                  romacs-term-emacs-input nil romacs-term-mark nil)
            (use-local-map romacs-term-mode-map)
            (setq romacs-term-session (romacs--term-create (or shell romacs-term-shell "") directory columns rows))
            (setq kill-buffer-hook '(romacs-term--close)
                  kill-buffer-query-functions '(romacs-term--kill-query)
                  mode-line-format '("  %b  |  " mode-name "  "
                                     (:eval (if (romacs-term-running-p) "running" "exited"))
                                     "  |  C-c C-c toggle · C-c C-k interrupt"))
            (run-hooks 'romacs-term-mode-hook))
          (romacs-select-buffer buffer)
          (setq completed t)
          buffer)
      (unless completed
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (romacs-term--close))
          (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer)))))))

(provide 'romacs-term)

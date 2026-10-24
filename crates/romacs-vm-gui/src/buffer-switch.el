;;; -*- lexical-binding: t -*-
(defvar buffer-name-history nil)
(defvar read-buffer-function 'romacs-read-buffer)
(defvar romacs-buffer--previous nil)

(defun romacs-select-buffer (buffer &optional norecord)
  "Display live BUFFER in the selected window, retaining each buffer's point.
Shared by interactive and programmatic file visits and buffer switches."
  (unless (buffer-live-p buffer)
    (signal 'error (list "Cannot select a dead buffer" buffer)))
  (let ((old (window-buffer (selected-window))))
    (unless (or norecord (eq old buffer)) (setq romacs-buffer--previous old))
    (set-window-buffer (selected-window) buffer)
    (set-buffer buffer)
    (set-window-point (selected-window) (point))
    buffer))

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

(defun romacs-buffer-complete ()
  "Complete the minibuffer input against live buffer names."
  (interactive)
  (let ((prefix (buffer-substring-no-properties romacs-prompt--input-start (point-max)))
        (buffers (buffer-list)) (common nil) (labels "") (count 0))
    (while buffers
      (let ((name (buffer-name (car buffers))))
        (when (and (<= (length prefix) (length name))
                   (equal prefix (substring name 0 (length prefix)))
                   (or (> (length prefix) 0) (not (equal (substring name 0 1) " "))))
          (if (null common) (setq common name)
            (let ((i 0) (limit (min (length common) (length name))))
              (while (and (< i limit) (= (aref common i) (aref name i))) (setq i (1+ i)))
              (setq common (substring common 0 i))))
          (when (< count 12) (setq labels (concat labels (if (= count 0) "" "  ") name)))
          (setq count (1+ count))))
      (setq buffers (cdr buffers)))
    (cond ((null common) (message "No matching buffers"))
          ((> (length common) (length prefix)) (romacs-prompt--replace-input common))
          ((= count 1) (message "Complete"))
          (t (message "%s%s" labels (if (> count 12) "  …" ""))))))

(defun romacs-read-buffer (prompt &optional default require-match predicate)
  "Read a buffer name through the existing minibuffer, completion and history.
RET uses DEFAULT; TAB completes live names; M-p/M-n recall buffer history;
C-g cancels.  PREDICATE filtering is explicitly unsupported."
  (when predicate (signal 'error '("Buffer reader predicates are unsupported")))
  (when (bufferp default) (setq default (buffer-name default)))
  (let* ((romacs-prompt--input-start (1+ (length prompt)))
         (romacs-prompt--history-position nil)
         (romacs-prompt--history-input nil)
         (romacs-prompt--history-variable 'buffer-name-history)
         (map (make-sparse-keymap))
         result)
    (set-keymap-parent map minibuffer-local-map)
    (define-key map [9] 'romacs-buffer-complete)
    (define-key map [13] 'exit-minibuffer)
    (define-key map [10] 'exit-minibuffer)
    (define-key map [7] 'abort-minibuffers)
    (define-key map [27 112] 'romacs-prompt-previous-history)
    (define-key map [27 110] 'romacs-prompt-next-history)
    (setq result (read-from-minibuffer prompt nil map nil 'buffer-name-history default))
    (when (= (length result) 0) (setq result default))
    (unless (and (stringp result) (> (length result) 0))
      (signal 'error '("Empty buffer name")))
    (when (and require-match (not (get-buffer result)))
      (signal 'error (list "No live buffer named" result)))
    (unless (equal result (car buffer-name-history))
      (if (fboundp 'add-to-history) (add-to-history 'buffer-name-history result)
        (setq buffer-name-history (cons result (delete result buffer-name-history)))))
    result))

(defun read-buffer (prompt &optional default require-match predicate)
  "Read a buffer name with the customizable `read-buffer-function'."
  (funcall read-buffer-function prompt default require-match predicate))

(defun switch-to-buffer (buffer-or-name &optional norecord force-same-window)
  "Display BUFFER-OR-NAME in the selected window, creating a named buffer.
Romacs's same-window GNU-conventional switch API.  Nil selects the previous
live buffer.  NORECORD preserves the previous-buffer default.  This slice
always uses the selected window, so FORCE-SAME-WINDOW is accepted."
  (interactive
   (let ((default (buffer-name (romacs-buffer-default))))
     (list (read-buffer (concat "Switch to buffer (default " default "): ") default))))
  (let ((buffer (cond ((null buffer-or-name) (romacs-buffer-default))
                      ((bufferp buffer-or-name) buffer-or-name)
                      ((stringp buffer-or-name)
                       (when (= (length buffer-or-name) 0) (signal 'error '("Empty buffer name")))
                       (get-buffer-create buffer-or-name))
                      (t (signal 'wrong-type-argument (list 'buffer-or-string-p buffer-or-name))))))
    (romacs-select-buffer buffer norecord)))

;;; -*- lexical-binding: t -*-
(defvar extended-command-history nil)

(defun romacs-command-p (command)
  "Return non-nil for an invocable named interactive COMMAND."
  (and (symbolp command) (fboundp command)
       (condition-case nil (interactive-form command) (error nil))))

(defun romacs-command-names ()
  "Return currently defined interactive command names, sorted for completion."
  (let ((names nil))
    (mapatoms (lambda (symbol)
                (when (romacs-command-p symbol)
                  (setq names (cons (symbol-name symbol) names)))))
    (sort names 'string-lessp)))

(defun romacs-command-complete ()
  "Extend command input to the common prefix and show ambiguous candidates."
  (interactive)
  (let ((prefix (buffer-substring-no-properties romacs-prompt--input-start (point-max)))
        (names (romacs-command-names)) (common nil) (labels "") (count 0))
    (while names
      (let ((name (car names)))
        (when (and (<= (length prefix) (length name))
                   (equal prefix (substring name 0 (length prefix))))
          (if (null common) (setq common name)
            (let ((i 0) (limit (min (length common) (length name))))
              (while (and (< i limit) (= (aref common i) (aref name i))) (setq i (1+ i)))
              (setq common (substring common 0 i))))
          (when (< count 12) (setq labels (concat labels (if (= count 0) "" "  ") name)))
          (setq count (1+ count))))
      (setq names (cdr names)))
    (cond ((null common) (message "No matching commands"))
          ((> (length common) (length prefix)) (romacs-prompt--replace-input common))
          ((= count 1) (message "Complete"))
          (t (message "%s%s" labels (if (> count 12) "  …" ""))))))

(defun read-extended-command (&optional prompt)
  "Read an existing interactive command name with completion and history."
  (let* ((prompt (or prompt "M-x "))
         (romacs-prompt--input-start (1+ (length prompt)))
         (romacs-prompt--history-position nil)
         (romacs-prompt--history-input nil)
         (romacs-prompt--history-variable 'extended-command-history)
         (map (make-sparse-keymap))
         name)
    (set-keymap-parent map minibuffer-local-map)
    (define-key map [9] 'romacs-command-complete)
    (define-key map [13] 'exit-minibuffer)
    (define-key map [10] 'exit-minibuffer)
    (define-key map [7] 'abort-minibuffers)
    (define-key map [27 112] 'romacs-prompt-previous-history)
    (define-key map [27 110] 'romacs-prompt-next-history)
    (setq name (read-from-minibuffer prompt nil map nil 'extended-command-history))
    (unless (romacs-command-p (intern-soft name))
      (signal 'user-error (list "Not an interactive command" name)))
    (unless (equal name (car extended-command-history))
      (if (fboundp 'add-to-history) (add-to-history 'extended-command-history name)
        (setq extended-command-history (cons name (delete name extended-command-history)))))
    name))

(defun execute-extended-command (prefixarg &optional command-name typed)
  "Invoke a named interactive command using Romacs's ordinary command path.
M-x reads a name with TAB completion and M-p/M-n history; C-g cancels.
With COMMAND-NAME supplied, no command-name prompt is opened.  The selected
command receives PREFIXARG and may open its own ordinary interactive prompt.
TYPED is accepted for GNU calling compatibility without binding suggestions."
  (interactive (list current-prefix-arg (read-extended-command)))
  (let* ((name (or command-name (read-extended-command)))
         (command (if (stringp name) (intern-soft name) name)))
    (unless (romacs-command-p command)
      (signal 'user-error (list "Not an interactive command" name)))
    (let ((current-prefix-arg prefixarg))
      (setq this-command command real-this-command command)
      (call-interactively command t))))

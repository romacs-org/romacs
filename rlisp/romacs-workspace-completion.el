;;; romacs-workspace-completion.el --- The workspace's minibuffer readers -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; The workspace regime runs no GNU Lisp, so it has no minibuffer.el.
;; These are its readers of command, file and buffer names
;; (`read-extended-command', `read-file-name', `read-buffer') and its M-x,
;; over one completer: TAB extends the input to the matching candidates'
;; common prefix, or else lists up to twelve of them.  M-p and M-n recall
;; history (romacs-editing.el); C-g cancels.
;;
;; --loadup does not load this file: GNU's own readers serve there, shown
;; by icomplete (romacs-completion-defaults.el).

;;; Code:

(defvar extended-command-history nil)
(defvar buffer-name-history nil)
(defvar read-file-name-function 'romacs-read-file-name)
;; minibuf.c's `read-buffer' calls `read-buffer-function', a C variable.
(setq read-buffer-function #'romacs-read-buffer)

(defvar romacs-complete--candidates nil
  "While a reader is active, a function of its input that returns
\(BASE PREFIX NAMES): NAMES complete PREFIX, the input's tail after BASE.")
(defvar romacs-complete--kind nil
  "While a reader is active, what its candidates are, for messages.")

(defun romacs-complete ()
  "Extend the input to the matching candidates' common prefix, or list them."
  (interactive)
  (let* ((input (buffer-substring-no-properties romacs-prompt--input-start (point-max)))
         (found (funcall romacs-complete--candidates input))
         (base (car found)) (prefix (nth 1 found)) (names (nth 2 found))
         (common nil) (labels "") (count 0))
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
    (cond ((null common) (message "No matching %s" romacs-complete--kind))
          ((> (length common) (length prefix)) (romacs-prompt--replace-input (concat base common)))
          ((= count 1) (message "Complete"))
          (t (message "%s%s" labels (if (> count 12) "  …" ""))))))

(defun romacs-complete--read (prompt initial history kind candidates &optional default)
  "Read with PROMPT and INITIAL input, completing over CANDIDATES.
CANDIDATES is a `romacs-complete--candidates' function, KIND names what it
offers and HISTORY is the history variable.  DEFAULT is the minibuffer's."
  (let ((romacs-prompt--input-start (1+ (length prompt)))
        (romacs-prompt--history-position nil)
        (romacs-prompt--history-input nil)
        (romacs-prompt--history-variable history)
        (romacs-complete--candidates candidates)
        (romacs-complete--kind kind)
        (map (make-sparse-keymap)))
    (set-keymap-parent map minibuffer-local-map)
    (define-key map [9] 'romacs-complete)
    (define-key map [13] 'exit-minibuffer)
    (define-key map [10] 'exit-minibuffer)
    (define-key map [7] 'abort-minibuffers)
    (define-key map [27 112] 'romacs-prompt-previous-history)
    (define-key map [27 110] 'romacs-prompt-next-history)
    (read-from-minibuffer prompt initial map nil history default)))

(defun romacs-complete--remember (history value)
  "Put VALUE at the head of HISTORY unless it is there already."
  (unless (equal value (car (symbol-value history)))
    (if (fboundp 'add-to-history) (add-to-history history value)
      (set history (cons value (delete value (symbol-value history)))))))

;;;; Commands

(defun romacs-command-p (command)
  "Return non-nil for an invocable named interactive COMMAND.
An interactive autoload qualifies without loading its file, as with
GNU's `commandp'; calling it loads the file."
  (and (symbolp command) (fboundp command) (commandp command)))

(defun romacs-command-names ()
  "Return currently defined interactive command names, sorted for completion."
  (let ((names nil))
    (mapatoms (lambda (symbol)
                (when (romacs-command-p symbol)
                  (setq names (cons (symbol-name symbol) names)))))
    (sort names 'string-lessp)))

(defun read-extended-command (&optional prompt)
  "Read an existing interactive command name with completion and history."
  (let ((name (romacs-complete--read (or prompt "M-x ") nil 'extended-command-history "commands"
                                     (lambda (input) (list "" input (romacs-command-names))))))
    (unless (romacs-command-p (intern-soft name))
      (signal 'user-error (list "Not an interactive command" name)))
    (romacs-complete--remember 'extended-command-history name)
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
    (setq this-command command real-this-command command)
    ;; GNU's command-execute (simple.el): a named keyboard macro runs as one.
    (if (arrayp (indirect-function command))
        (execute-kbd-macro (indirect-function command) prefixarg)
      (let ((current-prefix-arg prefixarg))
        (call-interactively command t)))))

;;;; Files

(defun romacs-complete--files (directory)
  "Return the candidates of a file name read relative to DIRECTORY.
A directory candidate ends in a slash.  A directory above 4096 entries is
refused rather than completed from a misleading truncated listing."
  (lambda (input)
    (let* ((default-directory directory)
           (full (romacs-find-file-local-name input))
           (full (if (and (> (length input) 0) (equal (substring input -1) "/")
                          (not (equal (substring full -1) "/")))
                     (concat full "/") full))
           (parent (file-name-directory full))
           (prefix (file-name-nondirectory full))
           (entries (directory-files parent nil nil nil 4097))
           (names nil))
      (when (> (length entries) 4096)
        (signal 'file-error (list "Completion directory exceeds 4096 entries" parent)))
      (while entries
        (let ((entry (car entries)))
          (when (and (not (member entry '("." "..")))
                     (<= (length prefix) (length entry))
                     (equal prefix (substring entry 0 (length prefix))))
            (setq names (cons (if (file-directory-p (concat parent entry)) (concat entry "/") entry)
                              names))))
        (setq entries (cdr entries)))
      (list (or (file-name-directory input) "") prefix (nreverse names)))))

(defun romacs-read-file-name (prompt &optional directory default-filename mustmatch initial predicate)
  "Read a local filename through the existing minibuffer and HostInput loop.
Use editable DIRECTORY (default `default-directory'), path completion and
filename history.  Non-nil MUSTMATCH requires an existing regular file."
  (let* ((directory (romacs-find-file-local-name (or directory default-directory "./")))
         (directory (if (equal (substring directory -1) "/") directory (concat directory "/")))
         (default-directory directory)
         (result (romacs-complete--read prompt (or initial directory) 'file-name-history
                                        "local files" (romacs-complete--files directory))))
    (when (and (= (length result) 0) default-filename) (setq result default-filename))
    (setq result (romacs-find-file-local-name result))
    (when mustmatch
      (let ((attributes (file-attributes result 'integer)))
        (when (and attributes (stringp (car attributes)))
          (signal 'file-error (list "MUSTMATCH symlink validation is unsupported" result)))
        (unless (and attributes (= (aref (nth 8 attributes) 0) 45))
          (signal 'file-missing (list "Regular file does not exist" result)))))
    (when (and predicate (not (funcall predicate result)))
      (signal 'file-error (list "Filename rejected by predicate" result)))
    (romacs-complete--remember 'file-name-history result)
    result))

(defun read-file-name (prompt &optional directory default-filename mustmatch initial predicate)
  "Read a filename with the customizable `read-file-name-function'."
  (funcall read-file-name-function prompt directory default-filename mustmatch initial predicate))

;;;; Buffers

(defun romacs-complete--buffers (input)
  "The live buffer names, and hidden ones only once INPUT is not empty."
  (let ((buffers (buffer-list)) (names nil))
    (while buffers
      (let ((name (buffer-name (car buffers))))
        (when (or (> (length input) 0) (not (equal (substring name 0 1) " ")))
          (setq names (cons name names))))
      (setq buffers (cdr buffers)))
    (list "" input (nreverse names))))

(defun romacs-complete--default-prompt (prompt default)
  "PROMPT showing DEFAULT before its colon, as `read-buffer' shows it."
  (if (not (stringp default)) prompt
    (let ((end (length prompt)))
      (while (and (> end 0) (memq (aref prompt (1- end)) '(?: ?\s))) (setq end (1- end)))
      (concat (substring prompt 0 end) " (default " default "): "))))

(defun romacs-read-buffer (prompt &optional default require-match predicate)
  "Read a buffer name through the existing minibuffer, completion and history.
RET uses DEFAULT; TAB completes live names; M-p/M-n recall buffer history;
C-g cancels.  PREDICATE filtering is explicitly unsupported."
  (when predicate (signal 'error '("Buffer reader predicates are unsupported")))
  (when (bufferp default) (setq default (buffer-name default)))
  (let ((result (romacs-complete--read (romacs-complete--default-prompt prompt default) nil
                                       'buffer-name-history "buffers" #'romacs-complete--buffers default)))
    (when (= (length result) 0) (setq result default))
    (unless (and (stringp result) (> (length result) 0))
      (signal 'error '("Empty buffer name")))
    (when (and require-match (not (get-buffer result)))
      (signal 'error (list "No live buffer named" result)))
    (romacs-complete--remember 'buffer-name-history result)
    result))

(provide 'romacs-workspace-completion)
;;; romacs-workspace-completion.el ends here

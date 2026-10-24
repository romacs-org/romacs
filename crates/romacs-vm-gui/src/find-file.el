;;; -*- lexical-binding: t -*-
(unless (boundp 'default-directory) (setq default-directory (expand-file-name "./")))
(defvar file-name-history nil)
(defvar find-file-hook nil)
(defvar find-file-not-found-functions nil)
(defvar auto-mode-alist '(("\\.el\\'" . emacs-lisp-mode)))
(defvar read-file-name-function 'romacs-read-file-name)
(defvar romacs-find-file-mode-function 'romacs-find-file-set-auto-mode)
(defvar romacs-prompt--input-start nil)
(defvar romacs-find-file--directory nil)
(defvar romacs-prompt--history-position nil)
(defvar romacs-prompt--history-input nil)
(defvar romacs-prompt--history-variable 'file-name-history)

(defun romacs-find-file--check-syntax (filename)
  "Reject unsupported filename syntax before any filesystem access."
  (unless (stringp filename) (signal 'wrong-type-argument (list 'stringp filename)))
  (when (string-match "\\`/[^/]*:" filename)
    (signal 'file-error (list "Remote and special filename prefixes are unsupported" filename)))
  (let ((i 0))
    (while (< i (length filename))
      (when (memq (aref filename i) '(0 42 63 91 36))
        (signal 'file-error (list "Unsupported remote, wildcard or special filename" filename)))
      (setq i (1+ i)))))

(defun romacs-find-file-local-name (filename)
  "Expand a plain local UTF-8 FILENAME, refusing unsupported path syntax.
Remote names, handlers, wildcards, environment substitution and special files
are not implemented.  Local directories use the owned Dired browser.
File-local variables are never evaluated."
  (romacs-find-file--check-syntax filename)
  (when (stringp default-directory) (romacs-find-file--check-syntax default-directory))
  (when (or (find-file-name-handler filename 'expand-file-name)
            (and (stringp default-directory)
                 (find-file-name-handler default-directory 'expand-file-name)))
    (signal 'file-error (list "File-name expansion handlers are unsupported" filename)))
  (let ((file (expand-file-name filename default-directory)))
    (when (or (find-file-name-handler file 'expand-file-name)
              (find-file-name-handler filename 'find-file-noselect)
              (find-file-name-handler file 'find-file-noselect)
              (find-file-name-handler file 'insert-file-contents))
      (signal 'file-error (list "File-name handlers are unsupported" filename)))
    (let ((operations '(file-name-directory file-name-nondirectory file-directory-p file-attributes)))
      (while operations
        (when (or (find-file-name-handler filename (car operations))
                  (find-file-name-handler file (car operations)))
          (signal 'file-error (list "File-name handlers are unsupported" filename)))
        (setq operations (cdr operations))))
    file))

(defun romacs-find-file-set-auto-mode ()
  "Choose the first callable filename mode from `auto-mode-alist'.
This is filename policy only, not GNU's full mode, coding or local-variable
lifecycle.  The default native Elisp mode records major-mode; it does not
claim the complete GNU Elisp editing environment."
  (let ((entries auto-mode-alist) (mode nil))
    (while (and entries (not mode))
      (when (string-match (car (car entries)) buffer-file-name)
        (setq mode (cdr (car entries))))
      (setq entries (cdr entries)))
    (when mode
      (unless (functionp mode)
        (signal 'error (list "Filename mode is not callable" mode)))
      (funcall mode))))

(defun romacs-find-file--visit-local (file &optional name)
  "Visit expanded local FILE without selecting a window.
Reuse native filename/inode identity without rereading dirty text or resetting
point, undo or mode.  A missing file needs an existing parent directory and is
not created until saved.  Failed fresh visits are removed before signalling."
  (let ((buffer (romacs-find-buffer-visiting file)))
    (unless buffer
      (when (file-directory-p file)
        (signal 'file-error (list "A directory cannot be visited as a regular file" file)))
      (unless (file-directory-p (file-name-directory file))
        (signal 'file-missing (list "Parent directory does not exist" file)))
      (setq buffer (get-buffer-create
                    (generate-new-buffer-name (or name (file-name-nondirectory file)))))
      (let ((completed nil))
        (unwind-protect
            (progn
              (with-current-buffer buffer
                (make-local-variable 'default-directory)
                (setq default-directory (file-name-directory file))
                (let ((missing nil))
                  (condition-case nil (insert-file-contents file t)
                    (file-missing (setq missing t)))
                  (set-buffer-modified-p nil)
                  (goto-char (point-min))
                  (when missing
                    (let ((hooks find-file-not-found-functions) (handled nil))
                      (while (and hooks (not handled))
                        (setq handled (funcall (car hooks)) hooks (cdr hooks)))))
                  (when romacs-find-file-mode-function
                    (funcall romacs-find-file-mode-function))
                  (run-hooks 'find-file-hook))
                (unless romacs-gui-preserve-mode-line
                  (make-local-variable 'mode-line-format)
                  (setq mode-line-format '("  " (:eval (if (romacs-gui-dirty-p) "*" "-")) " %b   |   Rum / VM"))))
              (unless (buffer-live-p buffer)
                (signal 'error (list "File visit hook killed its buffer" file)))
              (setq completed t))
          (when (and (not completed) (buffer-live-p buffer))
            (with-current-buffer buffer (setq buffer-file-name nil))
            (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer))))))
    (unless (memq buffer romacs-gui-visits)
      (setq romacs-gui-visits (cons buffer romacs-gui-visits)))
    buffer))

(defun romacs-find-file-noselect (filename &optional name)
  "Return a local file or Dired directory buffer without selecting it."
  (let ((file (romacs-find-file-local-name filename)))
    (if (file-directory-p file) (dired-noselect file)
      (romacs-find-file--visit-local file name))))

(defun find-file-noselect (filename &optional nowarn rawfile wildcards)
  "Return a buffer visiting local UTF-8 FILENAME without selecting it.
Romacs's slim file visitor; NOWARN is accepted.  RAWFILE and wildcard
expansion are unsupported.  Modes and hooks are independent Lisp policy."
  (when (or rawfile wildcards)
    (signal 'file-error (list "Raw or wildcard file visiting is unsupported" filename)))
  (romacs-find-file-noselect filename))

(defun find-file (filename &optional wildcards)
  "Edit local UTF-8 FILENAME in the selected window.
Romacs implementation of the GNU-conventional API.  RET visits, TAB completes
local paths, M-p/M-n recall filename history, and C-g cancels the prompt.
Local directories open the read-only Dired browser.  Remote handlers,
wildcards and file-local evaluation are unsupported."
  (interactive (list (read-file-name "Find file: ") nil))
  (let ((buffer (find-file-noselect filename nil nil wildcards)))
    (romacs-select-buffer buffer)))

(defun romacs-prompt--replace-input (text)
  "Replace the active minibuffer input with TEXT, retaining its prompt."
  (delete-region romacs-prompt--input-start (point-max))
  (goto-char (point-max))
  (insert text))

(defun romacs-find-file-complete ()
  "Complete a local path using native directory enumeration.
Append a slash to directory candidates, extend the common prefix, and show
up to twelve ambiguous candidates.  Refuse directories above 4096 entries
rather than present a misleading truncated completion."
  (interactive)
  (let* ((input (buffer-substring-no-properties romacs-prompt--input-start (point-max)))
         (default-directory romacs-find-file--directory)
         (full (romacs-find-file-local-name input))
         (full (if (and (> (length input) 0) (equal (substring input -1) "/")
                        (not (equal (substring full -1) "/"))) (concat full "/") full))
         (directory (file-name-directory full))
         (prefix (file-name-nondirectory full))
         (entries (directory-files directory nil nil nil 4097))
         (candidates nil) (common nil) (count 0) (labels ""))
    (when (> (length entries) 4096)
      (signal 'file-error (list "Completion directory exceeds 4096 entries" directory)))
    (while entries
      (let ((entry (car entries)))
        (when (and (not (member entry '("." "..")))
                   (<= (length prefix) (length entry))
                   (equal prefix (substring entry 0 (length prefix))))
          (when (file-directory-p (concat directory entry)) (setq entry (concat entry "/")))
          (setq candidates (cons entry candidates))))
      (setq entries (cdr entries)))
    (setq candidates (nreverse candidates))
    (let ((remaining candidates))
      (while remaining
        (let ((entry (car remaining)))
          (if (null common) (setq common entry)
            (let ((i 0) (limit (min (length common) (length entry))))
              (while (and (< i limit) (= (aref common i) (aref entry i))) (setq i (1+ i)))
              (setq common (substring common 0 i))))
          (when (< count 12) (setq labels (concat labels (if (= count 0) "" "  ") entry)))
          (setq count (1+ count)))
        (setq remaining (cdr remaining))))
    (cond ((null common) (message "No matching local files"))
          ((> (length common) (length prefix))
           (romacs-prompt--replace-input (concat (or (file-name-directory input) "") common)))
          ((= count 1) (message "Complete"))
          (t (message "%s%s" labels (if (> count 12) "  …" ""))))))

(defun romacs-prompt-previous-history ()
  "Recall an older entry from the active filename or buffer history variable."
  (interactive)
  (when (null romacs-prompt--history-position)
    (setq romacs-prompt--history-input
          (buffer-substring-no-properties romacs-prompt--input-start (point-max))))
  (let* ((next (if romacs-prompt--history-position (1+ romacs-prompt--history-position) 0))
         (entry (nth next (symbol-value romacs-prompt--history-variable))))
    (if entry
        (progn (setq romacs-prompt--history-position next) (romacs-prompt--replace-input entry))
      (message "No older history entry"))))

(defun romacs-prompt-next-history ()
  "Recall a newer history entry, or restore the input preceding recall."
  (interactive)
  (when romacs-prompt--history-position
    (if (= romacs-prompt--history-position 0)
        (progn (setq romacs-prompt--history-position nil)
               (romacs-prompt--replace-input romacs-prompt--history-input))
      (setq romacs-prompt--history-position (1- romacs-prompt--history-position))
      (romacs-prompt--replace-input (nth romacs-prompt--history-position (symbol-value romacs-prompt--history-variable))))))

(defun romacs-read-file-name (prompt &optional directory default-filename mustmatch initial predicate)
  "Read a local filename through the existing minibuffer and HostInput loop.
Use editable DIRECTORY (default `default-directory'), path completion and
filename history.  Non-nil MUSTMATCH requires an existing regular file."
  (let* ((directory (romacs-find-file-local-name (or directory default-directory "./")))
         (directory (if (equal (substring directory -1) "/") directory (concat directory "/")))
         (romacs-find-file--directory directory)
         (romacs-prompt--input-start (1+ (length prompt)))
         (romacs-prompt--history-position nil)
         (romacs-prompt--history-input nil)
         (romacs-prompt--history-variable 'file-name-history)
         (map (make-sparse-keymap))
         (default-directory directory)
         result)
    (set-keymap-parent map minibuffer-local-map)
    (define-key map [9] 'romacs-find-file-complete)
    (define-key map [13] 'exit-minibuffer)
    (define-key map [10] 'exit-minibuffer)
    (define-key map [7] 'abort-minibuffers)
    (define-key map [27 112] 'romacs-prompt-previous-history)
    (define-key map [27 110] 'romacs-prompt-next-history)
    (setq result (read-from-minibuffer prompt (or initial directory) map nil 'file-name-history))
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
    (unless (equal result (car file-name-history))
      (if (fboundp 'add-to-history) (add-to-history 'file-name-history result)
        (setq file-name-history (cons result (delete result file-name-history)))))
    result))

(defun read-file-name (prompt &optional directory default-filename mustmatch initial predicate)
  "Read a filename with the customizable `read-file-name-function'."
  (funcall read-file-name-function prompt directory default-filename mustmatch initial predicate))

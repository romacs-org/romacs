;;; romacs-find-file.el --- File visiting -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; File visiting for Romacs, in both regimes.  The workspace reads file
;; names with romacs-workspace-completion.el; --loadup with GNU's
;; `read-file-name'.

;;; Code:
(unless (boundp 'default-directory) (setq default-directory (expand-file-name "./")))
(defvar file-name-history nil)
(defvar find-file-hook nil)
(defvar find-file-not-found-functions nil)
(defvar auto-mode-alist '(("\\.el\\'" . emacs-lisp-mode)))
(defvar romacs-find-file-mode-function 'romacs-find-file-set-auto-mode)

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

;; GNU's files.el owns visiting once loaded (--loadup): its
;; `find-file-noselect' names buffers with uniquify, finds a visited
;; file by truename and runs `after-find-file'.  The workspace, without
;; it, visits with this slim reader.
(unless (fboundp 'find-file-noselect-1)
  (defun find-file-noselect (filename &optional nowarn rawfile wildcards)
    "Return a buffer visiting local UTF-8 FILENAME without selecting it.
Romacs's slim file visitor; NOWARN is accepted.  RAWFILE and wildcard
expansion are unsupported.  Modes and hooks are independent Lisp policy."
    (when (or rawfile wildcards)
      (signal 'file-error (list "Raw or wildcard file visiting is unsupported" filename)))
    (romacs-find-file-noselect filename))

  (defun find-file (filename &optional wildcards)
    "Edit local UTF-8 FILENAME in the selected window.
Romacs implementation of the GNU-conventional API, reading FILENAME with
`read-file-name'.  Local directories open the read-only Dired browser.
Remote handlers, wildcards and file-local evaluation are unsupported."
    (interactive (list (read-file-name "Find file: ") nil))
    (let ((buffer (find-file-noselect filename nil nil wildcards)))
      (romacs-select-buffer buffer))))

;;; romacs-find-file.el ends here

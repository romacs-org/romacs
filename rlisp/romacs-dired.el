;;; romacs-dired.el --- Directory browsing -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Directory browsing for Romacs.

;;; Code:
(defvar dired-directory nil)
(defvar dired-mode-hook nil)
(defvar romacs-dired-rows nil)
(defvar romacs-dired-return-buffer nil)
(defvar dired-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map [110] 'dired-next-line)
    (define-key map [112] 'dired-previous-line)
    (define-key map [13] 'dired-find-file)
    (define-key map [94] 'dired-up-directory)
    (define-key map [103] 'dired-revert)
    (define-key map [113] 'dired-quit-window)
    map))

(defun romacs-dired-label (name)
  "Escape a filename into one display line without changing its stored path."
  (let ((i 0) (text ""))
    (while (< i (length name))
      (let ((character (aref name i)))
        (setq text (concat text
                          (cond ((= character 92) "\\\\")
                                ((= character 10) "\\n")
                                ((= character 9) "\\t")
                                ((= character 13) "\\r")
                                ((or (< character 32) (= character 127))
                                 (format "\\x%02x" character))
                                (t (substring name i (1+ i)))))))
      (setq i (1+ i)))
    text))

(defun romacs-dired-directory (directory)
  "Return an expanded local directory name ending in a slash."
  (let ((name (romacs-find-file-local-name directory)))
    (unless (file-directory-p name)
      (signal 'file-missing (list "Directory does not exist" name)))
    (if (equal (substring name -1) "/") name (concat name "/"))))

(defface dired-header '((t (:foreground "#00b8ad"))) "Directory header.")
(defface dired-directory '((t (:foreground "#72b6e4"))) "Directory names.")
(defface dired-symlink '((t (:foreground "#00a6a6"))) "Symbolic links.")
(defface dired-broken-symlink '((t (:foreground "#d8796f"))) "Broken symbolic links.")
(defface dired-perm-write '((t (:foreground "#756b59"))) "Group and other write permissions.")
(defface dired-set-id '((t (:foreground "#e89e30"))) "Set-id permissions.")

(defun romacs-dired-pad (text width &optional right)
  "Pad TEXT to WIDTH, right-aligning when RIGHT is non-nil."
  (let ((padding (make-string (max 0 (- width (length text))) 32)))
    (if right (concat padding text) (concat text padding))))

(defun romacs-dired-date (time now)
  "Format TIME with GNU's recent-file or old-file date columns."
  (let ((age (- now (float-time time))))
    (format-time-string (if (and (>= age -3600) (< age 15778476))
                            "%b %e %H:%M" "%b %e  %Y") time)))

(defun romacs-dired-permissions (mode)
  "Apply semantic permission faces without changing the mode string."
  (let ((text (propertize mode)))
    (dolist (position '(5 8))
      (when (= (aref text position) 119)
        (put-text-property position (1+ position) 'face 'dired-perm-write text)))
    (dolist (position '(3 6))
      (when (or (= (aref text position) 115) (= (aref text position) 83))
        (put-text-property position (1+ position) 'face 'dired-set-id text)))
    text))

(defun romacs-dired-free-space (directory)
  "Describe available bytes using GNU's default IEC display convention."
  (let ((available (nth 2 (file-system-info directory)))
        (units '("B" "KiB" "MiB" "GiB" "TiB" "PiB" "EiB")))
    (when available
      (while (and (>= available 1024) (cdr units))
        (setq available (/ available 1024.0) units (cdr units)))
      (if (and (< available 10) (>= (mod available 1.0) 0.05)
               (< (mod available 1.0) 0.95))
          (let ((tenths (round (* available 10))))
            (format "%d.%d %s" (/ tenths 10) (mod tenths 10) (car units)))
        (format "%d %s" (round available) (car units))))))

(defun romacs-dired-listing (directory)
  "Prepare GNU-style long rows and exact filenames before modifying any buffer."
  (let ((entries (directory-files directory nil nil nil 4097))
        (text (concat "  " (propertize
                            (concat (romacs-dired-label (if (equal directory "/") "/" (substring directory 0 -1))) ":")
                            'face 'dired-header)
                      (let ((space (romacs-dired-free-space directory)))
                        (if space (concat " (" space " available)") "")) "\n"))
        (records nil) (rows nil) (first-entry nil) (links-width 1) (owner-width 1)
        (group-width 1) (size-width 1) (now (float-time)))
    (when (> (length entries) 4096)
      (signal 'file-error (list "Directory listing exceeds 4096 entries" directory)))
    (dolist (name entries)
      (let* ((path (expand-file-name name directory))
             (attributes (file-attributes path 'string)))
        (when attributes
          (let ((links (format "%s" (nth 1 attributes)))
                (owner (format "%s" (nth 2 attributes)))
                (group (format "%s" (nth 3 attributes)))
                (size (format "%s" (nth 7 attributes))))
            (setq links-width (max links-width (length links))
                  owner-width (max owner-width (length owner))
                  group-width (max group-width (length group))
                  size-width (max size-width (length size)))
            (push (list name path attributes links owner group size) records)))))
    (dolist (record (nreverse records))
      (let* ((name (nth 0 record)) (path (nth 1 record)) (attributes (nth 2 record))
             (kind (car attributes))
             (target (and (stringp kind)
                          (condition-case nil (romacs-dired-entry-attributes path) (file-error nil))))
             (face (cond ((eq kind t) 'dired-directory)
                         ((stringp kind) (if target 'dired-symlink 'dired-broken-symlink))))
             (label (propertize (romacs-dired-label name) 'dired-filename t 'mouse-face 'highlight))
             (prefix (concat "  " (romacs-dired-permissions (nth 8 attributes)) " "
                             (romacs-dired-pad (nth 3 record) links-width t) " "
                             (romacs-dired-pad (nth 4 record) owner-width) " "
                             (romacs-dired-pad (nth 5 record) group-width) " "
                             (romacs-dired-pad (nth 6 record) size-width t) " "
                             (romacs-dired-date (nth 5 attributes) now) " "))
             (start (1+ (length text))))
        (when face (put-text-property 0 (length label) 'face face label))
        (push (list (+ start (length prefix)) path start) rows)
        (when (and (not first-entry) (not (member name '("." ".."))))
          (setq first-entry (+ start (length prefix))))
        (setq text (concat text prefix label
                           (if (stringp kind)
                               (concat (propertize " -> " 'face 'dired-symlink)
                                       (propertize (romacs-dired-label kind) 'face
                                                   (cond ((eq (car target) t) 'dired-directory)
                                                         (target 'default)
                                                         (t 'dired-broken-symlink)))) "") "\n"))))
    (list text (nreverse rows) first-entry)))

(defun romacs-dired-current-row ()
  "Return the stored row containing point, never a path parsed from text."
  (let ((rows romacs-dired-rows) (found nil))
    (while rows
      (when (and (>= (point) (nth 2 (car rows)))
                 (if (cdr rows) (< (point) (nth 2 (car (cdr rows))))
                   (< (point) (point-max))))
        (setq found (car rows)))
      (setq rows (cdr rows)))
    found))

(defun dired-get-filename (&optional localp noerror)
  "Return the exact path at point, or its basename when LOCALP is non-nil."
  (let ((row (romacs-dired-current-row)))
    (if row
        (if localp (file-name-nondirectory (car (cdr row))) (car (cdr row)))
      (unless noerror (signal 'user-error '("No directory entry at point"))))))

(defun dired-goto-file (file)
  "Move to the stored row for FILE and return non-nil if present."
  (let ((path (expand-file-name file default-directory))
        (rows romacs-dired-rows) (found nil))
    (while (and rows (not found))
      (when (equal path (car (cdr (car rows)))) (setq found (car rows)))
      (setq rows (cdr rows)))
    (when found (goto-char (car found)) t)))

(defun dired-revert (&optional ignore-auto noconfirm)
  "Refresh this directory, preserving the selected path if it still exists.
Enumeration failure leaves the old listing and path mapping unchanged."
  (interactive)
  (let* ((selected (dired-get-filename nil t))
         (listing (romacs-dired-listing dired-directory))
         (inhibit-read-only t))
    (erase-buffer)
    (insert (car listing))
    (setq romacs-dired-rows (car (cdr listing)))
    (goto-char (or (nth 2 listing) (and romacs-dired-rows (car (car romacs-dired-rows))) (point-min)))
    (when selected (dired-goto-file selected))
    (set-buffer-modified-p nil)))

(defun dired-mode ()
  "Enable Romacs's read-only local directory browser.
This browsing slice has no delete, rename, copy, marking or shell operations."
  (interactive)
  (make-local-variable 'major-mode)
  (make-local-variable 'mode-name)
  (make-local-variable 'mode-line-format)
  (setq major-mode 'dired-mode mode-name "Dired"
        mode-line-format '("  %b   (Dired by name)"))
  (use-local-map dired-mode-map)
  (setq buffer-read-only t)
  (run-hooks 'dired-mode-hook))

(defun dired-noselect (directory &optional switches)
  "Return a reusable buffer for local DIRECTORY without selecting it.
SWITCHES, remote names and non-UTF-8 filenames are unsupported."
  (when switches (signal 'user-error '("Dired listing switches are unsupported")))
  (let ((directory (romacs-dired-directory directory))
        (entry (current-buffer)) (buffers (buffer-list)) (buffer nil))
    (while (and buffers (not buffer))
      (when (with-current-buffer (car buffers)
              (and (eq major-mode 'dired-mode) (equal dired-directory directory)))
        (setq buffer (car buffers)))
      (setq buffers (cdr buffers)))
    (unless buffer
      (let ((listing (romacs-dired-listing directory)) (completed nil))
        (setq buffer (get-buffer-create (generate-new-buffer-name
                                        (if (equal directory "/") "/"
                                          (file-name-nondirectory (substring directory 0 -1))))))
        (unwind-protect
            (progn
              (with-current-buffer buffer
                (make-local-variable 'default-directory)
                (make-local-variable 'dired-directory)
                (make-local-variable 'romacs-dired-rows)
                (make-local-variable 'romacs-dired-return-buffer)
                (setq default-directory directory dired-directory directory
                      romacs-dired-return-buffer entry buffer-file-name nil buffer-file-truename nil)
                (insert (car listing))
                (setq romacs-dired-rows (car (cdr listing)))
                (goto-char (or (nth 2 listing) (and romacs-dired-rows (car (car romacs-dired-rows))) (point-min)))
                (set-buffer-modified-p nil)
                (dired-mode))
              (setq completed t))
          (when (and (not completed) (buffer-live-p buffer))
            (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer))))))
    buffer))

(defun dired (directory &optional switches)
  "Browse a local DIRECTORY in the selected window using GNU-style keys."
  (interactive (list (read-file-name "Dired (directory): ")))
  (romacs-select-buffer (dired-noselect directory switches)))

(defun dired-next-line (&optional count)
  "Move COUNT rows forward, clamped to the available directory entries."
  (interactive "p")
  (let* ((current (romacs-dired-current-row)) (rows romacs-dired-rows)
         (index 0) (position 0))
    (while rows
      (when (eq current (car rows)) (setq index position))
      (setq position (1+ position) rows (cdr rows)))
    (when romacs-dired-rows
      (setq index (max 0 (min (1- position) (+ index (or count 1)))))
      (goto-char (car (nth index romacs-dired-rows))))))

(defun dired-previous-line (&optional count)
  "Move COUNT rows backward within this directory."
  (interactive "p")
  (dired-next-line (- (or count 1))))

(defun romacs-dired-entry-attributes (file)
  "Inspect an entry through at most forty links, returning nil if missing."
  (let ((attributes (file-attributes file 'integer)) (links 0))
    (while (and attributes (stringp (car attributes)))
      (when (>= links 40) (signal 'file-error (list "Too many symbolic links" file)))
      (setq file (expand-file-name (car attributes) (file-name-directory file))
            attributes (file-attributes file 'integer) links (1+ links)))
    attributes))

(defun dired-find-file ()
  "Open the exact entry at point, refusing removed entries and dangling links."
  (interactive)
  (let* ((file (dired-get-filename))
         (attributes (romacs-dired-entry-attributes file)))
    (unless attributes
      (signal 'file-missing (list "Directory entry no longer exists" file)))
    (unless (or (eq (car attributes) t) (= (aref (nth 8 attributes) 0) 45))
      (signal 'file-error (list "Only regular files and directories can be visited" file)))
    (find-file file)))

(defun dired-up-directory ()
  "Visit the lexical parent and select the child directory just departed."
  (interactive)
  (let* ((child (if (equal dired-directory "/") "/" (substring dired-directory 0 -1)))
         (parent (or (file-name-directory child) "/")))
    (dired parent)
    (dired-goto-file child)))

(defun dired-quit-window ()
  "Return to the entry buffer without killing it or the directory buffer."
  (interactive)
  (let ((target romacs-dired-return-buffer))
    (unless (and (buffer-live-p target) (not (eq target (current-buffer))))
      (setq target (romacs-buffer-default))
      (when (eq target (current-buffer)) (setq target (get-buffer-create "*scratch*"))))
    (romacs-select-buffer target)))

;;; romacs-pass-menu.el --- The password store as a list  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; `romacs-pass' lists the password store's entries, one per line, in
;; its folders, which open and close in place.  n and p move by line, f
;; opens the folder at point and b closes the one point is in; RET copies
;; the entry at point (or opens its folder), and the header line names
;; the other keys.  The list shows names only: no secret is decrypted
;; until a command asks for one.

;;; Code:

(require 'romacs-pass)

(defvar warm-restart-regenerable)

(defgroup romacs-pass-menu nil
  "The password store's list."
  :group 'romacs-pass)

(defface romacs-pass-folder '((t :inherit font-lock-function-name-face))
  "A folder's line in the password store's list.")

(defface romacs-pass-entry '((t :inherit default))
  "An entry's line in the password store's list.")

(defvar-local romacs-pass-menu--open nil
  "The folders shown open, as names ending in a slash.")

(defun romacs-pass-menu--rows (entries open)
  "The lines showing ENTRIES with the folders in OPEN open.
Each is (KIND NAME DEPTH): KIND `folder' (NAME ends in a slash) or
`entry'.  A folder comes before what it holds; what a closed folder
holds is not shown."
  (let ((rows nil) (seen (make-hash-table :test #'equal)))
    (dolist (entry entries)
      (let* ((parts (split-string entry "/"))
             (depth 0) (prefix "") (visible t))
        (dolist (part (butlast parts))
          (setq prefix (concat prefix part "/"))
          (when (and visible (not (gethash prefix seen)))
            (puthash prefix t seen)
            (push (list 'folder prefix depth) rows))
          (setq visible (and visible (member prefix open))
                depth (1+ depth)))
        (when visible (push (list 'entry entry depth) rows))))
    (nreverse rows)))

(defun romacs-pass-menu--line (row open)
  "ROW's text, as `romacs-pass-menu--rows' gives it, with OPEN folders."
  (pcase-let* ((`(,kind ,name ,depth) row)
               (base (file-name-nondirectory (directory-file-name name)))
               (text (if (eq kind 'folder)
                         (propertize (concat (if (member name open) "▾ " "▸ ") base "/")
                                     'face 'romacs-pass-folder)
                       (propertize (concat "  " base) 'face 'romacs-pass-entry))))
    (concat (make-string (* 2 depth) ?\s)
            (propertize text 'romacs-pass-row (cons kind name))
            "\n")))

(defun romacs-pass-menu--render ()
  "Show the store's entries again, keeping point on its line's name, else
on its line."
  (let ((at (romacs-pass-menu--at))
        (line (line-number-at-pos))
        (inhibit-read-only t))
    (erase-buffer)
    (dolist (row (romacs-pass-menu--rows (romacs-pass-entries) romacs-pass-menu--open))
      (insert (romacs-pass-menu--line row romacs-pass-menu--open)))
    (unless (and at (romacs-pass-menu--goto (cdr at)))
      (goto-char (point-min))
      (romacs-pass-menu-next (1- line)))))

(defun romacs-pass-menu--at ()
  "The line at point as (KIND . NAME), or nil."
  (save-excursion
    (back-to-indentation)
    (get-text-property (point) 'romacs-pass-row)))

(defun romacs-pass-menu--goto (name)
  "Move to NAME's line and return point, or nil when no line shows it."
  (let ((found (save-excursion
                 (goto-char (point-min))
                 (let (pos)
                   (while (and (not pos) (not (eobp)))
                     (back-to-indentation)
                     (if (equal (cdr (get-text-property (point) 'romacs-pass-row)) name)
                         (setq pos (point))
                       (forward-line 1)))
                   pos))))
    (when found (goto-char found))))

(defun romacs-pass-menu--entry ()
  "The entry at point, or an error."
  (pcase (romacs-pass-menu--at)
    (`(entry . ,name) name)
    (_ (user-error "No entry on this line"))))

(defun romacs-pass-menu-next (n)
  "Move N lines down, staying on the list's lines."
  (interactive "p" romacs-pass-menu-mode)
  (forward-line n)
  (when (and (eobp) (not (bobp)) (bolp)) (forward-line -1))
  (back-to-indentation))

(defun romacs-pass-menu-previous (n)
  "Move N lines up."
  (interactive "p" romacs-pass-menu-mode)
  (romacs-pass-menu-next (- n)))

(defun romacs-pass-menu-open ()
  "Open the folder at point and move into it."
  (interactive nil romacs-pass-menu-mode)
  (pcase (romacs-pass-menu--at)
    (`(folder . ,name)
     (unless (member name romacs-pass-menu--open)
       (push name romacs-pass-menu--open)
       (romacs-pass-menu--render))
     (romacs-pass-menu-next 1))
    (_ (user-error "No folder on this line"))))

(defun romacs-pass-menu-close ()
  "Close the folder at point, or the one point is in, and move to it."
  (interactive nil romacs-pass-menu-mode)
  (let* ((at (or (romacs-pass-menu--at) (user-error "Not in a folder")))
         (folder (if (and (eq (car at) 'folder) (member (cdr at) romacs-pass-menu--open))
                     (cdr at)
                   (file-name-directory (directory-file-name (cdr at))))))
    (unless folder (user-error "Not in a folder"))
    (setq romacs-pass-menu--open
          (seq-remove (lambda (open) (string-prefix-p folder open)) romacs-pass-menu--open))
    (romacs-pass-menu--render)
    (romacs-pass-menu--goto folder)))

(defun romacs-pass-menu-visit ()
  "Copy the entry at point, or open or close the folder at point."
  (interactive nil romacs-pass-menu-mode)
  (pcase (romacs-pass-menu--at)
    (`(entry . ,name) (romacs-pass-copy name))
    (`(folder . ,name) (if (member name romacs-pass-menu--open)
                           (romacs-pass-menu-close)
                         (romacs-pass-menu-open)))))

(defun romacs-pass-menu-copy ()
  "Copy the password of the entry at point."
  (interactive nil romacs-pass-menu-mode)
  (romacs-pass-copy (romacs-pass-menu--entry)))

(defun romacs-pass-menu-otp ()
  "Copy the one-time code of the entry at point."
  (interactive nil romacs-pass-menu-mode)
  (romacs-pass-otp (romacs-pass-menu--entry)))

(defun romacs-pass-menu-autotype ()
  "Type the user name and password of the entry at point."
  (interactive nil romacs-pass-menu-mode)
  (romacs-pass-autotype (romacs-pass-menu--entry)))

(defun romacs-pass-menu-edit ()
  "Edit the entry at point."
  (interactive nil romacs-pass-menu-mode)
  (romacs-pass-edit (romacs-pass-menu--entry)))

(defun romacs-pass-menu--folder ()
  "The folder of the line at point, as a name ending in a slash, or nil."
  (pcase (romacs-pass-menu--at)
    (`(folder . ,name) name)
    (`(entry . ,name) (file-name-directory name))))

(defun romacs-pass-menu-generate (entry)
  "Generate a password for a new ENTRY, read in the folder at point."
  (interactive (list (completing-read "Generate entry: " (romacs-pass-entries) nil nil
                                      (romacs-pass-menu--folder) 'romacs-pass-history))
               romacs-pass-menu-mode)
  (romacs-pass-generate entry))

(defun romacs-pass-menu-insert (entry)
  "Add an ENTRY, read in the folder at point."
  (interactive (list (completing-read "Insert entry: " (romacs-pass-entries) nil nil
                                      (romacs-pass-menu--folder) 'romacs-pass-history))
               romacs-pass-menu-mode)
  (romacs-pass--refuse-existing entry)
  (romacs-pass-insert entry (read-passwd (format "Password for %s: " entry) t)))

(defvar-keymap romacs-pass-menu-mode-map
  :parent special-mode-map
  "n" #'romacs-pass-menu-next
  "p" #'romacs-pass-menu-previous
  "f" #'romacs-pass-menu-open
  "b" #'romacs-pass-menu-close
  "RET" #'romacs-pass-menu-visit
  "c" #'romacs-pass-menu-copy
  "o" #'romacs-pass-menu-otp
  "a" #'romacs-pass-menu-autotype
  "e" #'romacs-pass-menu-edit
  "+" #'romacs-pass-menu-generate
  "i" #'romacs-pass-menu-insert)

(define-derived-mode romacs-pass-menu-mode special-mode "Pass"
  "The password store's entries, by folder.
\\{romacs-pass-menu-mode-map}"
  (setq-local revert-buffer-function (lambda (&rest _) (romacs-pass-menu--render))
              truncate-lines t
              warm-restart-regenerable "the password store's list")
  (setq header-line-format
        (substitute-command-keys
         (concat "\\<romacs-pass-menu-mode-map>"
                 "\\[romacs-pass-menu-visit] copy  \\[romacs-pass-menu-otp] code  "
                 "\\[romacs-pass-menu-autotype] type  \\[romacs-pass-menu-edit] edit  "
                 "\\[romacs-pass-menu-generate] generate  \\[romacs-pass-menu-insert] insert  "
                 "\\[romacs-pass-menu-open]/\\[romacs-pass-menu-close] fold  \\[revert-buffer] refresh"))))

;;;###autoload
(defun romacs-pass ()
  "List the password store's entries."
  (interactive)
  (with-current-buffer (get-buffer-create "*Password Store*")
    (unless (derived-mode-p 'romacs-pass-menu-mode) (romacs-pass-menu-mode))
    (romacs-pass-menu--render)
    (pop-to-buffer (current-buffer))))

(provide 'romacs-pass-menu)
;;; romacs-pass-menu.el ends here

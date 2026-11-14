;;; romacs-agent-gate.el --- What an agent's edit would do, before it lands  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A harness asks Romacs about an edit before it writes it (Claude Code's
;; PreToolUse hook).  The gate applies the edit to the file's text, in
;; memory, and refuses one that would leave an Emacs Lisp file that read
;; before unreadable, saying where: the model fixes its edit in the same
;; turn, and the broken file never lands.  An edit the gate cannot apply
;; (its old text not found) is the harness's to refuse: the gate lets it be.

;;; Code:

(require 'cl-lib)

(defcustom romacs-agent-gate t
  "Whether an agent's edit to an Emacs Lisp file that would leave it
unreadable is refused before it is written."
  :type 'boolean :group 'romacs-agent)

(defun romacs-agent-gate--replace (text old new all)
  "TEXT with OLD replaced by NEW, every one when ALL, else the one; nil
when OLD is not there to replace."
  (let ((at (and (stringp old) (not (string-empty-p old)) (string-search old text))))
    (when at
      (if all (string-replace old new text)
        (concat (substring text 0 at) new (substring text (+ at (length old))))))))

(defun romacs-agent-gate--after (tool input before)
  "The text TOOL's INPUT leaves of a file whose text is BEFORE (nil for
none), or nil when it cannot tell."
  (pcase tool
    ("Write" (gethash "content" input))
    ("Edit" (and before (romacs-agent-gate--replace before (gethash "old_string" input) (gethash "new_string" input)
                                                    (eq (gethash "replace_all" input) t))))
    ("MultiEdit" (let ((text before))
                   (cl-loop for edit across (or (gethash "edits" input) [])
                            while text
                            do (setq text (romacs-agent-gate--replace text (gethash "old_string" edit)
                                                                     (gethash "new_string" edit)
                                                                     (eq (gethash "replace_all" edit) t))))
                   text))))

(defun romacs-agent-gate--unreadable (text)
  "Where TEXT, as Emacs Lisp, stops reading: (LINE . MESSAGE), or nil when
it reads to its end."
  (with-temp-buffer
    (insert text)
    (emacs-lisp-mode)
    (goto-char (point-min))
    (condition-case err
        (progn (while (progn (forward-comment (buffer-size)) (not (eobp)))
                 (forward-sexp 1))
               nil)
      (scan-error (cons (line-number-at-pos (point)) (cadr err))))))

(defun romacs-agent-gate-check (tool input)
  "Nil when TOOL's INPUT may go on, else why not: an edit that leaves an
Emacs Lisp file which read before unreadable."
  (let ((file (and romacs-agent-gate (hash-table-p input) (gethash "file_path" input))))
    (when (and (stringp file) (string-match-p "\\.el\\'" file))
      (let* ((before (and (file-readable-p file)
                          (with-temp-buffer (insert-file-contents file) (buffer-string))))
             (after (romacs-agent-gate--after tool input before)))
        (when (and after (not (and before (romacs-agent-gate--unreadable before))))
          (when-let* ((where (romacs-agent-gate--unreadable after)))
            (format "Romacs refused this %s: it would leave %s unreadable as Emacs Lisp: the form that starts at line %d does not read (%s). \
The file is unchanged; correct the edit so its parentheses and strings balance."
                    tool (abbreviate-file-name file) (car where) (cdr where))))))))

(provide 'romacs-agent-gate)
;;; romacs-agent-gate.el ends here

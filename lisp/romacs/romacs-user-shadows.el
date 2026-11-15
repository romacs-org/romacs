;;; romacs-user-shadows.el --- A user's copy of loaded Lisp wins  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU loads most of its Lisp at first use, so a directory an init puts on
;; `load-path' shadows the bundled copy of any library it has not dumped.
;; A Romacs session starts with many more libraries loaded (its base world
;; holds them), which a `require' then finds loaded and reads no copy of.
;; Once the init has run, each loaded library that a directory added to
;; `load-path' since the session began now shadows is loaded from that
;; directory, as a source session would have loaded it at its first use.

;;; Code:

(defvar romacs--load-path-at-start nil
  "`load-path' as the session began, before its packages and its init.")

(defun romacs--same-library-p (a b)
  "Whether files A and B are one library's, whatever their suffixes."
  (equal (file-name-sans-extension (file-truename a))
         (file-name-sans-extension (file-truename b))))

(defun romacs--shadowing-copies (added)
  "The libraries in the directories ADDED that shadow loaded ones: each
file `locate-library' now finds first for a loaded feature of its name
that loaded from another file, in `load-path' order."
  (let (found)
    (dolist (dir added (nreverse found))
      (dolist (file (and (file-directory-p dir) (directory-files dir t "\\.el\\'")))
        (let* ((name (file-name-base file))
               (feature (intern-soft name))
               (first (and feature (featurep feature) (locate-library name)))
               (loaded (and first (symbol-file feature 'provide))))
          (when (and first (romacs--same-library-p first file)
                     (not (and loaded (romacs--same-library-p loaded file))))
            (push file found)))))))

(defun romacs--load-user-shadows (start)
  "Load the user's copy of each loaded library that a directory added to
`load-path' since START, its value as the session began, now shadows.
A copy that signals is a warning, and the rest still load."
  (dolist (file (romacs--shadowing-copies (seq-remove (lambda (dir) (member dir start)) load-path)))
    (condition-case err
        (load file nil t)
      (error (display-warning 'romacs (format-message "Loading `%s', which shadows a library loaded before the init: %s"
                                                      (abbreviate-file-name file) (error-message-string err)))))))

(provide 'romacs-user-shadows)
;;; romacs-user-shadows.el ends here

;;; romacs-saved-settings.el --- Saved settings stay out of the init  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU saves customizations into `custom-file', or into the user's init
;; when that is nil, and `enable-command' and `disable-command' append to
;; the init themselves.  Nothing here writes the init: a session's
;; `custom-file' is `romacs-saved-settings-default-file' unless the user names
;; another, the function `custom-file' never answers the init whatever the
;; variable says, and the two commands write the file it answers.
;; `romacs-saved-settings-start' loads that file after Romacs's defaults and before
;; the init, so a saved value replaces a default and the init has the last
;; word.  DIVERGENCES.md VM2-CUSTOM-LOAD, VM2-CUSTOM-FILE-GUARD and
;; VM2-NOVICE-CUSTOM-FILE.
;;
;; This file only defines.  The loadup chain holds it, so a batch program
;; has the same guard (romacs-loadup-bootstrap.el), and user_init.rs calls
;; `romacs-saved-settings-start' in a session that loads an init.

;;; Code:

(defun romacs-saved-settings-default-file ()
  "The file saved settings go to unless `custom-file' names another."
  (expand-file-name "custom.el" user-emacs-directory))

(defun romacs-saved-settings--init-p (file)
  "Whether writing FILE would write the user's init.
FILE is taken as a writer takes it, expanded first, so `..' past a link
means what `find-file-noselect' makes of it.  Then by name with links
followed, which holds for an init not written yet, and by identity, for
a hard link, whose buffer `find-file-noselect' would share."
  (let ((file (expand-file-name file)) (init (expand-file-name user-init-file)))
    (or (equal (file-truename file) (file-truename init))
        (file-equal-p file init))))

(defun romacs-saved-settings--file (resolve &optional no-error)
  "What RESOLVE, GNU's function `custom-file', answers, but never the init.
The answer is expanded, the name that was checked.  With the init as the
answer it is `romacs-saved-settings-default-file', unless that is the init too:
then NO-ERROR gives nil and otherwise it signals, as GNU's does when
there is no file to save to."
  (let ((file (funcall resolve no-error)) (default (romacs-saved-settings-default-file)))
    (cond ((null file) nil)
          ((not (romacs-saved-settings--init-p file)) (expand-file-name file))
          ((not (romacs-saved-settings--init-p default)) default)
          (no-error nil)
          (t (user-error "No file for customizations other than the init")))))

(defun romacs-saved-settings--record-command (command disable)
  "Set COMMAND's `disabled' property to DISABLE, and keep it in the saved settings.
novice.el's `en/disable-command' with the destination changed: it appends
to the init, and opens that name through `substitute-in-file-name', which
may not be the file checked.  With no file to save to, the session only."
  (unless (commandp command) (error "Invalid command name `%s'" command))
  (put command 'disabled disable)
  (require 'cus-edit)
  (when-let* ((file (custom-file t)))
    (with-current-buffer (find-file-noselect file)
      (goto-char (point-min))
      (when (search-forward (format "(put '%s " command) nil t)
        (delete-region (line-beginning-position) (line-beginning-position 2)))
      (goto-char (point-max))
      (unless (bolp) (newline))
      (insert (format "(put '%s 'disabled %s)\n" command disable))
      (save-buffer))))

(with-eval-after-load 'cus-edit
  (advice-add 'custom-file :around #'romacs-saved-settings--file))
(with-eval-after-load 'novice
  (advice-add 'en/disable-command :override #'romacs-saved-settings--record-command))

(defun romacs-saved-settings-start ()
  "Give this session its `custom-file', and load it.
A file that fails to load is reported and does not stop the init; one
that is the init, through a link, is the init's to run."
  (unless custom-file (setq custom-file (romacs-saved-settings-default-file)))
  (ignore-error file-error (make-directory (file-name-directory custom-file) t))
  (unless (romacs-saved-settings--init-p custom-file)
    (condition-case err
        (load custom-file t t t)
      (error (display-warning 'initialization
                              (format-message "Error loading `%s': %s" custom-file
                                              (error-message-string err))
                              :error)))))

(provide 'romacs-saved-settings)
;;; romacs-saved-settings.el ends here

;;; romacs-custom.el --- Saved settings stay out of the init  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU saves customizations into `custom-file', or into the user's init
;; when that is nil, and `enable-command' and `disable-command' append to
;; the init themselves.  Nothing here writes the init: a session's
;; `custom-file' is `romacs-custom-default-file' unless the user names
;; another, the function `custom-file' never answers the init whatever the
;; variable says, and the two commands write the file it answers.
;; `romacs-custom-start' loads that file after Romacs's defaults and before
;; the init, so a saved value replaces a default and the init has the last
;; word.  DIVERGENCES.md VM2-CUSTOM-LOAD, VM2-CUSTOM-FILE-GUARD and
;; VM2-NOVICE-CUSTOM-FILE.
;;
;; This file only defines.  The GUI's world holds it (startup.rs
;; `install_session_definitions'), and user_init.rs calls
;; `romacs-custom-start' unless the session skips the init (-q, -Q).

;;; Code:

(defun romacs-custom-default-file ()
  "The file saved settings go to unless `custom-file' names another."
  (expand-file-name "custom.el" user-emacs-directory))

(defun romacs-custom--safe-p (file)
  "Whether writing FILE leaves the user's init alone, links followed.
By name, so it also answers for an init that does not exist yet."
  (not (equal (file-truename file) (file-truename user-init-file))))

(defun romacs-custom--file (resolve &optional no-error)
  "What RESOLVE, GNU's function `custom-file', answers, but never the init.
With the init as the answer it is `romacs-custom-default-file', unless
that is the init too: then NO-ERROR gives nil and otherwise it signals,
as GNU's does when there is no file to save to."
  (let ((file (funcall resolve no-error))
        (default (romacs-custom-default-file)))
    (cond ((or (null file) (null user-init-file) (romacs-custom--safe-p file)) file)
          ((romacs-custom--safe-p default) default)
          (no-error nil)
          (t (user-error "No file for customizations other than the init")))))

(defun romacs-custom--record-command (command disable)
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
  (advice-add 'custom-file :around #'romacs-custom--file))
(with-eval-after-load 'novice
  (advice-add 'en/disable-command :override #'romacs-custom--record-command))

(defun romacs-custom-start ()
  "Give this session its `custom-file', and load it.
A file that fails to load is reported and does not stop the init."
  (unless custom-file (setq custom-file (romacs-custom-default-file)))
  (ignore-error file-error (make-directory (file-name-directory custom-file) t))
  (condition-case err
      (load custom-file t t t)
    (error (display-warning 'initialization
                            (format-message "Error loading `%s': %s" custom-file
                                            (error-message-string err))
                            :error))))

(provide 'romacs-custom)
;;; romacs-custom.el ends here

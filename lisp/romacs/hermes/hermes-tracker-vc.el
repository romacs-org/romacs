;;; hermes-tracker-vc.el --- Derive Tracker context from Git -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: tools, vc
;; Package-Requires: ((emacs "29.1"))

;;; Commentary:

;; Map configured Git remote hosts to Tracker repository slugs.

;;; Code:

(require 'subr-x)
(require 'url-parse)

(defcustom hermes-tracker-vc-hosts nil
  "Git remote hosts allowed to provide Tracker repository context.

Entries may be host names, URL origins, or SCP-style authorities such as
`git@theodora'.  Remote hosts not listed here never select a Tracker
repository."
  :type '(repeat string)
  :group 'hermes-tracker)

(declare-function hermes-tracker--render-todos "hermes-tracker"
                  (slug name &optional owner))

(defun hermes-tracker-vc--host (value)
  "Return normalized host name from remote or configured VALUE."
  (downcase
   (or (and (string-match-p "://" value)
            (url-host (url-generic-parse-url value)))
       (and (string-match "\\`[^@/:]+@\\([^:/]+\\)" value)
            (match-string 1 value))
       (and (string-match "\\`\\([^/:]+\\)" value)
            (match-string 1 value))
       "")))

(defun hermes-tracker-vc--remote-path (remote)
  "Return repository path from REMOTE, or nil when unsupported."
  (if (string-match-p "://" remote)
      (car (split-string (url-filename (url-generic-parse-url remote))
                         "[?]" t))
    (and (string-match "\\`\\(?:[^@/:]+@\\)?[^/:]+:\\(.+\\)\\'" remote)
         (match-string 1 remote))))

(defun hermes-tracker-vc-slug-from-remote (remote)
  "Return Tracker slug represented by configured Git REMOTE."
  (when (stringp remote)
    (let ((host (hermes-tracker-vc--host remote))
          (allowed-hosts
           (delete "" (mapcar #'hermes-tracker-vc--host
                               hermes-tracker-vc-hosts))))
      (when (and (not (string-empty-p host)) (member host allowed-hosts))
        (when-let* ((path (hermes-tracker-vc--remote-path remote))
                    (leaf (file-name-nondirectory (directory-file-name path)))
                    (slug (string-remove-suffix ".git" leaf))
                    ((not (string-empty-p slug))))
          slug)))))

(defun hermes-tracker-vc--origin-url (&optional directory)
  "Return Git origin URL for DIRECTORY, or nil on failure."
  (let ((default-directory
         (file-name-as-directory (expand-file-name (or directory
                                                      default-directory)))))
    (condition-case nil
        (with-temp-buffer
          (let ((status (process-file "git" nil (current-buffer) nil
                                      "config" "--local" "--get"
                                      "remote.origin.url")))
            (when (and (integerp status) (zerop status))
              (let ((remote (string-trim (buffer-string))))
                (unless (string-empty-p remote) remote)))))
      (file-error nil))))

(defun hermes-tracker-vc-repository (&optional directory)
  "Return configured Tracker repository slug for Git DIRECTORY."
  (when hermes-tracker-vc-hosts
    (when-let* ((remote (hermes-tracker-vc--origin-url directory)))
      (hermes-tracker-vc-slug-from-remote remote))))

;;;###autoload
(defun hermes-tracker-open-current-repository (&optional directory)
  "Open cached Tracker TODOs for the Git repository at DIRECTORY."
  (interactive)
  (require 'hermes-tracker)
  (if-let ((slug (hermes-tracker-vc-repository directory)))
      (hermes-tracker--render-todos slug slug)
    (user-error "Current Git remote has no Tracker mapping")))

(provide 'hermes-tracker-vc)
;;; hermes-tracker-vc.el ends here

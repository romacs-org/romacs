;;; romacs-auth-source-pass.el --- auth-source-pass reads through the password manager  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's auth-source-pass (lisp/gnu/auth-source-pass.el) keeps its
;; naming conventions: which entry answers a host, user and port
;; (`host/user', `user@host', `host:port', a folder's suffix).  It reads
;; an entry by visiting its file through EPA, which leaves the plaintext
;; in a buffer and a string, and lists the store with
;; `directory-files-recursively'.  Here both go through the password
;; manager (romacs-pass.el): gpg decrypts through gpg-agent, the text is
;; split in Rust and wiped, and the store is listed natively.  Every
;; secret it hands out carries the `romacs-secret' property.
;;
;; Loaded after auth-source-pass (romacs-loadup-bootstrap.el), so every
;; caller of auth-source-pass reads this way.  An interactive session
;; also puts the store first in `auth-sources' (romacs-defaults.el), so
;; packages that ask auth-source find their secrets with no
;; configuration.  VM-PASS1.

;;; Code:

(require 'auth-source-pass)
(require 'romacs-pass)

(defun auth-source-pass--read-entry (entry)
  "Return a string with the file content of ENTRY.
Decrypted by the password manager (`romacs-pass-read'); the caller
should wipe it with `clear-string'."
  (romacs-pass--secret (romacs-pass-read entry)))

(defun auth-source-pass-parse-entry (entry)
  "Return an alist of the data associated with ENTRY.
ENTRY is the name of a password-store entry.  The data is
\((secret . SECRET) (KEY . VALUE)...), each value a secret string;
nil when ENTRY cannot be read."
  (let ((text (ignore-errors (romacs-pass-read entry))))
    (when text
      (unwind-protect
          (mapcar (lambda (cell)
                    (if (stringp (cdr cell)) (cons (car cell) (romacs-pass--secret (cdr cell))) cell))
                  (romacs-pass--parse text))
        (clear-string text)))))

(defun auth-source-pass-entries ()
  "Return a list of all password store entries."
  (romacs-pass-entries))

(provide 'romacs-auth-source-pass)
;;; romacs-auth-source-pass.el ends here

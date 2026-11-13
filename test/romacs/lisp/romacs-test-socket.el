;;; romacs-test-socket.el --- Short directories for tests' local sockets -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; A local socket's path is capped at 108 bytes (`sun_path').  A test's
;; temporary directory sits under TMPDIR, which a test runner or a check
;; may make long, and a socket inside it then cannot be bound.  Tests put
;; their sockets in a directory under XDG_RUNTIME_DIR, short by design.

;;; Code:

(defconst romacs-test-socket-room 80
  "The longest socket directory a test may use: room is left for the name.")

(defun romacs-test-socket-directory (prefix)
  "A new private directory for local sockets, named PREFIX and a random suffix.
It is under XDG_RUNTIME_DIR when that is set, else under /tmp; an error
if it is still too long to hold a socket."
  (let* ((runtime (getenv "XDG_RUNTIME_DIR"))
         (temporary-file-directory
          (file-name-as-directory (if (and runtime (file-directory-p runtime)) runtime "/tmp")))
         (directory (make-temp-file prefix t)))
    (when (> (string-bytes directory) romacs-test-socket-room)
      (delete-directory directory)
      (error "Socket directory %s is longer than %d bytes" directory romacs-test-socket-room))
    directory))

(provide 'romacs-test-socket)
;;; romacs-test-socket.el ends here

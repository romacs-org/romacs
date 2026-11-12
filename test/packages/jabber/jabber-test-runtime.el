;;; jabber-test-runtime.el --- Bundled runtime tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; This file is distributed under the GNU GPL v3 or later.

;;; Commentary:
;; Exercise the bundled client's optional native capability boundary.

;;; Code:

(require 'ert)

(ert-deftest jabber-test-omemo-module-absent-loads-plain-chat ()
  "Load the client and expose unavailable encryption without a module."
  (let ((directory (make-temp-file "jabber-no-omemo-" t))
        (lisp-dir (file-name-directory (locate-library "jabber-omemo"))))
    (unwind-protect
        (with-temp-buffer
          (let ((status
                 (call-process
                  (expand-file-name invocation-name invocation-directory)
                  nil '(t nil) nil "-Q" "--batch"
                  "--eval" (format "(setq load-path '%S)" (cons lisp-dir load-path))
                  "--eval" (format "(setq jabber-db-path nil jabber-omemo-enable t jabber-omemo-module-directory %S)"
                                   directory)
                  "--eval" "(progn (require 'jabber) (require 'jabber-omemo)
(princ (list (featurep 'jabber) jabber-omemo--available
             (commandp 'jabber-chat-with))))")))
            (should (equal (list status (buffer-string))
                           '(0 "(t unavailable t)")))))
      (delete-directory directory t))))

(provide 'jabber-test-runtime)
;;; jabber-test-runtime.el ends here

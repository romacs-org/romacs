;;; romacs-sasl-scram-tests.el --- SCRAM's bounded key derivation  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; The RFC exchanges are GNU's test/lisp/net/sasl-scram-rfc-tests.el; this is
;; what lisp/romacs/romacs-sasl-scram.el adds to them.

(require 'ert)
(require 'sasl)
(require 'sasl-scram-rfc)

(defun romacs-sasl-scram-tests--final (count)
  "The client's final message for a server asking for COUNT iterations."
  (let ((client (sasl-make-client (sasl-find-mechanism '("SCRAM-SHA-1"))
                                  "user" "imap" "localhost"))
        (sasl-read-passphrase (lambda (_prompt) (copy-sequence "pencil"))))
    (sasl-client-set-property client 'c-nonce "nonce")
    (sasl-scram-sha-1-client-final-message
     client (vector nil (format "r=nonceserver,s=QSXCR+Q6sek8bf92,i=%d" count)))))

(ert-deftest romacs-sasl-scram-refuses-a-count-over-the-bound ()
  "A server's iteration count over `sasl-scram-max-iterations' is a SASL
error, not work; the bound itself is accepted."
  (let ((sasl-scram-max-iterations 8))
    (should (stringp (romacs-sasl-scram-tests--final 8)))
    (should-error (romacs-sasl-scram-tests--final 9) :type 'sasl-error)))

(provide 'romacs-sasl-scram-tests)
;;; romacs-sasl-scram-tests.el ends here

;;; romacs-sasl-scram.el --- SCRAM's key derivation, natively  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; GNU's `sasl-scram--client-final-message' (lisp/net/sasl-scram-rfc.el)
;; computes Hi() as one Lisp HMAC per iteration, thousands of them, on the
;; main VM while a login waits: 171 ms for the usual 4,096.  This is that
;; function with Hi() through `romacs-pbkdf2-hmac', and a bound on the count
;; a server may ask for, since the work is synchronous.  VM2-SCRAM1.

;;; Code:

(require 'sasl-scram-rfc)

(defvar sasl-scram-max-iterations 100000
  "The largest SCRAM iteration count a login accepts.
The key derivation is synchronous; at this count it takes about 50 ms.")

(defun sasl-scram--hi (hash-length password salt iterations)
  "SCRAM's Hi(PASSWORD, SALT, ITERATIONS) under the hash of HASH-LENGTH bytes."
  (when (> iterations sasl-scram-max-iterations)
    (sasl-error (format "Server asks for %d iterations, more than `sasl-scram-max-iterations'"
                        iterations)))
  (romacs-pbkdf2-hmac (pcase hash-length (20 'sha1) (32 'sha256) (64 'sha512))
                      (encode-coding-string password 'utf-8) salt iterations))

(defun sasl-scram--client-final-message (hash-fun block-length hash-length client step)
  "GNU's function of this name, with Hi() as `sasl-scram--hi'.
HASH-FUN, BLOCK-LENGTH, HASH-LENGTH, CLIENT and STEP are as there."
  (unless (string-match
	   "^r=\\([^,]+\\),s=\\([^,]+\\),i=\\([0-9]+\\)\\(?:$\\|,\\)"
	   (sasl-step-data step))
    (sasl-error "Unexpected server response"))
  (let* ((hmac-fun (lambda (text key)
		     (decode-hex-string
		      (rfc2104-hash hash-fun block-length hash-length key text))))
	 (step-data (sasl-step-data step))
	 (nonce (match-string 1 step-data))
	 (salt-base64 (match-string 2 step-data))
	 (iteration-count (string-to-number (match-string 3 step-data)))
	 (c-nonce (sasl-client-property client 'c-nonce))
	 ;; no channel binding, no authorization id
         (cbind-input (funcall sasl-scram-gs2-header-function client)))
    (unless (string-prefix-p c-nonce nonce)
      (sasl-error "Invalid nonce from server"))
    (let* ((client-final-message-without-proof
            (concat "c=" (base64-encode-string cbind-input t) ","
		    "r=" nonce))
	   (password
	    ;; TODO: either apply saslprep or disallow non-ASCII characters
	    (sasl-read-passphrase
	     (format "%s passphrase for %s: "
		     (sasl-mechanism-name (sasl-client-mechanism client))
		     (sasl-client-name client))))
	   (salt (base64-decode-string salt-base64))
           (string-xor (lambda (a b)
                         (apply #'unibyte-string (cl-mapcar #'logxor a b))))
	   (salted-password
	    (sasl-scram--hi hash-length password salt iteration-count))
	   (client-key
	    (funcall hmac-fun "Client Key" salted-password))
	   (stored-key (decode-hex-string (funcall hash-fun client-key)))
	   (auth-message
	    (concat
	     (sasl-scram--client-first-message-bare client) ","
	     step-data ","
	     client-final-message-without-proof))
	   (client-signature (funcall hmac-fun (encode-coding-string auth-message 'utf-8) stored-key))
	   (client-proof (funcall string-xor client-key client-signature))
	   (client-final-message
	    (concat client-final-message-without-proof ","
                    "p=" (base64-encode-string client-proof t))))
      (sasl-client-set-property client 'auth-message auth-message)
      (sasl-client-set-property client 'salted-password salted-password)
      client-final-message)))

(provide 'romacs-sasl-scram)
;;; romacs-sasl-scram.el ends here

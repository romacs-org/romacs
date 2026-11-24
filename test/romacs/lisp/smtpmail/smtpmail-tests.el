;;; smtpmail-tests.el --- SMTP cancellation tests  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'ert)
(require 'smtpmail)

(defvar smtpmail-quit-timeout)

(defun smtpmail-tests--listen (on-connect on-line)
  "Call ON-CONNECT with an accepted peer and ON-LINE for each SMTP line."
  (make-network-process
   :name "smtpmail-tests-server" :server t :host "127.0.0.1"
   :family 'ipv4 :service t :noquery t :coding 'binary
   :log (lambda (_server peer _message)
          (when peer
            (funcall on-connect peer)
            (process-send-string peer "220 localhost ESMTP\r\n")))
   :filter (lambda (peer chunk)
             (let ((pending (concat (process-get peer :pending) chunk))
                   end)
               (while (setq end (string-search "\r\n" pending))
                 (funcall on-line peer (substring pending 0 end))
                 (setq pending (substring pending (+ end 2))))
               (process-put peer :pending pending)))))

(defun smtpmail-tests--request (peer line)
  "Identify LINE as an SMTP request, tracking message data on PEER."
  (cond
   ((process-get peer :receiving)
    (when (equal line ".")
      (process-put peer :receiving nil)
      'message))
   ((string-prefix-p "EHLO " line) 'ehlo)
   ((string-prefix-p "MAIL FROM:" line) 'mail)
   ((string-prefix-p "RCPT TO:" line) 'recipient)
   ((equal line "DATA")
    (process-put peer :receiving t)
    'data)
   ((equal line "QUIT") 'quit)))

(defconst smtpmail-tests--replies
  '((ehlo . "250 localhost\r\n")
    (mail . "250 sender accepted\r\n")
    (recipient . "250 recipient accepted\r\n")
    (data . "354 send message\r\n")
    (message . "250 delivered\r\n")
    (quit . "221 goodbye\r\n"))
  "Replies of the loopback fixture.")

(defun smtpmail-tests--reply (peer request overrides)
  "Reply to REQUEST on PEER, taking replacements from OVERRIDES."
  (let ((reply (alist-get request overrides
                          (alist-get request smtpmail-tests--replies))))
    (pcase reply
      ('disconnect (delete-process peer))
      ('cancel (signal 'quit '(smtpmail-tests--cancel)))
      ('cancel-cleanup (signal 'quit '(smtpmail-tests--cancel-cleanup)))
      ((pred stringp) (process-send-string peer reply)))))

(defun smtpmail-tests--send (port timeout)
  "Send the fixture message to PORT with cleanup TIMEOUT."
  (let ((smtpmail-quit-timeout timeout)
        (smtpmail-smtp-server "127.0.0.1")
        (smtpmail-smtp-service port)
        (smtpmail-stream-type 'plain)
        (smtpmail-smtp-user nil)
        (smtpmail-servers-requiring-authorization nil)
        (smtpmail-mail-address "sender@example.invalid")
        (smtpmail-debug-info nil)
        (smtpmail-debug-verb nil)
        (auth-sources nil))
    (with-temp-buffer
      (insert "From: sender@example.invalid\nTo: recipient@example.invalid\n"
              "Subject: SMTP cancellation\n\nTest message\n")
      (condition-case err
          (smtpmail-via-smtp '("recipient@example.invalid") (current-buffer))
        ((quit error) err)))))

(defun smtpmail-tests--exchange (timeout bound overrides)
  "Run one send and record its outcome, wire requests, and teardown.
TIMEOUT is the cleanup option and BOUND sets the fixture watchdog.
OVERRIDES replaces replies for selected requests."
  (let* (peer client trace watchdog started quit-at requests
         (server
          (smtpmail-tests--listen
           (lambda (accepted) (setq peer accepted))
           (lambda (connection line)
             (when-let* ((request (smtpmail-tests--request connection line)))
               (push request requests)
               (pcase request
                 ('mail
                  (setq client (get-process "smtpmail")
                        trace (process-buffer client)
                        watchdog (run-at-time (+ bound 0.5) nil
                                              #'delete-process connection)
                        started (float-time)))
                 ('quit (setq quit-at (float-time))))
               (smtpmail-tests--reply connection request overrides))))))
    (unwind-protect
        (let* ((outcome (smtpmail-tests--send (process-contact server :service) timeout))
               (end (float-time)))
          (list :outcome outcome :requests (nreverse requests)
                :elapsed (and started (- end started))
                :since-quit (and quit-at (- end quit-at))
                :peer-open (and peer (process-live-p peer))
                :client-dead (and client (not (process-live-p client)))
                :client-unregistered (and client (not (memq client (process-list))))
                :trace-gone (and trace (not (buffer-live-p trace)))))
      (when watchdog (cancel-timer watchdog))
      (when client (delete-process client))
      (when peer (delete-process peer))
      (when (buffer-live-p trace) (kill-buffer trace))
      (delete-process server))))

(ert-deftest smtpmail-quit-with-silent-cleanup-peer ()
  "Deliver the original quit within the cleanup deadline plus one second."
  (let* ((timeout 0.25)
         (bound (+ timeout 1))
         (result (smtpmail-tests--exchange timeout bound '((mail . cancel) (quit)))))
    (should (equal (plist-get result :outcome) '(quit smtpmail-tests--cancel)))
    (should (memq 'quit (plist-get result :requests)))
    (should (< (plist-get result :elapsed) bound))
    (should (plist-get result :peer-open))
    (should (plist-get result :client-dead))
    (should (plist-get result :client-unregistered))
    (should (plist-get result :trace-gone))))

(ert-deftest smtpmail-delivery-with-prompt-quit-reply ()
  "Finish delivery promptly when the peer answers QUIT."
  (let ((result (smtpmail-tests--exchange 3 4 nil)))
    (should (equal (plist-get result :requests)
                   '(ehlo mail recipient data message quit)))
    (should-not (plist-get result :outcome))
    (should (< (plist-get result :since-quit) 1.25))))

(ert-deftest smtpmail-delivery-with-silent-cleanup-peer ()
  "Finish a successful delivery within the cleanup deadline plus one second."
  (let* ((timeout 0.25)
         (bound (+ timeout 1))
         (result (smtpmail-tests--exchange timeout bound '((quit)))))
    (should (equal (plist-get result :requests)
                   '(ehlo mail recipient data message quit)))
    (should-not (plist-get result :outcome))
    (should (< (plist-get result :since-quit) bound))
    (should (plist-get result :peer-open))))

(ert-deftest smtpmail-cleanup-preserves-data-disconnect-error ()
  "Return the DATA failure when the peer disconnects during the send."
  (let* ((result (smtpmail-tests--exchange 0.25 1.25 '((data . disconnect))))
         (outcome (plist-get result :outcome)))
    (should (equal (plist-get result :requests) '(ehlo mail recipient data)))
    (should (and (stringp outcome)
                 (string-suffix-p " in response to DATA" outcome)))))

(ert-deftest smtpmail-cleanup-preserves-first-quit ()
  "Interrupt cleanup with a second quit and deliver the first to the caller."
  (let ((result (smtpmail-tests--exchange
                 2 3 '((mail . cancel) (quit . cancel-cleanup)))))
    (should (memq 'quit (plist-get result :requests)))
    (should (< (plist-get result :since-quit) 1.25))
    (should (equal (plist-get result :outcome) '(quit smtpmail-tests--cancel)))))

(defun smtpmail-tests--assert-bounded-invalid-timeout (timeout)
  "Limit cleanup time with invalid TIMEOUT and a silent peer."
  (let* ((bound 3)
         (result (smtpmail-tests--exchange timeout bound '((quit)))))
    (should (memq 'message (plist-get result :requests)))
    (should (< (plist-get result :elapsed) bound))
    (should (plist-get result :peer-open))))

(ert-deftest smtpmail-cleanup-bounds-negative-timeout ()
  (smtpmail-tests--assert-bounded-invalid-timeout -1))

(ert-deftest smtpmail-cleanup-bounds-infinite-timeout ()
  (smtpmail-tests--assert-bounded-invalid-timeout 1.0e+INF))

(ert-deftest smtpmail-cleanup-bounds-nan-timeout ()
  (smtpmail-tests--assert-bounded-invalid-timeout 0.0e+NaN))

;;; smtpmail-tests.el ends here

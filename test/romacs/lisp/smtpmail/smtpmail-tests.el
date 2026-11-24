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

(ert-deftest smtpmail-quit-with-silent-cleanup-peer ()
  "Deliver the original quit within the cleanup deadline plus one second."
  (let* ((smtpmail-quit-timeout 0.25)
         (bound (+ smtpmail-quit-timeout 1))
         (smtpmail-smtp-server "127.0.0.1")
         (smtpmail-stream-type 'plain)
         (smtpmail-smtp-user nil)
         (smtpmail-servers-requiring-authorization nil)
         (smtpmail-mail-address "sender@example.invalid")
         (smtpmail-debug-info nil)
         (smtpmail-debug-verb nil)
         (auth-sources nil)
         peer client watchdog quit-at saw-quit elapsed
         (server
          (smtpmail-tests--listen
           (lambda (accepted) (setq peer accepted))
           (lambda (connection line)
             (cond
              ((string-prefix-p "EHLO " line)
               (process-send-string connection "250 localhost\r\n"))
              ((string-prefix-p "MAIL FROM:" line)
               (setq client (get-process "smtpmail")
                     watchdog (run-at-time (+ bound 0.5) nil #'delete-process connection)
                     quit-at (float-time))
               (signal 'quit '(smtpmail-tests--cancel)))
              ((equal line "QUIT") (setq saw-quit t))))))
         (smtpmail-smtp-service (process-contact server :service)))
    (unwind-protect
        (with-temp-buffer
          (insert "From: sender@example.invalid\nTo: recipient@example.invalid\n"
                  "Subject: SMTP cancellation\n\nTest message\n")
          (should (equal (condition-case err
                             (smtpmail-via-smtp '("recipient@example.invalid") (current-buffer))
                           (quit err))
                         '(quit smtpmail-tests--cancel)))
          (should quit-at)
          (setq elapsed (- (float-time) quit-at))
          (should saw-quit)
          (should (< elapsed bound))
          (should (process-live-p peer))
          (should (and client (not (process-live-p client)))))
      (when watchdog (cancel-timer watchdog))
      (when client (delete-process client))
      (when peer (delete-process peer))
      (delete-process server))))

;;; smtpmail-tests.el ends here

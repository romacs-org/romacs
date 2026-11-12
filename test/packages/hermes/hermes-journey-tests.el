;;; hermes-journey-tests.el --- Offline streaming journey -*- lexical-binding: t; -*-

;;; Code:

(require 'benchmark)
(require 'hermes-test-helpers)

(ert-deftest hermes-journey-json-stream ()
  "Render a mock JSON stream without a backend and report elapsed time and GC."
  (hermes-test-with-dashboard-prompt-session (client)
    (let* ((count 200)
           (delta (concat (make-string 125 ?x) " λ\n"))
           (expected (apply #'concat (make-list count delta)))
           (measurement
            (benchmark-run 1
              (dotimes (_ count)
                (hermes-test--emit-dashboard-prompt
                 client "message.delta" `((text . ,delta)))))))
      (let ((entry (cl-find 'assistant (hermes-chat--entries)
                            :key (lambda (item) (plist-get item :role)))))
        (should (equal (plist-get entry :content) expected))
        (should (string-match-p "λ" (buffer-string))))
      (hermes-test--emit-dashboard-prompt
       client "message.complete" '((status . "done")))
      (should-not hermes-chat--pending-assistant-id)
      (should (equal (hermes-chat-input-string) ""))
      (message "J3 deltas=%d chars=%d seconds=%.6f gc-count=%d gc-seconds=%.6f"
               count (length expected) (nth 0 measurement)
               (nth 1 measurement) (nth 2 measurement)))))

(provide 'hermes-journey-tests)
;;; hermes-journey-tests.el ends here

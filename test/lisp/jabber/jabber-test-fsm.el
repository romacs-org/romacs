;;; jabber-test-fsm.el --- fsm.el failures without tracing  -*- lexical-binding: t; -*-

;; lisp/romacs/deps/fsm/fsm.el traces nothing by default (`fsm-debug' nil); a
;; handler's or an enter function's failure is logged all the same.

(require 'ert)
(require 'fsm)
(require 'jabber-stanza)

(define-state-machine jabber-test-fsm
  :start (() "A machine whose states fail on demand." (list :idle nil)))

(define-state jabber-test-fsm :idle (_fsm state-data event _callback)
  (pcase event
    (:fail (error "Handler failed"))
    (:enter-broken (list :broken state-data))
    (_ (list :idle state-data))))

(define-enter-state jabber-test-fsm :broken (_fsm _state-data)
  (error "Enter failed"))

(define-state jabber-test-fsm :broken (_fsm state-data _event _callback)
  (list :broken state-data))

(ert-deftest jabber-test-fsm-failures-logged-without-tracing ()
  "With `fsm-debug' nil no trace buffer is made, and a failing handler and
a failing enter function are still logged to *Messages*."
  (let ((fsm-debug nil)
        (logged nil))
    (cl-letf (((symbol-function 'message)
               (lambda (format &rest args) (push (apply #'format format args) logged))))
      (let ((fsm (start-jabber-test-fsm)))
        (fsm-send-sync fsm :ignored)
        (should-not logged)
        (fsm-send-sync fsm :fail)
        (should (seq-some (lambda (text) (string-search "Handler failed" text)) logged))
        (fsm-send-sync fsm :enter-broken)
        (should (seq-some (lambda (text) (string-search "Enter failed" text)) logged))))
    (should-not (get-buffer "*fsm-debug*"))))

(ert-deftest jabber-test-fsm-stanza-handler-failure-logged-without-tracing ()
  "A stanza handler's error, contained by `jabber-process-input', is logged
with `fsm-debug' nil."
  (let ((fsm-debug nil)
        (jabber-message-chain (list (lambda (_jc _xml) (error "Handler boom"))))
        (logged nil))
    (cl-letf (((symbol-function 'message)
               (lambda (format &rest args) (push (apply #'format format args) logged))))
      (jabber-process-input 'jc '(message ((from . "juliet@example.com")) (body () "hi"))))
    (should (seq-some (lambda (text) (and (string-search "Handler boom" text)
                                          (string-search "juliet@example.com" text)))
                      logged))))

(provide 'jabber-test-fsm)
;;; jabber-test-fsm.el ends here

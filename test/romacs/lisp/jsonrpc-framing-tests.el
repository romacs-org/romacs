;;; jsonrpc-framing-tests.el --- jsonrpc.el over framed process output -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; jsonrpc.el's filter takes the bodies the process layer framed
;; (ledger/divergences/LISP-JSONRPC-FRAMING.md): messages keep their order
;; when a read happens inside the reading, and none is lost when the
;; reading is cut short.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jsonrpc)

(defmacro jsonrpc-framing-tests--with-proc (proc received &rest body)
  "Run BODY with PROC a framed jsonrpc process and RECEIVED the ids of the
messages its connection received, newest first.  Every dispatch has one
time, as two reads in the same clock tick would."
  (declare (indent 2))
  `(let* ((,proc (make-pipe-process :name "jsonrpc-framing" :noquery t))
          (,received nil)
          (now (current-time)))
     (set-process-buffer ,proc (generate-new-buffer " *jsonrpc-framing*" t))
     (unwind-protect
         (cl-letf (((symbol-function 'jsonrpc-connection-receive)
                    (lambda (_conn msg) (push (plist-get msg :id) ,received)))
                   ((symbol-function 'current-time) (lambda () now)))
           ,@body)
       (kill-buffer (process-buffer ,proc))
       (delete-process ,proc))))

(defun jsonrpc-framing-tests--run-timers (done)
  "Run timers until DONE says t, at most five seconds."
  (with-timeout (5 (error "Messages never received"))
    (while (not (funcall done)) (accept-process-output nil 0.01))))

(ert-deftest jsonrpc-framing-two-reads-are-received-in-order ()
  ;; Two reads' messages, their timers at one time, are received as sent.
  (jsonrpc-framing-tests--with-proc proc received
    (jsonrpc--process-filter proc (list "{\"id\":1}" "{\"id\":2}"))
    (jsonrpc--process-filter proc (list "{\"id\":3}"))
    (jsonrpc-framing-tests--run-timers (lambda () (= (length received) 3)))
    (should (equal (reverse received) '(1 2 3)))))

(ert-deftest jsonrpc-framing-a-read-inside-the-reading-keeps-the-order ()
  (jsonrpc-framing-tests--with-proc proc received
    (cl-letf (((symbol-function 'jsonrpc--warn)
               (lambda (&rest _) (jsonrpc--process-filter proc (list "{\"id\":4}")))))
      (jsonrpc--process-filter proc (list "{\"id\":1}" "not json" "{\"id\":2}" "{\"id\":3}")))
    (jsonrpc-framing-tests--run-timers (lambda () (= (length received) 4)))
    (should (equal (reverse received) '(1 2 3 4)))))

(ert-deftest jsonrpc-framing-a-read-cut-short-loses-nothing ()
  (jsonrpc-framing-tests--with-proc proc received
    (cl-letf (((symbol-function 'jsonrpc--warn) (lambda (&rest _) (throw 'cut nil))))
      (catch 'cut (jsonrpc--process-filter proc (list "{\"id\":1}" "not json" "{\"id\":2}"))))
    (jsonrpc-framing-tests--run-timers (lambda () (= (length received) 2)))
    (should (equal (reverse received) '(1 2)))))

;; A pty cannot carry JSON-RPC: the connection refuses one and names the fix.
(ert-deftest jsonrpc-framing-a-pty-is-refused-with-its-fix ()
  (let ((process (make-process :name "jsonrpc-framing-pty" :command '("sleep" "5")
                               :connection-type 'pty :noquery t)))
    (unwind-protect
        (let ((err (should-error (make-instance 'jsonrpc-process-connection
                                                :process process :name "jsonrpc-framing-pty"))))
          (should (string-match-p ":connection-type 'pipe" (error-message-string err))))
      (delete-process process))))

(provide 'jsonrpc-framing-tests)
;;; jsonrpc-framing-tests.el ends here

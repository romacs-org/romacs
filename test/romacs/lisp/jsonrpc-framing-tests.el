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

(defmacro jsonrpc-framing-tests--with-proc (proc dispatched &rest body)
  "Run BODY with PROC a framed jsonrpc process and DISPATCHED the ids its
messages were handed to timers with, in order."
  (declare (indent 2))
  `(let* ((,proc (make-pipe-process :name "jsonrpc-framing" :noquery t))
          (,dispatched nil))
     (set-process-buffer ,proc (generate-new-buffer " *jsonrpc-framing*" t))
     (unwind-protect
         (cl-letf (((symbol-function 'timer-activate)
                    (lambda (timer &rest _)
                      (push (plist-get (cadr (timer--args timer)) :id) ,dispatched))))
           ,@body)
       (kill-buffer (process-buffer ,proc))
       (delete-process ,proc))))

(ert-deftest jsonrpc-framing-a-read-inside-the-reading-keeps-the-order ()
  (jsonrpc-framing-tests--with-proc proc dispatched
    (cl-letf (((symbol-function 'jsonrpc--warn)
               (lambda (&rest _) (jsonrpc--process-filter proc (list "{\"id\":4}")))))
      (jsonrpc--process-filter proc (list "{\"id\":1}" "not json" "{\"id\":2}" "{\"id\":3}")))
    ;; Timers fire newest first among equal times: the list is the order.
    (should (equal dispatched '(1 2 3 4)))))

(ert-deftest jsonrpc-framing-a-read-cut-short-loses-nothing ()
  (jsonrpc-framing-tests--with-proc proc dispatched
    (let ((due nil))
      (cl-letf (((symbol-function 'jsonrpc--warn) (lambda (&rest _) (throw 'cut nil)))
                ((symbol-function 'run-at-time)
                 (lambda (_time _repeat function &rest args) (push (cons function args) due))))
        (catch 'cut (jsonrpc--process-filter proc (list "{\"id\":1}" "not json" "{\"id\":2}"))))
      (should (equal dispatched '(1)))
      (should (= (length due) 1))
      (apply (caar due) (cdar due))
      (should (equal dispatched '(2 1))))))

(provide 'jsonrpc-framing-tests)
;;; jsonrpc-framing-tests.el ends here

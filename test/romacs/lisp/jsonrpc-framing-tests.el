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

;; A pty cannot carry JSON-RPC: the connection refuses one and names the
;; fix; one it made goes with its stderr buffer, one it was given makes it
;; no buffer.
(ert-deftest jsonrpc-framing-a-pty-is-refused-with-its-fix ()
  (let* ((made nil)
         (err (should-error
               (make-instance 'jsonrpc-process-connection :name "jsonrpc-framing-pty"
                              :process (lambda () (setq made (make-process :name "jsonrpc-framing-pty"
                                                                           :command '("sleep" "5")
                                                                           :connection-type 'pty :noquery t)))))))
    (should (string-match-p ":connection-type 'pipe" (error-message-string err)))
    (should-not (process-live-p made))
    (should-not (get-buffer "*jsonrpc-framing-pty stderr*"))
    (should-not (get-buffer " *jsonrpc-framing-pty stderr*")))
  (let ((given (make-process :name "jsonrpc-framing-given" :command '("sleep" "5")
                             :connection-type 'pty :noquery t)))
    (unwind-protect
        (progn
          (should-error (make-instance 'jsonrpc-process-connection :name "jsonrpc-framing-given" :process given))
          (should (process-live-p given))
          (should-not (get-buffer "*jsonrpc-framing-given stderr*")))
      (delete-process given))))

;; TRAMP gives a pipe (`tramp-pipe-process') over the ssh, scp and rsync
;; methods on one hop, and refuses any other route before a backend runs.
(ert-deftest jsonrpc-framing-tramp-refuses-a-route-with-no-pipe ()
  (require 'tramp)
  (dolist (library '(tramp-sshfs tramp-sudoedit tramp-androidsu tramp-container))
    (require library nil t))
  (let ((tramp-methods (copy-tree tramp-methods)))
    ;; An ssh method whose host is not ssh's last option, and one with
    ;; other direct asynchronous arguments: shapes the route does not know.
    (push `("ssh-dashes" ,@(cdr (assoc "ssh" tramp-methods))) tramp-methods)
    (setf (cadr (assq 'tramp-login-args (cdr (assoc "ssh-dashes" tramp-methods))))
          '(("-l" "%u") ("-e" "none") ("--") ("%h")))
    (push `("ssh-tt" ,@(cdr (assoc "ssh" tramp-methods))) tramp-methods)
    (setf (cadr (assq 'tramp-direct-async (cdr (assoc "ssh-tt" tramp-methods)))) '("-t" "-tt"))
    (dolist (method '("ssh-dashes" "ssh-tt"))
      (should (string-match-p "needs an ssh method"
                              (error-message-string
                               (should-error (tramp--pipe-route
                                              (make-tramp-file-name :method method :host "none" :localname "/tmp/")))))))
    (dolist (dir '("/sudo::/tmp/" "/docker:none:/tmp/" "/ssh:a|ssh:b:/tmp/" "/sshfs:none:/tmp/"
                   "/sudoedit::/tmp/" "/androidsu::/tmp/"))
      (when (assoc (car (split-string (substring dir 1) "[:|]")) tramp-methods)
        (let ((default-directory dir) (tramp-pipe-process t) (before (length (process-list))))
          (should (string-match-p "needs an ssh method"
                                  (error-message-string
                                   (should-error (make-process :name "x" :command '("true")
                                                               :file-handler t)))))
          (should (= (length (process-list)) before)))))))

;; The pipe shares TRAMP's own connection as a client, never its master,
;; and a user's suppression of sharing stays theirs.
(ert-deftest jsonrpc-framing-tramp-shares-as-a-client ()
  (require 'tramp-sh)
  (let ((v (tramp-dissect-file-name "/ssh:localhost:/"))
        (tramp-ssh-controlmaster-options "-o ControlMaster=auto -o ControlPath=/tmp/tramp.%%C -o ControlPersist=no"))
    (let ((tramp-use-connection-share t))
      (should (equal (tramp--pipe-share-options v) "-o ControlMaster=no -o ControlPath=/tmp/tramp.%%C")))
    (let ((tramp-use-connection-share 'suppress) (tramp-ssh-controlmaster-options nil))
      (should (string-match-p "ControlMaster=no -o ControlPath=none" (tramp--pipe-share-options v))))))

;; A factory that fails leaves no stderr buffer behind.
(ert-deftest jsonrpc-framing-a-failed-factory-leaves-nothing ()
  (should-error (make-instance 'jsonrpc-process-connection :name "jsonrpc-framing-fails"
                               :process (lambda () (error "No route"))))
  (should-not (get-buffer "*jsonrpc-framing-fails stderr*")))

(provide 'jsonrpc-framing-tests)
;;; jsonrpc-framing-tests.el ends here

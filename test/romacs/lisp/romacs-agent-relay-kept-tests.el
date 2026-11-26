;;; romacs-agent-relay-kept-tests.el --- Kept attachment routing -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'romacs-agent-relay-tests)

(defun romacs-agent-relay-kept-tests--with-runtime (body)
  "Call BODY in a private runtime with a fresh launch table."
  (let ((romacs-agent--launched (make-hash-table :test #'equal))
        (romacs-agent-events-store nil))
    (romacs-agent-relay-tests--with-runtime body)))

(defun romacs-agent-relay-kept-tests--promote (root)
  "Replace the listener under ROOT and return its new socket path."
  (let ((socket (expand-file-name "named.sock" root)))
    (romacs-mcp-stop)
    (romacs-mcp-start socket)
    (romacs-agent--route-attached)
    socket))

(ert-deftest romacs-agent-relay-kept-promotion-leaves-launch-claims-alone ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (root)
     (let* ((pending (romacs-agent-relay-tests--session "pending"))
            (failed (romacs-agent-relay-tests--session "failed"))
            (pending-route (romacs-agent--route pending))
            (failed-route (romacs-agent--route failed))
            (other (expand-file-name "other.sock" root))
            (listener (make-network-process :name "another-editor" :family 'local :service other
                                            :server t :noquery t)))
       (unwind-protect
           (progn
             (romacs-agent--before-launch pending)
             (romacs-agent--before-launch failed)
             (should (equal (file-symlink-p pending-route) (romacs-agent--own-socket)))
             (should (equal (file-symlink-p failed-route) (romacs-agent--own-socket)))
             (delete-process
              (make-network-process :name "failed-supervisor" :family 'local :server t :noquery t
                                    :service (romacs-agent-host-socket failed)))
             (should (file-exists-p (romacs-agent-host-socket failed)))
             (write-region "" nil (expand-file-name "journal.jsonl" (romacs-agent-dir failed)) nil 'silent)
             (make-symbolic-link other failed-route t)
             (cl-letf (((symbol-function 'romacs-agent--reattach-later) #'ignore))
               (should-not (romacs-agent--connect failed 0 nil)))
             (should (equal (file-symlink-p failed-route) other))
             (should (process-live-p listener))
             (romacs-agent-relay-kept-tests--promote root)
             (should (equal (mapcar #'file-symlink-p (list pending-route failed-route))
                            (list (romacs-agent--own-socket) other))))
         (delete-process listener))))))

(defun romacs-agent-relay-kept-tests--send (client method id &optional params)
  "Send CLIENT a request for METHOD with ID and PARAMS."
  (process-send-string
   client (concat (json-serialize
                   (romacs-mcp--object "jsonrpc" "2.0" "method" method "id" id
                                       "params" (or params (romacs-mcp--object))))
                  "\n")))

(ert-deftest romacs-agent-relay-kept-failed-launch-preserves-deferred-reply ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (_root)
     (let* ((session (romacs-agent-relay-tests--session "deferred"))
            (romacs-mcp-methods (copy-hash-table romacs-mcp-methods))
            (output "") ready)
       (romacs-agent--before-launch session)
       (romacs-mcp-register-method
        "test/deferred" (lambda (_peer _params)
                          (romacs-mcp-defer (lambda () (when ready (list "finished"))))))
       (let ((client (make-network-process
                      :name "pending-relay" :family 'local :service (romacs-agent--route session)
                      :noquery t :sentinel #'ignore
                      :filter (lambda (_client chunk) (setq output (concat output chunk))))))
         (unwind-protect
             (progn
               (romacs-agent-relay-kept-tests--send
                client "romacs/hello" 1 (romacs-mcp--object "session" (romacs-agent-id session)))
               (romacs-agent-relay-tests--wait (lambda () (string-search "\n" output)))
               (setq output "")
               (romacs-agent-relay-kept-tests--send client "test/deferred" 2)
               (romacs-agent-relay-tests--wait (lambda () romacs-mcp--deferred))
               (let ((peer (car romacs-mcp--peers)))
                 (should (process-get peer 'romacs-mcp-owned))
                 (should-not (romacs-agent--connect session 0 nil))
                 (setq ready t)
                 (romacs-mcp--poll-deferred)
                 (romacs-agent-relay-tests--wait
                  (lambda () (or (string-search "\n" output) (not (process-live-p client)))))
                 (should (memq peer romacs-mcp--peers))
                 (let* ((reply (json-parse-string output))
                        (content (gethash "content" (gethash "result" reply))))
                   (should (= (gethash "id" reply) 2))
                   (should (equal (gethash "text" (aref content 0)) "finished")))))
           (delete-process client)))))))

(ert-deftest romacs-agent-relay-kept-live-reload-restores-promotion ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (root)
     (let* ((session (romacs-agent-relay-tests--session "reload"))
            (route (romacs-agent--route session)))
       (romacs-agent-relay-tests--with-supervisor
        session
        (lambda (peer)
          (process-send-string peer "{\"attached\":\"kept\"}\n")
          (romacs-agent-relay-tests--wait (lambda () (romacs-agent-caught-up session)))
          (should (equal (file-symlink-p route) (romacs-agent--own-socket)))
          (let ((connection (romacs-agent-connection session)))
            (load (locate-library "romacs-agent.el") nil t t)
            (should (eq connection (romacs-agent-connection session)))
            (should (process-live-p connection))
            (should (process-get connection 'romacs-agent-kept))
            (let ((socket (romacs-agent-relay-kept-tests--promote root)))
              (should (equal (file-symlink-p route) socket))))))))))

(ert-deftest romacs-agent-relay-kept-dead-connection-still-draining ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (root)
     (let* ((session (romacs-agent-relay-tests--session "draining"))
            (route (romacs-agent--route session))
            (old (romacs-agent--own-socket)))
       (romacs-agent-relay-tests--with-supervisor
        session
        (lambda (peer)
          (process-send-string peer "{\"attached\":\"kept\"}\n")
          (romacs-agent-relay-tests--wait (lambda () (romacs-agent-caught-up session)))
          (should (equal (file-symlink-p route) old))
          (let ((connection (romacs-agent-connection session)))
            (process-put connection 'drain-timer t)
            (romacs-agent--filter connection '("{\"seq\":1}"))
            (delete-process peer)
            (romacs-agent-relay-tests--wait (lambda () (process-get connection 'ended)))
            (should (eq (romacs-agent-connection session) connection))
            (should-not (process-live-p connection))
            (should (romacs-agent--output-waits-p connection))
            (romacs-agent-relay-kept-tests--promote root)
            (should (equal (file-symlink-p route) old)))))))))

(ert-deftest romacs-agent-relay-kept-notes-host-replay-releases-route ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (root)
     (let* ((session (romacs-agent-relay-tests--session "notes"))
            (route (romacs-agent--route session))
            (old (romacs-agent--own-socket)))
       (romacs-agent-relay-tests--with-supervisor
        session
        (lambda (peer)
          (process-send-string peer "{\"seq\":1}\n")
          (romacs-agent-relay-tests--wait (lambda () (= (romacs-agent-seq session) 1)))
          (should (equal (file-symlink-p route) old))
          (let ((connection (romacs-agent-connection session)))
            (cl-letf (((symbol-function 'run-at-time)
                       (lambda (_delay _repeat function &rest _args)
                         (should (eq function #'romacs-agent--attach)))))
              (romacs-agent--apply-attachment-record
               session connection (romacs-mcp--object "attached" "kept" "notesOnly" t)))
            (should-not (romacs-agent-connection session))
            (should-not (process-live-p connection))
            (romacs-agent-relay-kept-tests--promote root)
            (should (equal (file-symlink-p route) old)))))))))

(ert-deftest romacs-agent-relay-kept-promotion-repairs-failed-route-write ()
  (romacs-agent-relay-kept-tests--with-runtime
   (lambda (root)
     (let* ((session (romacs-agent-relay-tests--session "write-error"))
            (route (romacs-agent--route session))
            (directory (directory-file-name (file-name-directory route))))
       (make-directory (file-name-directory directory) t)
       (write-region "" nil directory nil 'silent)
       (romacs-agent-relay-tests--with-supervisor
        session
        (lambda (_peer)
          (let ((connection (romacs-agent-connection session)))
            (should-error
             (romacs-agent--apply-attachment-record session connection (romacs-mcp--object "seq" 1))
             :type 'file-error)
            (should (process-live-p connection))
            (should (process-get connection 'romacs-agent-kept))
            (delete-file directory)
            (should-not (file-symlink-p route))
            (let ((socket (romacs-agent-relay-kept-tests--promote root)))
              (should (equal (file-symlink-p route) socket))))))))))

(provide 'romacs-agent-relay-kept-tests)
;;; romacs-agent-relay-kept-tests.el ends here

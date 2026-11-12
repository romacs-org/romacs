;;; romacs-agent-reattach-tests.el --- A session whose attachment ended while its supervisor runs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude))

(defmacro romacs-agent-reattach-tests--with (&rest body)
  "BODY with `session', detached, its supervisor's socket a file under
`root'; `scheduled' collects each reattach's delay, `attached' each
attach."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-reattach-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "aaaaaaaa-0000-4000-8000-000000000000" :harness 'claude :title "lane"
                                       :state 'detached :dir (expand-file-name "s" root)
                                       :host-socket (expand-file-name "host.sock" root)))
          (scheduled nil) (attached nil))
     (make-directory (romacs-agent-dir session))
     (write-region "" nil (romacs-agent-host-socket session))
     (puthash (romacs-agent-id session) session romacs-agent--sessions)
     (cl-letf (((symbol-function 'run-at-time)
                (lambda (delay _repeat function &rest args)
                  (when (eq function #'romacs-agent--reattach) (push delay scheduled))))
               ((symbol-function 'romacs-agent--attach) (lambda (s &rest _) (push s attached) nil)))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(ert-deftest romacs-agent-reattach-when-the-attachment-ends ()
  (romacs-agent-reattach-tests--with
    (let ((process (make-process :name "romacs-reattach-fixture" :command '("cat") :noquery t)))
      (process-put process 'romacs-agent session)
      (setf (romacs-agent-state session) 'idle (romacs-agent-connection session) process)
      (delete-process process)
      (romacs-agent--sentinel process "deleted")
      (should (eq (romacs-agent-state session) 'detached))
      (should (equal scheduled '(1)))
      ;; Its time come: attached again.
      (romacs-agent--reattach session)
      (should (equal attached (list session))))))

(ert-deftest romacs-agent-reattach-is-no-longer-detached ()
  (romacs-agent-reattach-tests--with
    (let ((process (make-process :name "romacs-reattach-fixture" :command '("cat") :noquery t)))
      (unwind-protect
          (progn
            (process-put process 'romacs-agent session)
            (setf (romacs-agent-connection session) process (romacs-agent-reattaching session) 3)
            (cl-letf (((symbol-function 'romacs-agent--kept) #'ignore)
                      ((symbol-function 'romacs-agent--allow-tool-calls) #'ignore)
                      ((symbol-function 'romacs-agent--step) #'ignore))
              (romacs-agent--filter process "{\"attached\":2}\n")
              (should (eq (romacs-agent-state session) 'idle))
              (should (= (romacs-agent-reattaching session) 0))
              ;; Its last prompt's turn not done: running.
              (setf (romacs-agent-state session) 'detached (romacs-agent-input-seq session) 9)
              (romacs-agent--filter process "{\"attached\":3}\n")
              (should (eq (romacs-agent-state session) 'running))
              ;; A question it asked still waits for the user.
              (puthash "q1" t (romacs-agent-approvals session))
              (setf (romacs-agent-state session) 'detached)
              (let ((events nil))
                (let ((romacs-agent-event-functions (list (lambda (_s e) (push (plist-get e :text) events)))))
                  (romacs-agent--filter process "{\"attached\":4}\n"))
                (should (eq (romacs-agent-state session) 'waiting-approval))
                ;; Its views and its parent's rows are told.
                (should (member "attached" events)))))
        (delete-process process)))))

(ert-deftest romacs-agent-reattach-backs-off-and-stops ()
  (romacs-agent-reattach-tests--with
    (dotimes (_ (1+ (length romacs-agent--reattach-delays)))
      (romacs-agent--reattach-later session))
    (should (equal (reverse scheduled) romacs-agent--reattach-delays))
    ;; Cancelled once its look was scheduled: no attach when it comes.
    (setf (romacs-agent-cancelled session) t)
    (romacs-agent--reattach session)
    (should-not attached)
    (setf (romacs-agent-cancelled session) nil)
    ;; Nothing to look for: no socket, its journal ended, or cancelled.
    (setq scheduled nil)
    (setf (romacs-agent-reattaching session) 0)
    (setf (romacs-agent-state session) 'ended)
    (romacs-agent--reattach-later session)
    (setf (romacs-agent-state session) 'detached (romacs-agent-cancelled session) t)
    (romacs-agent--reattach-later session)
    (setf (romacs-agent-cancelled session) nil)
    (delete-file (romacs-agent-host-socket session))
    (romacs-agent--reattach-later session)
    (should-not scheduled)))

(ert-deftest romacs-agent-reattach-on-restore ()
  (romacs-agent-reattach-tests--with
    (let ((process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment)))
      (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                ((symbol-function 'romacs-agent--sweep) #'ignore))
        (setf (romacs-agent-reattaching session) 5)
        (romacs-agent-restore)
        (should (equal attached (list session)))
        (should (= (romacs-agent-reattaching session) 0))))))

(ert-deftest romacs-agent-reattach-never-a-second-attachment ()
  (let* ((root (make-temp-file "romacs-attach-" t))
         (session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir root :host-socket (expand-file-name "host.sock" root)))
         (first (make-process :name "romacs-attach-first" :command '("cat") :noquery t))
         (connected 0))
    (unwind-protect
        (cl-letf (((symbol-function 'make-network-process)
                   (lambda (&rest _) (cl-incf connected)
                     (make-process :name "romacs-attach-second" :command '("cat") :noquery t))))
          (setf (romacs-agent-connection session) first)
          ;; Attached already: the same attachment, no second one to orphan it.
          (should (eq (romacs-agent--attach session 0) first))
          (should (= connected 0))
          ;; Taken over: the first is let go.
          (let ((second (romacs-agent--attach session 0 t)))
            (should (= connected 1))
            (should (eq (romacs-agent-connection session) second))
            (should-not (process-live-p first))
            (delete-process second)))
      (when (process-live-p first) (delete-process first))
      (delete-directory root t))))

(ert-deftest romacs-agent-reattach-a-relay-brings-its-session-back ()
  (romacs-agent-reattach-tests--with
    (let ((peer (make-process :name "romacs-relay-fixture" :command '("cat") :noquery t))
          (romacs-mcp--peers nil))
      (unwind-protect
          (progn
            (push peer romacs-mcp--peers)
            (funcall (gethash "romacs/hello" romacs-mcp-methods) peer
                     (romacs-mcp--object "session" (romacs-agent-id session) "relay" "r1"))
            ;; The session's own peer, and its session attached for its next try.
            (should (process-get peer 'romacs-mcp-owned))
            (should (equal attached (list session))))
        (delete-process peer)))))

(ert-deftest romacs-agent-reattach-relays-do-not-fill-the-endpoint ()
  (let* ((romacs-mcp--listener 'listener) (romacs-mcp--peers nil) (closed nil) (fixtures nil))
    (cl-letf (((symbol-function 'romacs-mcp--close) (lambda (p) (push p closed))))
      (unwind-protect
          (progn
            (dotimes (_ romacs-mcp--peer-limit)
              (let ((p (make-process :name "romacs-owned" :command '("cat") :noquery t)))
                (push p fixtures) (romacs-mcp-own-peer p) (push p romacs-mcp--peers)))
            ;; As many relays as the limit: an anonymous client still gets in.
            (let ((client (make-process :name "romacs-client" :command '("cat") :noquery t)))
              (push client fixtures)
              (romacs-mcp--accept 'listener client nil)
              (should (memq client romacs-mcp--peers))
              (should-not closed))
            ;; Anonymous peers alone are bounded.
            (dotimes (_ (1- romacs-mcp--peer-limit))
              (let ((p (make-process :name "romacs-anon" :command '("cat") :noquery t)))
                (push p fixtures) (push p romacs-mcp--peers)))
            (let ((one-more (make-process :name "romacs-anon" :command '("cat") :noquery t)))
              (push one-more fixtures)
              (romacs-mcp--accept 'listener one-more nil)
              (should (equal closed (list one-more)))))
        (mapc #'delete-process fixtures)))))

(provide 'romacs-agent-reattach-tests)
;;; romacs-agent-reattach-tests.el ends here

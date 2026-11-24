;;; romacs-agent-relay-tests.el --- Attachment and relay routing -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'romacs-agent)

(defun romacs-agent-relay-tests--with-runtime (body)
  "Call BODY with the path of a private runtime and session state."
  (let* ((root (make-temp-file "/var/tmp/p2-" t))
         (process-environment (copy-sequence process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-mcp--listener nil)
         (romacs-mcp--socket nil)
         (romacs-mcp--socket-identity nil)
         (romacs-mcp--peers nil))
    (setenv "XDG_RUNTIME_DIR" root)
    (setenv "XDG_STATE_HOME" (expand-file-name "state" root))
    (unwind-protect (funcall body root)
      (when romacs-mcp--listener (romacs-mcp-stop))
      (delete-directory root t))))

(defun romacs-agent-relay-tests--session (id)
  "Register a detached session with ID in the private state directory."
  (let* ((dir (expand-file-name id (romacs-agent--state-root)))
         (session (romacs-agent--make :id id :dir dir :harness 'claude :state 'detached
                                      :seen-seq 0 :host-socket (expand-file-name "host.sock" dir))))
    (make-directory dir t)
    (puthash id session romacs-agent--sessions)
    session))

(defun romacs-agent-relay-tests--wait (predicate)
  "Service process output until PREDICATE succeeds or time expires."
  (with-timeout (5 (ert-fail "Timed out waiting for fixture process output"))
    (while (not (funcall predicate))
      (accept-process-output nil 0.05))))

(defun romacs-agent-relay-tests--with-peers (route id body)
  "Call BODY with two ID peers and one other peer connected via ROUTE."
  (let (clients peers)
    (unwind-protect
        (progn
          (dolist (session-id (list id id "other"))
            (let ((count (length romacs-mcp--peers)))
              (push (make-network-process :name "relay-client" :family 'local :service route
                                          :noquery t :sentinel #'ignore)
                    clients)
              (romacs-agent-relay-tests--wait (lambda () (> (length romacs-mcp--peers) count)))
              (let ((peer (car romacs-mcp--peers)))
                (process-put peer 'romacs-agent-session session-id)
                (push peer peers))))
          (apply body (nreverse peers)))
      (mapc #'delete-process clients))))

(defun romacs-agent-relay-tests--check-detach (session detach)
  "Invoke DETACH and check that only SESSION's MCP peers close."
  (let ((route (romacs-agent--route session)))
    (should (equal (file-symlink-p route) (romacs-agent--own-socket)))
    (romacs-agent-relay-tests--with-peers
     route (romacs-agent-id session)
     (lambda (first second other)
       (should (cl-every #'process-live-p (list first second other)))
       (funcall detach)
       (should-not (process-live-p first))
       (should-not (process-live-p second))
       (should (process-live-p other))
       (should (equal romacs-mcp--peers (list other)))))))

(defun romacs-agent-relay-tests--with-supervisor (session body)
  "Connect SESSION to a fake supervisor, then call BODY with its accepted peer."
  (let* ((socket (romacs-agent-host-socket session))
         (request "")
         peer
         (listener (make-network-process
                    :name "relay-supervisor" :family 'local :service socket :server t
                    :noquery t :log (lambda (_listener accepted _message) (setq peer accepted))
                    :filter (lambda (_process chunk) (setq request (concat request chunk))))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--keep-soon) #'ignore)
                  ((symbol-function 'romacs-agent--reattach-later) #'ignore))
          (should (process-live-p (romacs-agent--attach session 0)))
          (romacs-agent-relay-tests--wait (lambda () (and peer (string-search "\n" request))))
          (should (hash-table-p (gethash "attach" (json-parse-string request))))
          (funcall body peer))
      (when peer (delete-process peer))
      (when-let* ((connection (romacs-agent-connection session)))
        (set-process-sentinel connection #'ignore)
        (delete-process connection))
      (delete-process listener))))

(ert-deftest romacs-agent-relay-supervisor-kept-then-detached ()
  (romacs-agent-relay-tests--with-runtime
   (lambda (_root)
     (let* ((session (romacs-agent-relay-tests--session "kept"))
            (route (romacs-agent--route session)))
       (romacs-agent-relay-tests--with-supervisor
        session
        (lambda (peer)
          (should-not (file-symlink-p route))
          (process-send-string peer "{\"attached\":\"kept\"}\n")
          (romacs-agent-relay-tests--wait (lambda () (romacs-agent-caught-up session)))
          (romacs-agent-relay-tests--check-detach
           session (lambda ()
                     (delete-process peer)
                     (romacs-agent-relay-tests--wait
                      (lambda () (null (romacs-agent-connection session))))))))))))

(ert-deftest romacs-agent-relay-attach-and-detach-by-id ()
  (romacs-agent-relay-tests--with-runtime
   (lambda (_root)
     (let ((session (romacs-agent-relay-tests--session "by-id")))
       (romacs-agent-relay-tests--session "other")
       (should-not (romacs-agent-connection session))
       (funcall 'romacs-agent-relay-attached "by-id")
       (romacs-agent-relay-tests--check-detach
        session (lambda () (funcall 'romacs-agent-relay-detached "by-id")))))))

(defun romacs-agent-relay-tests--wrong-callers (form owner)
  "Return calls in FORM that bypass the public relay entry points in OWNER."
  (when (consp form)
    (pcase form
      (`(,(or 'defun 'cl-defun 'defsubst) ,name . ,body)
       (romacs-agent-relay-tests--wrong-callers-in body name))
      ((or `(romacs-agent--route-here . ,_)
           `(,(or 'function 'quote) romacs-agent--route-here))
       (unless (memq owner '(romacs-agent-relay-attached romacs-agent-relay-claimed))
         (list (list owner form))))
      ((or `(romacs-agent--drop-relays . ,_)
           `(,(or 'function 'quote) romacs-agent--drop-relays))
       (unless (eq owner 'romacs-agent-relay-detached)
         (list (list owner form))))
      (`(quote ,_) nil)
      (_ (romacs-agent-relay-tests--wrong-callers-in form owner)))))

(defun romacs-agent-relay-tests--wrong-callers-in (forms owner)
  "Find disallowed relay calls in the list or dotted pair FORMS under OWNER."
  (cl-loop while (consp forms)
           append (romacs-agent-relay-tests--wrong-callers (pop forms) owner)))

(defun romacs-agent-relay-tests--scan (file)
  "Find disallowed calls in FILE, recording their enclosing definitions."
  (with-temp-buffer
    (insert-file-contents file)
    (goto-char (point-min))
    (let (violations)
      (condition-case nil
          (while t
            (let ((calls (romacs-agent-relay-tests--wrong-callers (read (current-buffer)) nil)))
              (dolist (call calls)
                (push (cons (file-name-nondirectory file) call) violations))))
        (end-of-file (nreverse violations))))))

(ert-deftest romacs-agent-relay-routing-has-only-id-entry-points ()
  (let* ((directory (file-name-directory (locate-library "romacs-agent")))
         (files (directory-files directory t "\\.el\\'")))
    (should files)
    (should-not (apply #'append (mapcar #'romacs-agent-relay-tests--scan files))))
  (with-temp-buffer
    (insert-file-contents (locate-library "romacs-agent.el"))
    (goto-char (point-min))
    (should (re-search-forward "^(defun romacs-agent--route-attached\\_>" nil t))
    (goto-char (match-beginning 0))
    (let ((body (flatten-tree (nthcdr 3 (read (current-buffer))))))
      (should (memq 'romacs-agent--attached-p body))
      (should-not (memq 'romacs-agent-connection body))
      (should-not (seq-some (lambda (form)
                              (and (symbolp form) (string-prefix-p "process-" (symbol-name form))))
                            body)))))

(ert-deftest romacs-agent-relay-endpoint-change-uses-current-attachment-by-id ()
  (romacs-agent-relay-tests--with-runtime
   (lambda (root)
     (let* ((kept (romacs-agent-relay-tests--session "kept"))
            (pending (romacs-agent-relay-tests--session "pending"))
            (draining (romacs-agent-relay-tests--session "draining"))
            (elsewhere (romacs-agent-relay-tests--session "elsewhere"))
            (live (make-pipe-process :name "relay-kept" :noquery t))
            (unacknowledged (make-pipe-process :name "relay-pending" :noquery t))
            (dead (make-pipe-process :name "relay-draining" :noquery t))
            (old (romacs-agent--own-socket))
            (other (expand-file-name "other.sock" root))
            (listener (make-network-process :name "another-editor" :family 'local :service other
                                            :server t :noquery t))
            (new (expand-file-name "named.sock" root)))
       (unwind-protect
           (progn
             (setf (romacs-agent-connection kept) live
                   (romacs-agent-connection pending) unacknowledged
                   (romacs-agent-connection draining) dead)
             (romacs-agent--kept kept live)
             (romacs-agent-relay-claimed "pending")
             (romacs-agent--kept draining dead)
             (process-put dead 'draining t)
             (delete-process dead)
             (make-symbolic-link other (romacs-agent--route elsewhere))
             (should (equal (seq-filter #'romacs-agent--attached-p '("kept" "pending" "draining" "elsewhere"))
                            '("kept")))
             (romacs-mcp-stop)
             (romacs-mcp-start new)
             (romacs-agent--route-attached)
             (should (equal (mapcar (lambda (session) (file-symlink-p (romacs-agent--route session)))
                                    (list kept pending draining elsewhere))
                            (list new old old other))))
         (mapc #'delete-process (list live unacknowledged dead listener)))))))

(ert-deftest romacs-agent-relay-hello-ensures-attachment-by-id ()
  (romacs-agent-relay-tests--with-runtime
   (lambda (_root)
     (let* ((session (romacs-agent-relay-tests--session "hello"))
            (peer (make-pipe-process :name "relay-hello" :noquery t))
            calls)
       (unwind-protect
           (cl-letf (((symbol-function 'romacs-agent--ensure-attached)
                      (lambda (id) (push id calls) nil))
                     ((symbol-function 'romacs-agent--attach) #'ignore))
             (write-region "" nil (romacs-agent-host-socket session) nil 'silent)
             (funcall (gethash "romacs/hello" romacs-mcp-methods)
                      peer (romacs-mcp--object "session" "hello" "relay" "r1"))
             (should (equal calls '("hello")))
             (should (process-get peer 'romacs-mcp-owned)))
         (delete-process peer))))))

(defun romacs-agent-relay-tests--legacy (kind)
  "Check the old per-editor relay path in state KIND."
  (romacs-agent-relay-tests--with-runtime
   (lambda (_root)
     (let* ((session (romacs-agent-relay-tests--session "legacy"))
            (directory (expand-file-name "session-2147483647/" (romacs-agent--sessions-base)))
            (socket (expand-file-name "server.sock" directory))
            listener)
       (should-not (process-attributes 2147483647))
       (romacs-agent--private-directory directory)
       (with-temp-file (expand-file-name "mcp.json" (romacs-agent-dir session))
         (insert (json-serialize (romacs-mcp--object "args" (vector "--socket" socket)))))
       (unwind-protect
           (progn
             (unless (eq kind 'absent)
               (setq listener (make-network-process :name "relay-legacy" :family 'local
                                                    :service socket :server t :noquery t))
               (when (eq kind 'stale) (delete-process listener))
               (should (string-prefix-p "s" (file-attribute-modes (file-attributes socket)))))
             (let ((identity (romacs-mcp--socket-id socket)))
               (romacs-agent--route-here session)
               (if (eq kind 'live)
                   (progn
                     (should-not (file-symlink-p socket))
                     (should (equal identity (romacs-mcp--socket-id socket)))
                     (should (romacs-agent--listening-p socket)))
                 (should (equal (file-symlink-p socket) (romacs-agent--own-socket)))
                 (should (romacs-agent--listening-p socket)))))
         (when listener (delete-process listener)))))))

(ert-deftest romacs-agent-relay-legacy-path-leads-here ()
  (romacs-agent-relay-tests--legacy 'absent))

(ert-deftest romacs-agent-relay-legacy-stale-socket-is-replaced ()
  (romacs-agent-relay-tests--legacy 'stale))

(ert-deftest romacs-agent-relay-legacy-live-listener-is-preserved ()
  (romacs-agent-relay-tests--legacy 'live))

(provide 'romacs-agent-relay-tests)
;;; romacs-agent-relay-tests.el ends here

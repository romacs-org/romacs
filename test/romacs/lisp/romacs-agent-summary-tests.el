;;; romacs-agent-summary-tests.el --- Data sent from an agent session's owner -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-attached)
  (require 'romacs-agent-test-stored)
  (require 'romacs-test-socket))
(require 'romacs-agent-log)
(require 'romacs-agent-claude)

(defconst romacs-agent-summary-tests--row-slots
  '(id dir harness provider model title cwd parent effort plan resume-at
    state last-time cost input-seq done-seq seen-seq settled-seq failed
    permission-mode context-used context-window))

(defconst romacs-agent-summary-tests--live-slots
  '(activity turn-start children-summary children-rows pending-selection
    waiting-count queue-length attached))

(defun romacs-agent-summary-tests--get (id)
  "Read ID's summary through the P1a API."
  (if (fboundp 'romacs-agent-summary)
      (with-temp-buffer (romacs-agent-summary id))
    (ert-fail "Missing romacs-agent-summary")))

(defun romacs-agent-summary-tests--slot (summary slot)
  "Read SLOT of SUMMARY without assuming its representation."
  (funcall (intern (format "romacs-agent-summary-%s" slot)) summary))

(defun romacs-agent-summary-tests--row-value (session slot)
  "Read SLOT from SESSION, whether loaded or projected from SQLite."
  (pcase slot
    ('resume-at (romacs-agent-resume-time session))
    ((or 'parent 'effort 'plan)
     (plist-get (romacs-agent-options session) (intern (format ":%s" slot))))
    (_ (funcall (intern (format "romacs-agent-%s" slot)) session))))

(defun romacs-agent-summary-tests--check-row (summary session)
  "Compare SUMMARY's persisted fields with SESSION's."
  (dolist (slot romacs-agent-summary-tests--row-slots)
    (ert-info ((format "Row slot %s" slot))
      (should (equal (romacs-agent-summary-tests--slot summary slot)
                     (romacs-agent-summary-tests--row-value session slot))))))

(defun romacs-agent-summary-tests--child-row (row)
  "Read ROW's data, replacing its delegated session with the session ID."
  (if-let* ((child (plist-get row :session)))
      (plist-put (romacs-agent--without row '(:session)) :id (romacs-agent-id child))
    row))

(defun romacs-agent-summary-tests--check-live (summary session)
  "Compare SUMMARY with SESSION and its current chat."
  (romacs-agent-summary-tests--check-row summary session)
  (let ((expected
         `((activity . ,(if (romacs-agent-log--proceed-offered) 'idle (car (romacs-agent-log--doing session))))
           (turn-start . ,(romacs-agent-turn-start session))
           (children-summary . ,(romacs-agent-children-summary session))
           (children-rows . ,(mapcar #'romacs-agent-summary-tests--child-row
                                      (romacs-agent-children-rows session)))
           (pending-selection . ,(romacs-agent--without (romacs-agent-pending-selection session) '(:timer)))
           (waiting-count . ,(hash-table-count (romacs-agent-approvals session)))
           (queue-length . ,(romacs-agent-queue-length session))
           (attached . ,(romacs-agent--attached-p (romacs-agent-id session))))))
    (pcase-dolist (`(,slot . ,value) expected)
      (ert-info ((format "Live slot %s" slot))
        (should (equal (romacs-agent-summary-tests--slot summary slot) value))))))

(defun romacs-agent-summary-tests--session (dir)
  "Make a fake idle session with distinct persisted values, rooted at DIR."
  (romacs-agent--make
   :id "11111111-2222-4333-8444-555555555555" :harness 'claude :provider "work"
   :dir dir :cwd temporary-file-directory :title "Session title" :model "claude-x"
   :options '(:parent "parent" :effort "high" :plan t :resume-at 503.5)
   :state 'idle :last-time 501.25 :cost 0.625
   :seq 37 :input-seq 31 :done-seq 35 :seen-seq 33 :settled-seq 29
   :permission-mode "acceptEdits" :context-used 1200 :context-window 200000))

(defmacro romacs-agent-summary-tests--with-chat (session &rest body)
  "Run BODY in a private chat, binding SESSION to its registered session."
  (declare (indent 1))
  `(romacs-agent-test-with-store
     (let* ((,session (romacs-agent-summary-tests--session (getenv "XDG_STATE_HOME")))
            (romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent-log--chats (make-hash-table :test #'equal))
            (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
            (romacs-agent-ui--children-timer nil)
            (romacs-agent-approval-policy 'ask))
       (puthash (romacs-agent-id ,session) ,session romacs-agent--sessions)
       (with-temp-buffer
         (romacs-agent-log-mode)
         (setq romacs-agent-log--session ,session
               romacs-agent-log--end (copy-marker (point-min) t))
         (puthash (romacs-agent-id ,session) (current-buffer) romacs-agent-log--chats)
         (unwind-protect (progn ,@body)
           (when romacs-agent-ui--children-timer
             (cancel-timer romacs-agent-ui--children-timer)))))))

(ert-deftest romacs-agent-summary-idle ()
  (romacs-agent-summary-tests--with-chat session
    (should (equal (romacs-agent-log--doing session) '(idle)))
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(ert-deftest romacs-agent-summary-working-with-queue-and-selection ()
  (romacs-agent-summary-tests--with-chat session
    (romacs-agent--raise session '(:type input :text "Begin" :seq 38 :time 505.0))
    (romacs-agent--raise session '(:type tool :state started :name "Read" :time 506.0))
    (romacs-agent--enqueue session (romacs-agent-item "First" :delivered #'ignore))
    (romacs-agent--enqueue session (romacs-agent-item "Second"))
    (setf (romacs-agent-pending-selection session) '(:state armed :model "next-model" :effort "low"))
    (should (equal (romacs-agent-log--doing session) '(working)))
    (should (= (romacs-agent-queue-length session) 2))
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(ert-deftest romacs-agent-summary-waiting ()
  (romacs-agent-summary-tests--with-chat session
    (dolist (id '("first" "second"))
      (let ((event (list :type 'approval :request-id id :tool "Edit")))
        (puthash id t (romacs-agent-approvals session))
        (romacs-agent--raise session event)
        (romacs-agent-log-draw session event)))
    (should (eq (car (romacs-agent-log--doing session)) 'waiting))
    (should (= (romacs-agent-log--waiting-count) 2))
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(ert-deftest romacs-agent-summary-waiting-for-a-plan ()
  (romacs-agent-summary-tests--with-chat session
    (romacs-agent-log-draw session '(:type plan :text "Proceed when ready." :proceed t))
    (should (eq (car (romacs-agent-log--doing session)) 'waiting))
    (should (= (romacs-agent-log--waiting-count) 1))
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(defun romacs-agent-summary-tests--children (session)
  "Add a live and a settled child to SESSION, and compute the chat's rows."
  (let ((romacs-agent--children (make-hash-table :test #'equal))
        (romacs-agent-glyphs nil))
    (dolist (fields '((:id "live-child" :title "Live child" :state running :last-time 505.0)
                     (:id "settled-child" :title "Settled child" :state idle :last-time 504.0
                      :input-seq 2 :done-seq 3 :settled-seq 3)))
      (let ((child (apply #'romacs-agent--make :harness 'codex :provider "codex"
                          :options (list :parent (romacs-agent-id session)) fields)))
        (puthash (romacs-agent-id child) child romacs-agent--sessions)
        (romacs-agent--index child)))
    (romacs-agent-ui--summarize-children session)
    (should (equal (mapcar (lambda (row) (plist-get row :key)) (romacs-agent-children-rows session))
                   '(working settled)))
    (should (equal (romacs-agent-children-summary session) "1 threads live"))))

(ert-deftest romacs-agent-summary-live-and-settled-children ()
  (romacs-agent-summary-tests--with-chat session
    (romacs-agent-summary-tests--children session)
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(ert-deftest romacs-agent-summary-failed ()
  (romacs-agent-summary-tests--with-chat session
    (setf (romacs-agent-failed session) t)
    (should (eq (car (romacs-agent-log--doing session)) 'failed))
    (romacs-agent-summary-tests--check-live
     (romacs-agent-summary-tests--get (romacs-agent-id session)) session)))

(defun romacs-agent-summary-tests--with-supervisor (session body)
  "Call BODY with the accepted peer of SESSION's private fake supervisor."
  (let* ((runtime (romacs-test-socket-directory "p1a-"))
         (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
         (socket (expand-file-name "host.sock" runtime))
         (romacs-agent-caught-up-functions nil)
         listener peer connection)
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--keep-soon) #'ignore))
          (setq listener (make-network-process
                          :name "summary-supervisor" :family 'local :service socket
                          :server t :noquery t :filter #'ignore
                          :log (lambda (_server client _message) (setq peer client))))
          (setf (romacs-agent-host-socket session) socket)
          (setq connection (romacs-agent--attach session 0))
          (should (process-live-p connection))
          (set-process-sentinel connection #'ignore)
          (with-timeout (5 (ert-fail "Fake supervisor did not accept the connection"))
            (while (not peer) (accept-process-output nil 0.05)))
          (funcall body peer connection))
      (mapc #'delete-process (delq nil (list connection peer listener)))
      (delete-directory runtime t))))

(ert-deftest romacs-agent-summary-attached-follows-supervisor-ack ()
  (romacs-agent-summary-tests--with-chat session
    (let ((id (romacs-agent-id session)))
      (romacs-agent-summary-tests--with-supervisor
       session
       (lambda (peer connection)
         (should-not (romacs-agent--attached-p id))
         (romacs-agent-summary-tests--check-live (romacs-agent-summary-tests--get id) session)
         (process-send-string peer (concat (json-serialize (romacs-mcp--object "attached" id)) "\n"))
         (with-timeout (5 (ert-fail "Fake supervisor acknowledgement was not processed"))
           (while (not (romacs-agent-caught-up session)) (accept-process-output nil 0.05)))
         (should (romacs-agent--attached-p id))
         (romacs-agent-summary-tests--check-live (romacs-agent-summary-tests--get id) session)
         (delete-process connection)
         (should (romacs-agent-caught-up session))
         (should-not (romacs-agent--attached-p id))
         (romacs-agent-summary-tests--check-live (romacs-agent-summary-tests--get id) session))))))

(ert-deftest romacs-agent-summary-unloaded-row ()
  (romacs-agent-test-with-store
    (should (sqlite-available-p))
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (session (romacs-agent-summary-tests--session (expand-file-name "row-only" (getenv "XDG_STATE_HOME"))))
           (id (romacs-agent-id session)))
      (setf (romacs-agent-failed session) t)
      (should (romacs-agent-store--writer-open))
      (should (romacs-agent-store--put session nil))
      (let ((row (romacs-agent-store-row-session id)))
        (should row)
        (should (romacs-agent-failed row))
        (should-not (file-exists-p (romacs-agent-dir row)))
        (romacs-agent-summary-tests--check-row (romacs-agent-summary-tests--get id) row)
        (should-not (gethash id romacs-agent--sessions))))))

(ert-deftest romacs-agent-summary-unknown-id ()
  (romacs-agent-summary-tests--with-chat session
    (should (gethash (romacs-agent-id session) romacs-agent--sessions))
    (should-not (romacs-agent-summary-tests--get "ffffffff-ffff-4fff-8fff-ffffffffffff"))))

(defun romacs-agent-summary-tests--with-workers-and-switch (body)
  "Run BODY with a summary whose source has child records and a switch timer."
  (romacs-agent-summary-tests--with-chat session
    (romacs-agent-summary-tests--children session)
    (romacs-agent-test-attach session)
    (process-put (romacs-agent-connection session) 'romacs-agent-kept t)
    (let ((timer (run-at-time 3600 nil #'ignore)))
      (unwind-protect
          (progn
            (setf (romacs-agent-pending-selection session)
                  (list :state 'applying :model "next-model" :waiting '("change-model") :timer timer))
            (funcall body (romacs-agent-summary-tests--get (romacs-agent-id session)) session))
        (cancel-timer timer)
        (delete-process (romacs-agent-connection session))))))

(ert-deftest romacs-agent-summary-prin1-round-trip ()
  (romacs-agent-summary-tests--with-workers-and-switch
   (lambda (summary _session)
     (let* ((print-level nil) (print-length nil)
            (printed (prin1-to-string summary))
            (read-back (read-from-string printed)))
       (should (equal (car read-back) summary))
       (should (= (cdr read-back) (length printed)))))))

(defun romacs-agent-summary-tests--check-plain (object seen)
  "Assert OBJECT contains no live objects, visiting each node once via SEEN."
  (should-not (or (processp object) (bufferp object) (markerp object) (timerp object)
                  (windowp object) (hash-table-p object)
                  (and (not (symbolp object)) (functionp object))))
  (unless (gethash object seen)
    (puthash object t seen)
    (cond
     ((consp object)
      (romacs-agent-summary-tests--check-plain (car object) seen)
      (romacs-agent-summary-tests--check-plain (cdr object) seen))
     ((or (vectorp object) (recordp object))
      (dotimes (index (length object))
        (romacs-agent-summary-tests--check-plain (aref object index) seen)))
     ((stringp object)
      (let ((position 0) (limit (length object)))
        (while (< position limit)
          (romacs-agent-summary-tests--check-plain (text-properties-at position object) seen)
          (setq position (next-property-change position object limit))))))))

(ert-deftest romacs-agent-summary-has-no-live-objects ()
  (romacs-agent-summary-tests--with-workers-and-switch
   (lambda (summary session)
     (romacs-agent-summary-tests--check-plain summary (make-hash-table :test #'eq))
     (romacs-agent-summary-tests--check-live summary session))))

(defun romacs-agent-summary-tests--delta (session seq time text)
  "Deliver a Claude journal record carrying a text delta to SESSION."
  (romacs-agent--record
   session (romacs-mcp--object
            "seq" seq "t" time "out"
            (romacs-mcp--object
             "type" "stream_event" "event"
             (romacs-mcp--object "type" "content_block_delta" "index" 0 "delta"
                                 (romacs-mcp--object "type" "text_delta" "text" text))))))

(ert-deftest romacs-agent-summary-a-delta-only-moves-last-time ()
  (romacs-agent-summary-tests--with-chat session
    (let ((id (romacs-agent-id session)))
      (romacs-agent--record session (romacs-mcp--object
                                    "seq" 38 "t" 505.0 "in" (romacs-mcp--object "prompt" "Begin")))
      (romacs-agent-summary-tests--delta session 39 506.0 "one")
      (let* ((before (romacs-agent-summary-tests--get id))
             (slots (remq 'last-time (append romacs-agent-summary-tests--row-slots
                                            romacs-agent-summary-tests--live-slots)))
             (values (mapcar (lambda (slot) (romacs-agent-summary-tests--slot before slot)) slots)))
        (should (= (romacs-agent-summary-tests--slot before 'last-time) 506.0))
        (romacs-agent-summary-tests--delta session 40 507.5 " more")
        (let ((after (romacs-agent-summary-tests--get id)))
          (should (= (romacs-agent-summary-tests--slot after 'last-time) 507.5))
          (cl-mapc (lambda (slot value)
                     (ert-info ((format "Unchanged slot %s" slot))
                       (should (equal (romacs-agent-summary-tests--slot after slot) value))))
                   slots values))))))

(provide 'romacs-agent-summary-tests)
;;; romacs-agent-summary-tests.el ends here

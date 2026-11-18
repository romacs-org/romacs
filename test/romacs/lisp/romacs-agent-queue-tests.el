;;; romacs-agent-queue-tests.el --- Messages waiting for a turn's end -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-test-attached)
(require 'romacs-agent-test-stored)

;; These tests start no supervisor or harness: the build's own (beside the
;; binary) are not needed to make a session.
(advice-add 'romacs-agent--check-companions :override #'ignore)

;; Codex's app-server driver is persistent: a per-turn harness is a test
;; harness's.
(romacs-agent-define-harness 'perturn :lifetime 'per-turn :config-variable "PERTURN_HOME"
                             :config-default "~/.perturn")

;; A test session's write: caught by stubbing `romacs-agent--line'.
(defmacro romacs-agent-queue-tests--writing (sent &rest body)
  "BODY with each message line written pushed onto SENT."
  (declare (indent 1))
  `(cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ object &optional _journal) (push object ,sent)))
             ((symbol-function 'romacs-agent--fold-now) #'ignore))
     ,@body))

(ert-deftest romacs-agent-queued-message-stays-until-sent ()
  ;; A write that fails (the harness gone between the check and the
  ;; write) keeps the message first, counted, for the next chance; sent,
  ;; each goes off the queue in order, one turn at a time.
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "55555555-6666-4777-8888-999999999999" :harness 'claude :state 'running)
                  t)))
    (romacs-agent-send-later session "first")
    (romacs-agent-send-later session "second")
    (romacs-agent-send-later session "ahead" t)
    (should (= (romacs-agent-queue-length session) 3))
    (setf (romacs-agent-state session) 'idle)
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (error "Gone"))))
      (should-error (romacs-agent--dispatch session)))
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("ahead" "first" "second")))
    (should (= (romacs-agent-queue-length session) 3))
    (should-not (romacs-agent-dispatch session))
    (let ((sent nil))
      (romacs-agent-queue-tests--writing sent
        (romacs-agent--dispatch session)
        ;; Each at a turn's end: the first's turn runs till then.
        (should (eq (romacs-agent-state session) 'running))
        (setf (romacs-agent-state session) 'idle)
        (romacs-agent--dispatch session))
      (should (= (length sent) 2))
      (should (equal (mapcar #'car (romacs-agent-queue session)) '("second")))
      (should (= (romacs-agent-queue-length session) 1)))))

(ert-deftest romacs-agent-queue-dispatch-is-one-at-a-time ()
  ;; A per-turn harness's run that ends before it is attached ends the turn
  ;; while its message is being sent: the next is sent after, once each.
  (let ((session (romacs-agent--make :id "66666666-7777-4888-8999-aaaaaaaaaaaa" :harness 'perturn
                                     :state 'running))
        (turns nil))
    (romacs-agent-send-later session "first")
    (romacs-agent-send-later session "second")
    (setf (romacs-agent-state session) 'idle)
    (cl-letf (((symbol-function 'romacs-agent--turn)
               (lambda (s text &rest _)
                 (push text turns)
                 ;; The run ends at once: the run's end asks for the next.
                 (romacs-agent--dispatch s))))
      (romacs-agent--dispatch session))
    (should (equal (reverse turns) '("first" "second")))
    (should (null (romacs-agent-queue session)))
    (should (= (romacs-agent-queue-length session) 0))
    (should (null (romacs-agent-dispatch session)))))

(ert-deftest romacs-agent-resume-sends-what-waits ()
  ;; What waits for a resumed harness goes once it is attached, caught up
  ;; and ready, not before.
  (let ((session (romacs-agent--make :id "77777777-8888-4999-8aaa-bbbbbbbbbbbb" :harness 'claude :state 'starting))
        (sent nil))
    (romacs-agent-send-later session "waited")
    (romacs-agent-queue-tests--writing sent
      (romacs-agent--dispatch session)
      (should (null sent))
      (romacs-agent-test-attach session)
      (romacs-agent--dispatch session)
      (should (null sent))
      (romacs-agent-test-attach session t)
      (romacs-agent--dispatch session))
    (should (= (length sent) 1))
    (should (= (romacs-agent-queue-length session) 0))))

(ert-deftest romacs-agent-resume-sends-only-when-no-turn-was-cut ()
  ;; How the last run ended decides, from the journal's own records: a
  ;; harness that ended in the middle of a turn takes it up again when
  ;; resumed, and what waits goes at that turn's end; one that ended
  ;; between turns gets it once ready.
  (dolist (cut '(nil t))
    (let* ((session (romacs-agent-test-attach
                     (romacs-agent--make :id "88888888-9999-4aaa-8bbb-cccccccccccc" :harness 'claude
                                         :provider "claude")
                     t))
           (romacs-agent-event-functions nil)
           (sent nil))
      (romacs-agent--raise session (list :type 'input :text "a turn" :seq 1))
      (unless cut (romacs-agent--raise session (list :type 'done :text "ok" :seq 2)))
      (setf (romacs-agent-state session) 'running)
      (romacs-agent-send-later session "waited")
      (romacs-agent--raise session (list :type 'exit :status 0 :seq 3))
      ;; Its supervisor gone with it.
      (delete-process (romacs-agent-connection session))
      (should (eq (romacs-agent-interrupted session) cut))
      (cl-letf (((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                ((symbol-function 'romacs-agent--claude-argv) (lambda (&rest _) nil))
                ((symbol-function 'romacs-agent--launch) #'ignore)
                ((symbol-function 'romacs-agent--attach) #'ignore))
        (romacs-agent-resume session))
      (should (= (romacs-agent-queue-length session) 1))
      (romacs-agent-queue-tests--writing sent
        ;; Attached to the resumed harness, its startup answered.
        (romacs-agent-test-attach session t)
        (romacs-agent--dispatch session)
        (should (eq (null sent) cut))
        (when cut
          ;; The resumed turn's end sends it.
          (romacs-agent--raise session (list :type 'done :text "resumed" :seq 4))
          (setf (romacs-agent-state session) 'idle)
          (romacs-agent--dispatch session)))
      (should (= (length sent) 1))
      (should (= (romacs-agent-queue-length session) 0)))))

;;; One gate (romacs#308)

(defun romacs-agent-queue-tests--ack (session)
  "SESSION's attachment acknowledged, as its supervisor says it: its
replay over."
  (let ((connection (romacs-agent-connection session)))
    (process-put connection 'romacs-agent session)
    (romacs-agent--filter-lines connection (concat (json-serialize (romacs-mcp--object "attached" 1)) "\n"))))

(defun romacs-agent-queue-tests--settle ()
  "Run what the editor runs once idle (a deferred dispatch or settle)."
  (accept-process-output nil 0.05))

(defconst romacs-agent-queue-tests--gates
  `(;; Paths 1-4: kept state says ready, the attachment replays still;
    ;; a replayed turn's end does not open it, the acknowledgement does.
    (replaying ,(lambda (s) (setf (romacs-agent-caught-up s) nil))
               ,(lambda (s) (romacs-agent--raise s (list :type 'done :seq 2)) (romacs-agent--settle s))
               romacs-agent-queue-tests--ack)
    ;; Paths 5-6: a turn its records leave under way; its end opens it
    ;; for a turn of its own (a steer goes into it).
    (turn-under-way ,(lambda (s) (setf (romacs-agent-state s) 'running (romacs-agent-input-seq s) 3))
                    ,(lambda (s) (romacs-agent--step s) (romacs-agent--dispatch s))
                    ,(lambda (s) (romacs-agent--raise s (list :type 'done :seq 4))
                       (romacs-agent-queue-tests--settle))
                    romacs-agent-send-later)
    ;; A rollback's or a renewal's relaunch, its old run still stopping:
    ;; its exit, then the resume, opens it.
    (relaunching ,(lambda (s) (cl-letf (((symbol-function 'romacs-agent--stop-then) #'ignore))
                                (romacs-agent--relaunch s)))
                 ,(lambda (s) (romacs-agent-queue-tests--settle) (romacs-agent--dispatch s))
                 nil)
    ;; Path 7: stopped, its harness going: the resumed one takes it.
    (stopped ,(lambda (s) (romacs-agent--set-stopped s t))
             ,(lambda (s) (romacs-agent--dispatch s))
             ,(lambda (s) (romacs-agent--set-stopped s nil) (setf (romacs-agent-caught-up s) nil)
                (romacs-agent-queue-tests--ack s)))
    (selecting ,(lambda (s) (setf (romacs-agent-pending-selection s) (list :state 'resuming)))
               ,(lambda (s) (romacs-agent--dispatch s))
               romacs-agent--settle)
    (rolling-back ,(lambda (s) (romacs-agent-rollback--hold s 'token))
                  ,(lambda (s) (romacs-agent--dispatch s))
                  ,(lambda (s) (romacs-agent-rollback--release s 'token)))
    (renewing ,(lambda (s) (romacs-agent-renew--hold s 'token))
              ,(lambda (s) (romacs-agent--dispatch s))
              ,(lambda (s) (romacs-agent-renew--release s 'token)))
    ;; A line written not yet journaled back: its echo opens it.
    (unechoed ,(lambda (s) (setf (romacs-agent-startup-pending s) "r1"))
              ,(lambda (s) (romacs-agent--dispatch s))
              ,(lambda (s) (romacs-agent--record s (romacs-mcp--object "seq" 5 "in" (romacs-mcp--object
                                                                                    "type" "control_request" "request_id" "r1"
                                                                                    "request" (romacs-mcp--object "subtype" "interrupt"))))
                 (romacs-agent-queue-tests--settle))))
  "Each gate a message to a persistent harness waits at: (NAME CLOSE
EVENTS OPEN SEND), CLOSE shutting it on a session, EVENTS what must not
open it, OPEN its event, nil for one only a resume opens; SEND how the
message is sent, `romacs-agent-send' by default.")

(ert-deftest romacs-agent-send-waits-at-each-gate ()
  ;; romacs#308: whatever shuts it, a message waits, never written early;
  ;; the event that opens it writes it once, its DELIVERED called then.
  (require 'romacs-agent-rollback)
  (require 'romacs-agent-renew)
  (pcase-dolist (`(,name ,close ,events ,open ,send) romacs-agent-queue-tests--gates)
    (ert-info ((symbol-name name) :prefix "gate: ")
      (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
             (romacs-agent-event-functions nil)
             (dir (make-temp-file "romacs-gate-" t))
             (session (romacs-agent--make :id "abababab-0000-4000-8000-000000000000" :harness 'claude
                                          :provider "claude" :state 'idle :dir dir :title "gate"))
             (sent nil) (delivered 0))
        (unwind-protect
            (progn
              (puthash (romacs-agent-id session) session romacs-agent--sessions)
              ;; Started up: nothing of its startup left to write.
              (setf (romacs-agent-driver-state (romacs-agent-test-attach session)) (list :initialized t :ready t))
              (romacs-agent-queue-tests--writing sent
                (cl-letf (((symbol-function 'romacs-agent--keep-soon) #'ignore)
                          ((symbol-function 'romacs-agent--changed) #'ignore))
                  (funcall close session)
                  (if send
                      (progn (funcall send session "hello")
                             (setf (romacs-agent-item-delivered (car (romacs-agent-queue session)))
                                   (lambda () (cl-incf delivered))))
                    (romacs-agent-send session "hello" (lambda () (cl-incf delivered))))
                  (funcall events session)
                  (should-not sent)
                  (should (= delivered 0))
                  (should (= (romacs-agent-queue-length session) 1))
                  (when open
                    (funcall open session)
                    (should (= (length sent) 1))
                    (should (= delivered 1))
                    (should (= (romacs-agent-queue-length session) 0))))))
          (delete-process (romacs-agent-connection session))
          (delete-directory dir t))))))

(ert-deftest romacs-agent-a-refused-send-keeps-nothing ()
  ;; Its harness ended, or its write failing: the sender is told and keeps
  ;; its message; the queue keeps no copy to send again.
  (let ((ended (romacs-agent--make :id "acacacac-0000-4000-8000-000000000000" :harness 'claude :state 'ended
                                   :title "ended"))
        (failing (romacs-agent-test-attach
                  (romacs-agent--make :id "adadadad-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :title "failing")
                  t)))
    (should-error (romacs-agent-send ended "hello") :type 'user-error)
    (should-not (romacs-agent-queue ended))
    (should (= (romacs-agent-queue-length ended) 0))
    ;; Its exit read, its supervisor still attached (path 6).
    (romacs-agent-test-attach ended t)
    (should-error (romacs-agent-send ended "hello") :type 'user-error)
    (should (= (romacs-agent-queue-length ended) 0))
    ;; Ended as its kept state says, attached to a supervisor still
    ;; replaying: what ran since decides, so it waits.
    (setf (romacs-agent-caught-up ended) nil)
    (romacs-agent-send ended "hello")
    (should (= (romacs-agent-queue-length ended) 1))
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (error "Gone")))
              ((symbol-function 'romacs-agent--fold-now) #'ignore))
      (should-error (romacs-agent-send failing "hello")))
    (should-not (romacs-agent-queue failing))
    (should (= (romacs-agent-queue-length failing) 0))))

(ert-deftest romacs-agent-a-steer-is-its-turns-alone ()
  ;; Sent into a running turn, a message steers it at once; sent while
  ;; none runs (its harness starting), each waits for a turn of its own;
  ;; one whose turn ended before it could go waits for a turn of its own.
  (require 'romacs-agent-rollback)
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "aeaeaeae-0000-4000-8000-000000000000" :harness 'claude :state 'idle)))
        (sent nil))
    (romacs-agent-queue-tests--writing sent
      (cl-letf (((symbol-function 'romacs-agent--changed) #'ignore))
        ;; Starting: not ready.
        (romacs-agent-send session "one")
        (romacs-agent-send session "two")
        (should-not sent)
        (setf (romacs-agent-driver-state session) (list :initialized t :ready t))
        (romacs-agent--dispatch session)
        (should (= (length sent) 1))
        (should (= (romacs-agent-queue-length session) 1))
        ;; Into the turn "one" began: at once.
        (romacs-agent-send session "steer")
        (should (= (length sent) 2))
        (should (equal (mapcar #'car (romacs-agent-queue session)) '("two")))
        ;; A steer held (a rollback) past its turn's end: a turn of its own.
        (romacs-agent-rollback--hold session 'token)
        (romacs-agent-send session "late")
        (romacs-agent--raise session (list :type 'done :seq 5))
        (romacs-agent-rollback--release session 'token)
        (should (= (length sent) 3))
        (should (equal (gethash "content" (gethash "message" (car sent))) "two"))
        (should (equal (mapcar #'car (romacs-agent-queue session)) '("late")))))))

(ert-deftest romacs-agent-a-delivery-to-an-ended-harness-waits ()
  ;; A tool's delivery has no sender to keep it: refused, it waits for the
  ;; resume, the refusal said on the session.
  (require 'romacs-agent-tools)
  (let* ((session (romacs-agent-test-attach
                   (romacs-agent--make :id "afafafaf-0000-4000-8000-000000000000" :harness 'claude :state 'ended
                                       :title "parent")
                   t))
         (said nil)
         (romacs-agent-event-functions (list (lambda (_ event) (when (eq (plist-get event :type) 'error)
                                                                (push (plist-get event :text) said))))))
    (romacs-agent-tools--deliver session "a wake" #'ignore nil nil 'next)
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("a wake")))
    (should (string-match-p "has ended" (car said)))))

(ert-deftest romacs-agent-a-relaunch-that-does-not-stop-stays-held ()
  ;; Its old run never going, or its stop never started: nothing reaches
  ;; the conversation it left, and the session says how to go on.
  (let* ((session (romacs-agent-test-attach
                   (romacs-agent--make :id "a0a0a0a0-0000-4000-8000-000000000000" :harness 'claude :state 'idle)
                   t))
         (said nil) (resumed nil)
         (romacs-agent-event-functions (list (lambda (_ event) (when (eq (plist-get event :type) 'error)
                                                                (push (plist-get event :text) said))))))
    ;; Its stop answered, the old attachment there past its wait.
    (cl-letf (((symbol-function 'romacs-agent--stop-then) #'ignore)
              ((symbol-function 'romacs-agent-resume) (lambda (_) (setq resumed t))))
      (romacs-agent--relaunch session)
      (romacs-agent--resume-when-gone session 0))
    (should-not resumed)
    (should (romacs-agent--held-p session))
    (should (string-match-p "did not stop" (car said)))
    (setf (romacs-agent-driver-state session) (list :ready t) said nil)
    (cl-letf (((symbol-function 'romacs-agent--stop-then) (lambda (&rest _) (error "No systemctl"))))
      (should-error (romacs-agent--relaunch session)))
    (should (romacs-agent--held-p session))
    (should (string-match-p "did not stop" (car said)))))

(ert-deftest romacs-agent-queued-messages-outlive-a-restart ()
  ;; romacs#287: what waits in a session's queue, all data, is kept in the
  ;; store, its session marked; a session made again (a restart) takes it
  ;; back in order, a steer as a turn of its own; written, dropped or
  ;; cleared, it is let go, the mark with its last.  One with a function
  ;; (a tool's delivery) is not kept.
  (romacs-agent-test-with-store
    (make-directory (file-name-directory (romacs-agent-store--file)) t)
    (should (romacs-agent-store--open))
    (let* ((id "b0b0b0b0-0000-4000-8000-000000000000")
           (mark (expand-file-name (format "%s.queued" id) (romacs-agent--marks-dir)))
           (session (romacs-agent-test-attach
                     (romacs-agent--make :id id :harness 'claude :state 'running :title "kept")
                     t))
           (sent nil))
      (romacs-agent-send-later session "a")
      (romacs-agent-send-later session "b")
      (romacs-agent-send-later session "first" t)
      (romacs-agent-send session "steer")
      (romacs-agent-send-later session "checked" nil nil #'always)
      (should (file-exists-p mark))
      (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("first" "a" "b")))
      ;; Its steer went at once: never kept past its write.  Made again,
      ;; one sent to it as it loads goes after what was kept.
      (let ((again (romacs-agent--make :id id :harness 'claude :state 'running :title "kept")))
        (romacs-agent-test-attach again t)
        (romacs-agent-send-later again "meanwhile")
        ;; Put first as it loads: ahead of what was kept too.
        (romacs-agent-send-later again "urgent" t)
        (setf (romacs-agent-state again) 'idle)
        ;; Its harness ready: the first goes at once, let go.
        (romacs-agent-queue-tests--writing sent (romacs-agent-store--requeue again nil))
        (should (equal (gethash "content" (gethash "message" (car sent))) "urgent"))
        (should (equal (mapcar #'car (romacs-agent-queue again)) '("first" "a" "b" "meanwhile")))
        (should (cl-every (lambda (item) (eq (romacs-agent-item-kind item) 'next)) (romacs-agent-queue again)))
        ;; Their places kept for a restart after.
        (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("first" "a" "b" "meanwhile")))
        ;; Cleared (a task cancelled): its kept ones too, loaded or not.
        (let ((other (romacs-agent--make :id id :harness 'claude :state 'idle :title "kept")))
          (romacs-agent--clear-queue other)))
      (should-not (romacs-agent-store-queued id))
      (should-not (file-exists-p mark)))))

(ert-deftest romacs-agent-a-kept-message-written-is-never-sent-twice ()
  ;; Written to its harness, its id in the journal's input record, then
  ;; the editor ends before its kept row is let go: the session made
  ;; again sends it no second time, and its row goes.
  (romacs-agent-test-with-store
    (make-directory (file-name-directory (romacs-agent-store--file)) t)
    (should (romacs-agent-store--open))
    (let* ((id "b1b1b1b1-0000-4000-8000-000000000000")
           (dir (expand-file-name id (romacs-agent--state-root)))
           (session (progn (make-directory dir t)
                           (romacs-agent-test-attach
                            (romacs-agent--make :id id :harness 'claude :state 'running :title "once" :dir dir)
                            t)))
           (journaled nil) (sent nil))
      (romacs-agent-send-later session "once")
      (setf (romacs-agent-state session) 'idle)
      ;; Its write journals what the supervisor would; its row's delete
      ;; never commits (the editor ends first).
      (cl-letf (((symbol-function 'romacs-agent--line)
                 (lambda (_ object &optional journal) (setq journaled (or journal object))))
                ((symbol-function 'romacs-agent--fold-now) #'ignore)
                ((symbol-function 'romacs-agent-store-queued-delete) #'ignore))
        (romacs-agent--dispatch session))
      (should (gethash "romacsQueued" journaled))
      (write-region (concat (json-serialize (romacs-mcp--object "seq" 1 "t" 1.0 "in" journaled)) "\n")
                    nil (expand-file-name "journal.jsonl" dir) nil 'silent)
      (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("once")))
      (let ((again (romacs-agent-test-attach
                    (romacs-agent--make :id id :harness 'claude :state 'idle :title "once" :dir dir) t)))
        (romacs-agent-queue-tests--writing sent (romacs-agent-store--requeue again nil))
        (should-not sent)
        (should-not (romacs-agent-queue again)))
      (should-not (romacs-agent-store-queued id)))))

(ert-deftest romacs-agent-cut-turn-stays-cut-across-runs ()
  ;; A cut turn, resumed, cut again (its resumption journals no prompt):
  ;; still cut, until a real done; a journal replayed anew says the same.
  (let ((records (list (list :type 'input :text "a turn" :seq 1)
                       (list :type 'exit :status 1 :seq 2)
                       (list :type 'session :seq 3)
                       (list :type 'exit :status 1 :seq 4))))
    (dolist (_ '(live replayed))
      (let ((session (romacs-agent--make :id "99999999-aaaa-4bbb-8ccc-dddddddddddd" :harness 'claude))
            (romacs-agent-event-functions nil))
        (dolist (event records) (romacs-agent--raise session event))
        (should (romacs-agent-interrupted session))
        (romacs-agent--raise session (list :type 'done :text "at last" :seq 5))
        (romacs-agent--raise session (list :type 'exit :status 0 :seq 6))
        (should-not (romacs-agent-interrupted session))))))

(provide 'romacs-agent-queue-tests)
;;; romacs-agent-queue-tests.el ends here

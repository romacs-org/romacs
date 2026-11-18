;;; romacs-agent-queue-tests.el --- Messages waiting for a turn's end -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-test-attached)

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
    ;; Path 6: a run ended: nothing opens it but a resume.
    (ended ,(lambda (s) (romacs-agent--raise s (list :type 'exit :status 0 :seq 3)))
           ,(lambda (s) (romacs-agent--dispatch s))
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
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (error "Gone")))
              ((symbol-function 'romacs-agent--fold-now) #'ignore))
      (should-error (romacs-agent-send failing "hello")))
    (should-not (romacs-agent-queue failing))
    (should (= (romacs-agent-queue-length failing) 0))))

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

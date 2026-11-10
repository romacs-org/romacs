;;; romacs-agent-queue-tests.el --- Messages waiting for a turn's end -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

;; Codex's app-server driver is persistent: a per-turn harness is a test
;; harness's.
(romacs-agent-define-harness 'perturn :lifetime 'per-turn :config-variable "PERTURN_HOME"
                             :config-default "~/.perturn")

(ert-deftest romacs-agent-queued-message-stays-until-sent ()
  ;; The harness went between the turn's end and the send: the send fails,
  ;; and the message is still first, counted, for the next end or a resume.
  (let ((session (romacs-agent--make :id "55555555-6666-4777-8888-999999999999" :harness 'claude
                                     :state 'running :host-socket "/nonexistent/host.sock")))
    (romacs-agent-send-later session "first")
    (romacs-agent-send-later session "second")
    (romacs-agent-send-later session "ahead" t)
    (should (= (romacs-agent-queue-length session) 3))
    (setf (romacs-agent-state session) 'idle)
    (should-error (romacs-agent--send-next session) :type 'user-error)
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("ahead" "first" "second")))
    (should (= (romacs-agent-queue-length session) 3))
    ;; Sent now: off the queue, in order.
    (let ((sent nil))
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ object &optional _journal) (push object sent))))
        (romacs-agent--send-next session)
        (romacs-agent--send-next session))
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
                 (romacs-agent--send-next s))))
      (romacs-agent--send-next session))
    (should (equal (reverse turns) '("first" "second")))
    (should (null (romacs-agent-queue session)))
    (should (= (romacs-agent-queue-length session) 0))
    (should (null (romacs-agent-dispatch session)))))

(ert-deftest romacs-agent-resume-sends-what-waits ()
  ;; A send failed after the turn's end; the resumed harness is starting,
  ;; and what waits goes to it.
  (let ((session (romacs-agent--make :id "77777777-8888-4999-8aaa-bbbbbbbbbbbb" :harness 'claude
                                     :state 'starting))
        (sent nil))
    (romacs-agent-send-later session "waited")
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ object &optional _journal) (push object sent))))
      (romacs-agent--send-next session)
      (should (null sent))
      (romacs-agent--send-next session t))
    (should (= (length sent) 1))
    (should (= (romacs-agent-queue-length session) 0))))

(ert-deftest romacs-agent-resume-sends-only-when-no-turn-was-cut ()
  ;; How the last run ended decides, from the journal's own records, not
  ;; when attaching completes: a harness that ended in the middle of a turn
  ;; takes it up again when resumed, and what waits goes at that turn's end;
  ;; one that ended between turns gets it once attached.
  (dolist (cut '(nil t))
    (let* ((session (romacs-agent--make :id "88888888-9999-4aaa-8bbb-cccccccccccc" :harness 'claude
                                        :provider "claude"))
           (romacs-agent-event-functions nil)
           (waiting nil))
      (romacs-agent--raise session (list :type 'input :text "a turn" :seq 1))
      (unless cut (romacs-agent--raise session (list :type 'done :text "ok" :seq 2)))
      (setf (romacs-agent-state session) 'running)
      (romacs-agent-send-later session "waited")
      (romacs-agent--raise session (list :type 'exit :status 0 :seq 3))
      (should (eq (romacs-agent-interrupted session) cut))
      (cl-letf (((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                ((symbol-function 'romacs-agent--claude-argv) (lambda (&rest _) nil))
                ((symbol-function 'romacs-agent--launch) #'ignore)
                ((symbol-function 'romacs-agent--attach) #'ignore)
                ((symbol-function 'romacs-agent--when-attached) (lambda (&rest _) (push t waiting))))
        (romacs-agent-resume session))
      (should (eq (null waiting) cut))
      (should (= (romacs-agent-queue-length session) 1))
      ;; The resumed turn's end sends it (a live connection).
      (when cut
        (let ((sent nil))
          (cl-letf (((symbol-function 'process-live-p) (lambda (_) t))
                    ((symbol-function 'romacs-agent--line) (lambda (_ object &optional _journal) (push object sent))))
            (romacs-agent--raise session (list :type 'done :text "resumed" :seq 4))
            (romacs-agent--send-next session))
          (should (= (length sent) 1))
          (should (= (romacs-agent-queue-length session) 0)))))))

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

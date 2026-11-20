;;; romacs-agent-reap-tests.el --- A finished task's harness is stopped -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-tools))

(defmacro romacs-agent-reap-tests--with (&rest body)
  "BODY with `review', an idle reviewer task under a unit, and `lane', an
idle task of another role; `scheduled' collects each look's (SESSION
DELAY), `stopped' each session stopped and `asleep' each reaped to sleep;
`working' says a shell runs among a task's processes."
  (declare (indent 0))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-tools--reaping (make-hash-table :test #'equal))
          (romacs-agent-reap-after 300)
          (review (romacs-agent--make :id "r" :harness 'codex :state 'idle :unit "romacs-agent-r"
                                      :dir (make-temp-file "romacs-reap-" t)
                                      :last-time (- (float-time) 1000)
                                      :options '(:parent "lead" :role "review")))
          (lane (romacs-agent--make :id "l" :harness 'claude :state 'idle :unit "romacs-agent-l"
                                    :last-time (- (float-time) 1000)
                                    :options '(:parent "lead" :role "implementation")))
          (scheduled nil) (stopped nil) (asleep nil) (working nil))
     (dolist (s (list review lane)) (puthash (romacs-agent-id s) s romacs-agent--sessions))
     (cl-letf (((symbol-function 'run-at-time)
                (lambda (delay _repeat function session &rest _)
                  (when (eq function #'romacs-agent-tools--reap) (push (list session delay) scheduled))))
               ((symbol-function 'romacs-agent-stop) (lambda (s &optional reaped) (push s stopped) (when reaped (push s asleep))))
               ((symbol-function 'romacs-agent-tools--working-p) (lambda (_s) working)))
       (unwind-protect (progn ,@body) (delete-directory (romacs-agent-dir review) t)))))

(ert-deftest romacs-agent-reap-a-delegated-task-after-its-turn ()
  (romacs-agent-reap-tests--with
    ;; Its ends, replayed or live, schedule one look, whatever its role;
    ;; a user's own session's none.
    (dotimes (_ 3) (romacs-agent-tools--reap-on-event review (list :type 'done)))
    (romacs-agent-tools--reap-on-event lane (list :type 'done))
    (romacs-agent-tools--reap-on-event (romacs-agent--make :id "u" :state 'idle :options nil) (list :type 'done))
    (should (equal scheduled (list (list lane 300) (list review 300))))
    ;; Stopped to sleep, not ended.
    (romacs-agent-tools--reap review)
    (should (equal stopped (list review)))
    (should (equal asleep (list review)))))

(ert-deftest romacs-agent-reap-keeps-a-task-at-work ()
  (romacs-agent-reap-tests--with
    (cl-flet ((reaped-p (setup)
                (setq stopped nil)
                (let ((saved (copy-romacs-agent review)))
                  (funcall setup)
                  (romacs-agent-tools--reap review)
                  (puthash "r" saved romacs-agent--sessions)
                  (setq review saved)
                  ;; A copy shares its approvals table: none is left asked.
                  (clrhash (romacs-agent-approvals review))
                  (and stopped t))))
      (should-not (reaped-p (lambda () (setf (romacs-agent-state review) 'running))))
      (should-not (reaped-p (lambda () (romacs-agent--enqueue review (romacs-agent-item "more")))))
      (should-not (reaped-p (lambda () (puthash "q" t (romacs-agent-approvals review)))))
      (should-not (reaped-p (lambda () (setf (romacs-agent-cancelled review) t))))
      (should-not (reaped-p (lambda () (setf (romacs-agent-state review) 'ended))))
      ;; A shell it started still runs (a build in the background): kept,
      ;; looked at again a reap time later.
      (setq scheduled nil)
      (should-not (reaped-p (lambda () (setq working t))))
      (setq working nil)
      (should (equal (mapcar (lambda (look) (list (romacs-agent-id (car look)) (cadr look))) scheduled) '(("r" 300))))
      ;; Spoken to lately: looked at again when its time is up.
      (setq scheduled nil)
      (should-not (reaped-p (lambda () (setf (romacs-agent-last-time review) (- (float-time) 100)))))
      (should (= (length scheduled) 1))
      (should (< 190 (cadr (car scheduled)) 201)))))

(ert-deftest romacs-agent-reap-waits-for-what-it-was-told-and-its-replay ()
  (romacs-agent-reap-tests--with
    ;; A message written and not yet journaled back: still at work.
    (setf (romacs-agent-requests review) 7 (romacs-agent-inputs review) 6)
    (romacs-agent-tools--reap review)
    (should-not stopped)
    (setf (romacs-agent-inputs review) 7)
    ;; Attached, its replay not over: looked at again later.
    (let ((process (make-process :name "romacs-reap-fixture" :command '("cat") :noquery t)))
      (unwind-protect
          (progn (setf (romacs-agent-connection review) process (romacs-agent-caught-up review) nil)
                 (setq scheduled nil)
                 (romacs-agent-tools--reap review)
                 (should-not stopped)
                 (should (equal scheduled (list (list review 300))))
                 (setf (romacs-agent-caught-up review) t)
                 (romacs-agent-tools--reap review)
                 (should (equal stopped (list review))))
        (delete-process process)))))

(ert-deftest romacs-agent-reap-a-stopped-task-takes-its-messages-when-resumed ()
  (romacs-agent-reap-tests--with
    (let ((written nil) (resumed nil) (continued nil)
          (process (make-process :name "romacs-reap-fixture" :command '("cat") :noquery t)))
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent--input) (lambda (_s text &rest _) (push text written)))
                    ((symbol-function 'romacs-agent-resume) (lambda (s) (push s resumed)))
                    ((symbol-function 'romacs-agent--continue) (lambda (s) (push s continued) s)))
            ;; Stopping, its harness still attached: kept for after.
            (romacs-agent--set-stopped review t)
            (setf (romacs-agent-connection review) process
                  (romacs-agent-driver-state review) '(:ready t))
            (romacs-agent-send review "one more")
            (should-not written)
            (should (= (romacs-agent-queue-length review) 1))
            (should-not resumed)
            ;; Gone: what waits resumes it.
            (process-put process 'romacs-agent review)
            (delete-process process)
            (romacs-agent--sentinel process "killed")
            (should (eq (romacs-agent-state review) 'ended))
            (should (equal resumed (list review)))
            ;; Sent after it had gone: resumed at once.
            (setq resumed nil)
            (romacs-agent-send review "and another")
            (should (equal resumed (list review)))
            ;; A wake for a stopped parent goes on with it first.
            (romacs-agent-tools--deliver review "Delegated task x" #'ignore)
            (should (equal continued (list review))))
        (when (process-live-p process) (delete-process process))))))

(ert-deftest romacs-agent-reap-a-selection-restart-resumes-on-the-selection ()
  ;; A selection restarts a harness run without a unit: stopped here, a
  ;; message held.  Its end resumes it on the selection, once.
  (let* ((dir (make-temp-file "romacs-restart-" t))
         (session (romacs-agent--make :id "x" :harness 'claude :state 'idle :dir dir :options '(:model "old")
                                      :host-socket (expand-file-name "none.sock" dir)))
         (process (make-process :name "romacs-restart-fixture" :command '("cat") :noquery t))
         (resumed nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-resume)
                   (lambda (s) (push (plist-get (romacs-agent-options s) :model) resumed)))
                  ((symbol-function 'romacs-agent-stop) (lambda (s) (romacs-agent--set-stopped s t)))
                  ((symbol-function 'romacs-agent--manifest) #'ignore)
                  ((symbol-function 'romacs-agent--note) #'ignore)
                  ((symbol-function 'run-at-time) #'ignore))
          (setf (romacs-agent-connection session) process
                (romacs-agent-pending-selection session) (list :state 'applying :waiting '("x") :model "new"))
          (romacs-agent--enqueue session (romacs-agent-item "held"))
          (process-put process 'romacs-agent session)
          (romacs-agent--restart session)
          (delete-process process)
          (romacs-agent--sentinel process "killed")
          (should (equal resumed '("new")))
          ;; Under a unit, the harness gone before its unit is: a message
          ;; delivered then (a task's, a child's result) leaves the resume
          ;; to the restart.
          (setq resumed nil)
          (romacs-agent--set-stopped session nil)
          (setf (romacs-agent-connection session) nil
                (romacs-agent-options session) '(:model "old")
                (romacs-agent-pending-selection session) (list :state 'restarting :model "new"))
          (romacs-agent--continue session)
          (should-not resumed)
          (setf (romacs-agent-pending-selection session)
                (plist-put (romacs-agent-pending-selection session) :stopped t))
          (romacs-agent--restarted session)
          (should (equal resumed '("new"))))
      (when (process-live-p process) (delete-process process))
      (delete-directory dir t))))

(ert-deftest romacs-agent-reap-stopped-reads-no-field-after-a-reload ()
  ;; Loaded over a definition whose record had the field: its accessor's
  ;; inliner goes, and callers compiled now read the mark.
  (function-put 'romacs-agent-stopped 'compiler-macro (lambda (_form &rest _) '(error "inlined field")))
  (let ((romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (load "romacs-agent" nil t))
  (should-not (function-get 'romacs-agent-stopped 'compiler-macro))
  (let ((dir (make-temp-file "romacs-stopped-" t)))
    (unwind-protect
        (let ((session (romacs-agent--make :id "x" :harness 'claude :dir dir)))
          (romacs-agent--set-stopped session t)
          (should (funcall (byte-compile (lambda (s) (romacs-agent-stopped s))) session)))
      (delete-directory dir t))))

(ert-deftest romacs-agent-reap-a-stopped-socket-left-is-ended ()
  (let* ((root (make-temp-file "romacs-stopped-" t))
         ;; Its socket's directory is short: TMPDIR may not be.
         (run (romacs-test-socket-directory "reap-"))
         (session (romacs-agent--make :id "ssssssss-1111-4000-8000-000000000000" :harness 'claude :state 'detached :dir root :journal-start 0
                                      :host-socket (expand-file-name "gone.sock" run))))
    (unwind-protect
        (progn (write-region "" nil (expand-file-name "journal.jsonl" root))
               (romacs-agent--set-stopped session t)
               (romacs-agent--attach session 0)
               (should (eq (romacs-agent-state session) 'ended))
               ;; An earlier run's journaled exit replays first: still ended.
               (write-region (concat (json-serialize (romacs-mcp--object "seq" 1 "note" (romacs-mcp--object "type" "run"))) "\n"
                                     (json-serialize (romacs-mcp--object "seq" 2 "exit" 0)) "\n")
                             nil (expand-file-name "journal.jsonl" root))
               (setf (romacs-agent-state session) 'detached (romacs-agent-seq session) 0)
               (romacs-agent--attach session 0)
               (should (= (romacs-agent-seq session) 2))
               (should (eq (romacs-agent-state session) 'ended)))
      (delete-directory root t)
      (delete-directory run t))))

(ert-deftest romacs-agent-reap-any-delegated-task-and-on-restore ()
  (romacs-agent-reap-tests--with
    ;; A task some session delegated, whatever its role, or none.
    (should (= (romacs-agent-tools--reap-delay lane) 300))
    (should (= (romacs-agent-tools--reap-delay (romacs-agent--make :id "n" :options '(:parent "lead"))) 300))
    ;; A user's own session: never.
    (should-not (romacs-agent-tools--reap-delay (romacs-agent--make :id "u" :options nil)))
    (let ((romacs-agent-reap-after nil)) (should-not (romacs-agent-tools--reap-delay lane)))
    ;; A restart looks at the tasks already idle.
    (romacs-agent-tools--reap-restored)
    (should (equal (sort (mapcar (lambda (look) (romacs-agent-id (car look))) scheduled) #'string<) '("l" "r")))))

(ert-deftest romacs-agent-reap-a-reaped-task-sleeps-idle-and-resumes-its-conversation ()
  ;; Reaped, its harness gone: idle, not ended, here and after a restart,
  ;; its task still completed; its next message (task_send, or what a
  ;; watch tells it) runs its harness again on its own conversation.
  (let* ((root (make-temp-file "romacs-reaped-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "ssssssss-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir root :cwd root :host-socket (expand-file-name "host.sock" root)
                                      :turns 1 :options '(:parent "lead")))
         (process (make-process :name "romacs-reaped-fixture" :command '("cat") :noquery t))
         (launched nil))
    (unwind-protect
        (cl-letf (((symbol-function 'call-process) (lambda (&rest _) 0))
                  ((symbol-function 'signal-process) #'ignore)
                  ((symbol-function 'run-at-time) #'ignore))
          (puthash (romacs-agent-id session) session romacs-agent--sessions)
          (process-put process 'romacs-agent session)
          (setf (romacs-agent-connection session) process (romacs-agent-unit session) "romacs-agent-s")
          (write-region "" nil (romacs-agent-host-socket session))
          (romacs-agent-stop session t)
          (delete-process process)
          (romacs-agent--sentinel process "killed")
          (should (romacs-agent-reaped-p session))
          (should (eq (romacs-agent-state session) 'idle))
          (should (equal (romacs-agent-tools--status session) "completed"))
          ;; Its supervisor's socket left behind is no harness running unseen.
          (should-not (romacs-agent--harness-unseen-p session))
          ;; A restart reads it asleep too.
          (romacs-agent--write-json (expand-file-name "manifest.json" root)
                                    (romacs-mcp--object "id" "tttttttt-0000-4000-8000-000000000000" "harness" "claude"
                                                        "hostSocket" (expand-file-name "none.sock" root)))
          (write-region "" nil (expand-file-name "journal.jsonl" root))
          (should (eq (romacs-agent-state (romacs-agent--load root)) 'idle))
          ;; Its next message resumes it on its conversation, then goes.
          (cl-letf (((symbol-function 'romacs-agent--launch) (lambda (_s argv &rest _) (push argv launched)))
                    ((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                    ((symbol-function 'romacs-agent--check-companions) #'ignore)
                    ((symbol-function 'romacs-agent--attach) #'ignore))
            (romacs-agent-send session "and a second round"))
          (should (equal (length launched) 1))
          (should (equal (car launched) (romacs-agent--argv session t)))
          (should (member "--resume" (car launched)))
          (should (eq (romacs-agent-state session) 'starting))
          (should (= (romacs-agent-queue-length session) 1)))
      (when (process-live-p process) (delete-process process))
      (delete-directory root t))))

(ert-deftest romacs-agent-reap-a-stopped-session-reads-ended ()
  (let* ((root (make-temp-file "romacs-stopped-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "ssssssss-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir root :cwd root :host-socket (expand-file-name "host.sock" root)))
         (process (make-process :name "romacs-stopped-fixture" :command '("cat") :noquery t)))
    (unwind-protect
        (cl-letf (((symbol-function 'call-process) (lambda (&rest _) 0))
                  ((symbol-function 'signal-process) #'ignore)
                  ((symbol-function 'run-at-time) #'ignore))
          (puthash (romacs-agent-id session) session romacs-agent--sessions)
          (process-put process 'romacs-agent session)
          (setf (romacs-agent-connection session) process (romacs-agent-unit session) "romacs-agent-s")
          (write-region "" nil (romacs-agent-host-socket session))
          (romacs-agent-stop session)
          (delete-process process)
          (romacs-agent--sentinel process "killed")
          ;; Its supervisor journaled no end, yet it ended: not detached,
          ;; nothing looked for again.
          (should (eq (romacs-agent-state session) 'ended))
          (should (= (gethash session romacs-agent--reattaching 0) 0))
          ;; A restart knows.
          (romacs-agent--write-json (expand-file-name "manifest.json" root)
                                    (romacs-mcp--object "id" "tttttttt-0000-4000-8000-000000000000" "harness" "claude"
                                                        "hostSocket" (expand-file-name "none.sock" root)))
          (write-region "" nil (expand-file-name "journal.jsonl" root))
          (should (eq (romacs-agent-state (romacs-agent--load root)) 'ended))
          ;; Launched again: no longer stopped.
          (cl-letf (((symbol-function 'romacs-agent--route-here) #'ignore)
                    ((symbol-function 'romacs-agent--manifest) #'ignore))
            (romacs-agent--launch session '("harness")))
          (should-not (romacs-agent-stopped session))
          (should-not (file-exists-p (expand-file-name "stopped" root))))
      (when (process-live-p process) (delete-process process))
      (delete-directory root t))))

(ert-deftest romacs-agent-reap-works-out-a-tasks-own-processes ()
  ;; A supervisor run detached shares the cgroup it was started in (a
  ;; test's editor inside a lane's unit): its own processes are its
  ;; descendants, never what else runs in that cgroup.  A shell among them
  ;; is work going on; a harness's other processes are not.
  (let* ((idle (make-process :name "romacs-reap-idle" :command '("sleep" "30") :noquery t))
         (busy (make-process :name "romacs-reap-busy" :noquery t
                             :command '("python3" "-c" "import subprocess, time; subprocess.Popen(['sh', '-c', 'sleep 30; :']); time.sleep(30)")))
         (host nil)
         (session (romacs-agent--make :id "w" :state 'idle :unit nil :options '(:parent "lead"))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--host-pid) (lambda (_s) host)))
          (setq host (process-id idle))
          (should-not (romacs-agent-tools--working-p session))
          (setq host (process-id busy))
          ;; Its shell, once started.
          (let ((end (+ (float-time) 10)))
            (while (and (not (romacs-agent-tools--working-p session)) (< (float-time) end)) (sleep-for 0.1)))
          (should (romacs-agent-tools--working-p session))
          ;; A unit that is not the cgroup it runs in: its descendants still.
          (setf (romacs-agent-unit session) "romacs-agent-not-this-one")
          (setq host (process-id idle))
          (should-not (romacs-agent-tools--working-p session)))
      (delete-process idle)
      (delete-process busy))))

(provide 'romacs-agent-reap-tests)
;;; romacs-agent-reap-tests.el ends here

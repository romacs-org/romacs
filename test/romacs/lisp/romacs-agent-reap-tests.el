;;; romacs-agent-reap-tests.el --- A finished task's harness is stopped -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-tools))

(defmacro romacs-agent-reap-tests--with (&rest body)
  "BODY with `review', an idle reviewer task under a unit, and `lane', an
idle task of another role; `scheduled' collects each look's (SESSION
DELAY), `stopped' each session stopped."
  (declare (indent 0))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-tools--reaping (make-hash-table :test #'equal))
          (romacs-agent-reap-after '(("review" . 300)))
          (review (romacs-agent--make :id "r" :harness 'codex :state 'idle :unit "romacs-agent-r"
                                      :dir (make-temp-file "romacs-reap-" t)
                                      :last-time (- (float-time) 1000)
                                      :options '(:parent "lead" :role "review")))
          (lane (romacs-agent--make :id "l" :harness 'claude :state 'idle :unit "romacs-agent-l"
                                    :last-time (- (float-time) 1000)
                                    :options '(:parent "lead" :role "implementation")))
          (scheduled nil) (stopped nil))
     (dolist (s (list review lane)) (puthash (romacs-agent-id s) s romacs-agent--sessions))
     (cl-letf (((symbol-function 'run-at-time)
                (lambda (delay _repeat function session &rest _)
                  (when (eq function #'romacs-agent-tools--reap) (push (list session delay) scheduled))))
               ((symbol-function 'romacs-agent-stop) (lambda (s) (push s stopped))))
       (unwind-protect (progn ,@body) (delete-directory (romacs-agent-dir review) t)))))

(ert-deftest romacs-agent-reap-a-reviewer-after-its-turn ()
  (romacs-agent-reap-tests--with
    ;; Its ends, replayed or live, schedule one look; another role's none.
    (dotimes (_ 3) (romacs-agent-tools--reap-on-event review (list :type 'done)))
    (romacs-agent-tools--reap-on-event lane (list :type 'done))
    (should (equal scheduled (list (list review 300))))
    (romacs-agent-tools--reap review)
    (should (equal stopped (list review)))))

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
      (should-not (reaped-p (lambda () (romacs-agent--enqueue review (list "more" nil nil nil)))))
      (should-not (reaped-p (lambda () (puthash "q" t (romacs-agent-approvals review)))))
      (should-not (reaped-p (lambda () (setf (romacs-agent-cancelled review) t))))
      (should-not (reaped-p (lambda () (setf (romacs-agent-state review) 'ended))))
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
          (romacs-agent--enqueue session (list "held" nil nil nil))
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
         (session (romacs-agent--make :id "ssssssss-1111-4000-8000-000000000000" :harness 'claude :state 'detached :dir root :journal-start 0
                                      :host-socket (expand-file-name "gone.sock" root))))
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
      (delete-directory root t))))

(ert-deftest romacs-agent-reap-by-role-and-on-restore ()
  (romacs-agent-reap-tests--with
    ;; Any role, when t names a time.
    (let ((romacs-agent-reap-after '(("review" . 300) (t . 1800))))
      (should (= (romacs-agent-tools--reap-delay lane) 1800)))
    ;; A user's own session (no role): kept, whatever t says.
    (let ((romacs-agent-reap-after '((t . 60))))
      (should-not (romacs-agent-tools--reap-delay (romacs-agent--make :id "u" :options nil))))
    ;; A restart looks at the tasks already idle.
    (romacs-agent-tools--reap-restored)
    (should (equal (mapcar #'car scheduled) (list review)))))

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

(provide 'romacs-agent-reap-tests)
;;; romacs-agent-reap-tests.el ends here

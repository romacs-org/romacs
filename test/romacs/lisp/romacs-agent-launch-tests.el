;;; romacs-agent-launch-tests.el --- A session whose supervisor never comes up -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-tools)
  (require 'romacs-agent-claude))

(defmacro romacs-agent-launch-tests--with (&rest body)
  "BODY with the idle session `parent' and its task `child', which has a
directory under `root' and no supervisor, its socket's under `run' (short
whatever TMPDIR is); `delivered' collects wakes."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-launch-" t))
          (run (romacs-test-socket-directory "launch-"))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-tools--pending (make-hash-table :test #'equal))
          (romacs-agent-tools--waking (make-hash-table :test #'equal))
          (romacs-agent-tools--answered (make-hash-table :test #'equal))
          (romacs-agent-tools--scheduled (make-hash-table :test #'equal))
          (parent (romacs-agent--make :id "pppppppp-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir (expand-file-name "parent" root)))
          (child (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude :title "janitor"
                                     :dir (expand-file-name "child" root) :cwd root
                                     :host-socket (expand-file-name "host.sock" run)
                                     :options (list :parent "pppppppp-0000-4000-8000-000000000000")))
          (delivered nil))
     (make-directory (romacs-agent-dir child) t)
     (dolist (s (list parent child)) (puthash (romacs-agent-id s) s romacs-agent--sessions))
     (romacs-agent--index child)
     (cl-letf (((symbol-function 'romacs-agent-tools--deliver)
                (lambda (_session text done &rest _) (push text delivered) (funcall done)))
               ((symbol-function 'run-at-time) #'ignore))
       (unwind-protect (progn ,@body) (delete-directory root t) (delete-directory run t)))))

(ert-deftest romacs-agent-launch-a-missing-companion-is-an-error ()
  (let ((invocation-directory (file-name-as-directory (make-temp-file "romacs-bin-" t)))
        (exec-path nil))
    (unwind-protect
        (let ((err (should-error (romacs-agent--check-companions) :type 'user-error)))
          (should (string-match-p "romacs-agent-host is neither beside .* nor on PATH" (cadr err)))
          ;; Checked before anything is made: no half-made session.
          (let ((romacs-agent--sessions (make-hash-table :test #'equal))
                (romacs-agent-providers '(("claude" . nil))))
            (should-error (romacs-agent-start temporary-file-directory "go" "t" "claude") :type 'user-error)
            (should (= (hash-table-count romacs-agent--sessions) 0))))
      (delete-directory invocation-directory t))))

(ert-deftest romacs-agent-launch-never-started-is-a-failed-end ()
  (romacs-agent-launch-tests--with
    ;; The last look for its supervisor, with no record of one.
    (romacs-agent--attach child 0)
    (should (eq (romacs-agent-state child) 'unstarted))
    (let ((task (romacs-agent-tools--task child)))
      (should (equal (gethash "status" task) "failed"))
      (should (equal (gethash "state" task) "unstarted"))
      (should (string-match-p "never started" (gethash "result" task))))
    ;; Its parent is woken with the failure.
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should (equal (plist-get (romacs-agent-tools--wake-parse (car delivered)) :state) "failed"))
    ;; One that ran once is detached, as before.
    (let ((ran (romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000" :harness 'claude
                                   :dir (expand-file-name "ran" root) :host-socket (expand-file-name "x.sock" run))))
      (make-directory (romacs-agent-dir ran))
      (write-region "" nil (expand-file-name "journal.jsonl" (romacs-agent-dir ran)))
      (romacs-agent--attach ran 0)
      (should (eq (romacs-agent-state ran) 'detached)))))

(ert-deftest romacs-agent-launch-a-delegate-that-never-starts-fails-the-call ()
  (romacs-agent-launch-tests--with
    (let ((poll (cadr (romacs-agent-tools--started child (lambda () (romacs-agent-tools--task child))))))
      ;; Starting: no answer yet.
      (setf (romacs-agent-state child) 'starting)
      (should-not (funcall poll))
      (romacs-agent--attach child 0)
      (should-error (funcall poll) :type 'user-error)
      ;; The caller has the failure: no wake besides.
      (romacs-agent-tools--wake child)
      (romacs-agent-tools--flush parent)
      (should-not delivered))))

(ert-deftest romacs-agent-launch-each-failure-wakes-and-says-failed ()
  (romacs-agent-launch-tests--with
    ;; The first failure the delegate's caller had; a retry's fails again.
    (romacs-agent--attach child 0)
    (romacs-agent-tools--answer child (romacs-agent-done-seq child))
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should-not delivered)
    (setf (romacs-agent-state child) 'starting)
    (romacs-agent--attach child 0)
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should (= (length delivered) 1))
    ;; A message waiting for it does not make it running.
    (romacs-agent--enqueue child (list "hello" nil nil nil))
    (should (equal (romacs-agent-tools--status child) "failed"))))

(ert-deftest romacs-agent-launch-waits-for-attach-as-long-as-it-tries ()
  (let ((scheduled nil) (session (romacs-agent--make :id "x" :harness 'claude)))
    (cl-letf (((symbol-function 'run-at-time) (lambda (_time _repeat _fn _s _f tries) (push tries scheduled))))
      (romacs-agent--when-attached session #'ignore))
    (should (equal scheduled (list (1- romacs-agent--launch-tries))))))

(ert-deftest romacs-agent-launch-a-restart-knows-it-never-started ()
  (romacs-agent-launch-tests--with
    (let ((dir (romacs-agent-dir child)))
      (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                (romacs-mcp--object "id" "eeeeeeee-0000-4000-8000-000000000000" "harness" "claude"
                                                    "cwd" root "title" "janitor"
                                                    "hostSocket" (expand-file-name "none.sock" run)))
      (let ((loaded (romacs-agent--load dir)))
        (should (eq (romacs-agent-state loaded) 'unstarted))
        (should (equal (romacs-agent-tools--status loaded) "failed"))))))

(ert-deftest romacs-agent-launch-resume-begins-a-conversation-that-never-was ()
  (romacs-agent-launch-tests--with
    (let ((resumed nil))
      (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (_session resume) (push resume resumed) '("harness")))
                ((symbol-function 'romacs-agent--launch) #'ignore)
                ((symbol-function 'romacs-agent--attach) #'ignore)
                ((symbol-function 'romacs-agent--check-companions) #'ignore))
        (setf (romacs-agent-state child) 'unstarted)
        (romacs-agent-resume child)
        ;; A message reached it once: its conversation is there to resume.
        (setf (romacs-agent-input-seq child) 4)
        (romacs-agent-resume child))
      (should (equal resumed '(4 nil))))))

;; A launch's relay config written again only when it differs: an
;; unchanged one is neither written nor renamed.
(ert-deftest romacs-agent-launch-an-unchanged-config-is-not-written-again ()
  (let* ((root (make-temp-file "romacs-launch-json-" t))
         (file (expand-file-name "mcp.json" root))
         (writes 0)
         (write (symbol-function 'romacs-agent--write-json)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--write-json)
                   (lambda (&rest args) (cl-incf writes) (apply write args))))
          (romacs-agent--keep-json file (romacs-mcp--object "a" 1 "b" "é"))
          (romacs-agent--keep-json file (romacs-mcp--object "a" 1 "b" "é"))
          (should (= writes 1))
          (romacs-agent--keep-json file (romacs-mcp--object "a" 2 "b" "é"))
          (should (= writes 2))
          (should (equal (gethash "a" (romacs-agent--read-json file)) 2)))
      (delete-directory root t))))

(provide 'romacs-agent-launch-tests)
;;; romacs-agent-launch-tests.el ends here

(defvar romacs-server-name)

(ert-deftest romacs-agent-launch-a-relay-knows-where-a-restart-says-it-is-under-way ()
  ;; A named server's sessions' relays get its restart's pending record,
  ;; so a call made during a restart waits for the successor; an unnamed
  ;; one's get none, and a call waits a short while alone.
  (require 'warm-restart)
  (let* ((root (make-temp-file "romacs-relay-" t))
         (warm-restart-directory root)
         (session (romacs-agent--make :id "r" :harness 'claude :dir root)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--route) (lambda (_) "/route")))
          (let ((romacs-server-name "main"))
            (should (equal (member "--restart-pending" (romacs-agent--relay-args session))
                           (list "--restart-pending" (warm-restart-pending-file "main")))))
          (let ((romacs-server-name nil))
            (should-not (member "--restart-pending" (romacs-agent--relay-args session)))
            (should (equal (seq-take (romacs-agent--relay-args session) 5)
                           '("--socket" "/route" "--reconnect" "--session" "r")))))
      (delete-directory root t))))

(ert-deftest romacs-agent-launch-a-start-waits-for-its-records-place ()
  ;; A session whose records' place is not known yet (early in a fresh
  ;; start, the writer still to make the store) launches once it is, never
  ;; before: a supervisor launched before writes a journal the editor no
  ;; longer reads once the session is marked imported.
  (let* ((root (make-temp-file "romacs-start-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent-events--waiting (make-hash-table :test #'equal))
         (romacs-agent-providers (list (cons "claude" root)))
         (launched nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--check-companions) #'ignore)
                  ((symbol-function 'romacs-agent--state-root) (lambda () root))
                  ((symbol-function 'romacs-agent-events-own) (lambda (_) 'queued))
                  ((symbol-function 'romacs-agent-events--migrate-next) #'ignore)
                  ((symbol-function 'romacs-agent--argv) (lambda (&rest _) '("harness")))
                  ((symbol-function 'romacs-agent--launch) (lambda (session &rest _) (push (romacs-agent-id session) launched)))
                  ((symbol-function 'romacs-agent--attach) #'ignore)
                  ((symbol-function 'romacs-agent--when-attached) #'ignore))
          (let ((session (romacs-agent-start root nil nil "claude")))
            (should-not launched)
            (romacs-agent-events--waited (romacs-agent-id session))
            (should (equal launched (list (romacs-agent-id session))))))
      (delete-directory root t))))

(ert-deftest romacs-agent-launch-a-start-that-fails-later-ends-unstarted ()
  ;; Launched later (its records' place known only then), a launch that
  ;; fails ends the session unstarted and failed, as a supervisor that
  ;; never came up: what waits for its start is not left waiting.
  (let* ((root (make-temp-file "romacs-start-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent-events--waiting (make-hash-table :test #'equal))
         (romacs-agent-providers (list (cons "claude" root))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--check-companions) #'ignore)
                  ((symbol-function 'romacs-agent--state-root) (lambda () root))
                  ((symbol-function 'romacs-agent-events-own) (lambda (_) 'queued))
                  ((symbol-function 'romacs-agent-events--migrate-next) #'ignore)
                  ((symbol-function 'romacs-agent--argv) (lambda (&rest _) '("harness")))
                  ((symbol-function 'romacs-agent--launch) (lambda (&rest _) (error "No such directory")))
                  ((symbol-function 'romacs-agent--no-supervisor) #'ignore))
          (let ((session (romacs-agent-start root nil nil "claude")))
            (romacs-agent-events--waited (romacs-agent-id session))
            (should (eq (romacs-agent-state session) 'unstarted))
            (should (romacs-agent-failed session))
            (should (string-match-p "No such directory" (romacs-agent-reply session)))))
      (delete-directory root t))))

(defun romacs-agent-launch-tests--resuming (body)
  "Call BODY with an ended session that can be resumed, with no supervisor,
and three functions: PLACED gives the launch waiting for its records'
place, FIRE calls its launch attempt's last deadline, DUE returns the
timers armed, each (FUNCTION . ARGS)."
  (let* ((root (make-temp-file "romacs-launch-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent--attempts (make-hash-table :test #'eq :weakness 'key))
         (romacs-agent--resuming (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude :title "janitor"
                                      :dir (expand-file-name "child" root) :cwd root :state 'ended :input-seq 4
                                      ;; Never listened at: its launch and attach are the test's.
                                      :host-socket (expand-file-name "host.sock" root)))
         (thens nil) (due nil))
    (make-directory (romacs-agent-dir session) t)
    (puthash (romacs-agent-id session) session romacs-agent--sessions)
    (cl-letf (((symbol-function 'romacs-agent--check-companions) #'ignore)
              ((symbol-function 'romacs-agent--attach) #'ignore)
              ((symbol-function 'romacs-agent-events-then) (lambda (_session then) (push then thens)))
              ((symbol-function 'run-at-time) (lambda (_time _repeat function &rest args) (push (cons function args) due) nil)))
      (unwind-protect
          (funcall body session
                   (lambda () (funcall (pop thens)))
                   (lambda () (let ((entry (cl-find 'romacs-agent--attempt-due due :key #'car)))
                                (apply (car entry) (cdr entry))))
                   (lambda () due))
        (delete-directory root t)))))

(ert-deftest romacs-agent-launch-a-resume-that-fails-or-is-quit-ends-failed ()
  "romacs#271: a resume whose launch fails or is quit, once its records'
place is known (from a sentinel or a timer, which would swallow it) or at
once, ends the session failed, saying why: never `starting' for ever."
  (dolist (raised '((error "argv broke") (quit)))
    (romacs-agent-launch-tests--resuming
     (lambda (session placed _fire _due)
       (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (&rest _) (signal (car raised) (cdr raised)))))
         (romacs-agent-resume session)
         (should (eq (romacs-agent-state session) 'starting))
         (funcall placed)
         (should (eq (romacs-agent-state session) 'unstarted))
         (should (romacs-agent-failed session))
         (should (string-match-p (if (eq (car raised) 'quit) "Quit" "argv broke") (romacs-agent-reply session)))
         (should-not (gethash (romacs-agent-id session) romacs-agent--resuming))
         ;; At once, the caller has it too.
         (cl-letf (((symbol-function 'romacs-agent-events-then) (lambda (_session then) (funcall then))))
           (setf (romacs-agent-state session) 'ended)
           (should (eq (car raised) (car (condition-case err (romacs-agent-resume session) ((error quit) err)))))
           (should (eq (romacs-agent-state session) 'unstarted))))))))

(ert-deftest romacs-agent-launch-a-resume-with-no-run-ends-failed-at-its-deadline ()
  "romacs#271: a launch no run of its harness follows within
`romacs-agent-start-deadline' ends the session failed, saying so; a run
disarms it, a move under way waits on, and a launch come after it is not
made."
  (romacs-agent-launch-tests--resuming
   (lambda (session placed fire due)
     (let ((launched 0))
       (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (&rest _) '("harness")))
                 ((symbol-function 'romacs-agent--launch) (lambda (&rest _) (cl-incf launched))))
         ;; Launched, no run: failed at the deadline.
         (romacs-agent-resume session)
         (funcall placed)
         (should (= launched 1))
         (funcall fire)
         (should (eq (romacs-agent-state session) 'unstarted))
         (should (string-match-p "no run of its harness began" (romacs-agent-reply session)))
         ;; Its new supervisor never attached, the session taken for
         ;; detached by its earlier run's records: failed all the same.
         (romacs-agent-resume session)
         (funcall placed)
         (setf (romacs-agent-state session) 'detached)
         (funcall fire)
         (should (eq (romacs-agent-state session) 'unstarted))
         ;; A run began: the deadline does nothing.
         (romacs-agent-resume session)
         (funcall placed)
         (romacs-agent--raise session (list :type 'run :seq (1+ (romacs-agent-seq session))))
         (setf (romacs-agent-state session) 'starting)
         (funcall fire)
         (should (eq (romacs-agent-state session) 'starting))
         ;; Its records still moving: it waits on; past the deadline, the
         ;; launch is not made.
         (setf (romacs-agent-state session) 'ended)
         (romacs-agent-resume session)
         (let ((armed (cl-count 'romacs-agent--attempt-due (funcall due) :key #'car)))
           (cl-letf (((symbol-function 'romacs-agent-events-waiting-p) (lambda (_) t)))
             (funcall fire)
             (should (eq (romacs-agent-state session) 'starting))
             (should (= (cl-count 'romacs-agent--attempt-due (funcall due) :key #'car) (1+ armed)))))
         (funcall fire)
         (should (eq (romacs-agent-state session) 'unstarted))
         (funcall placed)
         (should (= launched 3)))))))

(ert-deftest romacs-agent-launch-a-renewal-whose-resume-failed-waits-for-the-next ()
  "romacs#271: a renewal whose new run failed to launch is finished by the
next resume, and its session says so."
  (require 'romacs-agent-renew)
  (romacs-agent-launch-tests--resuming
   (lambda (session placed _fire _due)
     (let* ((said nil)
            (romacs-agent-event-functions (cons (lambda (_ event) (when (eq (plist-get event :type) 'error)
                                                                     (push (plist-get event :text) said)))
                                                romacs-agent-event-functions)))
       (setf (romacs-agent-renewing session) t)
       (puthash session t romacs-agent-renew--finishing)
       (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (&rest _) (error "argv broke"))))
         (romacs-agent-resume session)
         (funcall placed))
       (should-not (gethash session romacs-agent-renew--finishing))
       (should (cl-some (lambda (text) (string-match-p "Renewal not finished" text)) said))))))

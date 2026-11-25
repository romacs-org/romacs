;;; romacs-agent-lead-tests.el --- A lead talks to, takes over and finds its tasks -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-claude)
  (require 'romacs-agent-history))
(require 'romacs-agent-test-attached)

(defconst romacs-agent-lead-tests--lead "aaaaaaaa-0000-4000-8000-000000000000")
(defconst romacs-agent-lead-tests--other "bbbbbbbb-0000-4000-8000-000000000000")
(defconst romacs-agent-lead-tests--thread "12345678-1234-4234-8234-123456789abc")

(defmacro romacs-agent-lead-tests--with (&rest body)
  "BODY in a private state, run and Claude Code directory, with the lead
`lead' idle and calling: `caller' names the calling session, `cwd' is a
project, `config' Claude Code's directory, holding the conversation
`romacs-agent-lead-tests--thread' held in `cwd'.  `sent' collects what
reaches a harness, `launched' each launch's (SESSION RESUME), `continued'
each session gone on with, `delivered' what wakes the lead.  Nothing
runs: harnesses are never started."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-lead-" t))
          ;; Its sockets' directory is short: TMPDIR may not be.
          (run (romacs-test-socket-directory "lead-"))
          (cwd (file-name-as-directory (expand-file-name "project" root)))
          (config (expand-file-name "claude" root))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" run))
                                       process-environment))
          (romacs-agent-providers (list (cons "claude" config)))
          (romacs-agent-restored-hook nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent--requests (make-hash-table :test #'equal))
          (romacs-agent-store--db nil)
          (romacs-agent-tools--pending (make-hash-table :test #'equal))
          (romacs-agent-tools--waking (make-hash-table :test #'equal))
          (romacs-agent-tools--answered (make-hash-table :test #'equal))
          (romacs-agent-history--descriptions (make-hash-table :test #'equal))
          (caller romacs-agent-lead-tests--lead)
          (lead (romacs-agent--make :id romacs-agent-lead-tests--lead :harness 'claude :provider "claude"
                                    :state 'idle :dir (expand-file-name "lead" root)))
          (sent nil) (launched nil) (continued nil) (delivered nil))
     (make-directory cwd t)
     (make-directory (expand-file-name "projects/-project" config) t)
     (with-temp-file (expand-file-name (format "projects/-project/%s.jsonl" romacs-agent-lead-tests--thread) config)
       (insert (json-serialize (romacs-mcp--object
                                "type" "user" "cwd" (directory-file-name cwd) "sessionId" romacs-agent-lead-tests--thread
                                "message" (romacs-mcp--object "role" "user" "content" "Build the parser")))
               "\n"))
     (puthash (romacs-agent-id lead) lead romacs-agent--sessions)
     (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () caller))
               ((symbol-function 'romacs-agent--argv) (lambda (_session resume) (list "harness" (if resume "resume" "new"))))
               ((symbol-function 'romacs-agent--launch)
                (lambda (session argv &rest _)
                  (push (list session (equal (cadr argv) "resume")) launched)
                  (romacs-agent--manifest session argv)))
               ((symbol-function 'romacs-agent--attach) #'ignore)
               ((symbol-function 'romacs-agent--continue) (lambda (session) (push session continued) session))
               ((symbol-function 'romacs-agent--input) (lambda (session text &rest _) (push (cons session text) sent)))
               ((symbol-function 'romacs-agent-tools--deliver)
                (lambda (_session text done &rest _) (push text delivered) (funcall done)))
               ((symbol-function 'run-at-time) #'ignore)
               ;; No process starts here: the build's companions are not needed.
               ((symbol-function 'romacs-agent--check-companions) #'ignore)
               ;; Children start at once here (romacs-agent-launch-tests
               ;; has one that never does).
               ((symbol-function 'romacs-agent-tools--started) (lambda (_child then &rest _) (funcall then))))
       (unwind-protect (progn ,@body)
         (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))
         (delete-directory root t)
         (delete-directory run t)))))

(defun romacs-agent-lead-tests--args (&rest pairs)
  "A tool's arguments of PAIRS, as the endpoint parses them."
  (apply #'romacs-mcp--object pairs))

(defun romacs-agent-lead-tests--end (child seq result)
  "CHILD's turn ends at SEQ with RESULT, as its harness's done says, and
what waited for that end goes."
  (setf (romacs-agent-state child) 'idle (romacs-agent-turns child) (1+ (romacs-agent-turns child))
        (romacs-agent-reply child) result (romacs-agent-done-seq child) seq
        (romacs-agent-driver-state child) '(:ready t))
  (push (list seq result nil) (romacs-agent-ends child))
  ;; Its summary told, as its done event tells it.
  (romacs-agent--summary-stale (romacs-agent-id child))
  ;; Said on its attachment.
  (unless (process-live-p (romacs-agent-connection child)) (romacs-agent-test-attach child))
  (romacs-agent-tools--wake child)
  (romacs-agent--settle child))

(defun romacs-agent-lead-tests--child (lead root &optional parent)
  "A task of PARENT's (default LEAD's), running, recorded under ROOT."
  (let ((child (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude :provider "claude"
                                   :title "lane" :state 'running :turns 0 :cwd root
                                   :dir (make-temp-file (expand-file-name "child-" root) t)
                                   :options (list :parent (or parent (romacs-agent-id lead))))))
    (puthash (romacs-agent-id child) child romacs-agent--sessions)
    (romacs-agent--index child)
    (romacs-agent-test-attach child)))

;;; romacs_task_send

(ert-deftest romacs-agent-lead-send-while-busy-queues-and-wakes-once ()
  (romacs-agent-lead-tests--with
    (let* ((child (romacs-agent-lead-tests--child lead root))
           (task (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child)
                                                                           "message" "Lead: also run the tests"))))
      (should (equal (gethash "taskId" task) (romacs-agent-id child)))
      ;; Busy: it waits for the running turn's end, sent nothing yet.
      (should (= (romacs-agent-queue-length child) 1))
      (should-not sent)
      (romacs-agent-lead-tests--end child 5 "first reply")
      (romacs-agent-tools--flush lead)
      ;; The end wakes the lead once, and the message then goes, once.
      (should (= (length delivered) 1))
      (should (equal (mapcar #'cdr sent) '("Lead: also run the tests")))
      (should (= (romacs-agent-queue-length child) 0))
      ;; Its answer's end wakes the lead with it, once.
      (romacs-agent-lead-tests--end child 9 "tests pass")
      (romacs-agent-tools--flush lead)
      (romacs-agent-tools--flush lead)
      (should (= (length delivered) 2))
      (should (equal (plist-get (romacs-agent-tools--wake-parse (car delivered)) :result) "tests pass"))
      (should (= (length sent) 1)))))

(ert-deftest romacs-agent-lead-send-to-an-idle-task-goes-now ()
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root)))
      (setf (romacs-agent-state child) 'idle (romacs-agent-driver-state child) '(:ready t))
      (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child) "message" "go on"))
      ;; Gone on with first: an ended harness resumes.
      (should (equal continued (list child)))
      (should (equal (mapcar #'cdr sent) '("go on"))))))

(ert-deftest romacs-agent-lead-send-only-by-the-parent ()
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root))
          (mine (romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000" :harness 'claude :title "chat"
                                    :state 'idle :dir root)))
      (puthash (romacs-agent-id mine) mine romacs-agent--sessions)
      (setq caller romacs-agent-lead-tests--other)
      (let ((err (should-error (romacs-agent-tools--send
                                (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child) "message" "hi"))
                               :type 'user-error)))
        (should (string-match-p "only the session that delegated it may send it messages" (cadr err))))
      ;; A session nobody delegated is no session's task.
      (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id mine) "message" "hi"))
                    :type 'user-error)
      (should (= (romacs-agent-queue-length child) 0))
      ;; Nor is an empty message, or one to a cancelled task, sent.
      (setq caller romacs-agent-lead-tests--lead)
      (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child) "message" " "))
                    :type 'user-error)
      (setf (romacs-agent-cancelled child) t)
      (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child) "message" "hi"))
                    :type 'user-error))))

;;; romacs_delegate_task with resume

(defun romacs-agent-lead-tests--adopt (cwd &rest more)
  (romacs-agent-tools--delegate
   (apply #'romacs-agent-lead-tests--args "task" "Lead: report your state" "resume" romacs-agent-lead-tests--thread
          "cwd" cwd "target" (romacs-agent-lead-tests--args "provider" "claude") more)))

(ert-deftest romacs-agent-lead-resume-adopts-and-wakes-the-parent ()
  (romacs-agent-lead-tests--with
    (let* ((child (romacs-agent-get (gethash "taskId" (romacs-agent-lead-tests--adopt cwd "wake" "end"))))
           (task (romacs-agent-tools--task (romacs-agent-summary (romacs-agent-id child)))))
      ;; The conversation's own session, its parent's task, resumed in its directory.
      (should (equal (gethash "conversation" task) romacs-agent-lead-tests--thread))
      (should (equal (gethash "parent" task) romacs-agent-lead-tests--lead))
      (should (equal (gethash "title" task) "Build the parser"))
      (should (equal (romacs-agent-cwd child) cwd))
      (should (equal (romacs-agent-config-dir child) config))
      (should (equal launched (list (list child t))))
      ;; Its launch attaches it: nothing attaches it a second time.
      (should-not continued)
      (should (memq child (romacs-agent-children lead)))
      ;; Its next message is the task as given: no brief around it.
      (should (equal (car (car (romacs-agent-queue child))) "Lead: report your state"))
      (romacs-agent-lead-tests--end child 3 "state: green")
      (should (equal (mapcar #'cdr sent) '("Lead: report your state")))
      (romacs-agent-lead-tests--end child 7 "reported")
      (romacs-agent-tools--flush lead)
      (should (= (length delivered) 1))
      (should (equal (mapcar (lambda (w) (plist-get w :result)) (romacs-agent-tools--wakes-parse (car delivered)))
                     '("state: green" "reported"))))))

(ert-deftest romacs-agent-lead-actions-never-repeat-a-result ()
  "What acts on a task answers its short record: a result of 50000
characters is its length there, whole in romacs_task_status alone.  A
wait gives the end it waited for once, in place of its wake."
  (romacs-agent-lead-tests--with
    (let* ((child (romacs-agent-get (gethash "taskId" (romacs-agent-lead-tests--adopt cwd))))
           (big (make-string 50000 ?r))
           (short (lambda (record)
                    (should (equal (gethash "taskId" record) (romacs-agent-id child)))
                    (should-not (gethash "result" record))
                    (should (= (gethash "resultChars" record) 50000))
                    (should (< (length (json-serialize record)) 2000))))
           (wait (lambda () (car (funcall (cadr (romacs-agent-lead-tests--adopt cwd "mode" "wait")))))))
      (romacs-agent-lead-tests--end child 3 big)
      (let ((waited (funcall wait)))
        (should (equal (gethash "status" waited) "completed"))
        (should (string-prefix-p (make-string 1000 ?r) (gethash "result" waited)))
        (should (string-match-p "romacs_task_status" (gethash "result" waited)))
        (should (< (length (gethash "result" waited)) 16500))
        (should (= (gethash "resultChars" waited) 50000)))
      ;; Its wake does not come besides.
      (romacs-agent-tools--flush lead)
      (should-not delivered)
      ;; Asked again, delegated again, sent a message: it had that result.
      (funcall short (funcall wait))
      (funcall short (romacs-agent-lead-tests--adopt cwd))
      (let ((record (romacs-agent-tools--send (romacs-agent-lead-tests--args "taskId" (romacs-agent-id child)
                                                                             "message" "again"))))
        (funcall short record)
        (should (string-prefix-p "m-" (gethash "messageId" record))))
      ;; Ended at its usage limit: the limit's text is that result too.
      (setf (romacs-agent-limited child) (list :provider "claude" :text big))
      (let ((limit (gethash "limit" (romacs-agent-tools--cancel (romacs-agent-lead-tests--args
                                                                  "taskId" (romacs-agent-id child))))))
        (should (equal (gethash "provider" limit) "claude"))
        (should-not (gethash "text" limit)))
      (should (equal (gethash "result" (romacs-agent-tools--task (romacs-agent-summary (romacs-agent-id child)))) big)))))

(ert-deftest romacs-agent-lead-resume-is-idempotent-and-owned ()
  (romacs-agent-lead-tests--with
    (let* ((first (gethash "taskId" (romacs-agent-lead-tests--adopt cwd)))
           (again (gethash "taskId" (romacs-agent-lead-tests--adopt cwd))))
      (should (equal first again))
      ;; Taken over once, told once.
      (should (= (length launched) 1))
      (should (= (romacs-agent-queue-length (romacs-agent-get first)) 1))
      ;; Another session cannot take it.
      (setq caller romacs-agent-lead-tests--other)
      (let ((err (should-error (romacs-agent-lead-tests--adopt cwd) :type 'user-error)))
        (should (string-match-p "only the session that delegated it may take it over" (cadr err)))))))

(ert-deftest romacs-agent-lead-resume-of-a-cancelled-task-says-so ()
  ;; Taken over again once cancelled, by resume or its request: the answer
  ;; says it is cancelled and that nothing was sent, at once, waited for or not.
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-get (gethash "taskId" (romacs-agent-lead-tests--adopt cwd "clientRequestId" "r1")))))
      (setf (romacs-agent-cancelled child) t)
      ;; As a cancel tells its views.
      (romacs-agent--changed child)
      (dolist (record (list (romacs-agent-lead-tests--adopt cwd)
                            (romacs-agent-lead-tests--adopt cwd "mode" "wait")
                            (romacs-agent-lead-tests--adopt cwd "clientRequestId" "r1")))
        (should (equal (gethash "taskId" record) (romacs-agent-id child)))
        (should (equal (gethash "status" record) "cancelled"))
        (should (string-match-p "cancelled: nothing was sent" (gethash "note" record))))
      (should (= (romacs-agent-queue-length child) 1))
      (should (= (length launched) 1)))))

(ert-deftest romacs-agent-lead-resume-refusals ()
  (romacs-agent-lead-tests--with
    ;; Its own directory, required and checked.
    (should-error (romacs-agent-tools--delegate
                   (romacs-agent-lead-tests--args "task" "x" "resume" romacs-agent-lead-tests--thread))
                  :type 'user-error)
    (let ((err (should-error (romacs-agent-lead-tests--adopt root) :type 'user-error)))
      (should (string-match-p "give that directory as cwd" (cadr err))))
    (should-error (romacs-agent-lead-tests--adopt cwd "isolation" "worktree") :type 'user-error)
    ;; A conversation the instance never ran.
    (let ((err (should-error (romacs-agent-tools--delegate
                              (romacs-agent-lead-tests--args "task" "x" "resume" "99999999-1234-4234-8234-123456789abc"
                                                             "cwd" cwd))
                             :type 'user-error)))
      (should (string-match-p "target.provider" (cadr err))))
    (should-not launched)
    (should (= (hash-table-count romacs-agent--sessions) 1))))

(ert-deftest romacs-agent-lead-resume-checks-an-untitled-conversation ()
  ;; A transcript whose prompts name no title still says where it was held.
  (romacs-agent-lead-tests--with
    (with-temp-file (expand-file-name (format "projects/-project/%s.jsonl" romacs-agent-lead-tests--thread) config)
      (insert (json-serialize (romacs-mcp--object
                               "type" "user" "cwd" (directory-file-name cwd)
                               "message" (romacs-mcp--object "role" "user" "content" "<task>Build the parser</task>")))
              "\n"))
    (should-error (romacs-agent-lead-tests--adopt root) :type 'user-error)
    (should-not launched)
    (should (gethash "taskId" (romacs-agent-lead-tests--adopt cwd)))))

(ert-deftest romacs-agent-lead-a-failed-import-leaves-nothing ()
  (romacs-agent-lead-tests--with
    (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (&rest _) (error "No MCP configuration"))))
      (should-error (romacs-agent-lead-tests--adopt cwd "clientRequestId" "r1")))
    (should (= (hash-table-count romacs-agent--sessions) 1))
    (should-not (directory-files (romacs-agent--state-root) nil "\\`[0-9a-f]\\{8\\}-"))
    ;; Not "another Romacs is resuming it": another session adopts it.
    (setq caller romacs-agent-lead-tests--other)
    (should (gethash "taskId" (romacs-agent-lead-tests--adopt cwd)))
    (should (= (length launched) 1))
    ;; The failed request names nothing: its retry is refused, not given another's task.
    (setq caller romacs-agent-lead-tests--lead)
    (should-error (romacs-agent-lead-tests--adopt cwd "clientRequestId" "r1") :type 'user-error)))

(ert-deftest romacs-agent-lead-a-started-supervisor-keeps-its-directory ()
  (romacs-agent-lead-tests--with
    ;; Failing once its supervisor runs: the directory is the supervisor's.
    (cl-letf (((symbol-function 'romacs-agent--launch) (lambda (&rest _) (error "manifest: no space"))))
      (should-error (romacs-agent-lead-tests--adopt cwd)))
    (should (directory-files (romacs-agent--state-root) nil "\\`[0-9a-f]\\{8\\}-"))
    (should (= (hash-table-count romacs-agent--sessions) 2))))

;;; Ends that wake no one

(ert-deftest romacs-agent-lead-empty-and-quiet-ends-wake-no-one ()
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root)))
      ;; A resumed harness's own turn, and a turn with nothing to act on.
      (romacs-agent-lead-tests--end child 3 "")
      (romacs-agent-lead-tests--end child 5 "[quiet] still building, 3 of 5 suites pass")
      (romacs-agent-tools--flush lead)
      (should-not delivered)
      (should-not (romacs-agent-tools--wanted-p child 3))
      (should-not (romacs-agent-tools--wanted-p child 5))
      ;; Still the task's result.
      (should (string-prefix-p "[quiet] still" (gethash "result" (romacs-agent-tools--task (romacs-agent-summary (romacs-agent-id child))))))
      ;; A landing wakes it, and a failure does, empty or not.
      (romacs-agent-lead-tests--end child 7 "landed abc123")
      (setf (romacs-agent-failed child) t)
      (push (list 9 "" t) (romacs-agent-ends child))
      (romacs-agent-tools--wake child)
      (romacs-agent-tools--flush lead)
      (should (equal (mapcar (lambda (w) (list (plist-get w :result) (plist-get w :state)))
                             (romacs-agent-tools--wakes-parse (car delivered)))
                     '(("landed abc123" "completed") ("" "failed")))))))

(ert-deftest romacs-agent-lead-ends-no-one-is-woken-by-are-let-go ()
  ;; Turn after turn that wakes no one, its parent never woken: none of
  ;; them stays among the child's ends, nor does one a wait answered.
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root)))
      (dotimes (i 20) (romacs-agent-lead-tests--end child (+ 3 (* 2 i)) "[quiet] still building"))
      (should-not delivered)
      (should-not (romacs-agent-ends child))
      (romacs-agent-lead-tests--end child 50 "landed abc123")
      (should (equal (mapcar #'car (romacs-agent-ends child)) '(50)))
      (romacs-agent-tools--answer child 50)
      (should-not (romacs-agent-ends child)))))

(ert-deftest romacs-agent-lead-quiet-ends-leave-the-pile-untouched ()
  ;; Behind a parent at work, wanted ends pile up; a quiet one, and a
  ;; replayed batch of them, go without the pile being walked or copied.
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root)))
      (setf (romacs-agent-state lead) 'running)
      (dotimes (i 5) (romacs-agent-lead-tests--end child (+ 3 i) "landed"))
      (let ((pile (romacs-agent-ends child)))
        (romacs-agent-lead-tests--end child 20 "[quiet] still")
        (should (eq (romacs-agent-ends child) pile))
        (push (list 21 "[quiet] a" nil) (romacs-agent-ends child))
        (push (list 22 "landed again" nil) (romacs-agent-ends child))
        (push (list 23 "[quiet] b" nil) (romacs-agent-ends child))
        (romacs-agent-tools--wake child)
        (should (equal (mapcar #'car (romacs-agent-ends child)) '(22 7 6 5 4 3)))
        (should (eq (cdr (romacs-agent-ends child)) pile))
        ;; A wait answering one of the pile takes it out alone.
        (romacs-agent-tools--answer child 5)
        (should (equal (mapcar #'car (romacs-agent-ends child)) '(22 7 6 4 3)))
        ;; One already gone (a quiet end its hand-on let go) changes nothing.
        (romacs-agent-tools--answer child 21)
        (should (equal (mapcar #'car (romacs-agent-ends child)) '(22 7 6 4 3)))
        (romacs-agent-tools--answer child 23)
        (romacs-agent-tools--answer child 3)
        (should (equal (mapcar #'car (romacs-agent-ends child)) '(22 7 6 4)))))))

;;; romacs_task_list

(ert-deftest romacs-agent-lead-list-survives-a-restart ()
  (romacs-agent-lead-tests--with
    (let ((adopted (gethash "taskId" (romacs-agent-lead-tests--adopt cwd)))
          (theirs (romacs-agent-start cwd "other work" "theirs" "claude" (list :parent romacs-agent-lead-tests--other))))
      ;; A new server knows only what is on disk.
      (setq romacs-agent--sessions (make-hash-table :test #'equal)
            romacs-agent--children (make-hash-table :test #'equal)
            romacs-agent--requests (make-hash-table :test #'equal))
      (cl-letf (((symbol-function 'romacs-agent--sweep) #'ignore))
        (romacs-agent-restore))
      (let ((mine (gethash "tasks" (romacs-agent-tools--list (romacs-agent-lead-tests--args))))
            (all (gethash "tasks" (romacs-agent-tools--list (romacs-agent-lead-tests--args "all" t)))))
        (should (equal (mapcar (lambda (row) (gethash "taskId" row)) mine) (list adopted)))
        (should (equal (sort (mapcar (lambda (row) (gethash "taskId" row)) all) #'string<)
                       (sort (list adopted (romacs-agent-id theirs)) #'string<)))
        (should (equal (gethash "conversation" (aref mine 0)) romacs-agent-lead-tests--thread))
        ;; A page at a time: the rest by its next offset.
        (let ((first (romacs-agent-tools--list (romacs-agent-lead-tests--args "all" t "limit" 1))))
          (should (= (length (gethash "tasks" first)) 1))
          (should (= (gethash "total" first) 2))
          (should (= (gethash "nextOffset" first) 1))
          (let ((rest (romacs-agent-tools--list (romacs-agent-lead-tests--args "all" t "limit" 1 "offset" 1))))
            (should (eq (gethash "nextOffset" rest) :null))
            (should-not (equal (gethash "taskId" (aref (gethash "tasks" rest) 0))
                               (gethash "taskId" (aref (gethash "tasks" first) 0))))))
        (should (equal (gethash "cwd" (aref mine 0)) cwd))
        ;; Its tasks still wake it, and are still its own to talk to.
        (should (equal (plist-get (romacs-agent-options (romacs-agent-get adopted)) :parent)
                       romacs-agent-lead-tests--lead))))))

(ert-deftest romacs-agent-lead-list-rows-carry-a-result-head ()
  (romacs-agent-lead-tests--with
    (let ((child (romacs-agent-lead-tests--child lead root)))
      (setf (romacs-agent-reply child) (make-string 1000 ?r) (romacs-agent-state child) 'idle
            (romacs-agent-turns child) 1)
      (let ((row (aref (gethash "tasks" (romacs-agent-tools--list (romacs-agent-lead-tests--args))) 0)))
        (should (= (gethash "resultChars" row) 1000))
        (should (<= (length (gethash "result" row)) 301))
        (should-not (gethash "next" row))))))

(defun romacs-agent-lead-tests--stored-task (id parent)
  "Register and store an idle session with ID and PARENT in the store."
  (let ((task (romacs-agent--make :id id :harness 'claude :provider "claude"
                                  :state 'idle :dir (expand-file-name id (romacs-agent--state-root))
                                  :options (list :parent parent))))
    (make-directory (romacs-agent-dir task) t)
    (puthash id task romacs-agent--sessions)
    (romacs-agent-store-put task)
    (should (romacs-agent-store-row-session id))
    task))

(ert-deftest romacs-agent-lead-short-prefixes-are-refused ()
  (romacs-agent-lead-tests--with
    (let* ((caller-id (romacs-agent-tools--caller))
           (task (romacs-agent-lead-tests--stored-task "abcdef01-0000-4000-8000-000000000000" caller-id)))
      (dolist (prefix '("a" "ab" "abc" "abcdef0"))
        (let* ((err (should-error (romacs-agent-tools--known prefix) :type 'user-error))
               (message (cadr err)))
          (should (string-prefix-p "No task " message))
          (should (string-search "romacs_task_list" message))))
      (should (eq task (romacs-agent-tools--known "abcdef01")))
      (let ((exact (romacs-agent-lead-tests--stored-task "a" caller-id)))
        (should (eq exact (romacs-agent-tools--known "a"))))
      (should (eq (romacs-agent-get caller-id) (romacs-agent-tools--known caller-id))))))

(ert-deftest romacs-agent-lead-task-prefix-obeys-list-visibility ()
  (romacs-agent-lead-tests--with
    (let* ((caller-id (romacs-agent-tools--caller))
           (prefix "12345678")
           (hidden (concat prefix "-0000-4000-8000-000000000000"))
           (first (concat prefix "-0000-4000-8000-000000000001"))
           (second (concat prefix "-0000-4000-8000-000000000002")))
      (romacs-agent-lead-tests--stored-task hidden nil)
      (let* ((all (gethash "tasks" (romacs-agent-tools--list (romacs-agent-lead-tests--args "all" t))))
             (err (should-error (romacs-agent-tools--known prefix) :type 'user-error)))
        (should (= (length all) 0))
        (should (string-prefix-p "No task " (cadr err)))
        (should-not (string-search hidden (cadr err))))
      (remhash hidden romacs-agent--sessions)
      (let ((task (romacs-agent-lead-tests--stored-task first caller-id)))
        (should (eq task (romacs-agent-tools--known prefix))))
      (romacs-agent-lead-tests--stored-task second caller-id)
      (let ((message (cadr (should-error (romacs-agent-tools--known prefix) :type 'user-error))))
        (should (string-search first message))
        (should (string-search second message))
        (should-not (string-search hidden message))))))

(ert-deftest romacs-agent-lead-cancel-requires-ownership-after-lookup ()
  (romacs-agent-lead-tests--with
    (let ((task (romacs-agent-lead-tests--stored-task
                 "abcdef01-0000-4000-8000-000000000000" romacs-agent-lead-tests--other)))
      (cl-letf (((symbol-function 'romacs-agent-stop) #'ignore))
        (dolist (id (list "abcdef01" (romacs-agent-id task)))
          (let ((err (should-error (romacs-agent-tools--cancel (romacs-agent-lead-tests--args "taskId" id))
                                   :type 'user-error)))
            (should (string-search romacs-agent-lead-tests--other (cadr err)))))
        (should-not (romacs-agent-cancelled task))
        (should-not (file-exists-p (expand-file-name "cancelled" (romacs-agent-dir task))))
        (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () romacs-agent-lead-tests--other)))
          (romacs-agent-tools--cancel (romacs-agent-lead-tests--args "taskId" "abcdef01")))
        (should (romacs-agent-cancelled task))))))

(ert-deftest romacs-agent-lead-owner-refusal-names-a-working-route ()
  (romacs-agent-lead-tests--with
    (let* ((sender (romacs-agent-get (romacs-agent-tools--caller)))
           (owner (romacs-agent-lead-tests--stored-task romacs-agent-lead-tests--other nil))
           (task (romacs-agent-lead-tests--stored-task
                  "abcdef01-0000-4000-8000-000000000000" (romacs-agent-id owner)))
           (args (romacs-agent-lead-tests--args "taskId" (romacs-agent-id task) "message" "hi"))
           (refusal (lambda ()
                      (cadr (should-error (romacs-agent-tools--send args) :type 'user-error)))))
      (let ((message (funcall refusal)))
        (should (string-search (romacs-agent-id owner) message))
        (should (string-search "ask the user" message))
        (should-not (string-search "romacs_task_send" message))
        (should-not (string-search "romacs_task_reply" message)))
      (romacs-agent--set-options sender (list :parent (romacs-agent-id owner)))
      (let ((message-id "m-abcdef01-0000-4000-8000-000000000000"))
        (romacs-agent--received sender (list (romacs-agent-make-message
                                           message-id "parent" (list :session (romacs-agent-id owner)))))
        (let ((message
               (cadr (should-error (romacs-agent-tools--cancel (romacs-agent-lead-tests--args
                                                                "taskId" (romacs-agent-id task)))
                                   :type 'user-error))))
          (should (string-search "romacs_task_reply" message))
          (should (string-search message-id message))))
      (romacs-agent--set-options sender nil)
      (romacs-agent--set-options owner (list :parent (romacs-agent-id sender)))
      (should (string-search "romacs_task_send" (funcall refusal)))
      (setf (romacs-agent-cancelled owner) t)
      ;; As a cancel tells its views.
      (romacs-agent--changed owner)
      (should (string-search "ask the user" (funcall refusal))))))

(ert-deftest romacs-agent-lead-reply-suggests-at-most-eight-ids ()
  (romacs-agent-lead-tests--with
    (let* ((receiver (romacs-agent-get (romacs-agent-tools--caller)))
           (ids (cl-loop for i below 200
                         collect (format "m-%08x-0000-4000-8000-000000000000" i)))
           (messages (mapcar (lambda (id) (romacs-agent-make-message id "child" '(:session "sender"))) ids))
           (prefix "m-0000000a"))
      (romacs-agent--received receiver messages)
      (should (= (length (romacs-agent-received receiver)) 200))
      (let* ((err (should-error (romacs-agent-tools--reply (romacs-agent-lead-tests--args
                                                           "messageId" prefix "text" "answer"))
                                :type 'user-error))
             (named (seq-filter (lambda (id) (string-search id (cadr err))) ids))
             (matching (nth 10 ids))
             (newest (car (last ids))))
        (should (equal named (cons matching (last ids 7))))
        (should (< (string-search matching (cadr err)) (string-search newest (cadr err))))))))

(ert-deftest romacs-agent-lead-owner-route-refuses-a-missing-provider-directory ()
  (let ((continue (symbol-function 'romacs-agent--continue)))
    (romacs-agent-lead-tests--with
      (let* ((caller-id (romacs-agent-tools--caller))
             (owner (romacs-agent-lead-tests--stored-task romacs-agent-lead-tests--other caller-id))
             (task (romacs-agent-lead-tests--stored-task
                    "abcdef01-0000-4000-8000-000000000000" (romacs-agent-id owner)))
             (provider-dir (make-temp-file (expand-file-name "provider-" (romacs-agent--state-root)) t)))
        (setf (romacs-agent-state owner) 'detached
              (romacs-agent-config-dir owner) provider-dir
              (romacs-agent-host-socket owner) (romacs-agent--host-socket (romacs-agent-id owner)))
        (delete-directory provider-dir)
        (cl-letf (((symbol-function 'romacs-agent--continue) continue))
          (let ((err (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args
                                                              "taskId" (romacs-agent-id owner) "message" "hi"))
                                   :type 'user-error)))
            (should (string-search "configuration directory" (cadr err)))
            (should (string-search provider-dir (cadr err))))
          (let* ((err (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args
                                                              "taskId" (romacs-agent-id task) "message" "hi"))
                                    :type 'user-error))
                 (message (cadr err)))
            (should (string-search (romacs-agent-id owner) message))
            (should-not (string-search "romacs_task_send" message))
            (should (string-search "ask the user" message))))))))

(ert-deftest romacs-agent-lead-owner-route-names-why-its-owner-cannot-take-a-message ()
  ;; Its owner asked whether it can take a message: the refusal says why not.
  (let ((continue (symbol-function 'romacs-agent--continue)))
    (romacs-agent-lead-tests--with
      (let* ((caller-id (romacs-agent-tools--caller))
             (owner (romacs-agent-lead-tests--stored-task romacs-agent-lead-tests--other caller-id))
             (task (romacs-agent-lead-tests--stored-task
                    "abcdef01-0000-4000-8000-000000000000" (romacs-agent-id owner)))
             (provider-dir (make-temp-file (expand-file-name "provider-" (romacs-agent--state-root)) t)))
        (setf (romacs-agent-state owner) 'detached
              (romacs-agent-config-dir owner) provider-dir
              (romacs-agent-host-socket owner) (romacs-agent--host-socket (romacs-agent-id owner)))
        (delete-directory provider-dir)
        (cl-letf (((symbol-function 'romacs-agent--continue) continue))
          (let ((message (cadr (should-error (romacs-agent-tools--send (romacs-agent-lead-tests--args
                                                                       "taskId" (romacs-agent-id task) "message" "hi"))
                                             :type 'user-error))))
            (should (string-search "It cannot take a message now" message))
            (should (string-search provider-dir message))
            (should (string-search "ask the user" message))))))))

(ert-deftest romacs-agent-lead-root-send-only-suggests-an-answer-to-that-session ()
  (romacs-agent-lead-tests--with
    (let* ((sender (romacs-agent-get (romacs-agent-tools--caller)))
           (target (romacs-agent-lead-tests--child sender (file-name-directory (romacs-agent-dir sender))))
           (target-id (romacs-agent-id target))
           (other "m-aaaaaaaa-0000-4000-8000-000000000001")
           (received "m-aaaaaaaa-0000-4000-8000-000000000002")
           (args (romacs-agent-lead-tests--args "taskId" target-id "message" "hi"))
           (refusal (lambda () (cadr (should-error (romacs-agent-tools--send args) :type 'user-error)))))
      (setf (romacs-agent-options target) nil
            (romacs-agent-state target) 'idle
            (romacs-agent-driver-state target) '(:ready t))
      (unwind-protect
          (progn
            (romacs-agent--received sender (list (romacs-agent-make-message other "external" '(:session "stranger"))))
            (let ((message (funcall refusal)))
              (should (string-search target-id message))
              (should (string-search "ask the user" message))
              (should-not (string-search "romacs_task_reply" message))
              (should-not (string-search other message)))
            (romacs-agent--received sender
                                    (list (romacs-agent-make-message received "external" (list :session target-id))))
            (let ((message (funcall refusal))
                  (reply (romacs-agent-tools--reply (romacs-agent-lead-tests--args "messageId" received "text" "answer"))))
              (should (string-search "romacs_task_reply" message))
              (should (string-search received message))
              (should-not (string-search other message))
              (should (equal target-id (gethash "taskId" reply)))))
        (delete-process (romacs-agent-connection target))))))

(provide 'romacs-agent-lead-tests)
;;; romacs-agent-lead-tests.el ends here

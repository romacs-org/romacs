;;; romacs-agent-lead-tests.el --- A lead talks to, takes over and finds its tasks -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-tools)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-history))

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
          (cwd (file-name-as-directory (expand-file-name "project" root)))
          (config (expand-file-name "claude" root))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" root "/run"))
                                       process-environment))
          (romacs-agent-providers (list (cons "claude" config)))
          (romacs-agent-restored-hook nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent--requests (make-hash-table :test #'equal))
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
                (lambda (_session text done &optional _valid) (push text delivered) (funcall done)))
               ((symbol-function 'run-at-time) #'ignore)
               ;; No process starts here: the build's companions are not needed.
               ((symbol-function 'romacs-agent--check-companions) #'ignore)
               ;; Children start at once here (romacs-agent-launch-tests
               ;; has one that never does).
               ((symbol-function 'romacs-agent-tools--started) (lambda (_child then) (funcall then))))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

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
    child))

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
    (let* ((task (romacs-agent-lead-tests--adopt cwd "wake" "end"))
           (child (romacs-agent-get (gethash "taskId" task))))
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
      (should (string-prefix-p "[quiet] still" (gethash "result" (romacs-agent-tools--task child))))
      ;; A landing wakes it, and a failure does, empty or not.
      (romacs-agent-lead-tests--end child 7 "landed abc123")
      (setf (romacs-agent-failed child) t)
      (push (list 9 "" t) (romacs-agent-ends child))
      (romacs-agent-tools--wake child)
      (romacs-agent-tools--flush lead)
      (should (equal (mapcar (lambda (w) (list (plist-get w :result) (plist-get w :state)))
                             (romacs-agent-tools--wakes-parse (car delivered)))
                     '(("landed abc123" "completed") ("" "failed")))))))

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

(provide 'romacs-agent-lead-tests)
;;; romacs-agent-lead-tests.el ends here

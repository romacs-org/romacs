;;; romacs-agent-issues-tests.el --- Orchestration problems, logged -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-issues))

(defmacro romacs-agent-issues-tests--with (&rest body)
  "BODY with a private state root, no sessions, and `task', a live
delegated review task of a registered parent."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-issues-" t))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" root "/run"))
                                       process-environment))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (task (romacs-agent--make :id "tttttttt-0000-4000-8000-000000000000" :harness 'claude :title "review it"
                                    :provider "claude" :state 'running :caught-up t
                                    :options (list :parent "pppppppp-0000-4000-8000-000000000000" :role "review"))))
     (puthash (romacs-agent-id task) task romacs-agent--sessions)
     (unwind-protect (progn ,@body) (delete-directory root t))))

(defun romacs-agent-issues-tests--kinds ()
  (mapcar (lambda (issue) (gethash "kind" issue)) (romacs-agent-issues-read)))

(defun romacs-agent-issues-tests--event (session &rest event)
  (run-hook-with-args 'romacs-agent-event-functions session event))

(ert-deftest romacs-agent-issues-record-writes-a-line ()
  (romacs-agent-issues-tests--with
    (romacs-agent-issues-record 'crossed task :queued 2 :result (make-string 1000 ?x) :absent nil)
    (let ((issue (car (romacs-agent-issues-read))))
      (should (equal (gethash "kind" issue) "crossed"))
      (should (equal (gethash "session" issue) (romacs-agent-id task)))
      (should (equal (gethash "title" issue) "review it"))
      (should (numberp (gethash "t" issue)))
      (let ((details (gethash "details" issue)))
        (should (= (gethash "queued" details) 2))
        (should (<= (length (gethash "result" details)) 300))
        (should (eq (gethash "absent" details) :null))))
    ;; One JSON line each, appended.
    (romacs-agent-issues-record 'moved nil :to "claude2")
    (should (equal (romacs-agent-issues-tests--kinds) '("crossed" "moved")))
    (should (= 2 (with-temp-buffer (insert-file-contents (romacs-agent-issues--file)) (count-lines (point-min) (point-max)))))))

(ert-deftest romacs-agent-issues-record-never-signals ()
  (romacs-agent-issues-tests--with
    (cl-letf (((symbol-function 'write-region) (lambda (&rest _) (error "Disk full"))))
      (should-not (condition-case nil (progn (romacs-agent-issues-record 'crossed task) nil) (error t))))))

(defun romacs-agent-issues-tests--turn (session seq text &rest done)
  "SESSION's turn, begun by a message, ending at SEQ with TEXT."
  (romacs-agent-issues-tests--event session :type 'input :text "do it" :seq (1- seq))
  (apply #'romacs-agent-issues-tests--event session :type 'done :seq seq :text text done))

(ert-deftest romacs-agent-issues-from-a-tasks-turn-ends ()
  (romacs-agent-issues-tests--with
    ;; A review with its verdict: nothing.
    (romacs-agent-issues-tests--turn task 3 "VERDICT: APPROVE\nNo findings.")
    (should-not (romacs-agent-issues-read))
    ;; One without, made while its parent's message waited, then one empty.
    (romacs-agent-issues-told task)
    (romacs-agent-issues-tests--turn task 5 "a memory consolidation summary")
    (romacs-agent-issues-tests--turn task 7 "  ")
    ;; At its limit, then failed.
    (romacs-agent-issues-tests--turn task 9 "limit reached" :limit 'account)
    (romacs-agent-issues-tests--turn task 11 "x" :error t :reason "API error")
    (should (equal (romacs-agent-issues-tests--kinds) '("crossed" "result-shape" "no-result" "limit" "failed")))
    ;; No result's text is kept, only its size.
    (should-not (string-search "consolidation" (with-temp-buffer (insert-file-contents (romacs-agent-issues--file))
                                                                 (buffer-string))))
    (should (= (gethash "chars" (gethash "details" (nth 1 (romacs-agent-issues-read)))) 30))))

(ert-deftest romacs-agent-issues-only-answers-are-judged ()
  ;; A resumed harness's own turn (no message began it) and a quiet
  ;; acknowledgement after a verdict are no task answers.
  (romacs-agent-issues-tests--with
    (romacs-agent-issues-tests--event task :type 'done :seq 3 :text "")
    (romacs-agent-issues-tests--turn task 5 "[quiet] Acknowledged.")
    ;; A message its parent sent once the turn ended waits for nothing.
    (setf (romacs-agent-queue-length task) 1)
    (romacs-agent-issues-tests--turn task 7 "VERDICT: CHANGES\nS1.")
    (should-not (romacs-agent-issues-read))))

(ert-deftest romacs-agent-issues-a-replayed-question-judges-its-live-answer ()
  ;; Restored mid-turn: the question is replayed, its answer comes live.
  (romacs-agent-issues-tests--with
    (setf (romacs-agent-caught-up task) nil)
    (romacs-agent-issues-tests--event task :type 'input :text "review it" :seq 2)
    ;; An earlier turn replayed whole logs nothing, and ends its question.
    (romacs-agent-issues-tests--event task :type 'done :seq 1 :text "")
    (romacs-agent-issues-tests--event task :type 'input :text "review it again" :seq 4)
    (setf (romacs-agent-caught-up task) t)
    (romacs-agent-issues-tests--event task :type 'done :seq 5 :text "Looks fine.")
    (should (equal (romacs-agent-issues-tests--kinds) '("result-shape")))))

(ert-deftest romacs-agent-issues-a-harness-exit-mid-turn ()
  (romacs-agent-issues-tests--with
    ;; Idle when it exits: no turn was cut.
    (romacs-agent-issues-tests--event task :type 'exit :seq 13 :status 0)
    (should-not (romacs-agent-issues-read))
    ;; Its exit ended a turn, failed.
    (setf (romacs-agent-failed task) t (romacs-agent-done-seq task) 15)
    (romacs-agent-issues-tests--event task :type 'exit :seq 15 :status 1)
    (should (equal (romacs-agent-issues-tests--kinds) '("no-result")))
    (should (equal (gethash "how" (gethash "details" (car (romacs-agent-issues-read)))) "exit"))))

(ert-deftest romacs-agent-issues-moves-and-failed-starts ()
  (romacs-agent-issues-tests--with
    (romacs-agent-issues-tests--event task :type 'selection :from '(:provider "claude" :model "m")
                                      :to '(:provider "claude" :model "n"))
    (should-not (romacs-agent-issues-read))
    (romacs-agent-issues-tests--event task :type 'selection :from '(:provider "claude") :to '(:provider "claude2"))
    ;; A launch that never started never caught up: still told.
    (setf (romacs-agent-caught-up task) nil)
    (romacs-agent-issues-tests--event task :type 'unstarted :text "its supervisor never started")
    (should (equal (romacs-agent-issues-tests--kinds) '("moved" "resume-failed")))))

(ert-deftest romacs-agent-issues-never-from-a-replay ()
  ;; A replay, or a copy drawn again, logs nothing: it happened once.
  (romacs-agent-issues-tests--with
    (setf (romacs-agent-caught-up task) nil)
    (romacs-agent-issues-tests--event task :type 'done :seq 3 :text "")
    (let ((copy (romacs-agent--make :id (romacs-agent-id task) :caught-up t :options (romacs-agent-options task))))
      (romacs-agent-issues-tests--event copy :type 'done :seq 3 :text ""))
    (should-not (romacs-agent-issues-read))))

(ert-deftest romacs-agent-issues-from-tool-calls ()
  ;; An agents' tool call refused, or broken; arguments its schema does
  ;; not take; an unknown one.  Another tool's failure is not ours.
  (romacs-agent-issues-tests--with
    (let ((romacs-mcp-tools nil) (romacs-mcp--active (list :peer nil :message (romacs-mcp--object "id" 1))))
      (romacs-mcp-register-tool "romacs_task_probe" "probe" (romacs-mcp--schema '(("taskId" . "string")) nil)
                                (lambda (arguments)
                                  (if (gethash "taskId" arguments) (user-error "No task %s" (gethash "taskId" arguments))
                                    (error "Broken"))))
      (romacs-mcp-register-tool "romacs_eval" "eval" (romacs-mcp--schema nil nil) (lambda (_) (error "Not ours")))
      (romacs-mcp--call (romacs-mcp--object "name" "romacs_task_probe" "arguments" (romacs-mcp--object "taskId" "x")) nil)
      (romacs-mcp--call (romacs-mcp--object "name" "romacs_task_probe" "arguments" (romacs-mcp--object)) nil)
      (should-error (romacs-mcp--call (romacs-mcp--object "name" "romacs_task_probe"
                                                          "arguments" (romacs-mcp--object "taskId" 3))
                                      nil))
      (should-error (romacs-mcp--call (romacs-mcp--object "name" "romacs_task_gone" "arguments" (romacs-mcp--object)) nil))
      (romacs-mcp--call (romacs-mcp--object "name" "romacs_eval" "arguments" (romacs-mcp--object)) nil))
    ;; A deferred call failing as it is polled.
    (let ((romacs-mcp--deferred (list (list (list :peer nil :message (romacs-mcp--object
                                                                        "params" (romacs-mcp--object
                                                                                  "name" "romacs_delegate_task"
                                                                                  "arguments" (romacs-mcp--object
                                                                                               "task" "secret brief"))))
                                            7 (lambda () (user-error "The task did not start")) nil)))
          (romacs-mcp--deferred-timer nil))
      (cl-letf (((symbol-function 'romacs-mcp--live-p) (lambda (&rest _) t))
                ((symbol-function 'romacs-mcp--send) #'ignore))
        (romacs-mcp--poll-deferred)))
    (let ((issues (romacs-agent-issues-read)))
      (should (equal (mapcar (lambda (i) (gethash "kind" i)) issues)
                     '("tool-refused" "tool-error" "tool-refused" "tool-refused" "tool-refused")))
      (should (equal (gethash "error" (gethash "details" (nth 4 issues))) "The task did not start"))
      ;; What a caller wrote is never kept: its size only.
      (should (equal (gethash "task" (gethash "arguments" (gethash "details" (nth 4 issues)))) "(12 characters)"))
      (should (equal (gethash "error" (gethash "details" (car issues))) "No task x"))
      (should (equal (gethash "taskId" (gethash "arguments" (gethash "details" (car issues)))) "x"))
      (should (equal (gethash "tool" (gethash "details" (nth 3 issues))) "romacs_task_gone")))))

(ert-deftest romacs-agent-issues-summary-counts-by-kind ()
  (romacs-agent-issues-tests--with
    (dotimes (i 5) (romacs-agent-issues-record 'tool-refused task :n i))
    (romacs-agent-issues-record 'crossed task)
    (let ((summary (romacs-agent-issues-summary nil nil 2)))
      (should (= (gethash "total" summary) 6))
      ;; Most first, each with its latest first.
      (should (equal (mapcar (lambda (k) (list (gethash "kind" k) (gethash "count" k))) (gethash "kinds" summary))
                     '(("tool-refused" 5) ("crossed" 1))))
      (should (equal (mapcar (lambda (i) (gethash "n" (gethash "details" i)))
                             (gethash "recent" (aref (gethash "kinds" summary) 0)))
                     '(4 3))))
    (should (= (gethash "total" (romacs-agent-issues-summary nil "crossed")) 1))
    (should (= (gethash "total" (romacs-agent-issues-summary (+ (float-time) 10))) 0))
    ;; The tool: hours and kind.
    (should (= (gethash "total" (romacs-agent-issues--tool (romacs-mcp--object "hours" 1))) 6))
    (should-error (romacs-agent-issues--tool (romacs-mcp--object "hours" -1)) :type 'user-error)))

;;; romacs-agent-issues-tests.el ends here

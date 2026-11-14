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

(ert-deftest romacs-agent-issues-from-a-tasks-turn-ends ()
  (romacs-agent-issues-tests--with
    ;; A review with its verdict: nothing.
    (romacs-agent-issues-tests--event task :type 'done :seq 3 :text "VERDICT: APPROVE\nNo findings.")
    (should-not (romacs-agent-issues-read))
    ;; One without, made while its parent's message waited, then one empty.
    (setf (romacs-agent-queue-length task) 1)
    (romacs-agent-issues-tests--event task :type 'done :seq 5 :text "a memory consolidation summary")
    (setf (romacs-agent-queue-length task) 0)
    (romacs-agent-issues-tests--event task :type 'done :seq 7 :text "  ")
    ;; At its limit, then failed, then cut short by its harness's exit.
    (romacs-agent-issues-tests--event task :type 'done :seq 9 :text "limit reached" :limit 'account)
    (romacs-agent-issues-tests--event task :type 'done :seq 11 :error t :reason "API error")
    (romacs-agent-issues-tests--event task :type 'exit :seq 13 :status 1)
    (should (equal (romacs-agent-issues-tests--kinds)
                   '("crossed" "result-shape" "no-result" "limit" "failed" "no-result")))
    (should (equal (gethash "how" (gethash "details" (car (last (romacs-agent-issues-read))))) "exit"))
    ;; Idle when its harness exits: no turn was cut.
    (setf (romacs-agent-state task) 'idle)
    (romacs-agent-issues-tests--event task :type 'exit :seq 15 :status 0)
    (should (= (length (romacs-agent-issues-read)) 6))))

(ert-deftest romacs-agent-issues-moves-and-failed-starts ()
  (romacs-agent-issues-tests--with
    (romacs-agent-issues-tests--event task :type 'selection :from '(:provider "claude" :model "m")
                                      :to '(:provider "claude" :model "n"))
    (should-not (romacs-agent-issues-read))
    (romacs-agent-issues-tests--event task :type 'selection :from '(:provider "claude") :to '(:provider "claude2"))
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
    (let ((issues (romacs-agent-issues-read)))
      (should (equal (mapcar (lambda (i) (gethash "kind" i)) issues)
                     '("tool-refused" "tool-error" "tool-refused" "tool-refused")))
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

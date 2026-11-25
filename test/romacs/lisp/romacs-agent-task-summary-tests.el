;;; romacs-agent-task-summary-tests.el --- What MCP answers of a task from its summary -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The MCP tools read another session from its summary
;; (`romacs-agent-summary').  A plan it offers waits on its user as an
;; approval does; a task named by id is asked of its owner, loaded, so
;; its answer is whole; its reply rides in its summary without a copy.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'json)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-tools)
  (require 'romacs-agent-summary)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex))

(defconst romacs-agent-task-summary-tests--lead "1ead0000-0000-4000-8000-000000000000")
(defconst romacs-agent-task-summary-tests--task "7a5c0000-0000-4000-8000-000000000001")

(defun romacs-agent-task-summary-tests--session (id &rest fields)
  "Register and index session ID, with FIELDS, idle in its own directory."
  (let ((session (apply #'romacs-agent--make
                        (append fields (list :id id :harness 'codex :provider "codex" :title id :state 'idle
                                             :cwd "/var/tmp/" :dir (expand-file-name id (romacs-agent--state-root)))))))
    (make-directory (romacs-agent-dir session) t)
    (puthash id session romacs-agent--sessions)
    (romacs-agent--index session)
    session))

(defmacro romacs-agent-task-summary-tests--with (&rest body)
  "BODY with a private store and sessions, the lead calling."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent--children (make-hash-table :test #'equal))
           (romacs-agent-tools--pending (make-hash-table :test #'equal))
           (romacs-agent-tools--answered (make-hash-table :test #'equal))
           (romacs-agent-event-functions nil)
           (romacs-mcp--active nil)
           (romacs-mcp-failure-functions nil)
           (romacs-mcp-full-access t))
       (should (romacs-agent-store--writer-open))
       (romacs-agent-task-summary-tests--session romacs-agent-task-summary-tests--lead)
       (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () romacs-agent-task-summary-tests--lead))
                 ((symbol-function 'romacs-agent--keep-models) #'ignore)
                 ((symbol-function 'romacs-agent--note) #'ignore))
         ,@body))))

(defun romacs-agent-task-summary-tests--call (name &rest arguments)
  "MCP tool NAME's answer to ARGUMENTS, decoded."
  (let* ((envelope (romacs-mcp--call (romacs-mcp--object "name" name "arguments" (apply #'romacs-mcp--object arguments))
                                     nil))
         (text (gethash "text" (aref (gethash "content" envelope) 0))))
    (ert-info (text) (should (eq (gethash "isError" envelope) :false)))
    (json-parse-string text :null-object :null :false-object :false)))

;;; A plan offered waits on its user

(defun romacs-agent-task-summary-tests--offer (session seq)
  "SESSION offers a plan to proceed with, as Codex's plan item says, at SEQ."
  (let ((event (car (romacs-agent--codex-item
                     session (romacs-mcp--object "type" "plan" "id" (format "plan-%d" seq) "text" "1. Do it") t))))
    (setf (romacs-agent-seq session) seq)
    (romacs-agent--raise session (append event (list :seq seq :last t)))))

(ert-deftest romacs-agent-task-summary-a-plan-offer-waits ()
  (romacs-agent-task-summary-tests--with
    (let* ((id romacs-agent-task-summary-tests--task)
           (task (romacs-agent-task-summary-tests--session
                  id :options (list :plan t :parent romacs-agent-task-summary-tests--lead :wake "all")))
           (lead (romacs-agent-get romacs-agent-task-summary-tests--lead)))
      ;; Before it offers one: idle, and no planOffered at all.
      (let ((answer (romacs-agent-task-summary-tests--call "romacs_task_status" "taskId" id)))
        (should (equal (gethash "status" answer) "idle"))
        (should-not (gethash "planOffered" answer)))
      (should-not (romacs-agent-tools--all-pending-p lead))
      (romacs-agent-task-summary-tests--offer task 7)
      (should (romacs-agent-plan-offer-waits task))
      (let ((answer (romacs-agent-task-summary-tests--call "romacs_task_status" "taskId" id)))
        (should (equal (gethash "status" answer) "waiting"))
        (should (eq (gethash "planOffered" answer) t))
        (should (eq (gethash "pendingApproval" answer) :false)))
      ;; A wait on it answers, as for an approval; its "all" siblings' ends wait.
      (let ((ack (car (funcall (cadr (romacs-agent-tools--wait task 600000 (float-time)))))))
        (should (equal (gethash "status" ack) "waiting")))
      (should (romacs-agent-tools--all-pending-p lead))
      ;; Out of plan mode, it offers none.
      (romacs-agent-set-plan task nil)
      (let ((answer (romacs-agent-task-summary-tests--call "romacs_task_status" "taskId" id)))
        (should (equal (gethash "status" answer) "idle"))
        (should-not (gethash "planOffered" answer))))))

;;; A finished task not loaded

(defun romacs-agent-task-summary-tests--journal (dir)
  "Record in DIR a Claude Code run whose one turn ended saying \"Done.\"."
  (let ((seq 0) (coding-system-for-write 'utf-8-unix))
    (cl-flet ((record (key value) (romacs-mcp--object key value "seq" (cl-incf seq) "t" (+ 1794600000.25 seq)))
              (o (&rest fields) (apply #'romacs-mcp--object fields)))
      (write-region
       (mapconcat
        (lambda (record) (concat (romacs-agent--json record) "\n"))
        (list (record "note" (o "type" "run"))
              (record "out" (o "type" "system" "subtype" "init" "session_id" "native-1"
                               "model" "claude-opus-5-5" "permissionMode" "default"))
              (record "in" (o "type" "user" "romacsRequest" 1 "message" (o "role" "user" "content" "Review it")))
              (record "out" (o "type" "assistant" "parent_tool_use_id" :null
                                "message" (o "id" "m1" "content" (vector (o "type" "text" "text" "Done.")))))
              (record "out" (o "type" "result" "result" "Done." "total_cost_usd" 0.5)))
        "")
       nil (expand-file-name "journal.jsonl" dir) nil 'silent))))

(ert-deftest romacs-agent-task-summary-an-unloaded-task-is-asked-of-its-owner ()
  (romacs-agent-task-summary-tests--with
    (let* ((id romacs-agent-task-summary-tests--task)
           (dir (expand-file-name id (romacs-agent--state-root)))
           (parent romacs-agent-task-summary-tests--lead))
      (make-directory dir t)
      (romacs-agent--write-json
       (expand-file-name "manifest.json" dir)
       (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" "/var/tmp/" "title" "Finished"
                           "titleAuto" :false "argv" [] "options" (romacs-mcp--object "parent" parent "role" "reviewer")
                           "seenSeq" 0))
      (romacs-agent-task-summary-tests--journal dir)
      ;; Its row, as a start leaves one: what the list shows of it.
      (should (romacs-agent-store--put (romacs-agent--make :id id :dir dir :harness 'claude :provider "claude"
                                                           :title "Finished" :cwd "/var/tmp/" :state 'idle
                                                           :options (list :parent parent))
                                       nil))
      ;; The list reads rows alone, as before: a known limit.
      (let ((row (aref (gethash "tasks" (romacs-agent-task-summary-tests--call "romacs_task_list")) 0)))
        (should (equal (gethash "taskId" row) id))
        (should (equal (gethash "status" row) "idle"))
        (should (eq (gethash "result" row) :null)))
      (should-not (romacs-agent-loaded id))
      ;; Named by id: loaded, and answered whole.
      (dolist (name (list id (substring id 0 8)))
        (let ((answer (romacs-agent-task-summary-tests--call "romacs_task_status" "taskId" name)))
          (should (equal (gethash "status" answer) "completed"))
          (should (= (gethash "turns" answer) 1))
          (should (equal (gethash "result" answer) "Done."))
          (should (equal (gethash "role" answer) "reviewer"))))
      (should (romacs-agent-loaded id)))))

;;; A reply carried, not copied

(ert-deftest romacs-agent-task-summary-carries-its-reply-uncopied ()
  (romacs-agent-task-summary-tests--with
    (let* ((reply (concat "Done.\n" (make-string 100000 ?x)))
           (task (romacs-agent-task-summary-tests--session romacs-agent-task-summary-tests--task
                                                          :turns 1 :reply reply))
           (id (romacs-agent-id task)))
      (should (eq (romacs-agent-summary-reply (romacs-agent-summary id)) reply))
      ;; Made again as it changes: still that string.
      (romacs-agent--changed task)
      (should (eq (romacs-agent-summary-reply (romacs-agent-summary id)) reply))
      ;; One with text properties is carried without them.
      (setf (romacs-agent-reply task) (propertize "Done." 'face 'bold))
      (romacs-agent--changed task)
      (let ((carried (romacs-agent-summary-reply (romacs-agent-summary id))))
        (should (equal carried "Done."))
        (should-not (text-properties-at 0 carried))))))

;;; Ends held for an "all" task go once it no longer works

(defconst romacs-agent-task-summary-tests--tool-hooks
  (seq-filter (lambda (function) (and (symbolp function)
                                      (string-prefix-p "romacs-agent-tools--" (symbol-name function))))
              romacs-agent-event-functions)
  "The tools' own event functions, as they hook in.")

(defun romacs-agent-task-summary-tests--held (withdraw)
  "What wakes the lead when task A, delegated with the \"all\" rule, offers
a plan, its sibling B, of the same rule, ends meanwhile, and WITHDRAW, a
function of A, then ends A's work without a turn's end."
  (romacs-agent-task-summary-tests--with
    (let* ((lead-id romacs-agent-task-summary-tests--lead)
           (a (romacs-agent-task-summary-tests--session
               "a1100000-0000-4000-8000-000000000001" :options (list :plan t :parent lead-id :wake "all")))
           (b (romacs-agent-task-summary-tests--session
               "b1100000-0000-4000-8000-000000000002" :state 'running :options (list :parent lead-id :wake "all")))
           (romacs-agent-event-functions romacs-agent-task-summary-tests--tool-hooks)
           (romacs-agent-tools--waking (make-hash-table :test #'equal))
           (romacs-agent-tools--scheduled (make-hash-table :test #'equal))
           (delivered nil))
      (cl-letf (((symbol-function 'run-at-time) (lambda (_time _repeat function &rest args) (apply function args)))
                ((symbol-function 'romacs-agent-tools--deliver) (lambda (_parent text &rest _) (push text delivered))))
        (romacs-agent-task-summary-tests--offer a 7)
        ;; B ends: its end waits while A's offer does.
        (setf (romacs-agent-state b) 'idle (romacs-agent-turns b) 1 (romacs-agent-done-seq b) 5)
        (push (list 5 "B done" nil) (romacs-agent-ends b))
        (romacs-agent--summary-stale (romacs-agent-id b))
        (romacs-agent-tools--wake b)
        (should-not delivered)
        (funcall withdraw a)
        delivered))))

(ert-deftest romacs-agent-task-summary-a-withdrawn-offer-lets-the-held-ends-go ()
  (dolist (withdraw (list (lambda (a) (romacs-agent-set-plan a nil))
                          #'romacs-agent-settle))
    (let ((delivered (romacs-agent-task-summary-tests--held withdraw)))
      (should (= (length delivered) 1))
      (should (string-match-p "B done" (car delivered))))))

;;; A plan offered says so in its wake

(ert-deftest romacs-agent-task-summary-a-wake-with-an-offer-says-waiting ()
  (romacs-agent-task-summary-tests--with
    (let ((task (romacs-agent-task-summary-tests--session
                 romacs-agent-task-summary-tests--task
                 :options (list :plan t :parent romacs-agent-task-summary-tests--lead))))
      (romacs-agent-task-summary-tests--offer task 7)
      (setf (romacs-agent-done-seq task) 8 (romacs-agent-turns task) 1)
      (romacs-agent--summary-stale (romacs-agent-id task))
      (let ((summary (romacs-agent-summary (romacs-agent-id task))))
        (should (equal (plist-get (romacs-agent-tools--wake-parse (romacs-agent-tools--wake-text summary '(8 "Plan" nil)))
                                  :state)
                       "waiting"))
        (should (equal (plist-get (plist-get (romacs-agent-tools--wake-message summary '(8 "Plan" nil)) :wake) :state)
                       "waiting"))
        ;; An earlier end was completed.
        (should (equal (plist-get (romacs-agent-tools--wake-parse (romacs-agent-tools--wake-text summary '(6 "Done" nil)))
                                  :state)
                       "completed")))
      (should (eq (gethash "planOffered" (romacs-agent-tools--ack task)) t)))))

;;; A key that reads its parent known

(defconst romacs-agent-task-summary-tests--orphan "0a1a0000-0000-4000-8000-000000000003")
(defconst romacs-agent-task-summary-tests--late "1a7e0000-0000-4000-8000-000000000004")

(defun romacs-agent-task-summary-tests--keys ()
  "The orphan's key as every view reads it: by its id, among the summaries
the board draws, and in the switcher's annotation, as (KEY BOARD-KEY
ANNOTATED)."
  (let* ((id romacs-agent-task-summary-tests--orphan)
         (summary (romacs-agent-summary id))
         (drawn (cl-find id (romacs-agent-summary nil) :key #'romacs-agent-summary-id :test #'equal)))
    (list (romacs-agent-summary-state-key summary) (romacs-agent-summary-state-key drawn)
          (substring-no-properties (romacs-agent-switch--annotation summary)))))

(defun romacs-agent-task-summary-tests--annotated (key)
  "The switcher's annotation of the orphan, its key KEY."
  (let ((summary (copy-sequence (romacs-agent-summary romacs-agent-task-summary-tests--orphan))))
    (setf (romacs-agent-summary-state-key summary) key)
    (substring-no-properties (romacs-agent-switch--annotation summary))))

(ert-deftest romacs-agent-task-summary-a-key-follows-its-parent-come-and-gone ()
  (require 'romacs-agent-switch)
  (romacs-agent-task-summary-tests--with
    (romacs-agent-task-summary-tests--session
     romacs-agent-task-summary-tests--orphan :done-seq 4
     :options (list :parent romacs-agent-task-summary-tests--late))
    (should (equal (romacs-agent-task-summary-tests--keys)
                   (list 'idle 'idle (romacs-agent-task-summary-tests--annotated 'idle))))
    ;; Its parent loaded: no fact of its own changed.
    (let ((late (romacs-agent-task-summary-tests--session romacs-agent-task-summary-tests--late)))
      (should (equal (romacs-agent-task-summary-tests--keys)
                     (list 'completed 'completed (romacs-agent-task-summary-tests--annotated 'completed))))
      ;; Gone from here: known still by its row, then not at all.
      (should (romacs-agent-store--put late nil))
      (remhash romacs-agent-task-summary-tests--late romacs-agent--sessions)
      (should (eq (car (romacs-agent-task-summary-tests--keys)) 'completed))
      (sqlite-execute (romacs-agent-store--open) "delete from sessions where id = ?"
                      (list romacs-agent-task-summary-tests--late))
      (should (equal (romacs-agent-task-summary-tests--keys)
                     (list 'idle 'idle (romacs-agent-task-summary-tests--annotated 'idle)))))))

(ert-deftest romacs-agent-task-summary-a-first-pass-makes-each-once ()
  (romacs-agent-task-summary-tests--with
    (dotimes (i 3)
      (romacs-agent-task-summary-tests--session (format "f1%06d-0000-4000-8000-000000000000" i)
                                                :options (list :parent romacs-agent-task-summary-tests--lead)))
    (let ((romacs-agent--summaries (make-hash-table :test #'equal)) (made 0))
      (cl-letf* ((make (symbol-function 'romacs-agent-summary--of))
                 ((symbol-function 'romacs-agent-summary--of)
                  (lambda (&rest args) (cl-incf made) (apply make args))))
        (romacs-agent-summary nil)
        (should (= made 4))
        (romacs-agent-summary nil)
        (should (= made 4))))))

;;; The task list's order

(ert-deftest romacs-agent-task-summary-ties-keep-their-order ()
  ;; As recent, they list as before: the loaded ones by their directories,
  ;; newest first, then those known by their rows, as the store lists them.
  (romacs-agent-task-summary-tests--with
    (let ((lead romacs-agent-task-summary-tests--lead) (root (romacs-agent--state-root)))
      (romacs-agent-task-summary-tests--session "a0000000-0000-4000-8000-00000000000a"
                                                :dir (expand-file-name "z-first" root) :last-time 50.0
                                                :options (list :parent lead))
      (romacs-agent-task-summary-tests--session "b0000000-0000-4000-8000-00000000000b"
                                                :dir (expand-file-name "a-second" root) :last-time 50.0
                                                :options (list :parent lead))
      (dolist (id '("f0000000-0000-4000-8000-00000000000f" "e0000000-0000-4000-8000-00000000000e"
                    "c0000000-0000-4000-8000-00000000000c"))
        (should (romacs-agent-store--put (romacs-agent--make :id id :dir (expand-file-name id root)
                                                             :harness 'codex :provider "codex" :title id
                                                             :state 'idle :last-time 50.0 :options (list :parent lead))
                                         nil)))
      (should (equal (mapcar (lambda (task) (gethash "taskId" task))
                             (gethash "tasks" (romacs-agent-task-summary-tests--call "romacs_task_list")))
                     (append '("a0000000-0000-4000-8000-00000000000a" "b0000000-0000-4000-8000-00000000000b")
                             (mapcar #'car (romacs-agent-store-unloaded-rows))))))))

;;; A task asked for while a worker folds it

(ert-deftest romacs-agent-task-summary-a-task-folding-answers-once-folded ()
  (romacs-agent-task-summary-tests--with
    (let* ((id romacs-agent-task-summary-tests--task)
           (dir (expand-file-name id (romacs-agent--state-root)))
           (handler (plist-get (alist-get "romacs_task_status" romacs-mcp-tools nil nil #'equal) :handler))
           (romacs-agent--folding (make-hash-table :test #'equal))
           (romacs-agent--fold-queue nil)
           (romacs-agent--fold-waiters (make-hash-table :test #'equal))
           (romacs-agent-fold-jobs 1)
           ;; The one worker's slot taken.
           (romacs-agent--fold-running 1))
      (make-directory dir t)
      (romacs-agent--write-json
       (expand-file-name "manifest.json" dir)
       (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" "/var/tmp/" "title" "Finished"
                           "titleAuto" :false "argv" [] "seenSeq" 0
                           "options" (romacs-mcp--object "parent" romacs-agent-task-summary-tests--lead)))
      (romacs-agent-task-summary-tests--journal dir)
      (should (romacs-agent-store--put (romacs-agent--make :id id :dir dir :harness 'claude :provider "claude"
                                                           :title "Finished" :cwd "/var/tmp/" :state 'idle
                                                           :options (list :parent romacs-agent-task-summary-tests--lead))
                                       nil))
      (let ((answer (cl-letf (((symbol-function 'romacs-agent--fold-aside-p) (lambda (&rest _) t)))
                      (funcall handler (romacs-mcp--object "taskId" id)))))
        ;; Not its row's facts: an answer once its records are folded.
        (should (eq (car-safe answer) 'romacs-mcp--deferred))
        (should-not (funcall (cadr answer)))
        ;; Its fold hurried, first in line.
        (should (eq (car (car romacs-agent--fold-queue)) (romacs-agent-loaded id)))
        (romacs-agent--fold-now (romacs-agent-loaded id))
        (let ((task (car (funcall (cadr answer)))))
          (should (equal (gethash "status" task) "completed"))
          (should (= (gethash "turns" task) 1))
          (should (equal (gethash "result" task) "Done.")))))))

(ert-deftest romacs-agent-task-summary-a-fold-that-fails-is-an-error ()
  ;; Its worker keeps no state, and no retry does: the deferred answer is
  ;; an error, never what its row alone says.
  (romacs-agent-task-summary-tests--with
    (let* ((id romacs-agent-task-summary-tests--task)
           (dir (expand-file-name id (romacs-agent--state-root)))
           (handler (plist-get (alist-get "romacs_task_status" romacs-mcp-tools nil nil #'equal) :handler))
           (romacs-agent--folding (make-hash-table :test #'equal))
           (romacs-agent--fold-queue nil)
           (romacs-agent--fold-waiters (make-hash-table :test #'equal))
           (romacs-agent--fold-tries (make-hash-table :test #'equal))
           (romacs-agent--fold-parked (make-hash-table :test #'equal))
           (romacs-agent-fold-jobs 1)
           (romacs-agent--fold-running 1))
      (make-directory dir t)
      (romacs-agent--write-json
       (expand-file-name "manifest.json" dir)
       (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" "/var/tmp/" "title" "Finished"
                           "titleAuto" :false "argv" [] "seenSeq" 0
                           "options" (romacs-mcp--object "parent" romacs-agent-task-summary-tests--lead)))
      (romacs-agent-task-summary-tests--journal dir)
      (let ((answer (cl-letf (((symbol-function 'romacs-agent--fold-aside-p) (lambda (&rest _) t)))
                      (funcall handler (romacs-mcp--object "taskId" id))))
            (session (romacs-agent-loaded id)))
        (should (eq (car-safe answer) 'romacs-mcp--deferred))
        ;; The worker's last try kept no state: said unfolded.
        (puthash id romacs-agent--fold-retries romacs-agent--fold-tries)
        (romacs-agent--folded-one session (cdr (gethash id romacs-agent--folding)) "it kept no state")
        (should-not (romacs-agent--folding-p session))
        (should-error (funcall (cadr answer)) :type 'user-error)))))

(ert-deftest romacs-agent-task-summary-a-journal-that-cannot-be-read-is-an-error ()
  ;; No worker could start, and its journal cannot be read here: the
  ;; deferred answer is an error, never what its row alone says.
  (romacs-agent-task-summary-tests--with
    (let* ((id romacs-agent-task-summary-tests--task)
           (dir (expand-file-name id (romacs-agent--state-root)))
           (handler (plist-get (alist-get "romacs_task_status" romacs-mcp-tools nil nil #'equal) :handler))
           (romacs-agent--folding (make-hash-table :test #'equal))
           (romacs-agent--fold-queue nil)
           (romacs-agent--fold-waiters (make-hash-table :test #'equal))
           (romacs-agent--fold-tries (make-hash-table :test #'equal))
           (romacs-agent--fold-parked (make-hash-table :test #'equal))
           (romacs-agent-fold-jobs 1)
           (romacs-agent--fold-running 1))
      (make-directory dir t)
      (romacs-agent--write-json
       (expand-file-name "manifest.json" dir)
       (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" "/var/tmp/" "title" "Finished"
                           "titleAuto" :false "argv" [] "seenSeq" 0
                           "options" (romacs-mcp--object "parent" romacs-agent-task-summary-tests--lead)))
      ;; Its journal a directory: no read of it goes through.
      (make-directory (expand-file-name "journal.jsonl" dir))
      (let ((answer (cl-letf (((symbol-function 'romacs-agent--fold-aside-p) (lambda (&rest _) t)))
                      (funcall handler (romacs-mcp--object "taskId" id))))
            (session (romacs-agent-loaded id)))
        (should (eq (car-safe answer) 'romacs-mcp--deferred))
        (romacs-agent--folded-one session (cdr (gethash id romacs-agent--folding)) (list 'no-worker "no worker"))
        (should-not (romacs-agent--folding-p session))
        (should-error (funcall (cadr answer)) :type 'user-error)))))

;;; A Claude rollback's conversation anew

(ert-deftest romacs-agent-task-summary-a-rollback-that-cannot-write-changes-nothing ()
  (romacs-agent-task-summary-tests--with
    (let* ((task (romacs-agent-task-summary-tests--session
                  romacs-agent-task-summary-tests--task :harness 'claude :harness-session "native-1"))
           (records (list (romacs-mcp--object "seq" 1 "in" (romacs-mcp--object "type" "user" "romacsRequest" "p1"
                                                                               "message" (romacs-mcp--object "content" "Go")))))
           (relaunch (cadr (romacs-agent--claude-rollback task records "p1")))
           (id (romacs-agent-id task)))
      (should (functionp relaunch))
      (should (equal (romacs-agent-summary-conversation (romacs-agent-summary id)) "native-1"))
      (cl-letf (((symbol-function 'romacs-agent--manifest-put) (lambda (&rest _) (signal 'file-error '("No space")))))
        (should-error (funcall relaunch) :type 'file-error))
      (should (equal (romacs-agent-harness-session task) "native-1"))
      (should (equal (romacs-agent-summary id)
                     (let ((romacs-agent--summaries (make-hash-table :test #'equal))) (romacs-agent-summary id))))
      ;; Written, it says the new one.
      (funcall relaunch)
      (should-not (equal (romacs-agent-harness-session task) "native-1"))
      (should (equal (romacs-agent-summary-conversation (romacs-agent-summary id)) (romacs-agent-harness-session task))))))

;;; A copy of a message to a task since gone

(ert-deftest romacs-agent-task-summary-a-copy-to-a-task-gone-reads-as-before ()
  (romacs-agent-task-summary-tests--with
    (let* ((lead romacs-agent-task-summary-tests--lead)
           (sender (romacs-agent-task-summary-tests--session "5e000000-0000-4000-8000-000000000005"
                                                             :title "Sender" :options (list :parent lead)))
           (target (romacs-agent-task-summary-tests--session "7a000000-0000-4000-8000-000000000006"
                                                             :title "Target" :options (list :parent lead)))
           (copy (list target "queue" "hello")))
      (remhash (romacs-agent-id target) romacs-agent--sessions)
      (should-not (romacs-agent-summary (romacs-agent-id target)))
      (should (equal (romacs-agent-tools--copy-body copy)
                     (format "To task %s (Target), queue:\nhello" (romacs-agent-id target))))
      (should (string-prefix-p (format "Delegated task %s (Sender) [sent," (romacs-agent-id sender))
                               (romacs-agent-tools--copy-text sender copy))))))

(provide 'romacs-agent-task-summary-tests)
;;; romacs-agent-task-summary-tests.el ends here

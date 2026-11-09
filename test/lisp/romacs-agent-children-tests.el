;;; romacs-agent-children-tests.el --- What runs under a session -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-ui))

(defmacro romacs-agent-children-tests--with (&rest body)
  "BODY with `parent', a session of no children, and `child', which makes one."
  (declare (indent 0))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
          (romacs-agent-ui--children-timer nil)
          (parent (romacs-agent--make :id "parent" :harness 'claude :provider "claude" :state 'running))
          (made 0))
     (puthash "parent" parent romacs-agent--sessions)
     (cl-flet ((child (state &rest fields)
                 (let ((session (apply #'romacs-agent--make
                                       :id (format "child-%d" (cl-incf made)) :harness 'codex :provider "codex"
                                       :title (format "task %d" made) :state state :options '(:parent "parent")
                                       fields)))
                   (puthash (romacs-agent-id session) session romacs-agent--sessions)
                   (romacs-agent--index session)
                   session)))
       ,@body)))

(defun romacs-agent-children-tests--summary (parent)
  (romacs-agent-ui--summarize-children parent)
  (substring-no-properties (or (romacs-agent-children-summary parent) "")))

(ert-deftest romacs-agent-children-counts ()
  (romacs-agent-children-tests--with
    (child 'running) (child 'starting) (child 'waiting-approval)
    (child 'idle :failed t :done-seq 3)
    (child 'idle :turns 1 :done-seq 3 :seen-seq 3)
    (should (equal (romacs-agent-children-tests--summary parent) "2 running · 1 waiting · 1 failed"))
    ;; What wants the user first, then what runs, then the rest.
    (should (equal (mapcar (lambda (row) (plist-get row :state)) (romacs-agent-children-rows parent))
                   '("waiting" "failed" "running" "running" "completed")))))

(ert-deftest romacs-agent-children-faces-mean-the-state ()
  (romacs-agent-children-tests--with
    (child 'running) (child 'waiting-approval) (child 'idle :failed t :done-seq 3)
    (romacs-agent-ui--summarize-children parent)
    (let ((summary (romacs-agent-children-summary parent)))
      (should (eq (get-text-property 0 'face summary) 'romacs-agent-running))
      (should (eq (get-text-property (string-search "waiting" summary) 'face summary) 'warning))
      (should (eq (get-text-property (string-search "failed" summary) 'face summary) 'romacs-agent-error)))))

(ert-deftest romacs-agent-children-nothing-to-say ()
  (romacs-agent-children-tests--with
    (should (equal (romacs-agent-children-tests--summary parent) ""))
    (child 'idle :turns 1 :done-seq 2 :seen-seq 2)
    ;; All done: the header says nothing; the menu still lists it.
    (should (equal (romacs-agent-children-tests--summary parent) ""))
    (should (= (length (romacs-agent-children-rows parent)) 1))))

(ert-deftest romacs-agent-children-a-seen-failure-leaves-the-count ()
  (romacs-agent-children-tests--with
    (let ((failed (child 'idle :failed t :done-seq 3)))
      (should (equal (romacs-agent-children-tests--summary parent) "1 failed"))
      (setf (romacs-agent-seen-seq failed) 3)
      (should (equal (romacs-agent-children-tests--summary parent) ""))
      (should (= (length (romacs-agent-children-rows parent)) 1)))))

(ert-deftest romacs-agent-children-only-state-events-mark-the-parent ()
  (romacs-agent-children-tests--with
    (let ((session (child 'running)))
      (dolist (type '(delta thinking status usage message-end unknown))
        (romacs-agent-ui--track-children session (list :type type)))
      (should (zerop (hash-table-count romacs-agent-ui--children-changed)))
      (should-not romacs-agent-ui--children-timer)
      (dolist (type '(run session input approval answered done exit))
        (clrhash romacs-agent-ui--children-changed)
        (romacs-agent-ui--track-children session (list :type type))
        (should (gethash parent romacs-agent-ui--children-changed)))
      (cancel-timer romacs-agent-ui--children-timer))))

(ert-deftest romacs-agent-children-one-batch-one-summary ()
  (romacs-agent-children-tests--with
    (let ((sessions (list (child 'running) (child 'running) (child 'running))) (made-rows 0))
      (cl-letf* ((summarize (symbol-function 'romacs-agent-ui--summarize-children))
                 ((symbol-function 'romacs-agent-ui--summarize-children)
                  (lambda (session) (cl-incf made-rows) (funcall summarize session))))
        (dolist (session sessions)
          (setf (romacs-agent-state session) 'idle (romacs-agent-turns session) 1)
          (romacs-agent-ui--track-children session (list :type 'done)))
        (cancel-timer romacs-agent-ui--children-timer)
        (romacs-agent-ui--children-flush))
      (should (= made-rows 1))
      (should (equal (substring-no-properties (or (romacs-agent-children-summary parent) "")) "")))))

(ert-deftest romacs-agent-children-the-harness-own-subagents ()
  (romacs-agent-children-tests--with
    (let ((event (lambda (state &rest fields)
                   (romacs-agent-ui--track-children
                    parent (append (list :type 'tool :state state :id "t1") fields)))))
      (funcall event 'started :name "Agent" :input (romacs-mcp--object "description" "read the code"))
      ;; A subagent's own tool, and any other tool, are no subagents.
      (romacs-agent-ui--track-children parent '(:type tool :state started :id "t2" :name "Bash"))
      (romacs-agent-ui--track-children parent (list :type 'tool :state 'started :id "t3" :name "Agent" :parent "t1"
                                                    :input (romacs-mcp--object "description" "nested")))
      (should (equal (romacs-agent-children-tests--summary parent) "1 running"))
      (should (equal (plist-get (car (romacs-agent-children-rows parent)) :title) "read the code"))
      (funcall event 'completed)
      (should (equal (romacs-agent-children-tests--summary parent) ""))
      (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "completed"))
      ;; Finished, it stays listed.
      (romacs-agent-ui--track-children parent '(:type input))
      (romacs-agent-ui--summarize-children parent)
      (should (= (length (romacs-agent-children-rows parent)) 1))
      (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer)))))

(ert-deftest romacs-agent-children-a-subagent-failure-counts-for-its-turn ()
  (romacs-agent-children-tests--with
    (romacs-agent-ui--track-children parent (list :type 'tool :state 'started :id "t1" :name "Agent"
                                                  :input (romacs-mcp--object "description" "broke")))
    (romacs-agent-ui--track-children parent '(:type tool :state failed :id "t1"))
    (should (equal (romacs-agent-children-tests--summary parent) "1 failed"))
    (romacs-agent-ui--track-children parent '(:type input))
    (should (equal (romacs-agent-children-tests--summary parent) ""))
    (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "failed"))
    (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))))

(ert-deftest romacs-agent-children-a-detached-or-cancelled-child-updates-the-parent ()
  (romacs-agent-children-tests--with
    (let ((session (child 'running)))
      (dolist (text '("detached" "supervisor not reachable" "cancelled"))
        (clrhash romacs-agent-ui--children-changed)
        (romacs-agent-ui--track-children session (list :type 'status :text text))
        (should (gethash parent romacs-agent-ui--children-changed)))
      ;; Detached, it no longer reads as running.
      (setf (romacs-agent-state session) 'detached)
      (should (equal (romacs-agent-children-tests--summary parent) ""))
      (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "detached"))
      (cancel-timer romacs-agent-ui--children-timer))))

(ert-deftest romacs-agent-children-cancelling-an-idle-child-is-heard ()
  (romacs-agent-children-tests--with
    (let ((session (child 'idle :failed t :done-seq 3)) (dir (make-temp-file "romacs-children-" t)))
      (setf (romacs-agent-dir session) dir)
      (unwind-protect
          (progn
            (should (equal (romacs-agent-children-tests--summary parent) "1 failed"))
            (cl-letf (((symbol-function 'romacs-mcp--instance) #'ignore)
                      ((symbol-function 'romacs-agent-stop) #'ignore))
              (romacs-agent-tools--cancel (romacs-mcp--object "taskId" (romacs-agent-id session))))
            (should (gethash parent romacs-agent-ui--children-changed))
            (should (equal (romacs-agent-children-tests--summary parent) ""))
            (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "cancelled")))
        (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))
        (delete-directory dir t)))))

(ert-deftest romacs-agent-children-replay-finds-the-harness-subagents ()
  ;; No supervisor: the session raised no events; opening its chat replays
  ;; the journal, and that is where they are found.
  (romacs-agent-children-tests--with
    (let ((dir (make-temp-file "romacs-children-" t)) (coding-system-for-write 'utf-8-unix))
      (unwind-protect
          (let ((session (romacs-agent--make :id "p2" :harness 'claude :provider "claude" :dir dir :state 'ended)))
            (write-region
             (concat (romacs-agent--json
                      (romacs-mcp--object
                       "seq" 1 "t" 1.0
                       "out" (romacs-mcp--object
                              "type" "assistant" "parent_tool_use_id" :null
                              "message" (romacs-mcp--object
                                         "id" "m" "content"
                                         (vector (romacs-mcp--object "type" "tool_use" "id" "t1" "name" "Agent"
                                                                     "input" (romacs-mcp--object "description" "read")))))))
                     "\n")
             nil (expand-file-name "journal.jsonl" dir) nil 'silent)
            (with-temp-buffer
              (romacs-agent-chat-mode)
              (let ((inhibit-read-only t))
                (setq romacs-agent-ui--session session
                      romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                      romacs-agent-ui--tasks (make-hash-table :test #'equal)
                      romacs-agent-ui--messages (make-hash-table :test #'equal)
                      romacs-agent-ui--end (point-marker))
                (romacs-agent-ui--replay session)))
            (romacs-agent-ui--summarize-children session)
            (should (equal (plist-get (car (romacs-agent-children-rows session)) :title) "read"))
            ;; A journal that ends mid-call leaves it running: only a turn's
            ;; end or an exit says otherwise.
            (should (equal (plist-get (car (romacs-agent-children-rows session)) :state) "running")))
        (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))
        (delete-directory dir t)))))

(ert-deftest romacs-agent-children-a-cut-turn-ends-its-subagents ()
  (dolist (type '(done exit))
    (romacs-agent-children-tests--with
      (romacs-agent-ui--track-children parent (list :type 'tool :state 'started :id "t1" :name "Task"
                                                    :input (romacs-mcp--object "description" "cut")))
      (romacs-agent-ui--track-children parent (list :type type))
      (should (equal (romacs-agent-children-tests--summary parent) ""))
      (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "cancelled"))
      (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer)))))

(ert-deftest romacs-agent-children-the-header-only-reads ()
  (romacs-agent-children-tests--with
    (child 'running)
    (romacs-agent-ui--summarize-children parent)
    (with-temp-buffer
      (let ((romacs-agent-ui--session parent))
        (cl-letf (((symbol-function 'romacs-agent-ui--summarize-children) (lambda (&rest _) (error "Made at redisplay")))
                  ((symbol-function 'romacs-agent-children) (lambda (&rest _) (error "Scanned at redisplay"))))
          (should (string-search "⇢ 1 running" (romacs-agent-ui--header-line))))))))

(ert-deftest romacs-agent-children-menu-rows-open-children ()
  (romacs-agent-children-tests--with
    (let ((waiting (child 'waiting-approval)) (opened nil))
      (child 'running)
      (romacs-agent-ui--summarize-children parent)
      (with-temp-buffer
        (let ((romacs-agent-ui--session parent))
          (should (string-prefix-p "Workers 1 running" (romacs-agent-ui--children-title)))
          (should (string-search "task 1" (romacs-agent-ui--child-label 0)))
          (should (string-search "waiting" (romacs-agent-ui--child-label 0)))
          (should-not (romacs-agent-ui--child-label 2))
          (cl-letf (((symbol-function 'romacs-agent-switch) (lambda (session) (setq opened session))))
            (let ((last-command-event ?1)) (romacs-agent-chat-open-child))
            (should (eq opened waiting))
            (let ((last-command-event ?5))
              (should-error (romacs-agent-chat-open-child) :type 'user-error))))))))

(ert-deftest romacs-agent-children-menu-rows-take-a-click ()
  ;; A row carries the popup's click data, its key among it: the popup's
  ;; mouse path replays that key.
  (romacs-agent-children-tests--with
    (child 'running)
    (romacs-agent-ui--summarize-children parent)
    (with-temp-buffer
      (let* ((romacs-agent-ui--session parent)
             (entry (keymap-popup--entry "1" (lambda () (romacs-agent-ui--child-label 0)) 'suffix
                                         :command #'romacs-agent-chat-open-child))
             (line (keymap-popup--render-entry entry))
             (at (string-search "task 1" line)))
        (should at)
        (should (equal (plist-get (get-text-property at 'keymap-popup--entry line) :key) "1"))
        (should (eq (get-text-property at 'mouse-face line) 'highlight))))))

(ert-deftest romacs-agent-children-a-subagent-row-goes-to-its-card ()
  (romacs-agent-children-tests--with
    (with-temp-buffer
      (insert "before\ncard\n")
      (let ((romacs-agent-ui--session parent)
            (romacs-agent-ui--tool-lines (make-hash-table :test #'equal))
            (mark (copy-marker 8)))
        (puthash "t1" mark romacs-agent-ui--tool-lines)
        (setf (romacs-agent-children-rows parent) (list (list :tool "t1" :state "running" :title "x")))
        (cl-letf (((symbol-function 'recenter) #'ignore))
          (let ((last-command-event ?1)) (romacs-agent-chat-open-child)))
        (should (= (point) 8))))))

;; Work the harness runs on past its tool call: a background subagent, or
;; a command left running.

(ert-deftest romacs-agent-children-claude-task-notices ()
  (let ((line (lambda (json) (romacs-agent--claude-events (romacs-agent--make :harness 'claude) (json-parse-string json)))))
    (should (equal (funcall line "{\"type\":\"system\",\"subtype\":\"task_started\",\"task_id\":\"a1\",\"tool_use_id\":\"t1\",\"description\":\"Read package.json\",\"task_type\":\"local_agent\"}")
                   '((:type task :id "t1" :state started :task "a1" :title "Read package.json" :background t))))
    ;; A subagent's own command is its.
    (should-not (funcall line "{\"type\":\"system\",\"subtype\":\"task_started\",\"task_id\":\"b2\",\"tool_use_id\":\"t5\",\"parent_task_id\":\"a1\",\"owned_by_subagent\":true}"))
    (should (equal (funcall line "{\"type\":\"system\",\"subtype\":\"task_updated\",\"task_id\":\"b3\",\"patch\":{\"is_backgrounded\":true}}")
                   '((:type task :task "b3" :state backgrounded))))
    (should-not (funcall line "{\"type\":\"system\",\"subtype\":\"task_updated\",\"task_id\":\"b3\",\"patch\":{\"status\":\"completed\"}}"))
    (should (equal (funcall line "{\"type\":\"system\",\"subtype\":\"task_progress\",\"task_id\":\"a1\",\"tool_use_id\":\"t1\",\"description\":\"Reading\",\"last_tool_name\":\"Read\"}")
                   '((:type task :id "t1" :state progress :activity "Read"))))
    (should (equal (funcall line "{\"type\":\"system\",\"subtype\":\"task_notification\",\"task_id\":\"a1\",\"tool_use_id\":\"t1\",\"status\":\"completed\",\"summary\":\"Read it\"}")
                   '((:type task :id "t1" :state completed :summary "Read it"))))
    (should (eq (plist-get (car (funcall line "{\"type\":\"system\",\"subtype\":\"task_notification\",\"tool_use_id\":\"t1\",\"status\":\"killed\"}")) :state)
                'stopped))
    ;; No tool to hang it on: nothing.
    (should-not (funcall line "{\"type\":\"system\",\"subtype\":\"task_started\",\"task_id\":\"a1\"}"))))

(ert-deftest romacs-agent-children-a-background-subagent-runs-past-its-call ()
  (romacs-agent-children-tests--with
    (let ((row (lambda () (car (romacs-agent-children-rows parent))))
          (track (lambda (event) (romacs-agent-ui--track-children parent event) (romacs-agent-ui--summarize-children parent))))
      (funcall track (list :type 'tool :state 'started :id "t1" :name "Agent"
                           :input (romacs-mcp--object "description" "build the driver")))
      (funcall track '(:type task :id "t1" :state started :task "a1" :title "build the driver"))
      ;; Its call returns at once; its turn ends: it still runs.
      (funcall track '(:type tool :state completed :id "t1"))
      (funcall track '(:type done))
      (should (equal (plist-get (funcall row) :state) "running"))
      (funcall track '(:type task :id "t1" :state progress :activity "Bash"))
      (with-temp-buffer
        (let ((romacs-agent-ui--session parent))
          (should (string-search "running Bash" (substring-no-properties (romacs-agent-ui--child-label 0))))))
      ;; Its own notice ends it.
      (funcall track '(:type task :id "t1" :state failed :summary "no"))
      (should (equal (plist-get (funcall row) :state) "failed"))
      (should (equal (romacs-agent-children-tests--summary parent) "1 failed"))
      (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer)))))

(ert-deftest romacs-agent-children-a-task-ends-with-its-harness ()
  (romacs-agent-children-tests--with
    ;; A command in the foreground is its card's; sent to the background,
    ;; it has a row of its own.
    (romacs-agent-ui--track-children parent '(:type task :id "t9" :state started :task "b1" :title "make test"))
    (romacs-agent-ui--summarize-children parent)
    (should-not (romacs-agent-children-rows parent))
    (romacs-agent-ui--track-children parent '(:type task :task "b1" :state backgrounded))
    (romacs-agent-ui--summarize-children parent)
    (should (equal (plist-get (car (romacs-agent-children-rows parent)) :title) "make test"))
    (should (equal (romacs-agent-children-tests--summary parent) "1 running"))
    (romacs-agent-ui--track-children parent '(:type exit))
    (romacs-agent-ui--summarize-children parent)
    (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "cancelled"))
    (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))))

(ert-deftest romacs-agent-children-menu-keeps-the-latest-finished ()
  ;; The latest to finish, however each ended: a seen failure among them.
  (let ((rows (append (list '(:state "running" :title "r") '(:state "failed" :unseen t :title "f"))
                      (cl-loop for i below 4 collect (list :state "completed" :title (format "c%d" i) :time i))
                      (list '(:state "failed" :title "seen" :time 10)))))
    (should (equal (mapcar (lambda (row) (plist-get row :title)) (romacs-agent-ui--workers rows))
                   '("r" "f" "seen" "c3" "c2")))))

(ert-deftest romacs-agent-children-a-worker-is-dated-by-its-end ()
  (romacs-agent-children-tests--with
    (romacs-agent-ui--track-children parent '(:type task :id "t1" :state started :task "a" :title "long" :background t :time 1))
    (romacs-agent-ui--track-children parent '(:type task :id "t1" :state completed :time 50))
    (romacs-agent-ui--summarize-children parent)
    (should (= (plist-get (car (romacs-agent-children-rows parent)) :time) 50))
    ;; Cut by its harness's going: dated by that.
    (romacs-agent-ui--track-children parent '(:type task :id "t2" :state started :task "b" :title "cut" :background t :time 2))
    (romacs-agent-ui--track-children parent '(:type exit :time 70))
    (romacs-agent-ui--summarize-children parent)
    (should (equal (plist-get (car (romacs-agent-ui--workers (romacs-agent-children-rows parent))) :title) "cut"))
    (should (= (plist-get (car (romacs-agent-ui--workers (romacs-agent-children-rows parent))) :time) 70))
    (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))))

(ert-deftest romacs-agent-children-a-label-fits-the-menu ()
  (romacs-agent-children-tests--with
    (romacs-agent-ui--track-children parent '(:type task :id "t1" :state started :task "a" :background t
                                                 :title "a rather long title for a worker row"))
    (romacs-agent-ui--track-children parent `(:type task :id "t1" :state progress :activity "mcp__romacs__romacs_eval"
                                                    :time ,(float-time)))
    (romacs-agent-ui--summarize-children parent)
    (with-temp-buffer
      (let* ((romacs-agent-ui--session parent) (label (romacs-agent-ui--child-label 0)))
        (should (<= (string-width label) keymap-popup--max-description-width))
        (should (string-suffix-p "romacs_eval now" (substring-no-properties label)))))
    (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))))

(provide 'romacs-agent-children-tests)
;;; romacs-agent-children-tests.el ends here

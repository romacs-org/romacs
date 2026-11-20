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
  `(let* ((romacs-agent-glyphs nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
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
    (should (equal (romacs-agent-children-tests--summary parent) "1 needs approval · 1 failed · 3 threads live"))
    ;; What wants the user first, then what runs, then the rest.
    (should (equal (mapcar (lambda (row) (plist-get row :key)) (romacs-agent-children-rows parent))
                   '(approval failed working working idle)))))

(ert-deftest romacs-agent-children-faces-mean-the-state ()
  (romacs-agent-children-tests--with
    (child 'running) (child 'waiting-approval) (child 'idle :failed t :done-seq 3)
    (romacs-agent-ui--summarize-children parent)
    (let ((summary (romacs-agent-children-summary parent)))
      (should (eq (get-text-property (string-search "threads" summary) 'face summary) 'romacs-agent-running))
      (should (eq (get-text-property (string-search "approval" summary) 'face summary) 'romacs-agent-approval))
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
      (should (equal (romacs-agent-children-tests--summary parent) "1 working"))
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
      (should (eq (plist-get (car (romacs-agent-children-rows parent)) :key) 'detached))
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
            (should (eq (plist-get (car (romacs-agent-children-rows parent)) :key) 'ended)))
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
    (cl-letf (((symbol-function 'romacs-agent-ui--summarize-children) (lambda (&rest _) (error "Made for the header")))
              ((symbol-function 'romacs-agent-children) (lambda (&rest _) (error "Scanned for the header"))))
      (should (string-search "Working" (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs parent)))))))

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
    ;; A worker, not a subagent: the header counts none.
    (should (equal (romacs-agent-children-tests--summary parent) ""))
    (romacs-agent-ui--track-children parent '(:type exit))
    (romacs-agent-ui--summarize-children parent)
    (should (equal (plist-get (car (romacs-agent-children-rows parent)) :state) "cancelled"))
    (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))))

;; Past nine workers, and glyphs for states.

(ert-deftest romacs-agent-children-glyphs ()
  (romacs-agent-children-tests--with
    (child 'running) (child 'waiting-approval)
    (let ((romacs-agent-glyphs t))
      (cl-letf (((symbol-function 'char-displayable-p) (lambda (_) t)))
        (should (equal (romacs-agent-children-tests--summary parent) "✋1 🧵2"))
        ;; Hovering lists them, what wants the user first.
        (should (equal (get-text-property 0 'help-echo (romacs-agent-children-summary parent))
                       "task 2: needs approval\ntask 1: working"))
        (should (equal (substring-no-properties (romacs-agent-ui--label (romacs-agent-ui--state parent))) "🤖")))
      ;; Turned off, a summary made before says words at once.
      (cl-letf (((symbol-function 'char-displayable-p) (lambda (_) t))
                ((symbol-function 'romacs-agent-sessions) (lambda () (list parent))))
        (romacs-agent-ui--summarize-children parent)
        (customize-set-variable 'romacs-agent-glyphs nil)
        (should (equal (substring-no-properties (romacs-agent-children-summary parent)) "1 needs approval · 2 threads live"))
        (customize-set-variable 'romacs-agent-glyphs t))
      ;; A frame that cannot draw them gets the words.
      (cl-letf (((symbol-function 'char-displayable-p) #'ignore))
        (should (equal (romacs-agent-children-tests--summary parent) "1 needs approval 2 threads live"))
        (should (equal (substring-no-properties (romacs-agent-ui--label (romacs-agent-ui--state parent))) "Working"))))))

(ert-deftest romacs-agent-children-a-command-is-no-subagent ()
  ;; A command the harness runs in the background is a worker, not a subagent.
  (romacs-agent-children-tests--with
    (romacs-agent-ui--track-task parent '(:type task :id "c1" :state started :task "b1" :title "make test" :background t))
    (should (equal (romacs-agent-children-tests--summary parent) ""))
    (should (= (length (romacs-agent-children-rows parent)) 1))
    (romacs-agent-ui--track-subagent parent '(:type tool :id "s1" :state started :name "Agent" :input nil))
    (should (equal (romacs-agent-children-tests--summary parent) "1 working"))))

(ert-deftest romacs-agent-children-an-old-failure-is-seen-after-a-turn ()
  ;; Tasks that failed long ago: news while the parent's turn that knew of
  ;; them runs, seen once its next turn starts.
  (romacs-agent-children-tests--with
    (let ((romacs-agent-ui--turn-starts (make-hash-table :test #'eq :weakness 'key))
          (old (child 'idle :failed t :done-seq 3 :last-time 100.0))
          (seen nil))
      (child 'idle :failed t :done-seq 4 :last-time 100.0)
      (setf (romacs-agent-caught-up parent) t)
      (cl-letf (((symbol-function 'romacs-agent--manifest-put) #'ignore))
        (should (equal (romacs-agent-children-tests--summary parent) "2 failed"))
        (romacs-agent-ui--see-failed-children parent '(:type input :time 200.0))
        ;; The turn that knew of them runs, steered: still news.
        (romacs-agent-ui--see-failed-children parent '(:type input :time 210.0))
        (should (equal (romacs-agent-children-tests--summary parent) "2 failed"))
        (setf (romacs-agent-done-seq parent) 9)
        ;; A failure during that turn stays news through the next.
        (setq seen (child 'idle :failed t :done-seq 5 :last-time 250.0))
        (romacs-agent-ui--see-failed-children parent '(:type input :time 300.0))
        (should (equal (romacs-agent-children-tests--summary parent) "1 failed"))
        (should-not (romacs-agent-unseen-p old))
        (should (romacs-agent-unseen-p seen))
        (setf (romacs-agent-done-seq parent) 11)
        ;; A copy replaying a parent's journal sees nothing.
        (setf (romacs-agent-caught-up parent) nil)
        (romacs-agent-ui--see-failed-children parent '(:type input :time 400.0))
        (should (romacs-agent-unseen-p seen))))))

(provide 'romacs-agent-children-tests)
;;; romacs-agent-children-tests.el ends here

(ert-deftest romacs-agent-children-views-are-told-once-a-restore-ends ()
  ;; While a start restores, the rows of what it loads are made as they
  ;; come, but the views (the board) are told by the batch its end runs:
  ;; not a turn of loading at a time, each redrawing every row.
  (let ((romacs-agent-ui--children-changed (make-hash-table :test #'eq))
        (romacs-agent-ui--children-timer nil) (told 0) (rows 0))
    (cl-letf (((symbol-function 'romacs-agent-ui--flush-rows) (lambda (sessions) (cl-incf rows (length sessions))))
              ((symbol-function 'romacs-agent-ui--status-bar) #'ignore)
              ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
      (let ((romacs-agent-ui-changed-hook (list (lambda () (cl-incf told)))))
        (let ((romacs-agent--restoring t))
          (dotimes (i 3) (romacs-agent-ui--children-changed (intern (format "s%d" i)))))
        (should (= rows 3))
        (should (= told 0))
        ;; Its end: the restored hook's batch.
        (romacs-agent-ui--batch)
        (should (= told 1))))))

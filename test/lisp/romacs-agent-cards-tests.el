;;; romacs-agent-cards-tests.el --- Delegations in the transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defconst romacs-agent-cards-tests--parent "11111111-2222-4333-8444-555555555555")
(defconst romacs-agent-cards-tests--child "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")

(defun romacs-agent-cards-tests--table (&rest pairs)
  (let ((table (make-hash-table :test #'equal)))
    (while pairs (puthash (pop pairs) (pop pairs) table))
    table))

(defmacro romacs-agent-cards-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR with one known child."
  (declare (indent 1))
  `(let* ((,var (romacs-agent--make :id romacs-agent-cards-tests--parent :harness 'claude :state 'running
                                    :title "parent" :tools (make-hash-table :test #'equal)))
          (child (romacs-agent--make :id romacs-agent-cards-tests--child :harness 'claude :state 'idle :title "child"
                                     :options (list :parent romacs-agent-cards-tests--parent))))
     (puthash (romacs-agent-id child) child romacs-agent--sessions)
     (unwind-protect
         (with-temp-buffer
           (romacs-agent-chat-mode)
           (setq romacs-agent-ui--session ,var
                 romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                 romacs-agent-ui--tasks (make-hash-table :test #'equal)
                 romacs-agent-ui--messages (make-hash-table :test #'equal)
                 romacs-agent-ui--end (point-marker))
           ,@body)
       (remhash (romacs-agent-id child) romacs-agent--sessions))))

(defun romacs-agent-cards-tests--draw (session &rest event)
  (romacs-agent-ui--draw session event (current-buffer)))

(defun romacs-agent-cards-tests--visible ()
  "The transcript as shown: folds hidden."
  (let ((text "") (at (point-min)))
    (while (< at (point-max))
      (let ((next (next-single-property-change at 'invisible nil (point-max))))
        (unless (get-text-property at 'invisible) (setq text (concat text (buffer-substring-no-properties at next))))
        (setq at next)))
    text))

(defun romacs-agent-cards-tests--delegate (session)
  (romacs-agent-cards-tests--draw
   session :type 'tool :state 'started :id "t1" :name "mcp__romacs__romacs_delegate_task"
   :input (romacs-agent-cards-tests--table "title" "Review it" "task" "long prompt"
                                           "target" (romacs-agent-cards-tests--table "provider" "codex")))
  (romacs-agent-cards-tests--draw
   session :type 'tool :state 'completed :id "t1"
   :output (format "{\"taskId\":\"%s\",\"status\":\"running\",\"title\":\"Review it\"}" romacs-agent-cards-tests--child)))

(ert-deftest romacs-agent-delegate-call-is-a-card-with-a-thread-link ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Review it  codex  running  ↗ thread\n"))
    (let ((shown nil))
      (cl-letf (((symbol-function 'romacs-agent-show) (lambda (s) (setq shown s))))
        (goto-char (point-min))
        (romacs-agent-chat-open-task))
      (should (equal (romacs-agent-id shown) romacs-agent-cards-tests--child)))))

(ert-deftest romacs-agent-wake-settles-the-card-and-is-no-prompt ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (let ((text (romacs-agent-tools--wake-text child '(7 "all good\nsecond line" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 9))
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "  ✓ ⇢ Review it  codex  completed  ↗ thread\n"
                           "  ⇠ child  completed  ↗ thread  all good second line\n")))
    (should-not (text-property-any (point-min) (point-max) 'romacs-agent-turn t))
    ;; The whole result is one fold away.
    (goto-char (point-min))
    (forward-line 1)
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "all good\nsecond line\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-waited-task-keeps-its-result-folded ()
  ;; A `wait' answers with the result and no wake follows: the card holds it.
  (dolist (status '("completed" "failed"))
    (romacs-agent-cards-tests--with-chat session
      (romacs-agent-cards-tests--draw
       session :type 'tool :state 'started :id "t1" :name "mcp__romacs__romacs_delegate_task"
       :input (romacs-agent-cards-tests--table "title" "Review it"))
      (romacs-agent-cards-tests--draw
       session :type 'tool :state 'completed :id "t1"
       :output (format "{\"taskId\":\"%s\",\"status\":\"%s\",\"result\":\"REVIEW RESULT\"}"
                       romacs-agent-cards-tests--child status))
      (should (equal (romacs-agent-cards-tests--visible) (format "  ✓ ⇢ Review it  %s  ↗ thread\n" status)))
      (goto-char (point-min))
      (romacs-agent-chat-toggle)
      (should (string-suffix-p "REVIEW RESULT\n" (romacs-agent-cards-tests--visible))))))

(ert-deftest romacs-agent-history-shows-a-delegation-card ()
  (let ((file (make-temp-file "romacs-cards" nil ".jsonl")) (buffer nil))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\","
                    "\"name\":\"mcp__romacs__romacs_delegate_task\",\"input\":{\"title\":\"Review it\"}}]}}\n"
                    "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t1\","
                    "\"content\":\"{\\\"taskId\\\":\\\"" romacs-agent-cards-tests--child "\\\",\\\"status\\\":\\\"running\\\"}\"}]}}\n"))
          (setq buffer (romacs-agent-history-open (list :path file :title "h" :harness 'claude :id "h1" :cwd "/")))
          (with-current-buffer buffer
            (should (string-match-p "⇢ Review it  running  ↗ thread" (buffer-string)))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file file))))

(ert-deftest romacs-agent-failed-wake-shows-failed ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (romacs-agent-cards-tests--draw session :type 'input :seq 9
                                    :text (romacs-agent-tools--wake-text child '(7 "no" t)))
    (should (string-match-p "Review it  codex  failed" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-user-text-shaped-like-a-wake-stays-a-prompt ()
  ;; A task this session did not delegate, or none: the user's own words.
  (romacs-agent-cards-tests--with-chat session
    (let ((text "Delegated task 99999999-2222-4333-8444-555555555555 (x) [completed]:\nhi"))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 1)
      (should (equal (romacs-agent-cards-tests--visible) (concat "> " text "\n")))
      (should (text-property-any (point-min) (point-max) 'romacs-agent-turn t)))))

(ert-deftest romacs-agent-wake-text-reads-back ()
  (romacs-agent-cards-tests--with-chat session
    (ignore session)
    (setf (romacs-agent-title child) "a (nested) title")
    (should (equal (romacs-agent-tools--wake-parse (romacs-agent-tools--wake-text child '(1 "r\ns" t)))
                   (list :id romacs-agent-cards-tests--child :title "a (nested) title" :state "failed" :result "r\ns")))
    (should-not (romacs-agent-tools--wake-parse "hello"))))

(ert-deftest romacs-agent-native-subagent-is-the-same-card-without-a-link ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "t2" :name "Agent"
     :input (romacs-agent-cards-tests--table "description" "Find callers" "subagent_type" "Explore" "prompt" "..."))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "t2" :output "found 3")
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Find callers  Explore\n"))
    (should-not (text-property-any (point-min) (point-max) 'romacs-agent-chip t))
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "found 3\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-other-tools-keep-their-rows ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'tool :state 'started :id "t3" :name "Bash"
                                    :input (romacs-agent-cards-tests--table "command" "ls"))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "t3"
                                    :output "{\"taskId\":\"x\"}")
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ Bash  ls\n"))
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "{\"taskId\":\"x\"}\n" (romacs-agent-cards-tests--visible)))))

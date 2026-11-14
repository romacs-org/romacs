;;; romacs-agent-cards-tests.el --- Delegations in the transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-history))

(defconst romacs-agent-cards-tests--parent "11111111-2222-4333-8444-555555555555")
(defconst romacs-agent-cards-tests--child "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")

(defun romacs-agent-cards-tests--table (&rest pairs)
  (let ((table (make-hash-table :test #'equal)))
    (while pairs (puthash (pop pairs) (pop pairs) table))
    table))

(defmacro romacs-agent-cards-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR with one known child."
  (declare (indent 1))
  `(let* ((romacs-agent-glyphs nil)
          (,var (romacs-agent--make :id romacs-agent-cards-tests--parent :harness 'claude :state 'running
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
   :output (format "{\"taskId\":\"%s\",\"status\":\"running\",\"title\":\"Review it\"}" romacs-agent-cards-tests--child))
  (romacs-agent-ui--run-settle))

(ert-deftest romacs-agent-delegate-call-is-a-card-with-a-thread-link ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Review it  codex  Working  thread\n"))
    (let ((shown nil))
      (cl-letf (((symbol-function 'romacs-agent-show) (lambda (s) (setq shown s))))
        (goto-char (point-min))
        (romacs-agent-chat-open-task))
      (should (equal (romacs-agent-id shown) romacs-agent-cards-tests--child)))))

(ert-deftest romacs-agent-wake-of-a-turn-that-left-work-running ()
  ;; Its turn ended with a command still running in the background (an
  ;; interim report): not the task done.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (setf (romacs-agent-children-rows child) (list (list :key 'working :state "running" :title "check")))
    (let ((text (romacs-agent-tools--wake-text child '(7 "waiting on the check" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 9))
    (should (string-search "⇠ child  Waiting on its workers  thread" (romacs-agent-cards-tests--visible)))
    (setf (romacs-agent-children-rows child) nil)
    (let ((text (romacs-agent-tools--wake-text child '(8 "done" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 10))
    (should (string-search "⇠ child  Completed  thread  done" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-wake-settles-the-card-and-is-no-prompt ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (let ((text (romacs-agent-tools--wake-text child '(7 "all good\nsecond line" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 9))
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "  ✓ ⇢ Review it  codex  Completed  thread\n"
                           "  ⇠ child  Completed  thread  all good second line\n")))
    (should-not (text-property-not-all (point-min) (point-max) 'romacs-agent-turn nil))
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
      (romacs-agent-ui--run-settle)
      (should (equal (romacs-agent-cards-tests--visible) (format "  ✓ ⇢ Review it  %s  thread\n" (capitalize status))))
      (goto-char (point-min))
      (romacs-agent-chat-toggle)
      (should (string-suffix-p "REVIEW RESULT\n" (romacs-agent-cards-tests--visible))))))

(ert-deftest romacs-agent-history-shows-a-delegation-card ()
  (let ((file (make-temp-file "romacs-cards" nil ".jsonl")) (buffer nil) (romacs-agent-glyphs nil))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\","
                    "\"name\":\"mcp__romacs__romacs_delegate_task\",\"input\":{\"title\":\"Review it\"}}]}}\n"
                    "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t1\","
                    "\"content\":\"{\\\"taskId\\\":\\\"" romacs-agent-cards-tests--child "\\\",\\\"status\\\":\\\"running\\\"}\"}]}}\n"))
          (setq buffer (romacs-agent-history-open (list :path file :title "h" :harness 'claude :id "h1" :cwd "/")))
          (with-current-buffer buffer
            (should (string-match-p "⇢ Review it  Working  thread" (buffer-string)))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file file))))

(ert-deftest romacs-agent-failed-wake-shows-failed ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (romacs-agent-cards-tests--draw session :type 'input :seq 9
                                    :text (romacs-agent-tools--wake-text child '(7 "no" t)))
    (should (string-match-p "Review it  codex  Failed" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-user-text-shaped-like-a-wake-stays-a-prompt ()
  ;; A task this session did not delegate, or none: the user's own words.
  (romacs-agent-cards-tests--with-chat session
    (let ((text "Delegated task 99999999-2222-4333-8444-555555555555 (x) [completed]:\nhi"))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 1)
      (should (equal (romacs-agent-cards-tests--visible) (concat "> " text "\n")))
      (should (text-property-not-all (point-min) (point-max) 'romacs-agent-turn nil)))))

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
    (romacs-agent-ui--run-settle)
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Find callers  Explore\n"))
    (should-not (text-property-any (point-min) (point-max) 'romacs-agent-chip t))
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "found 3\n" (romacs-agent-cards-tests--visible)))))

;; Runs: the steps between words, one row each.

(defun romacs-agent-cards-tests--call (session id name input &optional state output time)
  "Draw call ID of tool NAME on INPUT (a command), and its end in STATE."
  (romacs-agent-cards-tests--draw session :type 'tool :state 'started :id id :name name
                                  :input (romacs-agent-cards-tests--table "command" input) :time time)
  (when state
    (romacs-agent-cards-tests--draw session :type 'tool :state state :id id :output (or output "") :time time)))

(ert-deftest romacs-agent-a-call-folds-into-its-runs-row ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--call session "t3" "Bash" "ls" 'completed "{\"taskId\":\"x\"}")
    ;; While its run is open, the live rule says it, not the transcript.
    (should (equal (romacs-agent-cards-tests--visible) ""))
    (romacs-agent-ui--run-settle)
    (should (equal (romacs-agent-cards-tests--visible) "▸ Bash  ls\n"))
    ;; TAB on the run's row shows its call; on the call's, its output.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "▾ Bash  ls\n  ✓ Bash  ls\n"))
    (forward-line 1)
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "{\"taskId\":\"x\"}\n" (romacs-agent-cards-tests--visible)))
    ;; Folding the run folds what it shows; showing it again, as it was.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "▸ Bash  ls\n"))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "  ✓ Bash  ls\n{\"taskId\":\"x\"}\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-run-says-what-it-did-and-failures-stay ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'delta :text "Looking.")
    (romacs-agent-cards-tests--call session "a" "Bash" "ls" 'completed "a b" 100.0)
    (romacs-agent-cards-tests--call session "b" "Bash" "make" 'failed "error 2" 110.0)
    (romacs-agent-cards-tests--call session "c" "Read" "x.el" 'completed "" 142.5)
    (romacs-agent-cards-tests--draw session :type 'delta :text "It failed.")
    ;; Two quiet calls in 42 s; the failed one shows, folded or not.
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▸ 2 tools · 42s\n  ! Bash  make\nIt failed."))
    (goto-char (point-min))
    (forward-line 1)
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▾ 2 tools · 42s\n  ✓ Bash  ls\n  ! Bash  make\n  ✓ Read  x.el\nIt failed."))
    ;; Collapsing all folds it again, the failure still shown.
    (romacs-agent-chat-collapse-all)
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▸ 2 tools · 42s\n  ! Bash  make\nIt failed."))))

(ert-deftest romacs-agent-a-failure-after-an-approval-leaves-the-runs-count ()
  ;; An approval the user is asked settles the run mid-call; the call
  ;; denied then fails.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--call session "a" "Bash" "ls")
    (romacs-agent-cards-tests--call session "b" "Bash" "rm x")
    (let ((romacs-agent-approval-policy 'ask))
      (romacs-agent-cards-tests--draw session :type 'approval :request-id "q" :tool "Bash"
                                      :input (romacs-agent-cards-tests--table "command" "rm x")))
    (should (string-prefix-p "▸ 2 tools\n" (romacs-agent-cards-tests--visible)))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "a" :output "")
    (romacs-agent-cards-tests--draw session :type 'tool :state 'failed :id "b" :output "denied")
    (should (string-prefix-p "▸ Bash  ls\n  ! Bash  rm x\n◆ Bash asks" (romacs-agent-cards-tests--visible)))
    ;; Shown, the run's row says so as it counts again.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (romacs-agent-cards-tests--draw session :type 'tool :state 'failed :id "a" :output "late")
    (should (string-prefix-p "  ! Bash  ls\n  ! Bash  rm x\n◆ Bash asks" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-live-rule-names-a-call-still-running ()
  (romacs-agent-cards-tests--with-chat session
    (setf (romacs-agent-turn-start session) (float-time))
    (setq romacs-agent-ui--composer (make-overlay (point-max) (point-max) nil nil nil))
    (cl-flet ((rule () (substring-no-properties (overlay-get romacs-agent-ui--composer 'before-string))))
      (romacs-agent-cards-tests--call session "a" "Grep" "first")
      (romacs-agent-cards-tests--call session "b" "Grep" "second")
      (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "b" :output "")
      (should (string-search "· Grep  first ·" (rule)))
      (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "a" :output "")
      (should-not (string-search "Grep" (rule))))))

(ert-deftest romacs-agent-an-open-run-only-grows-the-transcript ()
  ;; Calm: what shows above an open run never moves, and its settling
  ;; adds to the end alone.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'delta :text "First words.")
    (romacs-agent-cards-tests--draw session :type 'thinking :text "Weigh it.")
    (romacs-agent-cards-tests--call session "a" "Bash" "ls")
    (should (equal (romacs-agent-cards-tests--visible) "First words.\n"))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "a" :output "x")
    (romacs-agent-cards-tests--call session "b" "Bash" "pwd" 'completed "/")
    (should (equal (romacs-agent-cards-tests--visible) "First words.\n"))
    (romacs-agent-cards-tests--draw session :type 'delta :text "Next.")
    (should (equal (romacs-agent-cards-tests--visible) "First words.\n▸ 2 tools\nNext."))))

(ert-deftest romacs-agent-live-rule-says-what-the-run-does ()
  (romacs-agent-cards-tests--with-chat session
    (setf (romacs-agent-turn-start session) (float-time))
    (setq romacs-agent-ui--composer (make-overlay (point-max) (point-max) nil nil nil))
    (cl-flet ((rule () (substring-no-properties (overlay-get romacs-agent-ui--composer 'before-string))))
      (romacs-agent-cards-tests--draw session :type 'thinking :text "Weigh it.")
      (should (string-search "● Running 0s · thinking ─" (rule)))
      (romacs-agent-cards-tests--call session "a" "Bash" "ls" 'completed "x")
      (romacs-agent-cards-tests--call session "b" "Grep" "TODO")
      (should (string-search "● Running 0s · Grep  TODO · 2 tools ─" (rule)))
      ;; Its row starts the line: the rule needs no newline before it.
      (should (string-prefix-p "──" (rule)))
      (romacs-agent-cards-tests--draw session :type 'delta :text "Found it.")
      (should (string-match-p "\\`\n── ● Running 0s ─" (rule))))))

(ert-deftest romacs-agent-thinking-is-a-state-not-a-row ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'thinking :text "  ")
    (romacs-agent-cards-tests--draw session :type 'thinking :text "Weigh ")
    (romacs-agent-cards-tests--draw session :type 'thinking :text "it all.")
    (romacs-agent-cards-tests--draw session :type 'delta :text "Answer.")
    ;; Thinking straight into words leaves nothing behind.
    (should (equal (romacs-agent-cards-tests--visible) "Answer."))
    ;; Expanding all shows it, where it was; collapsing folds it again.
    (romacs-agent-chat-expand-all)
    (should (equal (romacs-agent-cards-tests--visible) "  ∴ Weigh it all.\nAnswer."))
    (romacs-agent-chat-collapse-all)
    (should (equal (romacs-agent-cards-tests--visible) "Answer."))
    ;; Thinking among calls is one of the run's steps.
    (romacs-agent-cards-tests--draw session :type 'thinking :text "Consider.\n")
    (romacs-agent-cards-tests--call session "r1" "Read" "/a" 'completed "")
    (romacs-agent-cards-tests--draw session :type 'delta :text "Done.")
    (should (equal (romacs-agent-cards-tests--visible) "Answer.\n▸ Read  /a\nDone."))
    (goto-char (text-property-not-all (point-min) (point-max) 'romacs-agent-run-head nil))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "Answer.\n▾ Read  /a\n  ∴ Consider.\n  ✓ Read  /a\nDone."))
    (should-not (string-search "thinking" (buffer-string)))))

(ert-deftest romacs-agent-subagent-words-are-its-calls-steps ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "t2" :name "Agent"
     :input (romacs-agent-cards-tests--table "description" "Find callers" "subagent_type" "Explore"))
    (puthash "t2" t (romacs-agent-tools session))
    (romacs-agent-cards-tests--draw session :type 'delta :parent "t2" :text "Searching the tree.")
    (puthash "g" "t2" (romacs-agent-tools session))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'started :id "g" :name "Grep" :parent "t2"
                                    :input (romacs-agent-cards-tests--table "pattern" "callers"))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "g" :output "3")
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "t2" :output "found 3")
    (romacs-agent-cards-tests--draw session :type 'delta :text "Three callers.")
    ;; The card shows; its subagent's words and calls are its steps,
    ;; not the run's own calls.
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Find callers  Explore\nThree callers."))
    (romacs-agent-chat-expand-all)
    (should (string-search "    Searching the tree.\n    ✓ Grep  callers\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-header-says-only-what-is-worth-a-glance ()
  (romacs-agent-cards-tests--with-chat session
    (setf (romacs-agent-model session) "claude-opus-5-5" (romacs-agent-permission-mode session) "bypassPermissions"
          (romacs-agent-options session) '(:effort "high")
          (romacs-agent-context-used session) 440000 (romacs-agent-context-window session) 1000000
          (romacs-agent-provider session) "claude")
    (let ((romacs-agent-glyphs nil) (romacs-agent-default-provider "claude"))
      (cl-flet ((header () (substring-no-properties
                            (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs session)))))
        (should (equal (header) (concat (romacs-agent-ui--harness-icon 'claude) "  Working  opus-5-5 high  44%")))
        ;; A mode other than bypassing approvals is worth a glance.
        (setf (romacs-agent-permission-mode session) "plan")
        (should (string-search "  plan  " (header)))
        ;; Another instance of the harness is named, after what matters more.
        (setf (romacs-agent-provider session) "claude2")
        (should (string-search "  opus-5-5 high  claude2  plan  " (header)))))))

(ert-deftest romacs-agent-composer-starts-on-its-own-line ()
  ;; A reply left open while no turn runs: the composer is not glued to it.
  (romacs-agent-cards-tests--with-chat session
    (setq romacs-agent-ui--composer (make-overlay (point-max) (point-max) nil nil nil))
    (romacs-agent-cards-tests--draw session :type 'delta :text "Half a reply")
    (setf (romacs-agent-state session) 'idle)
    (romacs-agent-ui--composer-update)
    (should (string-prefix-p "\n" (overlay-get romacs-agent-ui--composer 'before-string)))
    ;; At a line's start, nothing is added; a running turn adds its rule.
    (romacs-agent-ui--insert "\n")
    (romacs-agent-ui--composer-update)
    (should (equal (overlay-get romacs-agent-ui--composer 'before-string) ""))
    (setf (romacs-agent-state session) 'running)
    (romacs-agent-ui--composer-update)
    (should (string-match-p "Running" (overlay-get romacs-agent-ui--composer 'before-string)))))

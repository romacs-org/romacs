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
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ◈ ⇢ Review it · codex  Working  thread\n"))
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
    (should (string-search "⇠ child · Waiting on its workers  thread" (romacs-agent-cards-tests--visible)))
    (setf (romacs-agent-children-rows child) nil)
    (let ((text (romacs-agent-tools--wake-text child '(8 "done" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 10))
    (should (string-search "⇠ child · Completed  thread  done" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-wake-marks-where-its-rows-start ()
  ;; A wake settles a card above as it draws its own row: the input it is
  ;; marks its row's start, where a chat cuts its oldest turns.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (setf (romacs-agent-children-rows child) (list (list :key 'working :state "running" :title "check")))
    (romacs-agent-cards-tests--draw session :type 'input :seq 9
                                    :text (romacs-agent-tools--wake-text child '(7 "waiting on the check" nil)))
    (setf (romacs-agent-children-rows child) nil)
    (romacs-agent-cards-tests--draw session :type 'input :seq 10
                                    :text (romacs-agent-tools--wake-text child '(8 "done" nil)))
    (let ((starts (romacs-agent-ui--input-starts)))
      (should (= (length starts) 2))
      (dolist (at starts)
        (goto-char at)
        (should (looking-at-p "⇠ child"))))))

(ert-deftest romacs-agent-wake-settles-the-card-and-is-no-prompt ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (let ((text (romacs-agent-tools--wake-text child '(7 "all good\nsecond line" nil))))
      (romacs-agent-cards-tests--draw session :type 'input :text text :seq 9))
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "  ✓ ◈ ⇢ Review it · codex  Completed  thread\n"
                           "⇠ child · Completed  thread  all good\n")))
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
      (should (equal (romacs-agent-cards-tests--visible) (format "  ✓ ◈ ⇢ Review it  %s  thread\n" (capitalize status))))
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
    (should (string-match-p "Review it · codex  Failed" (romacs-agent-cards-tests--visible)))))

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
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Find callers · Explore\n"))
    (should-not (text-property-any (point-min) (point-max) 'romacs-agent-chip t))
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "found 3\n" (romacs-agent-cards-tests--visible)))))

;; Runs: the steps between words, one row each.

(defun romacs-agent-cards-tests--call (session id name input &optional state output time)
  "Draw call ID of tool NAME on INPUT (its command, file or pattern), and
its end in STATE."
  (romacs-agent-cards-tests--draw session :type 'tool :state 'started :id id :name name
                                  :input (romacs-agent-cards-tests--table
                                          (pcase name ("Read" "file_path") ("Grep" "pattern") (_ "command")) input)
                                  :time time)
  (when state
    (romacs-agent-cards-tests--draw session :type 'tool :state state :id id :output (or output "") :time time)))

(ert-deftest romacs-agent-a-call-folds-into-its-runs-row ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--call session "t3" "Bash" "ls" 'completed "{\"taskId\":\"x\"}")
    ;; While its run is open, the live rule says it, not the transcript.
    (should (equal (romacs-agent-cards-tests--visible) ""))
    (romacs-agent-ui--run-settle)
    (should (equal (romacs-agent-cards-tests--visible) "▸ $ ls\n"))
    ;; TAB on the run's row shows its call; on the call's, its output.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "▾ $ ls\n  ✓ $ ls\n"))
    (forward-line 1)
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "{\"taskId\":\"x\"}\n" (romacs-agent-cards-tests--visible)))
    ;; Folding the run folds what it shows; showing it again, as it was.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "▸ $ ls\n"))
    (romacs-agent-chat-toggle)
    (should (string-suffix-p "  ✓ $ ls\n{\"taskId\":\"x\"}\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-a-call-drawn-anew-is-its-runs-row ()
  ;; A renderer's words at the call's end are the run's row too, and its
  ;; colours stay over the settled row's dim.
  (romacs-agent-cards-tests--with-chat session
    (let ((romacs-icons-ascii t))
      (romacs-agent-cards-tests--draw
       session :type 'tool :state 'started :id "d" :name "mcp__romacs__romacs_dev_check"
       :input (romacs-agent-cards-tests--table "worktree" "/w/tree/"))
      (romacs-agent-cards-tests--draw
       session :type 'tool :state 'completed :id "d"
       :output (json-serialize (romacs-agent-cards-tests--table "verdict" "ok" "report" "lint: ok\ndev-check: ok")))
      (romacs-agent-ui--run-settle)
      (should (equal (romacs-agent-cards-tests--visible) "▸ ◈ ⚑ dev-check ok · 1/1 stages\n"))
      (goto-char (point-min))
      (romacs-agent-chat-toggle)
      (search-forward "dev-check ok")
      (should (eq (car (ensure-list (get-text-property (1- (point)) 'face))) 'romacs-agent-done)))))

(ert-deftest romacs-agent-run-says-what-it-did-and-failures-stay ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'delta :text "Looking.")
    (romacs-agent-cards-tests--call session "a" "Bash" "ls" 'completed "a b" 100.0)
    (romacs-agent-cards-tests--call session "b" "Bash" "make" 'failed "error 2" 110.0)
    (romacs-agent-cards-tests--call session "c" "Read" "x.el" 'completed "" 142.5)
    (romacs-agent-cards-tests--draw session :type 'delta :text "It failed.")
    ;; Two quiet calls in 42 s; the failed one shows, folded or not.
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▸ 2 tools · 42s\n  ! $ make\nIt failed."))
    (goto-char (point-min))
    (forward-line 1)
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▾ 2 tools · 42s\n  ✓ $ ls\n  ! $ make\n  ✓ ◧ x.el\nIt failed."))
    ;; Collapsing all folds it again, the failure still shown.
    (romacs-agent-chat-collapse-all)
    (should (equal (romacs-agent-cards-tests--visible)
                   "Looking.\n▸ 2 tools · 42s\n  ! $ make\nIt failed."))))

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
    (should (string-prefix-p "▸ $ ls\n  ! $ rm x\n◆ Bash asks" (romacs-agent-cards-tests--visible)))
    ;; Shown, the run's row says so as it counts again.
    (goto-char (point-min))
    (romacs-agent-chat-toggle)
    (romacs-agent-cards-tests--draw session :type 'tool :state 'failed :id "a" :output "late")
    (should (string-prefix-p "  ! $ ls\n  ! $ rm x\n◆ Bash asks" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-live-rule-names-a-call-still-running ()
  (romacs-agent-cards-tests--with-chat session
    (setf (romacs-agent-turn-start session) (float-time))
    (setq romacs-agent-ui--composer (make-overlay (point-max) (point-max) nil nil nil))
    (cl-flet ((rule () (substring-no-properties (overlay-get romacs-agent-ui--composer 'before-string))))
      (romacs-agent-cards-tests--call session "a" "Grep" "first")
      (romacs-agent-cards-tests--call session "b" "Grep" "second")
      (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "b" :output "")
      (should (string-search "· ⌕ first ·" (rule)))
      (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "a" :output "")
      (should-not (string-search "⌕" (rule))))))

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
      (should (string-search "● Running 0s · ⌕ TODO · 2 tools ─" (rule)))
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
    (should (equal (romacs-agent-cards-tests--visible) "Answer.\n▸ ◧ a  /\nDone."))
    (goto-char (text-property-not-all (point-min) (point-max) 'romacs-agent-run-head nil))
    (romacs-agent-chat-toggle)
    (should (equal (romacs-agent-cards-tests--visible) "Answer.\n▾ ◧ a  /\n  ∴ Consider.\n  ✓ ◧ a  /\nDone."))
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
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ⇢ Find callers · Explore\nThree callers."))
    (romacs-agent-chat-expand-all)
    (should (string-search "    Searching the tree.\n    ✓ ⌕ callers · 1 line\n" (romacs-agent-cards-tests--visible)))))

;; Messages between sessions: cards drawn from their record, never prompts.

(defun romacs-agent-cards-tests--message (id kind &rest from)
  "A message (`romacs-agent-make-message') ID of KIND, from FROM: :session,
:name, :address."
  (list :id id :kind kind
        :from (list :session (plist-get from :session) :name (or (plist-get from :name) "a session outside Romacs")
                    :address (plist-get from :address))))

(defun romacs-agent-cards-tests--receive (session text message)
  "SESSION's input of TEXT, MESSAGE's delivery: the message keeps its text."
  (romacs-agent-cards-tests--draw session :type 'input :text text :request 4 :seq 4
                                  :messages (list (append message (list :text text)))))

(ert-deftest romacs-agent-a-message-is-a-card-not-a-prompt ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--receive
     session "all good\nsecond line"
     (romacs-agent-cards-tests--message "m-1" "child" :session romacs-agent-cards-tests--child :name "old name"))
    ;; The sender by its title now, its kind, its thread, the reply hint,
    ;; then the text, quoted: the bar is not copied.
    (should (equal (romacs-agent-cards-tests--visible) "✉ child · child  thread  ↩\n┃ all good\n┃ second line\n"))
    (should-not (text-property-any (point-min) (point-max) 'face 'romacs-agent-prompt))
    ;; No prompt, as a task's result is none: n and p pass it.
    (should-not (text-property-not-all (point-min) (point-max) 'romacs-agent-turn nil))
    ;; RET on the link opens the sender's thread.
    (goto-char (point-min))
    (let ((shown nil))
      (cl-letf (((symbol-function 'romacs-agent-show) (lambda (s) (setq shown s))))
        (romacs-agent-chat-open-task))
      (should (equal (romacs-agent-id shown) romacs-agent-cards-tests--child)))))

(ert-deftest romacs-agent-an-outside-message-says-what-it-can ()
  (romacs-agent-cards-tests--with-chat session
    ;; No route back: no thread, no hint.
    (romacs-agent-cards-tests--receive session "hello" (romacs-agent-cards-tests--message "m-1" "external"))
    (should (equal (romacs-agent-cards-tests--visible) "✉ a session outside Romacs · external\n┃ hello\n"))
    ;; A Claude Code session's, with an address: answerable, no thread here.
    (romacs-agent-cards-tests--receive
     session "ping" (romacs-agent-cards-tests--message "m-2" "external" :name "dotfiles" :address "dotfiles [ab12]"))
    (should (string-suffix-p "✉ dotfiles · external  ↩\n┃ ping\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-a-reply-is-a-card-and-answers-its-message ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--receive
     session "Can you rebase?" (romacs-agent-cards-tests--message "m-1" "child" :session romacs-agent-cards-tests--child))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "r" :name "mcp__romacs__romacs_task_reply"
     :input (romacs-agent-cards-tests--table "messageId" "m-1" "text" "Rebased.\nAll green."))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'completed :id "r"
     :output (format "{\"taskId\":\"%s\",\"title\":\"child\",\"status\":\"idle\",\"messageId\":\"m-2\"}"
                     romacs-agent-cards-tests--child))
    (romacs-agent-cards-tests--draw session :type 'delta :text "Done.")
    ;; Answered; the reply goes to the message's sender, loud in its run,
    ;; the message quoted under it; and copied, it has no bar.
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "✉ child · child  thread  ↩ ✓\n┃ Can you rebase?\n"
                           "  ✓ ◈ ✉ to child  idle  thread\n┃ Rebased.\n┃ All green.\nDone.")))
    (should (string-search "Can you rebase?\n" (romacs-agent-ui--text-of (point-min) (point-max))))
    (should-not (string-search "┃" (romacs-agent-ui--text-of (point-min) (point-max))))))

(ert-deftest romacs-agent-a-result-as-a-record-is-its-arrival ()
  ;; A delivery of a result and a message: each its own row, in order.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--delegate session)
    (romacs-agent-cards-tests--draw
     session :type 'input :text "both" :seq 9
     :messages (list (append (romacs-agent-cards-tests--message "m-1" "child" :session romacs-agent-cards-tests--child
                                                                :name "child")
                             (list :text "all good" :wake (list :state "completed" :chars 8)))
                     (append (romacs-agent-cards-tests--message "m-2" "child" :session romacs-agent-cards-tests--child)
                             (list :text "one more thing"))))
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "  ✓ ◈ ⇢ Review it · codex  Completed  thread\n"
                           "⇠ child · Completed  thread  all good\n"
                           "✉ child · child  thread  ↩\n┃ one more thing\n")))
    ;; A reply to the result goes to its task, named by the task's record.
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "r" :name "mcp__romacs__romacs_task_reply"
     :input (romacs-agent-cards-tests--table "messageId" "m-1" "text" "Thanks."))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'completed :id "r"
     :output (format "{\"taskId\":\"%s\",\"title\":\"child\",\"status\":\"idle\"}" romacs-agent-cards-tests--child))
    (romacs-agent-ui--run-settle)
    (should (string-suffix-p "  ✓ ◈ ✉ to child  idle  thread\n┃ Thanks.\n" (romacs-agent-cards-tests--visible)))))

(ert-deftest romacs-agent-an-answered-message-links-its-reply ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--receive
     session "Can you rebase?" (romacs-agent-cards-tests--message "m-1" "child" :session romacs-agent-cards-tests--child))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "r" :name "mcp__romacs__romacs_task_reply"
     :input (romacs-agent-cards-tests--table "messageId" "m-1" "text" "Rebased."))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'completed :id "r" :output "{}")
    (romacs-agent-cards-tests--draw session :type 'delta :text "Done.")
    ;; RET on ↩ ✓ goes to the reply's row.
    (goto-char (point-min))
    (search-forward "↩ ✓")
    (goto-char (match-beginning 0))
    (should (eq (key-binding (kbd "RET")) #'romacs-agent-chat-goto-reply))
    (romacs-agent-chat-goto-reply)
    (should (eq (point) (marker-position (gethash "r" romacs-agent-ui--tool-lines))))))

(ert-deftest romacs-agent-a-failed-reply-keeps-its-message ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--receive
     session "Can you rebase?" (romacs-agent-cards-tests--message "m-1" "child" :session romacs-agent-cards-tests--child))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "r" :name "mcp__romacs__romacs_task_reply"
     :input (romacs-agent-cards-tests--table "messageId" "m-1" "text" "Rebased."))
    (romacs-agent-cards-tests--draw session :type 'tool :state 'failed :id "r" :output "Message m-1's sender is gone")
    (romacs-agent-ui--run-settle)
    ;; Not answered; the failure is loud, and shows the message and why.
    (should (equal (romacs-agent-cards-tests--visible)
                   (concat "✉ child · child  thread  ↩\n┃ Can you rebase?\n"
                           "  ! ◈ ✉ reply\n┃ Rebased.\n\nMessage m-1's sender is gone\n")))))

(ert-deftest romacs-agent-a-message-sent-is-a-card ()
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "s" :name "mcp__romacs__romacs_task_send"
     :input (romacs-agent-cards-tests--table "taskId" romacs-agent-cards-tests--child "message" "Rebase, please."))
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'completed :id "s"
     :output (format "{\"taskId\":\"%s\",\"status\":\"running\",\"messageId\":\"m-9\"}" romacs-agent-cards-tests--child))
    (romacs-agent-ui--run-settle)
    (should (equal (romacs-agent-cards-tests--visible) "  ✓ ◈ ✉ to child  Working  thread\n┃ Rebase, please.\n"))))

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
        ;; Another account of the harness is named only on hover: the harness is what shows.
        (setf (romacs-agent-provider session) "claude2")
        (should-not (string-search "claude2" (header)))
        (let ((text (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs session))))
          (should (equal (get-text-property (string-search "opus-5-5" text) 'help-echo text) "account claude2")))))))

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

(ert-deftest romacs-agent-tool-rows-are-one-line ()
  ;; A renderer's words with a newline in them stay on the row's line.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw
     session :type 'tool :state 'started :id "t1" :name "TodoWrite"
     :input (romacs-agent-cards-tests--table
             "todos" (vector (romacs-agent-cards-tests--table "content" "a" "activeForm" "Fixing parser\nand tests"
                                                              "status" "in_progress"))))
    (should (string-search "Fixing parser and tests" (buffer-string)))))

(ert-deftest romacs-agent-a-steer-is-its-own-quiet-row ()
  ;; A message written into a turn under way: ↪ and its words, dim, no
  ;; prompt band; it is an input as a prompt is.
  (romacs-agent-cards-tests--with-chat session
    (romacs-agent-cards-tests--draw session :type 'input :text "do X first" :request 3 :seq 1)
    (romacs-agent-cards-tests--draw session :type 'delta :text "On it." :seq 2)
    (romacs-agent-cards-tests--draw session :type 'input :text "also Y" :request 4 :steer t :seq 3)
    (let ((text (buffer-string)))
      (should (string-search "\n↪ also Y\n" text))
      (should-not (string-search "> also Y" text)))
    (goto-char (point-min))
    (search-forward "also Y")
    (should (eq (get-text-property (1- (point)) 'face) 'romacs-agent-steer))
    (should-not (memq 'romacs-agent-prompt (ensure-list (get-text-property (1- (point)) 'face))))
    (should (= (length (romacs-agent-ui--input-starts)) 2))))

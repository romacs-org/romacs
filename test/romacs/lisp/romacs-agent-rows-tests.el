;;; romacs-agent-rows-tests.el --- How a chat draws Romacs's own tools -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-agent-rows)

(defun romacs-agent-rows-tests--table (&rest pairs)
  (let ((table (make-hash-table :test #'equal)))
    (while pairs (puthash (pop pairs) (pop pairs) table))
    table))

(defun romacs-agent-rows-tests--draw (name key &rest args)
  "What tool NAME's renderer's KEY function draws for ARGS, as plain text."
  (let ((value (apply (plist-get (romacs-agent-rows-get name) key) args)))
    (and value (substring-no-properties value))))

(defmacro romacs-agent-rows-tests--plain (&rest body)
  "BODY with no icon font: rows read as their words."
  `(let ((romacs-icons-ascii t)) ,@body))

(ert-deftest romacs-agent-rows-romacs-tools-never-show-json ()
  (romacs-agent-rows-tests--plain
   ;; A Romacs tool with no renderer of its own: its name as words and
   ;; what it was given, no braces.
   (let ((row (romacs-agent-rows-tests--draw
               "romacs_buffer_read" :row (romacs-agent-rows-tests--table "instance" "i:1" "name" "notes" "start" 1))))
     (should (equal row "◈ buffer read  notes · 1")))
   ;; A harness's tool with no renderer keeps the chat's generic row.
   (should-not (romacs-agent-rows-get "NotebookRead"))))

(ert-deftest romacs-agent-rows-harness-tools-are-one-line ()
  (let ((T #'romacs-agent-rows-tests--table) (draw #'romacs-agent-rows-tests--draw))
    ;; A file named without its directory.
    (should (equal (funcall draw "Read" :row (funcall T "file_path" "x.el")) "◧ x.el"))
    (should (equal (funcall draw "Bash" :row (funcall T "command" "make check\nmore" "description" "x"))
                   "$ make check"))
    ;; Codex's argument lists: a shell's script alone.
    (should (equal (funcall draw "Bash" :row (funcall T "command" ["bash" "-lc" "make check"])) "$ make check"))
    (should (equal (funcall draw "Bash" :row (funcall T "command" ["git" "status"])) "$ git status"))
    ;; Edits counted one by one: two deletions add nothing.
    (should (equal (funcall draw "MultiEdit" :row
                            (funcall T "file_path" "/w/a.el"
                                     "edits" (vector (funcall T "old_string" "a" "new_string" "")
                                                     (funcall T "old_string" "b" "new_string" ""))))
                   "✎ a.el  /w  +0 −2"))
    ;; Nothing found is none.
    (should (equal (funcall draw "Grep" :ended (funcall T "pattern" "x") "No matches found") "⌕ x · 0 lines"))
    (should (equal (funcall draw "Glob" :ended (funcall T "pattern" "x") "No files found") "⌕ x · 0 files"))
    ;; A question's end with no questions given keeps its row.
    (should (equal (funcall draw "AskUserQuestion" :ended (funcall T) "\"Q\"=\"A\"") "? a question  → A"))
    (should (equal (funcall draw "Read" :row (funcall T "file_path" "/w/r/lisp/a.el" "offset" 10 "limit" 5))
                   "◧ a.el:10-14  /w/r/lisp"))
    (should (equal (funcall draw "Edit" :row (funcall T "file_path" "/w/a.el" "old_string" "x" "new_string" "y\nz"))
                   "✎ a.el  /w  +2 −1"))
    (should (equal (funcall draw "MultiEdit" :row
                            (funcall T "file_path" "/w/a.el"
                                     "edits" (vector (funcall T "old_string" "a" "new_string" "b")
                                                     (funcall T "old_string" "c" "new_string" "d"))))
                   "✎ a.el  /w  +2 −2"))
    ;; Codex's file changes: the first file and how many more.
    (should (equal (funcall draw "Edit" :row
                            (funcall T "changes" (vector (funcall T "path" "/w/a.el") (funcall T "path" "/w/b.el"))))
                   "✎ a.el  /w  +1 more"))
    (should (equal (funcall draw "Write" :row (funcall T "file_path" "/w/n.el" "content" "1\n2\n3\n"))
                   "✎ n.el  /w  written · 3 lines"))
    (should (equal (funcall draw "Grep" :row (funcall T "pattern" "defun x" "path" "/w/lisp"))
                   "⌕ defun x  in /w/lisp"))
    (should (equal (funcall draw "Grep" :ended (funcall T "pattern" "defun x") "a.el:1\nb.el:2\n")
                   "⌕ defun x · 2 lines"))
    (should (equal (funcall draw "Glob" :ended (funcall T "pattern" "**/*.el") "a.el\n")
                   "⌕ **/*.el · 1 file"))
    (should (equal (funcall draw "WebFetch" :row (funcall T "url" "https://www.gnu.org/s/emacs"))
                   "↗ gnu.org/s/emacs"))
    (should (equal (funcall draw "WebSearch" :row (funcall T "query" "wgpu blur")) "◍ wgpu blur"))
    (should (equal (funcall draw "TodoWrite" :row
                            (funcall T "todos" (vector (funcall T "content" "a" "status" "completed")
                                                       (funcall T "content" "b" "activeForm" "Doing b" "status" "in_progress")
                                                       (funcall T "content" "c" "status" "pending"))))
                   "☐ 1/3 done  Doing b"))
    (let ((asked (funcall T "questions" (vector (funcall T "question" "Which one?")))))
      (should (equal (funcall draw "AskUserQuestion" :row asked) "? Which one?"))
      (should (equal (funcall draw "AskUserQuestion" :ended asked
                              "User has answered your questions: \"Which one?\"=\"Tail\". You can now continue.")
                     "? Which one?  → Tail")))))

(ert-deftest romacs-agent-rows-messages-and-delegations ()
  (romacs-agent-rows-tests--plain
   (should (equal (romacs-agent-rows-tests--draw "romacs_task_send" :row (romacs-agent-rows-tests--table "taskId" "gone"))
                  "◈ ✉ to a task"))
   (should (equal (romacs-agent-rows-tests--draw "romacs_task_send" :body
                                                 (romacs-agent-rows-tests--table "message" "one\ntwo") "{}")
                  "┃ one\n┃ two"))
   (should (equal (romacs-agent-rows-tests--draw "romacs_task_reply" :ended (romacs-agent-rows-tests--table)
                                                 "{\"title\":\"lane x\"}")
                  "◈ ✉ to lane x"))
   (should (equal (funcall (plist-get (romacs-agent-rows-get "romacs_task_reply") :answers)
                           (romacs-agent-rows-tests--table "messageId" "m-1"))
                  "m-1"))
   (let ((input (romacs-agent-rows-tests--table
                 "title" "lane x" "target" (romacs-agent-rows-tests--table "provider" "claude3"))))
     (should (equal (romacs-agent-rows-tests--draw "romacs_delegate_task" :row input) "◈ ⇢ lane x · claude3"))
     ;; The record says what it runs on, the model without its harness's name.
     (should (equal (romacs-agent-rows-tests--draw
                     "romacs_delegate_task" :ended input
                     "{\"title\":\"lane x\",\"model\":\"claude-opus-5-5\",\"harness\":\"claude\",\"provider\":\"claude3\"}")
                    "◈ ⇢ lane x · opus-5-5@claude3")))))

(ert-deftest romacs-agent-rows-tasks-are-one-line ()
  (romacs-agent-rows-tests--plain
   (let ((tasks "{\"tasks\":[{\"title\":\"a\",\"status\":\"running\"},{\"title\":\"b\",\"status\":\"failed\"},{\"title\":\"c\",\"status\":\"completed\"}]}"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_task_list" :ended (romacs-agent-rows-tests--table) tasks)
                    "◈ ◷ 3 tasks · 1 running · 1 failed"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_task_list" :body (romacs-agent-rows-tests--table) tasks)
                    "● a\n! b\n✓ c")))
   (should (equal (romacs-agent-rows-tests--draw "romacs_task_status" :ended (romacs-agent-rows-tests--table "taskId" "t")
                                                 "{\"title\":\"lane x\",\"status\":\"waiting\"}")
                  "◈ ◷ lane x · waiting"))
   (let ((input (romacs-agent-rows-tests--table "unit" "check-1")))
     (should (equal (romacs-agent-rows-tests--draw "romacs_task_watch" :row input) "◈ ◷ watch check-1"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_task_watch" :ended input "{\"watching\":[\"check-1\",\"q.md\",\"x\"]}")
                    "◈ ◷ watching check-1 · 3 watches")))))

(ert-deftest romacs-agent-rows-eval-shows-its-form-coloured ()
  (romacs-agent-rows-tests--plain
   (let* ((input (romacs-agent-rows-tests--table "code" "(defun f ()\n  \"doc\")"))
          (row (funcall (plist-get (romacs-agent-rows-get "romacs_eval") :row) input)))
     ;; One line, in Lisp's colours.
     (should (equal (substring-no-properties row) "◈ λ (defun f () \"doc\")"))
     (should (memq 'font-lock-keyword-face (ensure-list (get-text-property (string-search "defun" row) 'face row))))
     ;; Folded: the whole form, then what it gave.
     (should (equal (romacs-agent-rows-tests--draw "romacs_eval" :body input "f")
                    "(defun f ()\n  \"doc\")\n⇒ f")))))

(ert-deftest romacs-agent-rows-tables-line-up ()
  (romacs-agent-rows-tests--plain
   (let ((providers "{\"providers\":[{\"providerInstanceId\":\"claude\",\"harness\":\"claude\",\"enabled\":true,\"models\":[1,2]},{\"providerInstanceId\":\"codex-second\",\"harness\":\"codex\",\"enabled\":false,\"models\":[]}]}"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_orchestrator_capabilities" :ended (romacs-agent-rows-tests--table) providers)
                    "◈ ▤ 2 instances · 1 enabled"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_orchestrator_capabilities" :body (romacs-agent-rows-tests--table) providers)
                    "✓  claude        claude  2 models\n·  codex-second  codex   0 models")))
   (let ((limits "{\"harnesses\":[{\"harness\":\"claude\",\"suggestion\":\"New work goes to claude2.\",\"accounts\":[{\"account\":\"claude2\",\"pick\":true,\"sessions\":3,\"windows\":[{\"window\":\"5h\",\"used\":58},{\"window\":\"week\",\"used\":27}]}]}]}"))
     (should (equal (romacs-agent-rows-tests--draw "romacs_limits" :ended (romacs-agent-rows-tests--table) limits)
                    "◈ ▤ limits · New work goes to claude2."))
     (should (equal (romacs-agent-rows-tests--draw "romacs_limits" :body (romacs-agent-rows-tests--table) limits)
                    "●  claude2  5h 58%  week 27%  3 sessions")))
   ;; No account: no table, and no error.
   (should (equal (romacs-agent-rows-tests--draw "romacs_limits" :body (romacs-agent-rows-tests--table)
                                                 "{\"harnesses\":[{\"harness\":\"claude\",\"accounts\":[]}]}")
                  ""))))

(ert-deftest romacs-agent-rows-restart-and-dev-check ()
  (romacs-agent-rows-tests--plain
   (should (equal (romacs-agent-rows-tests--draw "romacs_restart_request" :row (romacs-agent-rows-tests--table))
                  "◈ ↻ restart after this turn"))
   (let ((input (romacs-agent-rows-tests--table "worktree" "/w/agent-chat-ui/")))
     (should (equal (romacs-agent-rows-tests--draw "romacs_dev_check" :row input) "◈ ⚑ dev-check · agent-chat-ui"))
     (should (equal (romacs-agent-rows-tests--draw
                     "romacs_dev_check" :ended input
                     (json-serialize (romacs-agent-rows-tests--table
                                      "verdict" "ok" "report" "forecast: ok merges\ngenerated: ok none\nlint: ok 2 files\ndev-check: ok")))
                    "◈ ⚑ dev-check ok · 3/3 stages"))
     (should (equal (romacs-agent-rows-tests--draw
                     "romacs_dev_check" :ended input
                     (json-serialize (romacs-agent-rows-tests--table
                                      "verdict" "fail" "report" "forecast: ok merges\nlint: FAIL 1 warning\ndev-check: FAIL (lint)")))
                    "◈ ⚑ dev-check fail · lint")))))

(ert-deftest romacs-agent-rows-a-new-tool-adds-a-renderer ()
  (unwind-protect
      (progn
        (romacs-agent-rows-define "romacs_test_tool" :row (lambda (_input) "drawn"))
        (should (equal (romacs-agent-rows-tests--draw "romacs_test_tool" :row (romacs-agent-rows-tests--table)) "drawn")))
    (remhash "romacs_test_tool" romacs-agent-rows)))

(ert-deftest romacs-agent-rows-say-a-form-and-an-answer-once ()
  ;; A short form is its row alone: under it, its value.
  (should (equal (romacs-agent-rows-tests--draw "romacs_eval" :body (romacs-agent-rows-tests--table "code" "(buffer-size)") "42")
                 "⇒ 42"))
  ;; A Romacs tool's answer is words, never JSON.
  (should (equal (romacs-agent-rows-tests--draw "romacs_task_status" :body (romacs-agent-rows-tests--table)
                                                "{\"title\":\"lane x\",\"status\":\"running\"}")
                 ""))
  (should (equal (romacs-agent-rows-tests--draw "romacs_buffer_read" :body (romacs-agent-rows-tests--table)
                                                "{\"name\":\"notes\",\"size\":12,\"lines\":[\"a\",\"b\"],\"rows\":[{\"x\":1}]}")
                 "name  notes\nsize  12\nlines  a, b\nrows  1 items"))
  (should (equal (romacs-agent-rows-tests--draw "romacs_buffer_read" :body (romacs-agent-rows-tests--table) "plain text")
                 "plain text"))
  ;; A text of several lines, whole, under its key.
  (should (equal (romacs-agent-rows-tests--draw "romacs_buffer_read" :body (romacs-agent-rows-tests--table)
                                                "{\"text\":\"one\\ntwo\\nthree\"}")
                 "text  \n  one\n  two\n  three"))
  ;; A renderer whose body says nothing: the answer as words, here none.
  (should (equal (romacs-agent-rows-tests--draw "romacs_limits" :body (romacs-agent-rows-tests--table) "{\"harnesses\":[]}")
                 "")))

;;; romacs-agent-rows-tests.el ends here

;;; romacs-agent-log-tests.el --- An agent chat as its log -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log))

(defmacro romacs-agent-log-tests--with (&rest body)
  "BODY in a chat of `session', its events drawn by `draw'."
  (declare (indent 0))
  `(let ((session (romacs-agent--make :id "L" :harness 'claude :state 'running
                                      :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal)))
         (romacs-agent-approval-policy 'ask))
     (with-temp-buffer
       (romacs-agent-log-mode)
       (setq romacs-agent-log--session session
             romacs-agent-log--end (point-min-marker)
             romacs-agent-log--input (point-min-marker))
       (set-marker-insertion-type romacs-agent-log--end t)
       (romacs-agent-log--separator t)
       (cl-flet ((draw (&rest event) (romacs-agent-log-draw session event) (romacs-agent-log--separator)))
         ,@body))))

(defun romacs-agent-log-tests--transcript ()
  (buffer-substring-no-properties (point-min) romacs-agent-log--end))

(defun romacs-agent-log-tests--input (&rest pairs)
  (let ((table (make-hash-table :test #'equal)))
    (cl-loop for (k v) on pairs by #'cddr do (puthash k v table))
    table))

(ert-deftest romacs-agent-log-shows-every-step-in-order ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "count the lines")
    (draw :type 'thinking :text "I should run wc.")
    (draw :type 'tool :state 'started :id "t1" :name "Bash"
          :input (romacs-agent-log-tests--input "command" "wc -l notes.txt"))
    (draw :type 'delta :text "Running it now.")
    (draw :type 'tool :state 'completed :id "t1" :output "2 notes.txt")
    (draw :type 'message-end)
    (draw :type 'done :duration 9 :cost 0.12 :time 0)
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "❯ count the lines" text))
      (should (string-match-p "∴ I should run wc." text))
      (should (string-match-p "wc -l notes.txt" text))
      ;; The output under its call, before what followed the call.
      (should (string-match-p "wc -l notes.txt[^\n]*\n  ⎿ 2 notes.txt\nRunning it now." text))
      (should (string-match-p "✻ 9s · \\$0.12" text)))))

(ert-deftest romacs-agent-log-streams-without-blank-trails ()
  (romacs-agent-log-tests--with
    (draw :type 'thinking :text "I think.")
    ;; The separator is on a line of its own while text streams.
    (should (eq (char-before romacs-agent-log--end) ?\n))
    (draw :type 'thinking :text "\n\n")
    (draw :type 'thinking :text "\n\n")
    (draw :type 'delta :text "Para one.\n\n\n\nPara")
    (draw :type 'delta :text " two.\n\n")
    (draw :type 'message-end)
    (draw :type 'input :text "next")
    (should (equal (romacs-agent-log-tests--transcript)
                   "∴ I think.\nPara one.\n\nPara two.\n\n❯ next\n"))))

(ert-deftest romacs-agent-log-long-output-says-how-much-more ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log-output-lines 2))
      (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "seq 5"))
      (draw :type 'tool :state 'completed :id "t" :output "1\n2\n3\n4\n5")
      (let ((text (romacs-agent-log-tests--transcript)))
        (should (string-match-p "⎿ 1\n    2\n    … 3 more lines · RET shows all" text))
        (should-not (string-match-p "\n    3\n" text))
        ;; RET shows all of it.
        (goto-char (point-min))
        (search-forward "more lines")
        (save-window-excursion (romacs-agent-log-open))
        (with-current-buffer "*agent output*"
          (should (string-match-p "seq 5" (buffer-string)))
          (should (string-match-p "Output:\n1\n2\n3\n4\n5" (buffer-string)))
          (kill-buffer))))))

(ert-deftest romacs-agent-log-a-running-command-shows-its-output-live ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log-output-lines 2))
      (draw :type 'tool :state 'started :id "c" :name "Bash" :input (romacs-agent-log-tests--input "command" "make"))
      (draw :type 'delta :text "after")
      (draw :type 'tool-output :id "c" :text "one\ntwo\n")
      (draw :type 'tool-output :id "c" :text "three\n")
      (let ((text (romacs-agent-log-tests--transcript)))
        (should (string-match-p "make[^\n]*\n  ⋮ 3 lines so far\n    two\n    three\nafter" text)))
      (draw :type 'tool :state 'completed :id "c" :output "one\ntwo\nthree")
      (let ((text (romacs-agent-log-tests--transcript)))
        (should-not (string-match-p "so far" text))
        (should (string-match-p "make[^\n]*\n  ⎿ one\n    two\n    … 1 more lines · RET shows all\nafter" text))))))

(ert-deftest romacs-agent-log-subagent-work-goes-under-its-call ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a" :name "Agent" :input (romacs-agent-log-tests--input "description" "count"))
    (draw :type 'delta :text "Main goes on.")
    (draw :type 'tool :state 'started :id "r" :name "Read" :parent "a"
          :input (romacs-agent-log-tests--input "file_path" "/n.txt"))
    (draw :type 'tool :state 'completed :id "r" :parent "a" :output "hello")
    (draw :type 'tool :state 'completed :id "a" :output "2 words")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "count[^\n]*\n│ ● [^\n]*n.txt[^\n]*\n│   ⎿ hello\n  ⎿ 2 words\nMain goes on." text)))))

(ert-deftest romacs-agent-log-an-approval-waits-in-plain-sight ()
  (romacs-agent-log-tests--with
    (puthash "b1" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "b1" :tool "Bash" :input (romacs-agent-log-tests--input "command" "rm -rf build"))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "⚠ The agent wants to run Bash" text))
      (should (string-match-p "rm -rf build" text))
      (should (string-match-p "y allow" text))
      (should (string-match-p "d deny" text)))
    (should (string-match-p "waits on you" (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input)))
    (should (string-match-p "1 waits on you" (romacs-agent-log--header)))
    ;; Answered by the user: who and how, and the block no longer waits.
    (remhash "b1" (romacs-agent-approvals session))
    (draw :type 'attributed :request-id "b1" :by "user" :decision "deny" :scope "once" :reason "not now")
    (draw :type 'answered :request-id "b1" :behavior "deny")
    (should (string-match-p "✘ denied by you: not now" (romacs-agent-log-tests--transcript)))
    (should-not (string-match-p "not recorded" (romacs-agent-log-tests--transcript)))
    (should-not romacs-agent-log--asks)))

(ert-deftest romacs-agent-log-an-answer-says-who-gave-it ()
  (romacs-agent-log-tests--with
    (draw :type 'attributed :request-id "x" :by "policy" :decision "allow" :scope "once")
    (draw :type 'answered :request-id "x" :behavior "allow")
    (draw :type 'answered :request-id "y" :behavior "deny")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "✔ allowed by Romacs" text))
      ;; An answer of before, with no note: said so, never hidden.
      (should (string-match-p "✘ denied (who: not recorded)" text)))))

(ert-deftest romacs-agent-log-policy-answers-draw-no-warning ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-approval-policy 'allow))
      (draw :type 'approval :request-id "p" :tool "Edit")
      (should-not (string-match-p "⚠" (romacs-agent-log-tests--transcript)))
      ;; In plan mode the user is asked.
      (setf (romacs-agent-options session) '(:plan t))
      (draw :type 'approval :request-id "q" :tool "Edit")
      (should (string-match-p "⚠ The agent wants to run Edit" (romacs-agent-log-tests--transcript))))))

(ert-deftest romacs-agent-log-point-moves-freely-while-a-turn-runs ()
  (romacs-agent-log-tests--with
    (dotimes (i 5) (draw :type 'input :text (format "prompt %d" i)))
    (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "sleep 9"))
    (draw :type 'delta :text "streaming")
    (goto-char (point-max))
    (insert "my draft")
    (should (equal (romacs-agent-log--draft) "my draft"))
    ;; Up from the composer into the transcript, line by line.
    (let ((lines 0))
      (while (and (> (point) (point-min)) (< lines 50))
        (let ((before (point)))
          (forward-line -1)
          (should (< (point) before)))
        (cl-incf lines))
      (should (bobp)))
    ;; Events drawn while point reads above do not move it.
    (forward-line 2)
    (let ((here (point)))
      (draw :type 'delta :text " more")
      (draw :type 'tool :state 'completed :id "t" :output "done")
      (should (= (point) here)))
    (should (equal (romacs-agent-log--draft) "my draft"))
    ;; A point in the composer stays there as the separator changes.
    (goto-char (point-max))
    (puthash "w" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "w" :tool "Bash")
    (should (= (point) (point-max)))
    (should (>= (point) romacs-agent-log--input))))

(ert-deftest romacs-agent-log-composer-edits-stop-at-its-start ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "a prompt")
    (goto-char (point-max))
    (insert "two words")
    (let ((separator (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input)))
      (dotimes (_ 5) (backward-kill-word 1))
      (should (equal (romacs-agent-log--draft) ""))
      (should (equal (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input) separator)))
    ;; Line motion leaves the composer all the same.
    (goto-char (point-max))
    (previous-line 2)
    (should (< (point) romacs-agent-log--end))))

(ert-deftest romacs-agent-log-n-and-p-move-between-entries ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "one")
    (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'input :text "two")
    (goto-char (point-min))
    (romacs-agent-log-next 1)
    (should (eq (get-text-property (point) 'romacs-agent-log-entry) 'tool))
    (romacs-agent-log-next 1)
    (should (eq (get-text-property (point) 'romacs-agent-log-entry) 'prompt))
    (romacs-agent-log-previous 1)
    (should (eq (get-text-property (point) 'romacs-agent-log-entry) 'tool))))

;; Sol review round 1 (2026-11-19).

(ert-deftest romacs-agent-log-prompt-motion-ends ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "one")
    (draw :type 'delta :text "reply")
    (draw :type 'done :time 0)
    (draw :type 'input :text "two")
    (goto-char (point-min))
    (romacs-agent-log-next-prompt 1)
    (should (looking-at-p "\n❯ two"))
    ;; Past the last prompt: said, never a hang.
    (should-error (romacs-agent-log-next-prompt 1) :type 'user-error)
    (goto-char (point-min))
    (should-error (romacs-agent-log-previous-prompt 1) :type 'user-error)))

(ert-deftest romacs-agent-log-subagent-text-goes-in-whole-lines ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a" :name "Agent" :input (romacs-agent-log-tests--input "description" "d"))
    (draw :type 'delta :text "hello " :parent "a")
    (draw :type 'delta :text "world\nnext " :parent "a")
    (draw :type 'tool :state 'started :id "r" :name "Read" :parent "a")
    (draw :type 'tool :state 'completed :id "a" :output "done")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "│ hello world\n│ next\n│ ● " text))
      (should (string-match-p "⎿ done" text)))
    ;; A background subagent's later word goes under its call still.
    (draw :type 'task :id "a" :state 'completed :summary "counted")
    (draw :type 'delta :text "root text")
    (should (string-match-p "│ · completed: counted\n[^│]*root text" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-a-block-waits-until-its-answer-is-written ()
  (romacs-agent-log-tests--with
    (puthash "q" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "q" :tool "AskUserQuestion" :user t
          :questions '(("Colour?" :options ("Red" "Blue"))))
    (let ((answers (make-hash-table :test #'equal)))
      (puthash "Colour?" "Blue" answers)
      (draw :type 'attributed :request-id "q" :by "user" :decision "allow" :scope "once" :answers answers))
    ;; Said who answered, and what; still answerable until the harness has it.
    (should (string-match-p "allowed by you\n  Colour\\? → Blue" (romacs-agent-log-tests--transcript)))
    (should (assoc "q" romacs-agent-log--asks))
    (remhash "q" (romacs-agent-approvals session))
    (draw :type 'answered :request-id "q" :behavior "allow")
    (should-not romacs-agent-log--asks)
    (should-not (string-match-p "not recorded" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-ret-on-an-approval-only-shows-it ()
  (romacs-agent-log-tests--with
    (puthash "b" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "b" :tool "Bash" :input (romacs-agent-log-tests--input "command" "make"))
    (goto-char (cadr (assoc "b" romacs-agent-log--asks)))
    (should (eq (lookup-key (get-text-property (point) 'keymap) (kbd "RET")) #'romacs-agent-log-open))
    (let (answered)
      (cl-letf (((symbol-function 'romacs-agent-respond) (lambda (&rest args) (push args answered))))
        (save-window-excursion (romacs-agent-log-open)))
      (should-not answered))))

(ert-deftest romacs-agent-log-what-the-session-waits-on-is-drawn ()
  (romacs-agent-log-tests--with
    ;; A request the records drew under another id, or not at all.
    (let ((request (make-hash-table :test #'equal)))
      (puthash "tool_name" "Bash" request)
      (puthash "input" (romacs-agent-log-tests--input "command" "ls") request)
      (puthash "7:1" request (romacs-agent-approvals session)))
    (puthash "0:1" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "0:1" :tool "Bash")
    (remhash "0:1" (romacs-agent-approvals session))
    (romacs-agent-log--sync-asks session)
    (should-not (assoc "0:1" romacs-agent-log--asks))
    (should (assoc "7:1" romacs-agent-log--asks))))

(ert-deftest romacs-agent-log-recall-keeps-the-draft ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "first")
    (draw :type 'input :text "second")
    (goto-char (point-max))
    (insert "my draft")
    (romacs-agent-log-history-previous)
    (should (equal (romacs-agent-log--draft) "second"))
    (romacs-agent-log-history-previous)
    (should (equal (romacs-agent-log--draft) "first"))
    (romacs-agent-log-history-next)
    (romacs-agent-log-history-next)
    (should (equal (romacs-agent-log--draft) "my draft"))))

(provide 'romacs-agent-log-tests)
;;; romacs-agent-log-tests.el ends here

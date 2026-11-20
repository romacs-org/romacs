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
  (require 'romacs-agent-log)
  (require 'romacs-agent-menu))

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

;; What runs before a redisplay: a batch session has no host to run it.
(unless (fboundp 'romacs--run-window-change-functions)
  (load (expand-file-name "../vm/window-change.el" (file-name-directory (locate-library "romacs-agent.el"))) nil t))

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
        (should (string-match-p "make[^\n]*\n  ⋮ running: 3 lines so far, the last 2\n    two\n    three\nafter" text)))
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
    (draw :type 'attributed :request-id "b1" :actor '(:kind "user" :client "editor") :decision "deny" :scope "once" :reason "not now")
    (draw :type 'answered :request-id "b1" :behavior "deny")
    (should (string-match-p "✘ denied by you: not now" (romacs-agent-log-tests--transcript)))
    (should-not (string-match-p "not recorded" (romacs-agent-log-tests--transcript)))
    (should-not romacs-agent-log--asks)))

(ert-deftest romacs-agent-log-an-answer-says-who-gave-it ()
  (romacs-agent-log-tests--with
    (draw :type 'attributed :request-id "x" :actor '(:kind "policy") :decision "allow" :scope "once")
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
    (should (string-match-p "│ · completed\n[^│]*root text" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-rule-fits-the-windows-showing-the-chat ()
  (romacs-agent-log-tests--with
    (ignore #'draw)
    (let ((rule (lambda () (save-excursion (goto-char romacs-agent-log--end)
                                           (how-many "─" (point) (line-end-position)))))
          (chat (current-buffer)))
      (puthash "L" chat romacs-agent-log--chats)
      (unwind-protect
          (save-window-excursion
            (delete-other-windows)
            (switch-to-buffer chat)
            ;; The hooks run as the host delivers window changes before a
            ;; redisplay.
            (romacs--run-window-change-functions)
            (let ((narrow (split-window-right (- (window-total-width) 30))))
              (set-window-buffer narrow chat)
              (romacs--run-window-change-functions)
              (let ((short (funcall rule)))
                ;; The narrower window leaves no room for the rule to wrap in.
                (should (< (+ romacs-agent-log--thread-cols short) (window-body-width narrow)))
                ;; It shows another buffer: the rule fits the wider one again.
                (set-window-buffer narrow (get-buffer-create "*romacs-agent-log-tests other*"))
                (romacs--run-window-change-functions)
                (should (> (funcall rule) short)))))
        (remhash "L" romacs-agent-log--chats)))))

(ert-deftest romacs-agent-log-menu-lists-the-workers-at-work ()
  (romacs-agent-log-tests--with
    (ignore #'draw)
    (setf (romacs-agent-title session) "lane chat")
    (let ((children (list (romacs-agent--make :id "c1" :state 'running :title "builds")
                          (romacs-agent--make :id "c2" :state 'idle :title "finished")
                          (romacs-agent--make :id "c3" :state 'waiting-approval :title "asks"))))
      (cl-letf (((symbol-function 'romacs-agent-children) (lambda (_) children)))
        (should (equal (substring-no-properties (romacs-agent-menu--title)) "Agent · lane chat"))
        ;; A finished one is the board's; one waiting on the user first.
        (should (equal (mapcar #'romacs-agent-title (romacs-agent-menu--workers)) '("asks" "builds")))
        (should (equal (substring-no-properties (romacs-agent-menu--worker-label 0)) "◆ asks you · asks"))
        (should (equal (substring-no-properties (romacs-agent-menu--worker-label 1)) "● running · builds"))
        (should-not (romacs-agent-menu--worker-label 2))
        ;; A long title is what a cut row loses, its state never.
        (setf (romacs-agent-title (nth 0 children)) (make-string 60 ?x))
        (should (string-prefix-p "● running · " (substring-no-properties (romacs-agent-menu--worker-label 1))))
        (setf (romacs-agent-title (nth 0 children)) "builds")
        ;; A digit opens the worker it showed; one finished since is the board's.
        (let (shown)
          (cl-letf (((symbol-function 'romacs-agent-show) (lambda (child &rest _) (setq shown child))))
            (setf (romacs-agent-state (nth 2 children)) 'idle)
            (let ((last-command-event ?2)) (romacs-agent-menu-open-worker))
            (should (equal (romacs-agent-title shown) "builds"))
            (let ((last-command-event ?1)) (should-error (romacs-agent-menu-open-worker) :type 'user-error))
            ;; All finished while it is open: its keys still dispatch (the
            ;; group has no :if), each saying so.
            (setf (romacs-agent-state (car children)) 'idle)
            (should-not (romacs-agent-menu--workers))
            (let ((group (seq-find (lambda (group) (equal (plist-get group :name) "Workers"))
                                   (apply #'append (keymap-popup--meta romacs-agent-menu-map 'descriptions)))))
              (should group)
              (should-not (plist-get group :if)))
            (let ((last-command-event ?2)) (should-error (romacs-agent-menu-open-worker) :type 'user-error))
            ;; Each chat its own rows.
            (should-not (eq romacs-agent-menu--shown (default-value 'romacs-agent-menu--shown)))
            (setf (romacs-agent-state (car children)) 'running)
            (setf (romacs-agent-state (nth 2 children)) 'waiting-approval)))
        ;; Its values read as values, never dim as a command it cannot run.
        (let ((romacs-agent-log--session session))
          (dolist (label (list (romacs-agent-menu--effort-label) (romacs-agent-menu--model-label)))
            (should (string-match-p "default" label))
            (should-not (memq 'romacs-agent-effort-default
                              (ensure-list (get-text-property (string-match "default" label) 'face label))))))))))

(ert-deftest romacs-agent-log-menu-rows-are-each-chats-own ()
  ;; Even over a shared default a load left behind.
  (let ((shared (make-vector 9 nil)))
    (cl-letf (((default-value 'romacs-agent-menu--shown) shared))
      (with-temp-buffer
        (let ((one (romacs-agent-menu--slots)))
          (with-temp-buffer
            (should-not (eq (romacs-agent-menu--slots) one))
            (should-not (eq (romacs-agent-menu--slots) shared))))))))

(ert-deftest romacs-agent-log-a-turn-that-says-no-time-shows-none ()
  ;; A transcript's turn may not say when it ended: never the clock now.
  (romacs-agent-log-tests--with
    (draw :type 'done :duration 9)
    (should (string-match-p "✻ 9s\n" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-a-turn-at-point-is-its-prompts ()
  ;; Each prompt says the turn it starts: a diff or a rollback of the turn
  ;; at point reads it; the attach puts a file's path in the composer.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "first" :request 4)
    (draw :type 'delta :text "reply one")
    (draw :type 'done :time 0)
    (draw :type 'input :text "second" :request 7)
    (draw :type 'delta :text "reply two")
    (goto-char (point-min))
    (should (equal (romacs-agent-log-turns-here) 4))
    (search-forward "reply one")
    (should (equal (romacs-agent-log-turns-here) 4))
    (search-forward "reply two")
    (should (equal (romacs-agent-log-turns-here) 7))
    (let (asked)
      (cl-letf (((symbol-function 'romacs-agent-diff--checkpoints) #'ignore)
                ((symbol-function 'y-or-n-p) #'always)
                ((symbol-function 'romacs-agent-rollback) (lambda (_s request &rest _) (setq asked request))))
        (search-backward "reply one")
        (romacs-agent-chat-rollback))
      (should (equal asked 4)))
    (romacs-agent-chat-attach "/tmp/shot one.png")
    (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "/tmp/shot one.png"))))

(ert-deftest romacs-agent-log-prompt-motion-lands-on-its-turn ()
  ;; Prompt motion lands on a prompt's first character, its leading blank
  ;; line: the turn there is that prompt's.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "first" :request 4)
    (draw :type 'done :time 0)
    (draw :type 'input :text "second" :request 7)
    (goto-char romacs-agent-log--end)
    (romacs-agent-log-previous-prompt 1)
    (should (equal (romacs-agent-log-turns-here) 7))
    (should (equal (romacs-agent-log-turns-text 7) "❯ second"))
    ;; Rolled back to before it: from there on dimmed, and said.
    (draw :type 'rolled-back :to 7 :turns 1 :prompt "second" :files t)
    (should (memq 'romacs-agent-log-note (ensure-list (get-text-property (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-turn 7) 'face))))
    (should (string-match-p "↶ rolled back 1 turn and their files to before: second" (romacs-agent-log-tests--transcript)))
    ;; From the composer, a rollback asks for the turn.
    (goto-char (point-max))
    (should-error (romacs-agent-chat-rollback) :type 'user-error)))

(ert-deftest romacs-agent-log-a-rollback-before-the-drawn-turns-dims-them-all ()
  (romacs-agent-log-tests--with
    (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line 3))))
    (draw :type 'input :text "shown" :request 9)
    (draw :type 'rolled-back :to 2 :turns 3 :prompt "long ago")
    (let ((at (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-turn 9)))
      (should (memq 'romacs-agent-log-note (ensure-list (get-text-property at 'face)))))))

(ert-deftest romacs-agent-log-a-rollback-reaches-over-earlier-turns-put-above ()
  ;; The rollback is drawn on the page shown; the prompt it went back to
  ;; before is on the page put above it later: from there on, dimmed.
  (romacs-agent-log-tests--with
    (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line 5))))
    (setq romacs-agent-log--from 5)
    (draw :type 'input :text "shown" :request 9)
    (draw :type 'rolled-back :to 2 :turns 3 :prompt "long ago")
    (cl-letf (((symbol-function 'romacs-agent--journal-tail) (lambda (&rest _) '(nil)))
              ((symbol-function 'romacs-agent-events-p) #'ignore)
              ((symbol-function 'romacs-agent-log--replay)
               (lambda (s &rest _)
                 (romacs-agent-log-draw s '(:type input :text "before it" :request 1))
                 (romacs-agent-log-draw s '(:type input :text "long ago" :request 2)))))
      (romacs-agent-log-earlier))
    (let ((dim (lambda (request) (memq 'romacs-agent-log-note
                                       (ensure-list (get-text-property (text-property-any (point-min) romacs-agent-log--end
                                                                                          'romacs-agent-log-turn request)
                                                                       'face))))))
      (should-not (funcall dim 1))
      (should (funcall dim 2))
      (should (funcall dim 9)))))

(ert-deftest romacs-agent-log-a-rollback-waits-over-pages-between ()
  ;; Its prompt two pages back: the page between is dimmed as it comes
  ;; above, and the prompt's own page from the prompt on.
  (romacs-agent-log-tests--with
    (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line 9))))
    (setq romacs-agent-log--from 9)
    (draw :type 'input :text "shown" :request 9)
    (draw :type 'rolled-back :to 2 :turns 7 :prompt "long ago")
    (cl-flet ((page (from &rest requests)
                (cl-letf (((symbol-function 'romacs-agent--journal-tail) (lambda (&rest _) (list from)))
                          ((symbol-function 'romacs-agent-events-p) #'ignore)
                          ((symbol-function 'romacs-agent-log--replay)
                           (lambda (s &rest _)
                             (dolist (request requests)
                               (romacs-agent-log-draw s (list :type 'input :text (format "p%d" request) :request request))))))
                  (romacs-agent-log-earlier)))
              (dim (request) (memq 'romacs-agent-log-note
                                   (ensure-list (get-text-property (text-property-any (point-min) romacs-agent-log--end
                                                                                      'romacs-agent-log-turn request)
                                                                   'face)))))
      (page 5 5 6)
      (should (dim 5))
      (should (dim 6))
      (page nil 1 2)
      (should-not (dim 1))
      (should (dim 2))
      (should-not romacs-agent-log-turns--rollbacks))))

(ert-deftest romacs-agent-log-a-compaction-opens-whole ()
  (romacs-agent-log-tests--with
    (draw :type 'compacted :text (mapconcat #'number-to-string (number-sequence 1 30) "\n"))
    ;; From where entry motion lands, its first character.
    (goto-char (point-min))
    (should (equal (cdr (get-text-property (point) 'romacs-agent-log-full)) (mapconcat #'number-to-string (number-sequence 1 30) "\n")))
    (should (string-match-p "more lines" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-an-old-chat-s-desktop-comes-back-here ()
  (should (eq (cdr (assq 'romacs-agent-chat-mode desktop-buffer-mode-handlers)) #'romacs-agent-log-desktop-restore))
  (should (eq (indirect-function 'romacs-agent-chat-mode) (indirect-function 'romacs-agent-log-mode))))

(ert-deftest romacs-agent-log-a-command-acts-on-the-registered-session ()
  ;; A restored chat shows its session's row until used: a command acts
  ;; on the session itself, loaded.
  (romacs-agent-log-tests--with
    (ignore #'draw)
    (let ((registered (romacs-agent--make :id "L" :harness 'claude :state 'idle
                                          :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
          stopped)
      (cl-letf (((symbol-function 'romacs-agent-loaded) (lambda (_) nil))
                ((symbol-function 'romacs-agent-get) (lambda (_) registered))
                ((symbol-function 'yes-or-no-p) #'always)
                ((symbol-function 'romacs-agent-stop) (lambda (s) (setq stopped s))))
        (romacs-agent-chat-stop))
      (should (eq stopped registered)))))

(ert-deftest romacs-agent-log-several-answers-keep-their-questions ()
  (let ((one (make-hash-table :test #'equal)) (two (make-hash-table :test #'equal)))
    (puthash "Colour?" "Blue" one)
    (puthash "Colour?" "Blue" two)
    (puthash "Size?" "Large" two)
    (should (string-suffix-p "answered: Blue" (romacs-agent-log--answer-text '(:kind "user") "allow" "once" nil one)))
    (let ((text (romacs-agent-log--answer-text '(:kind "user") "allow" "once" nil two)))
      (should (string-match-p "Colour\\? → Blue" text))
      (should (string-match-p "Size\\? → Large" text)))))

(ert-deftest romacs-agent-log-an-answer-says-its-actor ()
  ;; Another client's answer says which; the harness's own says so.
  (should (string-match-p "allowed by you (via phone)"
                          (romacs-agent-log--answer-text '(:kind "user" :client "phone") "allow" "once" nil)))
  (should (string-match-p "denied by the harness" (romacs-agent-log--answer-text '(:kind "harness") "deny" "once" nil)))
  (should (string-match-p "\\`✔ allowed\\'" (substring-no-properties
                                               (romacs-agent-log--answer-text '(:kind "unknown") "allow" "once" nil))))
  (romacs-agent-log-tests--with
    (puthash "h" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "h" :tool "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (remhash "h" (romacs-agent-approvals session))
    (draw :type 'answered :request-id "h" :how 'harness)
    (should (string-match-p "settled by the harness" (romacs-agent-log-tests--transcript)))
    ;; Answered, then settled by the harness before it took the answer.
    (puthash "g" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "g" :tool "Bash")
    (draw :type 'attributed :request-id "g" :actor '(:kind "user") :decision "allow" :scope "once")
    (remhash "g" (romacs-agent-approvals session))
    (draw :type 'answered :request-id "g" :how 'harness)
    (should (string-match-p "an answer not delivered: the harness settled it first" (romacs-agent-log-tests--transcript))))
  ;; An older note named no one: its answer is said, never as Romacs's.
  (let ((answers (make-hash-table :test #'equal)))
    (puthash "Colour?" "Blue" answers)
    (should (equal (substring-no-properties (romacs-agent-log--answer-text '(:kind "unknown") "allow" "once" nil answers))
                   "✔ answered: Blue")))
  ;; Policy's answer from another client says which too.
  (should (string-match-p "allowed by Romacs.*(via phone)"
                          (romacs-agent-log--answer-text '(:kind "policy" :client "phone") "allow" "once" nil))))

(ert-deftest romacs-agent-log-asks-left-open-say-how-they-ended ()
  (romacs-agent-log-tests--with
    (dolist (id '("a" "b" "c")) (puthash id t (romacs-agent-approvals session)))
    (dolist (id '("a" "b" "c")) (draw :type 'approval :request-id id :tool "Bash"))
    ;; Answered by the user, its answer never taken; one still waited on.
    (draw :type 'attributed :request-id "a" :actor '(:kind "user") :decision "allow" :scope "once")
    (remhash "a" (romacs-agent-approvals session))
    (remhash "b" (romacs-agent-approvals session))
    (draw :type 'done)
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "an answer not delivered" text))
      (should (string-match-p "the turn ended with it unanswered" text)))
    (should (equal (mapcar #'car romacs-agent-log--asks) '("c")))
    ;; In a replay, what the session waited on then decides, not now.
    (remhash "c" (romacs-agent-approvals session))
    (let ((romacs-agent-log--replaying (romacs-agent--make :id "L" :approvals (make-hash-table :test #'equal))))
      (puthash "c" t (romacs-agent-approvals romacs-agent-log--replaying))
      (romacs-agent-log-draw session '(:type done)))
    (should (equal (mapcar #'car romacs-agent-log--asks) '("c")))
    (draw :type 'exit :status 1)
    (should (string-match-p "the harness exited with it unanswered" (romacs-agent-log-tests--transcript)))
    (should-not romacs-agent-log--asks)))

(ert-deftest romacs-agent-log-earlier-turns-draw-no-live-asks ()
  ;; An approval in the earlier turns whose settling the shown ones hold.
  (romacs-agent-log-tests--with
    (setq romacs-agent-log--from 5)
    (cl-letf (((symbol-function 'romacs-agent--journal-tail) (lambda (&rest _) '(nil)))
              ((symbol-function 'romacs-agent-events-p) #'ignore)
              ((symbol-function 'romacs-agent-log--replay)
               (lambda (s &rest _) (romacs-agent-log-draw s '(:type approval :request-id "old" :tool "Bash")))))
      (romacs-agent-log-earlier))
    (should-not (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-keys t))
    (should-not (text-property-any (point-min) romacs-agent-log--end 'keymap romacs-agent-log-ask-map))
    (should-not romacs-agent-log--asks)
    ;; One still waited on: its keys only in its live block, at the end.
    (puthash "live" t (romacs-agent-approvals session))
    (romacs-agent-log--sync-asks session)
    (setq romacs-agent-log--from 5)
    (cl-letf (((symbol-function 'romacs-agent--journal-tail) (lambda (&rest _) '(nil)))
              ((symbol-function 'romacs-agent-events-p) #'ignore)
              ((symbol-function 'romacs-agent-log--replay)
               (lambda (s &rest _) (romacs-agent-log-draw s '(:type approval :request-id "live" :tool "Bash")))))
      (romacs-agent-log-earlier))
    (let ((live (cadr (assoc "live" romacs-agent-log--asks))))
      (should (= (length romacs-agent-log--asks) 1))
      (should-not (text-property-any (point-min) live 'romacs-agent-log-keys t))
      ;; The earlier copy answers nothing itself: the chat's keys there
      ;; answer the live block, shown first.
      (should-not (text-property-any (point-min) live 'romacs-agent-log-request "live"))
      (goto-char (point-min))
      (should (equal (romacs-agent-log--ask-here) "live"))
      (should (= (point) live)))
    (remhash "live" (romacs-agent-approvals session))
    (draw :type 'answered :request-id "live")
    (should-not (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-keys t))))

(ert-deftest romacs-agent-log-a-block-waits-until-its-answer-is-written ()
  (romacs-agent-log-tests--with
    (puthash "q" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "q" :tool "AskUserQuestion" :user t
          :questions '(("Colour?" :options ("Red" "Blue"))))
    (let ((answers (make-hash-table :test #'equal)))
      (puthash "Colour?" "Blue" answers)
      (draw :type 'attributed :request-id "q" :actor '(:kind "user" :client "editor") :decision "allow" :scope "once" :answers answers))
    ;; Said who answered, and what; still answerable until the harness has it.
    (should (string-match-p "✔ answered: Blue" (romacs-agent-log-tests--transcript)))
    (should-not (string-match-p "allowed by you" (romacs-agent-log-tests--transcript)))
    (should (assoc "q" romacs-agent-log--asks))
    (remhash "q" (romacs-agent-approvals session))
    (draw :type 'answered :request-id "q" :behavior "allow")
    (should-not romacs-agent-log--asks)
    ;; Its keys answer nothing any more: gone.
    (should-not (string-match-p "decline" (romacs-agent-log-tests--transcript)))
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

;; Sol review round 2.

(ert-deftest romacs-agent-log-a-recovered-question-is-still-asked ()
  (romacs-agent-log-tests--with
    (let ((request (make-hash-table :test #'equal)))
      (puthash "tool_name" "AskUserQuestion" request)
      (puthash "input" (json-parse-string "{\"questions\":[{\"question\":\"Colour?\",\"header\":\"C\",\"options\":[{\"label\":\"Red\"},{\"label\":\"Blue\"}]}]}")
               request)
      (puthash "q9" request (romacs-agent-approvals session)))
    (romacs-agent-log--sync-asks session)
    (goto-char (cadr (assoc "q9" romacs-agent-log--asks)))
    (should (get-text-property (point) 'romacs-agent-log-questions))
    (let (opened)
      (cl-letf (((symbol-function 'romacs-agent-ask-open) (lambda (_s id questions &rest _) (setq opened (list id questions))))
                ((symbol-function 'romacs-agent-respond) (lambda (&rest _) (error "Answered without its answers"))))
        (romacs-agent-log-allow))
      (should (equal (car opened) "q9"))
      (should (equal (caar (cadr opened)) "Colour?")))))

(ert-deftest romacs-agent-log-background-work-stays-under-its-call ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "b" :name "Bash" :input (romacs-agent-log-tests--input "command" "sleep 60"))
    (draw :type 'task :id "b" :state 'started :task "t1" :background t)
    (draw :type 'tool :state 'completed :id "b" :output "Running in the background")
    (draw :type 'delta :text "root goes on")
    (draw :type 'message-end)
    ;; Known by its task alone, then ended: under its call still.
    (draw :type 'task :task "t1" :state 'backgrounded)
    (draw :type 'task :id "b" :state 'completed :summary "slept")
    (should (string-match-p "  ⎿ Running in the background\n│ · moved to the background\n│ · completed: slept\nroot goes on"
                            (romacs-agent-log-tests--transcript)))
    (should-not (romacs-agent-log--block "b"))))

(ert-deftest romacs-agent-log-a-call-shows-its-input-as-it-ended ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "w" :name "WebSearch" :input (romacs-agent-log-tests--input))
    (draw :type 'tool :state 'completed :id "w" :input (romacs-agent-log-tests--input "query" "romacs") :output "found")
    (goto-char (point-min))
    (should (equal (gethash "query" (car (get-text-property (point) 'romacs-agent-log-full))) "romacs"))))

;; Sol review round 3.

(ert-deftest romacs-agent-log-interleaved-calls-keep-their-own-rows ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a" :name "WebSearch" :input (romacs-agent-log-tests--input))
    (draw :type 'tool :state 'started :id "b" :name "WebSearch" :input (romacs-agent-log-tests--input))
    (draw :type 'tool :state 'completed :id "a" :input (romacs-agent-log-tests--input "query" "one") :output "A")
    (draw :type 'tool :state 'completed :id "b" :input (romacs-agent-log-tests--input "query" "two") :output "B")
    (goto-char (point-min))
    (should (equal (gethash "query" (car (get-text-property (point) 'romacs-agent-log-full))) "one"))
    (search-forward "⎿ A")
    (should (equal (gethash "query" (car (get-text-property (point) 'romacs-agent-log-full))) "one"))
    (goto-char (point-min))
    (romacs-agent-log-next 2)
    (should (equal (gethash "query" (car (get-text-property (point) 'romacs-agent-log-full))) "two"))))

(ert-deftest romacs-agent-log-work-ended-first-closes-with-its-call ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "c" :name "Bash")
    (draw :type 'task :id "c" :state 'started :task "t")
    (draw :type 'task :id "c" :state 'completed)
    (draw :type 'tool :state 'completed :id "c" :output "ok")
    (should-not (romacs-agent-log--block "c"))
    ;; Progress of a call this chat never drew is said all the same.
    (draw :type 'task :id "gone" :state 'progress :activity "Read")
    (should (string-match-p "task now: Read" (romacs-agent-log-tests--transcript)))))

;; Sol review round 5.

(ert-deftest romacs-agent-log-a-line-it-does-not-read-is-named ()
  (romacs-agent-log-tests--with
    (draw :type 'unknown :method "thread/name/updated" :raw (romacs-agent-log-tests--input "name" "x"))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "· thread/name/updated" text))
      (should-not (string-match-p "hash-table" text)))
    ;; A Claude Code line, by its type.
    (draw :type 'unknown :raw (romacs-agent-log-tests--input "type" "tool_progress"))
    (should (string-match-p "· tool_progress" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-a-shown-running-chat-keeps-its-clock ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log--ticker nil) started)
      (setq romacs-agent-log--filled t)
      (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) (setq started t) 'timer))
                ((symbol-function 'timerp) (lambda (x) (eq x 'timer))))
        (romacs-agent-log--shown nil))
      (should started))))

;; Screenshot reviews, round 1.

(ert-deftest romacs-agent-log-a-subagent-reports-without-its-frame ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a" :name "Agent" :input (romacs-agent-log-tests--input "description" "count"))
    (draw :type 'tool :state 'completed :id "a"
          :output "[Subagent hand-back] The text below is the final report. The report follows:\n  2 words\nagentId: abc (use SendMessage)\n<usage>tokens: 5\ntool_uses: 1</usage>")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "⎿ 2 words\n" text))
      (should-not (string-match-p "hand-back\\|agentId\\|usage" text)))
    (romacs-agent-log--separator)
    (should (string-match-p "working" (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input)))))

(ert-deftest romacs-agent-log-proceeding-with-a-plan-leaves-plan-mode ()
  (let ((session (romacs-agent--make :id "P" :harness 'claude :options '(:plan t)
                                     :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
        (request (make-hash-table :test #'equal))
        planned)
    (puthash "tool_name" "ExitPlanMode" request)
    (puthash "p" request (romacs-agent-approvals session))
    (romacs-agent--raise session '(:type approval :request-id "p" :tool "ExitPlanMode" :plan t :user t))
    (cl-letf (((symbol-function 'romacs-agent--line) #'ignore)
              ((symbol-function 'romacs-agent--note) #'ignore)
              ((symbol-function 'romacs-agent-set-plan) (lambda (_s on) (push on planned))))
      ;; Kept planning: still in plan mode.
      (romacs-agent-respond session "p" nil "more" 'user)
      (should-not planned)
      ;; Proceeding, however answered (here, the board's way): out of it.
      (romacs-agent-respond session "p" t nil 'user)
      (should (equal planned '(nil))))))

(ert-deftest romacs-agent-log-an-answer-is-noted-with-its-actor ()
  (let ((session (romacs-agent--make :id "N" :harness 'claude :seq 0
                                     :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
        (request (make-hash-table :test #'equal))
        notes raised)
    (puthash "tool_name" "Bash" request)
    (puthash "b" request (romacs-agent-approvals session))
    (cl-letf (((symbol-function 'romacs-agent--line) #'ignore)
              ((symbol-function 'romacs-agent--note) (lambda (_s note) (push note notes))))
      (romacs-agent-respond session "b" t nil 'user)
      (romacs-agent-respond session "b" nil "no" '(:kind policy :login "ci" :client "phone")))
    (pcase-let ((`(,second ,first) notes))
      (should (equal (gethash "by" first) "user"))
      (should (equal (gethash "login" first) (user-login-name)))
      (should (equal (gethash "client" first) "editor"))
      (should (equal (gethash "client" second) "phone"))
      ;; Read back, the note says its actor; an older one only who.
      (cl-letf (((symbol-function 'romacs-agent--raise) (lambda (_s event) (push event raised))))
        (let ((record (make-hash-table :test #'equal)))
          (puthash "seq" 1 record)
          (puthash "note" second record)
          (romacs-agent--record session record))
        (let ((record (make-hash-table :test #'equal)) (old (make-hash-table :test #'equal)))
          (puthash "type" "answer" old)
          (puthash "request" "b" old)
          (puthash "decision" "allow" old)
          (puthash "seq" 2 record)
          (puthash "note" old record)
          (romacs-agent--record session record)))
      (let ((events (seq-filter (lambda (event) (eq (plist-get event :type) 'attributed)) (reverse raised))))
        (should (equal (plist-get (car events) :actor) '(:kind "policy" :login "ci" :client "phone")))
        (should (equal (plist-get (cadr events) :actor) '(:kind "unknown" :login nil :client nil)))))))

(ert-deftest romacs-agent-log-a-turn-end-ends-what-runs ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "b" :name "Bash")
    (draw :type 'tool-output :id "gone" :text "x\n")
    (draw :type 'exit :status 1)
    (should-not romacs-agent-log--current)
    (should-not romacs-agent-log--early)))

(ert-deftest romacs-agent-log-a-report-keeps-its-words ()
  (should (equal (romacs-agent-log--report "Plain words. The report follows:\nmore <usage>inline</usage> text")
                 "Plain words. The report follows:\nmore <usage>inline</usage> text"))
  (should (equal (romacs-agent-log--report "[Subagent hand-back] frame. The report follows:\nbody\nagentId: a1\n<usage>t: 1</usage>\n")
                 "body"))
  (should (equal (romacs-agent-log--report "[Subagent hand-back] f. The report follows:\nsee <usage>x</usage> here\nagentId: a\n<usage>t</usage>")
                 "see <usage>x</usage> here"))
  (should (equal (romacs-agent-log--report "Example: <usage>x</usage>\nThe closing tag is </usage>")
                 "Example: <usage>x</usage>\nThe closing tag is </usage>")))

(ert-deftest romacs-agent-log-output-before-its-call-is-kept ()
  (romacs-agent-log-tests--with
    (draw :type 'tool-output :id "x" :text "line 1\n")
    (draw :type 'tool :state 'started :id "x" :name "Bash" :input (romacs-agent-log-tests--input "command" "seq 2"))
    (draw :type 'tool-output :id "x" :text "line 2\n")
    (should (string-match-p "⋮ running: 2 lines so far\n    line 1\n    line 2" (romacs-agent-log-tests--transcript)))))

(provide 'romacs-agent-log-tests)
;;; romacs-agent-log-tests.el ends here

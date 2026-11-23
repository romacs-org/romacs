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
  (require 'romacs-agent-menu)
  (require 'romacs-agent-board))

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

(defun romacs-agent-log-tests--face-p (face faces)
  "Whether FACE is one of FACES, a face property's value, or what one of
them inherits."
  (seq-some (lambda (f) (or (eq f face) (and (symbolp f) (facep f) (memq face (ensure-list (face-attribute f :inherit))))))
            (ensure-list faces)))

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
      (should (string-match-p "🤔 I should run wc." text))
      (should (string-match-p "wc -l notes.txt" text))
      ;; The output under its call, before what followed the call.
      (should (string-match-p "\\$ wc -l notes.txt · 2 notes.txt\n\nRunning it now." text))
      ;; Its rule says how long and when; what it cost is RET away.
      (should (string-match-p "✻ 9s · [0-9][0-9]:[0-9][0-9]" text))
      (should-not (string-search "0.12" text)))))

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
                   "🤔 I think.\n\nPara one.\n\nPara two.\n\n\n▌ ❯ next\n\n"))))

(ert-deftest romacs-agent-log-tool-rows-indent-and-mark-failures ()
  (romacs-agent-log-tests--with
    (draw :type 'input :text "Check the build")
    (draw :type 'tool :state 'started :id "b" :seq 2 :name "Bash"
          :input (romacs-agent-log-tests--input "command" "make"))
    (draw :type 'tool :state 'started :id "w" :name "Wait for subagents")
    (draw :type 'tool :state 'completed :id "w")
    (should (string-match-p "^  \\$ make\n  · Wait for subagents\n"
                            (romacs-agent-log-tests--transcript)))
    (goto-char (point-min))
    (search-forward "$ make")
    (should (memq 'romacs-agent-log-tool (ensure-list (get-text-property (match-beginning 0) 'face))))
    (draw :type 'tool :state 'failed :id "b" :output "Build failed")
    (should (string-match-p "^  ✘ \\$ make\n    ⎿ Build failed\n  · Wait for subagents\n"
                            (romacs-agent-log-tests--transcript)))
    (goto-char (point-min))
    (search-forward "✘ $ make")
    (should (memq 'romacs-agent-log-tool-failed (ensure-list (get-text-property (match-beginning 0) 'face))))
    (should (= (get-text-property (match-beginning 0) 'romacs-agent-log-seq) 2))
    (should (equal (cdr (get-text-property (match-beginning 0) 'romacs-agent-log-full)) "Build failed"))))

(ert-deftest romacs-agent-log-a-tool-row-keeps-outcome-emphasis ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "m" :name "mcp__build__check")
    (draw :type 'tool :state 'completed :id "m" :output "{\"error\":\"Build rejected\"}")
    (goto-char (point-min))
    (search-forward "Build rejected")
    (should (eq (car (ensure-list (get-text-property (match-beginning 0) 'face))) 'romacs-agent-error))))

(ert-deftest romacs-agent-log-long-output-says-how-much-more ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log-output-lines 2))
      (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "seq 5"))
      (draw :type 'tool :state 'completed :id "t" :output "1\n2\n3\n4\n5")
      (let ((text (romacs-agent-log-tests--transcript)))
        ;; A command that ended well is its row: how many lines it gave.
        (should (equal text "  $ seq 5 · 5 lines\n"))
        ;; RET on it shows all of it.
        (goto-char (point-min))
        (search-forward "seq 5")
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
        (should (string-match-p "make[^\n]*\n    ⋮ running: 3 lines so far, the last 2\n      two\n      three\n\nafter" text)))
      (draw :type 'tool :state 'completed :id "c" :output "one\ntwo\nthree")
      (let ((text (romacs-agent-log-tests--transcript)))
        (should-not (string-match-p "so far" text))
        ;; Ended well: its row alone, what it gave RET away.
        (should (string-match-p "make · 3 lines\n\nafter" text))))))

(ert-deftest romacs-agent-log-subagent-work-goes-under-its-call ()
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a" :name "Agent" :input (romacs-agent-log-tests--input "description" "count"))
    (draw :type 'delta :text "Main goes on.")
    (draw :type 'tool :state 'started :id "r" :name "Read" :parent "a"
          :input (romacs-agent-log-tests--input "file_path" "/n.txt"))
    (draw :type 'tool :state 'completed :id "r" :parent "a" :output "hello")
    (draw :type 'tool :state 'completed :id "a" :output "2 words")
    (let ((text (romacs-agent-log-tests--transcript)))
      ;; A read says how much it read, its text RET away; a report, under its call.
      (should (string-match-p "⑂ count · 2 words\n│   ◧ [^\n]*n.txt[^\n]*· 1 line\n\nMain goes on." text)))))

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

(ert-deftest romacs-agent-log-live-subagents-stay-with-the-composer ()
  (romacs-agent-log-tests--with
    (let ((romacs-agent-ui--children-changed (make-hash-table :test #'eq))
          (romacs-agent-ui--children-timer 'coalesced)
          (romacs-agent-ui-changed-hook '(romacs-agent-log--headers-refresh))
          (romacs-agent-log-separator-rows-function (lambda () "A queued message\n")))
      (save-window-excursion
        (switch-to-buffer (current-buffer))
        (cl-flet ((deliver (event)
                    (romacs-agent-ui-track-workers session event)
                    (apply #'draw event)
                    (romacs-agent-ui--children-flush)
                    (setq romacs-agent-ui--children-timer 'coalesced))
                  (chrome () (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input)))
          (goto-char (point-max))
          (insert "draft")
          (backward-char 2)
          (deliver (list :type 'tool :state 'started :id "a" :name "Agent"
                         :input (romacs-agent-log-tests--input "description" "Read the docs")))
          (deliver '(:type task :id "a" :task "worker" :state started :background t :subagent t))
          (deliver '(:type tool :id "r" :parent "a" :state started :name "Read"))
          (deliver '(:type task :id "a" :state progress :activity "Reading guide.org"))
          (should (string-match-p "Read the docs  ·  Reading guide.org" (chrome)))
          (should (equal (cdr (romacs-agent-log-activity-bounds)) romacs-agent-log--divider))
          (should (< (car (romacs-agent-log-activity-bounds)) romacs-agent-log--divider))
          (should (= (length romacs-agent-log-activity--threads) 1))
          (pcase-let ((`(,id . ,mark) (car romacs-agent-log-activity--threads)))
            (should (eq (get-text-property mark 'face) 'romacs-agent-log-subagent))
            (should (eq (nth 3 (plist-get (romacs-working--element id) :kind))
                        'romacs-agent-log-subagent))
            (save-excursion
              (goto-char mark)
              (search-forward "Read the docs" (line-end-position))
              (should (eq (get-text-property (1- (point)) 'face) 'romacs-agent-log-subagent))))
          (should (string-suffix-p "A queued message\n❯ " (chrome)))
          (save-excursion
            (goto-char (1+ (car (romacs-agent-log-activity-bounds))))
            (should-error (insert "x") :type 'text-read-only))
          (should-not (string-match-p "Reading guide.org" (romacs-agent-log-tests--transcript)))
          (deliver '(:type tool :id "a" :state completed :output "Worker started"))
          (draw :type 'delta :text "Parent output")
          (setf (romacs-agent-state session) 'idle)
          (deliver '(:type done :duration 1))
          (should (string-match-p "Read the docs  ·  Reading guide.org" (chrome)))
          (should (string-match-p "Parent output" (romacs-agent-log-tests--transcript)))
          (let ((romacs-agent-log-separator-rows-function (lambda () "A different queue\n")))
            (romacs-agent-log--separator t)
            (deliver '(:type task :id "a" :state progress :activity "Checking the result"))
            (should (string-match-p "Read the docs  ·  Checking the result" (chrome)))
            (should (string-suffix-p "A different queue\n❯ " (chrome))))
          (deliver '(:type task :id "a" :state completed :summary "Finished reading"))
          (should-not (romacs-agent-log-activity-bounds))
          (should-not romacs-agent-log-activity--threads)
          (deliver '(:type task :id "a" :state progress :activity "Stale update"))
          (should-not (string-match-p "Stale update\\|Read the docs" (chrome)))
          (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "draft"))
          (should (= (- (point) romacs-agent-log--input) 3)))))))

(ert-deftest romacs-agent-log-opens-an-inline-delegated-thread ()
  (romacs-agent-log-tests--with
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent--children (make-hash-table :test #'equal))
           (child (romacs-agent--make :id "worker" :title "Review the patch"
                                      :state 'running :options '(:parent "L")))
           (parent-buffer (current-buffer))
           (opened nil))
      (puthash "worker" child romacs-agent--sessions)
      (romacs-agent--index child)
      (save-window-excursion
        (switch-to-buffer parent-buffer)
        (goto-char (point-max))
        (insert "parent draft")
        (romacs-agent-ui--summarize-children session)
        (romacs-agent-log--activity-refresh)
        (let* ((bounds (romacs-agent-log-activity-bounds))
               (start (text-property-not-all (car bounds) (cdr bounds) 'romacs-agent-log-delegated-thread nil))
               (mark (or (romacs-icon 'romacs) (romacs-agent-glyph 'romacs))))
          (should (= (cdr bounds) romacs-agent-log--divider))
          (should (equal (get-text-property start 'romacs-agent-log-delegated-thread) "worker"))
          (should (string-search mark (buffer-substring-no-properties start (cdr bounds))))
          (should (= (length romacs-agent-log-activity--threads) 1))
          (let ((id (caar romacs-agent-log-activity--threads)))
            (should (eq (nth 3 (plist-get (romacs-working--element id) :kind))
                        'romacs-agent-log-delegated-work)))
          (save-excursion
            (goto-char start)
            (search-forward "Review the patch" (line-end-position))
            (should (eq (get-text-property (1- (point)) 'face) 'romacs-agent-log-delegated-title))
            (should (eq (face-attribute 'romacs-agent-log-delegated-title :underline) t)))
          (goto-char start)
          (should (eq (key-binding (kbd "RET")) 'romacs-agent-log-open))
          (cl-letf (((symbol-function 'romacs-agent-show)
                     (lambda (shown action) (setq opened (list shown action)))))
            (call-interactively (key-binding (kbd "RET"))))
          (should (equal opened (list child '(display-buffer-same-window)))))
        (setf (romacs-agent-state child) 'waiting-approval)
        (romacs-agent-ui--summarize-children session)
        (romacs-agent-log--activity-refresh)
        (pcase-let ((`(,start . ,end) (romacs-agent-log-activity-bounds)))
          (should (string-search "Needs approval" (buffer-substring-no-properties start end))))
        (should-not romacs-agent-log-activity--threads)
        (setf (romacs-agent-state child) 'idle)
        (romacs-agent-ui--summarize-children session)
        (romacs-agent-log--activity-refresh)
        (should-not (romacs-agent-log-activity-bounds))
        (should-not romacs-agent-log-activity--threads)
        (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "parent draft"))))))

(ert-deftest romacs-agent-log-remote-history-leaves-live-workers-alone ()
  (require 'romacs-agent-remote)
  (romacs-agent-log-tests--with
    (ignore #'draw)
    (let ((romacs-agent-ui--children-changed (make-hash-table :test #'eq))
          (romacs-agent-ui--children-timer 'coalesced))
      (cl-flet ((unit (state seq)
                  (romacs-mcp--object "seq" seq "events"
                                      (vector (romacs-agent-wire-event
                                               (list :type 'task :id "a" :task "a" :state state
                                                     :subagent t :title "Audit" :seq seq))))))
        (romacs-agent-remote--drawn session (list (unit 'started 5)) 5)
        (romacs-agent-log--draw-above
         session nil (lambda () (romacs-agent-remote--draw-unit session (unit 'completed 4))))
        (should (equal (plist-get (gethash "a" (romacs-agent-subagents session)) :state) "running"))
        (romacs-agent-remote--draw-unit session (unit 'completed 6) 'live)
        (romacs-agent-remote--draw-unit session (unit 'started 5) 'live)
        (should (equal (plist-get (gethash "a" (romacs-agent-subagents session)) :state) "completed"))))))

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
      ;; In plan mode too: plan mode is the harness's own.
      (setf (romacs-agent-options session) '(:plan t))
      (draw :type 'approval :request-id "q" :tool "Edit")
      (should-not (string-match-p "⚠" (romacs-agent-log-tests--transcript))))))

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
    (should (looking-at-p "\n+▌ ❯ two"))
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
      (should (string-match-p "│ hello world\n│ next\n│   ◧ " text))
      ;; Ended well: its row says its report's first line.
      (should (string-match-p "⑂ d · done\n" text)))
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
    (romacs-agent-menu--link "/tmp/shot one.png")
    (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "[image](</tmp/shot one.png>)"))))

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
    (should (equal (mapcar #'romacs-agent-log-ask-id romacs-agent-log--asks) '("c")))
    ;; In a replay, what the session waited on then decides, not now.
    (remhash "c" (romacs-agent-approvals session))
    (let ((romacs-agent-log--replaying (romacs-agent--make :id "L" :approvals (make-hash-table :test #'equal))))
      (puthash "c" t (romacs-agent-approvals romacs-agent-log--replaying))
      (romacs-agent-log-draw session '(:type done)))
    (should (equal (mapcar #'romacs-agent-log-ask-id romacs-agent-log--asks) '("c")))
    (draw :type 'exit :status 1)
    (should (string-match-p "the harness exited with it unanswered" (romacs-agent-log-tests--transcript)))
    (should-not romacs-agent-log--asks)
    (should-not romacs-agent-log--ended-asks)))

(ert-deftest romacs-agent-log-a-history-drawn-whole-says-each-ask-once ()
  ;; Another machine's history, drawn before what it waits on now: an ask
  ;; settled later in it says how, once; one left open says its turn ended.
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log--closes-deferred t))
      (draw :type 'approval :request-id "e" :tool "Bash")
      (draw :type 'approval :request-id "f" :tool "Bash")
      (draw :type 'done)
      (draw :type 'answered :request-id "e" :how 'harness))
    (should (equal (mapcar #'romacs-agent-log-owed-id romacs-agent-log--ended-asks) '("f")))
    (romacs-agent-log--sync-asks session)
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (= 1 (how-many "the turn ended with it unanswered" (point-min) (point-max))))
      (should (string-match-p "harness" text)))
    (should-not romacs-agent-log--asks)
    (should-not romacs-agent-log--ended-asks)))

(ert-deftest romacs-agent-log-an-ask-said-late-keeps-its-place ()
  ;; Said where its turn ended: a reply streaming since goes on unbroken.
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log--closes-deferred t))
      (draw :type 'approval :request-id "h" :tool "Bash")
      (draw :type 'done)
      (draw :type 'delta :message "m" :text "Hel"))
    (romacs-agent-log--sync-asks session)
    (draw :type 'delta :message "m" :text "lo there")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "Hello there" text))
      (should (< (string-search "the turn ended with it unanswered" text) (string-search "Hello" text))))))

(ert-deftest romacs-agent-log-an-ask-ends-where-it-first-ended ()
  ;; Its harness exited with it open; a later run's turn ends: it ended
  ;; at the exit, said once.
  (romacs-agent-log-tests--with
    (let ((romacs-agent-log--closes-deferred t))
      (draw :type 'approval :request-id "i" :tool "Bash")
      (draw :type 'exit :status 1)
      (draw :type 'delta :message "m2" :text "later run")
      (draw :type 'done))
    (romacs-agent-log--sync-asks session)
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (= 1 (how-many "the harness exited with it unanswered" (point-min) (point-max))))
      (should-not (string-match-p "the turn ended with it unanswered" text))
      (should (< (string-search "exited with it unanswered" text) (string-search "later run" text))))
    (should-not romacs-agent-log--ended-asks)))

(ert-deftest romacs-agent-log-an-answer-without-its-actor-says-who ()
  ;; A service older than :actor sends who alone.
  (romacs-agent-log-tests--with
    (draw :type 'attributed :request-id "g" :by "policy" :decision "allow" :scope "once")
    (should (string-match-p "allowed by Romacs" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-an-ask-let-go-after-its-exit-says-so ()
  ;; Another machine's service sends the exit before it lets the ask go:
  ;; said once it does, not lost.
  (romacs-agent-log-tests--with
    (puthash "d" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "d" :tool "Bash")
    (draw :type 'exit :status 1)
    (should-not (string-match-p "exited with it unanswered" (romacs-agent-log-tests--transcript)))
    (should (equal (mapcar #'romacs-agent-log-ask-id romacs-agent-log--asks) '("d")))
    (remhash "d" (romacs-agent-approvals session))
    (romacs-agent-log--sync-asks session)
    (romacs-agent-log--sync-asks session)
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "the harness exited with it unanswered" text))
      (should (= 1 (how-many "exited with it unanswered" (point-min) (point-max)))))
    (should-not romacs-agent-log--asks)
    (should-not romacs-agent-log--ended-asks)))

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
    (let ((live (romacs-agent-log-ask-start (romacs-agent-log--ask-of "live"))))
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

(ert-deftest romacs-agent-log-a-plan-takes-no-rule-for-the-session ()
  (romacs-agent-log-tests--with
    (puthash "p" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "p" :tool "ExitPlanMode" :plan t :user t)
    (let (answered)
      (cl-letf (((symbol-function 'romacs-agent-respond) (lambda (&rest args) (push args answered))))
        (should-error (romacs-agent-log-allow-always) :type 'user-error)
        (should-not answered)
        (romacs-agent-log-allow)
        (should (equal (nth 2 (car answered)) t))))))

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
    (should (romacs-agent-log--ask-of "q"))
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
    (goto-char (romacs-agent-log-ask-start (romacs-agent-log--ask-of "b")))
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
    (should-not (romacs-agent-log--ask-of "0:1"))
    (should (romacs-agent-log--ask-of "7:1"))))

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
    (goto-char (romacs-agent-log-ask-start (romacs-agent-log--ask-of "q9")))
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
    (should (string-match-p "\\$ sleep 60 · Running in the background\n│ · moved to the background\n│ · completed: slept\n\nroot goes on"
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
    ;; Each row says its own query and what came of it.
    (goto-char (search-forward "◍ one · A"))
    (goto-char (match-beginning 0))
    (should (equal (gethash "query" (car (get-text-property (point) 'romacs-agent-log-full))) "one"))
    (should (search-forward "◍ two · B" nil t))
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
      (should (string-match-p "⑂ count · 2 words\n" text))
      (should-not (string-match-p "hand-back\\|agentId\\|usage" text)))
    (romacs-agent-log--separator)
    (should (eq (car romacs-agent-log--state) 'working))))

(ert-deftest romacs-agent-log-an-answer-is-noted-with-its-actor ()
  (let ((session (romacs-agent--make :id "N" :harness 'claude :seq 0
                                     :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
        (request (make-hash-table :test #'equal))
        notes raised)
    (puthash "tool_name" "Bash" request)
    (puthash "b" request (romacs-agent-approvals session))
    (puthash "c" request (romacs-agent-approvals session))
    (cl-letf (((symbol-function 'romacs-agent--line) #'ignore)
              ((symbol-function 'romacs-agent--note) (lambda (_s note) (push note notes))))
      (romacs-agent-respond session "b" t nil 'user)
      ;; A request is answered once: a second answer is refused, noted by no one.
      (should-error (romacs-agent-respond session "b" nil "no" 'policy) :type 'user-error)
      (should (= (length notes) 1))
      (romacs-agent-respond session "c" nil "no" '(:kind policy :login "ci" :client "phone")))
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
  (should (equal (romacs-agent-rows-report "Plain words. The report follows:\nmore <usage>inline</usage> text")
                 "Plain words. The report follows:\nmore <usage>inline</usage> text"))
  (should (equal (romacs-agent-rows-report "[Subagent hand-back] frame. The report follows:\nbody\nagentId: a1\n<usage>t: 1</usage>\n")
                 "body"))
  (should (equal (romacs-agent-rows-report "[Subagent hand-back] f. The report follows:\nsee <usage>x</usage> here\nagentId: a\n<usage>t</usage>")
                 "see <usage>x</usage> here"))
  (should (equal (romacs-agent-rows-report "Example: <usage>x</usage>\nThe closing tag is </usage>")
                 "Example: <usage>x</usage>\nThe closing tag is </usage>")))

(ert-deftest romacs-agent-log-output-before-its-call-is-kept ()
  (romacs-agent-log-tests--with
    (draw :type 'tool-output :id "x" :text "line 1\n")
    (draw :type 'tool :state 'started :id "x" :name "Bash" :input (romacs-agent-log-tests--input "command" "seq 2"))
    (draw :type 'tool-output :id "x" :text "line 2\n")
    (should (string-match-p "⋮ running: 2 lines so far\n      line 1\n      line 2" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-interrupt-and-send-asks-the-live-session ()
  ;; A restored chat shows its row, idle; its session, loaded, runs a
  ;; turn: that turn is interrupted, the text sent first once it ends.
  (romacs-agent-log-tests--with
    (let ((live (romacs-agent--make :id "L" :harness 'claude :state 'running)) (calls nil))
      (setf (romacs-agent-state session) 'idle)
      (cl-letf (((symbol-function 'romacs-agent-loaded) (lambda (_) nil))
                ((symbol-function 'romacs-agent-get) (lambda (_) live))
                ((symbol-function 'romacs-agent-interrupt) (lambda (s) (push (list 'interrupt s) calls)))
                ((symbol-function 'romacs-agent-send-later) (lambda (s text &optional first &rest _) (push (list 'later s text first) calls)))
                ((symbol-function 'romacs-agent-send) (lambda (s text &rest _) (push (list 'send s text) calls))))
        (goto-char (point-max))
        (insert "stop that")
        (romacs-agent-log-interrupt-and-send))
      (should (equal (reverse calls) (list (list 'interrupt live) (list 'later live "stop that" t)))))))

;; A chat's source.

(cl-defstruct (romacs-agent-log-tests--source (:constructor romacs-agent-log-tests--source))
  calls approvals (chats (make-hash-table :test #'equal)))

(defun romacs-agent-log-tests--called (source &rest call)
  (push call (romacs-agent-log-tests--source-calls source)))

(cl-defmethod romacs-agent-log-source-fill ((source romacs-agent-log-tests--source) session)
  (romacs-agent-log-tests--called source 'fill)
  (romacs-agent-log-draw session '(:type input :text "from the source" :seq 1 :last t)))
(cl-defmethod romacs-agent-log-source-send ((source romacs-agent-log-tests--source) _session text &optional when)
  (romacs-agent-log-tests--called source 'send text when))
(cl-defmethod romacs-agent-log-source-interrupt ((source romacs-agent-log-tests--source) _session)
  (romacs-agent-log-tests--called source 'interrupt))
(cl-defmethod romacs-agent-log-source-respond ((source romacs-agent-log-tests--source) _session id allow reason)
  (romacs-agent-log-tests--called source 'respond id allow reason))
(cl-defmethod romacs-agent-log-source-seen ((source romacs-agent-log-tests--source) _session)
  (romacs-agent-log-tests--called source 'seen))
(cl-defmethod romacs-agent-log-source-chats ((source romacs-agent-log-tests--source))
  (romacs-agent-log-tests--source-chats source))
(cl-defmethod romacs-agent-log-source-approvals ((source romacs-agent-log-tests--source) _session)
  (romacs-agent-log-tests--source-approvals source))

(ert-deftest romacs-agent-log-reads-and-acts-through-its-source ()
  ;; A chat of another source draws what it gives and acts through it
  ;; alone: none of this editor's sessions is touched, its policy answers
  ;; none of its requests, and its session commands are not offered.
  (let* ((source (romacs-agent-log-tests--source :approvals (make-hash-table :test #'equal)))
         (session (romacs-agent--make :id "S-remote" :harness 'claude :state 'running :title "far"))
         (romacs-agent-approval-policy 'allow)
         (buffer (romacs-agent-log-chat session nil source)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-send) (lambda (&rest _) (error "Sent here")))
                  ((symbol-function 'romacs-agent-send-later) (lambda (&rest _) (error "Queued here")))
                  ((symbol-function 'romacs-agent-interrupt) (lambda (&rest _) (error "Interrupted here")))
                  ((symbol-function 'romacs-agent-respond) (lambda (&rest _) (error "Answered here")))
                  ((symbol-function 'romacs-agent-mark-seen) (lambda (&rest _) (error "Seen here"))))
          (with-current-buffer buffer
            (should (string-match-p "from the source" (buffer-string)))
            (should-not (romacs-agent-log--commands-p))
            (should-not romacs-agent-ui--session)
            (should-not desktop-save-buffer)
            (should (stringp warm-restart-left-out))
            ;; Drawn, it holds nothing typed; a draft typed, it does.
            (should-not (buffer-modified-p))
            (cl-flet ((type (text) (goto-char (point-max)) (insert text)))
              (type "now")
              (should (buffer-modified-p))
              (romacs-agent-log-send)
              (should-not (buffer-modified-p))
              (type "later") (romacs-agent-log-queue)
              (romacs-agent-log-interrupt)
              (type "after") (romacs-agent-log-interrupt-and-send))
            (puthash "r1" t (romacs-agent-log-tests--source-approvals source))
            (romacs-agent-log-draw session '(:type approval :request-id "r1" :tool "Bash"))
            (goto-char (point-max))
            (romacs-agent-log-allow)
            (romacs-agent-log-deny "no")
            (romacs-agent-log-show session nil source)
            (should (equal (reverse (romacs-agent-log-tests--source-calls source))
                           '((fill) (send "now" nil) (send "later" later) (interrupt) (send "after" interrupt)
                             (respond "r1" t nil) (respond "r1" nil "no") (seen)))))
          ;; This editor's own session of the same id has a chat of its own.
          (let ((local (romacs-agent-log-chat (romacs-agent--make :id "S-remote" :harness 'claude :state 'idle) t)))
            (unwind-protect
                (progn (should-not (eq local buffer))
                       (should (eq (buffer-local-value 'romacs-agent-log--source local) romacs-agent-log-local))
                       (with-current-buffer local
                         (should (romacs-agent-log--commands-p))
                         (should romacs-agent-ui--session)
                         (should desktop-save-buffer)))
              (remhash "S-remote" romacs-agent-log--chats)
              (kill-buffer local))))
      (kill-buffer buffer))))

;;; A reply's faces as it streams

(ert-deftest romacs-agent-log-a-reply-is-faced-as-it-streams ()
  ;; Each batch faces what came; the turn's end the rest.
  (romacs-agent-log-tests--with
    (draw :type 'delta :text "Some **bold** words.\n\n```elisp\n(message \"hi\")\n")
    (romacs-agent-log-faces--batch (current-buffer))
    (goto-char (point-min))
    (search-forward "bold")
    (should (memq 'romacs-markdown-strong (ensure-list (get-text-property (match-beginning 0) 'face))))
    (search-forward "\"hi")
    (should (memq 'font-lock-string-face (ensure-list (get-text-property (match-beginning 0) 'face))))
    (draw :type 'delta :text "```\n\nAnd *more*.")
    (draw :type 'done :time 0)
    (goto-char (point-min))
    (search-forward "more")
    (should (memq 'romacs-markdown-emphasis (ensure-list (get-text-property (match-beginning 0) 'face))))))

(ert-deftest romacs-agent-log-a-reply-whose-faces-fail-says-so ()
  ;; Its text stays; a line under it says why.
  (romacs-agent-log-tests--with
    (cl-letf (((symbol-function 'romacs-markdown-fontify-region) (lambda (&rest _) (error "Parser broke"))))
      (draw :type 'delta :text "Some text.")
      (draw :type 'done :time 0))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "Some text\\." text))
      (should (string-match-p "⚠ shown without its faces: Parser broke" text)))))

;;; What a call came to

(ert-deftest romacs-agent-log-a-call-says-what-it-came-to-not-its-payload ()
  (romacs-agent-log-tests--with
    ;; A Romacs tool's JSON answer: its row says it, nothing under it.
    (draw :type 'tool :state 'started :id "s" :name "mcp__romacs__romacs_task_status"
          :input (romacs-agent-log-tests--input "taskId" "t1"))
    (draw :type 'tool :state 'completed :id "s" :output "{\"title\":\"lane x\",\"status\":\"running\",\"result\":\"a long report\"}")
    ;; A read: how much, its text RET away.
    (draw :type 'tool :state 'started :id "r" :name "Read" :input (romacs-agent-log-tests--input "file_path" "/a/n.txt"))
    (draw :type 'tool :state 'completed :id "r" :output "one\ntwo")
    ;; A failure: its error, as it says it.
    (draw :type 'tool :state 'started :id "f" :name "Bash" :input (romacs-agent-log-tests--input "command" "make"))
    (draw :type 'tool :state 'failed :id "f" :output "<tool_use_error>make: no rule</tool_use_error>")
    ;; A command's tail without the harness's own trailer.
    (draw :type 'tool :state 'started :id "l" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'tool :state 'completed :id "l" :output "a.txt\nShell cwd was reset to /home/x")
    (should-not (string-match-p "Shell cwd" (romacs-agent-log-tests--transcript)))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "◷ lane x · running\n" text))
      (should-not (string-match-p "a long report\\|{" text))
      (should (string-match-p "◧ n\\.txt  /a · 2 lines\n" text))
      (should-not (string-match-p "⎿ one" text))
      (should (string-match-p "  ✘ \\$ make\n    ⎿ make: no rule\n" text))
      (goto-char (point-min))
      (search-forward "make: no rule")
      (should (memq 'romacs-agent-log-denied (ensure-list (get-text-property (match-beginning 0) 'face))))
      ;; RET on a row: its details, then the raw answer.
      (goto-char (point-min))
      (search-forward "lane x")
      (save-window-excursion (romacs-agent-log-open))
      (with-current-buffer "*agent output*"
        ;; Its renderer's details first, then the raw answer.
        (should (string-match-p "lane x · running\n\na long report\n\nInput:" (buffer-string)))
        (should (string-match-p "Output:\n{" (buffer-string)))
        (kill-buffer)))
    ;; A title of several lines: the row on one line, under its mark.
    (draw :type 'tool :state 'started :id "d" :name "mcp__romacs__romacs_delegate_task"
          :input (romacs-agent-log-tests--input "title" "two\nlines"))
    (draw :type 'tool :state 'completed :id "d" :output "{\"title\":\"two\\nlines\",\"state\":\"running\"}")
    (should (string-match-p "⑂ two lines · started\n" (romacs-agent-log-tests--transcript)))
    ;; A subagent's call: RET on its rewritten row shows its details too.
    (draw :type 'tool :state 'started :id "a" :name "Agent" :input (romacs-agent-log-tests--input "description" "sub"))
    (draw :type 'tool :state 'started :id "n" :name "mcp__romacs__romacs_task_status" :parent "a"
          :input (romacs-agent-log-tests--input "taskId" "t2"))
    (draw :type 'tool :state 'completed :id "n" :parent "a" :output "{\"title\":\"inner\",\"status\":\"done\",\"result\":\"nested report\"}")
    (goto-char (point-min))
    (search-forward "◷ inner")
    (save-window-excursion (romacs-agent-log-open))
    (with-current-buffer "*agent output*"
      (should (string-match-p "nested report\n\nInput:" (buffer-string)))
      (kill-buffer))
    (should-not (string-match-p "^lines" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-a-streamed-reply-keeps-its-link-definitions ()
  ;; A definition before or after its reference, across batches: faced as
  ;; the whole reply would be.
  (dolist (pieces '(("[r]: https://example.org\n\nSee [docs][r].\n\nTail." " more.")
                    ("See [docs][r].\n\nMid.\n\n" "[r]: https://example.org\n\nTail.")
                    ;; In a quote, after the reference has gone by.
                    ("See [docs][r].\n\nOne.\n\nTwo.\n\n" "> [r]: https://example.org\n\nTail.")
                    ;; Its label across two lines.
                    ("See [docs][two words].\n\nOne.\n\nTwo.\n\n" "[two\nwords]: https://example.org\n\nTail.")))
    (romacs-agent-log-tests--with
      (dolist (piece pieces)
        (draw :type 'delta :text piece)
        (romacs-agent-log-faces--batch (current-buffer)))
      (goto-char (point-min))
      (search-forward "docs")
      (should (memq 'romacs-markdown-link (ensure-list (get-text-property (match-beginning 0) 'face))))
      (draw :type 'done :time 0)
      (search-backward "docs")
      (should (memq 'romacs-markdown-link (ensure-list (get-text-property (point) 'face)))))))

(ert-deftest romacs-agent-log-a-definition-in-code-is-no-definition ()
  ;; A [key]: value line in a code block leaves the cheap restart on.
  (romacs-agent-log-tests--with
    (draw :type 'delta :text "```ts\n[key]: value\n```\n\nAn [inline](http://[::1]:8080/) link.\n\nAfter.")
    (romacs-agent-log-faces--batch (current-buffer))
    (should-not romacs-agent-log-faces--whole)))

(ert-deftest romacs-agent-log-header-key-takes-any-colour-name ()
  ;; A theme's named colours, not #rrggbb: the key's face is still made.
  (cl-letf (((symbol-function 'face-foreground) (lambda (&rest _) "grey90"))
            ((symbol-function 'face-background) (lambda (&rest _) "white")))
    (romacs-agent-log--derive)))

;;; Rounds

(ert-deftest romacs-agent-log-rounds-tell-the-user-from-elsewhere ()
  (should (equal (romacs-agent-incoming-of "<romacs-message id=\"m-1\" kind=\"parent\" from=\"lead\">\nGo on\n</romacs-message>\nReply to this message with romacs_task_reply(messageId=\"m-1\", text=...).")
                 '((:from "lead" :state "parent" :text "Go on"))))
  (should-not (romacs-agent-incoming-of "my own words"))
  (romacs-agent-log-tests--with
    ;; The user's message: its surface.
    (draw :type 'input :text "do it" :request 1)
    ;; A delegated task's result, written as a turn: from elsewhere.
    (draw :type 'input :text (concat "Delegated task 19a4803e-9e10-44f0-bf91-824484dcfb8c (lane x) [completed, 4 chars]:\n"
                                     "done\n\nReply to this result with romacs_task_reply(messageId=\"m-x\", text=...)."))
    ;; Claude Code's own notice of a background task.
    (dolist (event (romacs-agent--claude-injected "<task-notification>\n<status>completed</status>\n<summary>Build done</summary>\n</task-notification>" nil))
      (romacs-agent-log-draw session event))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "❯ do it" text))
      (should (string-match-p "┃ ← lane x · completed\n┃ done" text))
      (should (string-match-p "┃ ← background task · completed\n┃ Build done" text))
      (should-not (string-match-p "❯ Delegated" text)))
    (goto-char (point-min))
    (search-forward "do it")
    (should (romacs-agent-log-tests--face-p 'romacs-agent-log-you-bar (get-text-property (line-beginning-position) 'face)))
    (search-forward "lane x")
    (should (memq 'romacs-agent-log-other-head (ensure-list (get-text-property (match-beginning 0) 'face))))
    ;; Nothing the GUI host does not draw.
    (should-not (text-property-not-all (point-min) (point-max) 'line-prefix nil))
    (should-not (text-property-not-all (point-min) (point-max) 'wrap-prefix nil))
    (should-not (romacs-agent-log-tests--face-p 'romacs-agent-log-you-bar (get-text-property (line-beginning-position) 'face))))
  ;; Only whole envelopes, each of a batch; anything else is the user's.
  (should (equal (mapcar (lambda (i) (plist-get i :text))
                         (romacs-agent-incoming-of (concat "<romacs-message id=\"1\" kind=\"child\" from=\"a\">\none\n</romacs-message>\nReply to this message with romacs_task_reply(messageId=\"1\", text=...).\n\n"
                                                           "<romacs-message id=\"2\" kind=\"child\" from=\"b\">\ntwo\n</romacs-message>\nReply to this message with romacs_task_reply(messageId=\"2\", text=...).")))
                 '("one" "two")))
  (should-not (romacs-agent-incoming-of "<romacs-message-example> is a tag I mean"))
  ;; Only as its sender writes it: with the line saying how to answer.
  (should-not (romacs-agent-incoming-of "<romacs-message id=\"1\" from=\"a\">\nan example\n</romacs-message>"))
  ;; A line of the user's own after it is no sender's footer.
  (should-not (romacs-agent-incoming-of "<romacs-message id=\"1\" from=\"a\">\nan example\n</romacs-message>\nReply to this message with an explanation of these tags."))
  (let ((message (romacs-agent-make-message "m-3" "external" '(:name "far") nil "from outside")))
    (should (equal (romacs-agent-incoming-of (romacs-agent--message-envelope message "from outside"))
                   '((:from "far" :state "external" :text "from outside")))))
  (should-not (romacs-agent-incoming-of "<romacs-message id=\"1\">\nnever closed"))
  ;; Delivered by Romacs: each message its own, a wake's state its state.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "x" :request 9
          :messages (list (list :kind "child" :from '(:name "lane a") :wake '(:state "completed") :text "A done")
                          (list :kind "child" :from '(:name "lane b") :text "B says")))
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "┃ ← lane a · completed\n┃ A done" text))
      (should (string-match-p "┃ ← lane b · child\n┃ B says" text)))
    ;; Its turn anchored: diff and rollback read it.
    (goto-char (point-min))
    (search-forward "A done")
    (should (equal (get-text-property (match-beginning 0) 'romacs-agent-log-turn) 9))))

(ert-deftest romacs-agent-log-an-arrival-mid-turn-ends-the-round-before ()
  ;; A notification arriving within a turn: the words before it recede.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "go" :request 1)
    (draw :type 'delta :text "Working on it.")
    (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'tool :state 'completed :id "t" :output "a")
    (draw :type 'delta :text "Started the build.")
    (draw :type 'arrival :from "background task" :state "completed" :text "Build done" :full "<task-notification>…")
    (draw :type 'delta :text "The build passed.")
    (draw :type 'done :time 0)
    (goto-char (point-min))
    (search-forward "Working on")
    (should (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))
    (search-forward "Started the build")
    (should-not (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))
    (search-forward "The build passed")
    (should-not (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))))

(ert-deftest romacs-agent-log-a-round-s-answer-stands-out ()
  ;; Its end: the agent's words before its last recede.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "go" :request 1)
    (draw :type 'delta :text "Looking first.")
    (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'tool :state 'completed :id "t" :output "a")
    (draw :type 'delta :text "Here is the answer.")
    (draw :type 'done :time 0)
    (goto-char (point-min))
    (search-forward "Looking")
    (should (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))
    (search-forward "the answer")
    (should-not (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))))

(ert-deftest romacs-agent-log-an-exit-ends-the-round ()
  ;; A harness that exits mid-turn: its round ends, its words before the
  ;; last recede.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "go" :request 1)
    (draw :type 'delta :text "First words.")
    (draw :type 'tool :state 'started :id "t" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'tool :state 'completed :id "t" :output "a")
    (draw :type 'delta :text "Last words.")
    (draw :type 'exit :status 1)
    (goto-char (point-min))
    (search-forward "First words")
    (should (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))))))

(ert-deftest romacs-agent-log-a-hidden-request-is-not-answered-blind ()
  ;; Narrowed away from what waits, C-c C-y says so rather than answer.
  (romacs-agent-log-tests--with
    (draw :type 'delta :text "before")
    (puthash "q" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "q" :tool "Bash" :input (romacs-agent-log-tests--input "command" "make"))
    (let (answered)
      (cl-letf (((symbol-function 'romacs-agent-respond) (lambda (&rest args) (push args answered))))
        (save-restriction
          (narrow-to-region (point-min) (+ (point-min) 3))
          (goto-char (point-min))
          (should-error (romacs-agent-log-allow) :type 'user-error)))
      (should-not answered))))

;;; Keys

(defconst romacs-agent-log-tests--chat-keys
  '(("RET" . romacs-agent-log-send) ("C-j" . newline) ("S-<return>" . newline)
    ("C-c C-c" . romacs-agent-log-interrupt) ("C-c C-i" . romacs-agent-log-interrupt)
    ("C-c C-k" . romacs-agent-log-interrupt-and-send) ("C-c C-q" . romacs-agent-log-queue)
    ("C-c C-s" . romacs-agent-log-steer) ("C-c C-a" . romacs-agent-chat-attach)
    ("C-c C-d" . romacs-agent-log-deny) ("C-c C-o" . romacs-agent-menu-map-popup)
    ("C-c C-/" . describe-mode) ("C-c C-n" . romacs-agent-chat-new)
    ("C-c C-r" . romacs-agent-chat-rename) ("C-c C-e" . romacs-agent-chat-effort)
    ("C-c C-b" . romacs-agents) ("C-c C-t" . romacs-agents)
    ("C-c C-j" . romacs-agent-log-composer)
    ("M-p" . romacs-agent-log-history-previous) ("M-n" . romacs-agent-log-history-next)
    ;; Dropped with what they acted on: no prompt is predicted to
    ;; complete, and an attached file is a path in the composer's text,
    ;; with no tray of images to take one off.
    ("TAB") ("C-c C-w"))
  "The keys of the chat before this one, each with its command here, or
none when dropped.")

(ert-deftest romacs-agent-log-has-the-keys-of-the-chat-before ()
  (dolist (key romacs-agent-log-tests--chat-keys)
    (should (equal (cons (car key) (keymap-lookup romacs-agent-log-mode-map (car key)))
                   key))))

(defun romacs-agent-log-tests--commands (map)
  (let (commands)
    (map-keymap (lambda (_ binding)
                  ;; A menu row's (menu-item NAME COMMAND . PROPS), its
                  ;; filter not run.
                  (when (eq (car-safe binding) 'menu-item) (setq binding (nth 2 binding)))
                  (cond ((keymapp binding) (setq commands (append (romacs-agent-log-tests--commands binding) commands)))
                        ((and binding (symbolp binding)) (push binding commands))))
                map)
    commands))

(ert-deftest romacs-agent-log-every-key-runs-in-the-chat ()
  ;; A command a chat's key or menu row runs is defined, and offered for
  ;; this mode, not for a mode since deleted.
  (should (memq 'romacs-agent-chat-rename (romacs-agent-log-tests--commands romacs-agent-menu-map)))
  (dolist (command (append (romacs-agent-log-tests--commands romacs-agent-log-mode-map)
                           (romacs-agent-log-tests--commands romacs-agent-log-ask-map)
                           (romacs-agent-log-tests--commands romacs-agent-menu-map)))
    (let ((modes (command-modes command)))
      ;; Named, so a failure says which.
      (should (equal (list command (commandp command) (or (null modes) (and (memq 'romacs-agent-log-mode modes) t)))
                     (list command t t))))))

(ert-deftest romacs-agent-log-session-keys-act-on-the-chat ()
  ;; From the transcript: C-c C-a puts a path in the composer, C-c C-s
  ;; sends it, C-c C-r names the session.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "first" :request 4)
    (draw :type 'delta :text "reply")
    (goto-char (point-min))
    (let (sent)
      (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) #'ignore)
                ((symbol-function 'read-file-name) (lambda (&rest _) "/tmp/notes.org"))
                ((symbol-function 'read-string) (lambda (&rest _) "named"))
                ((symbol-function 'romacs-agent--update-manifest) #'ignore)
                ((symbol-function 'romacs-agent--changed) #'ignore)
                ((symbol-function 'romacs-agent-ui--batch) #'ignore)
                ((symbol-function 'romacs-agent-log--loaded) #'identity)
                ((symbol-function 'romacs-agent-send) (lambda (_s text &rest _) (setq sent text))))
        (call-interactively (keymap-lookup romacs-agent-log-mode-map "C-c C-a"))
        (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "[file](/tmp/notes.org)"))
        (goto-char (point-min))
        (call-interactively (keymap-lookup romacs-agent-log-mode-map "C-c C-s"))
        (should (equal sent "[file](/tmp/notes.org)"))
        (should (= romacs-agent-log--input (point-max)))
        (call-interactively (keymap-lookup romacs-agent-log-mode-map "C-c C-r"))
        (should (equal (romacs-agent-title session) "named"))))))

(ert-deftest romacs-agent-log-attach-links-the-clipboard-s-image ()
  ;; C-c C-a with an image in the clipboard links the file it is, where
  ;; it is, at the composer's end.
  (romacs-agent-log-tests--with
    (ignore #'draw)
    (goto-char (point-max))
    (insert "see")
    (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'romacs-agent-log--loaded) #'identity)
              ((symbol-function 'romacs-attach-clipboard-file)
               (lambda (type then)
                 (should (equal type "image/png"))
                 (funcall then "/pictures/Screenshots/shot.png"))))
      (call-interactively (keymap-lookup romacs-agent-log-mode-map "C-c C-a"))
      (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max))
                     "see [image](/pictures/Screenshots/shot.png)")))))

;;; A command's output

(ert-deftest romacs-agent-log-output-lines-are-one-row-each ()
  ;; A long line is cut to the width with an ellipsis, never wrapped.
  (let* ((romacs-agent-log-output-width 20)
         (shown (car (romacs-agent-log--lines (concat (make-string 50 ?x) "\nshort") 5))))
    (should (equal (split-string shown "\n")
                   (list (truncate-string-to-width (make-string 50 ?x) 20 nil nil (truncate-string-ellipsis)) "short")))))

(ert-deftest romacs-agent-log-output-faces-a-diagnostic-whole ()
  ;; All of a diagnostic line, from its first character; a plain line none.
  (let ((text (romacs-agent-log-faces-output "sed: can't read x.el: No such file or directory\nrg: a: IO error (os error 2)\nplain")))
    (dolist (at '(0 5 48 52))
      (should (memq 'error (ensure-list (get-text-property at 'face text)))))
    (should-not (memq 'error (ensure-list (get-text-property (- (length text) 2) 'face text))))))

;;; The user's surface

(ert-deftest romacs-agent-log-rounds-surface-is-a-block-that-shows ()
  ;; On black as on white, apart from the background.
  (dolist (pair '(("#000000" . "#e0e0e0") ("#ffffff" . "#202020") ("#121212" . "#bebebe") ("#000000" . "#767676")))
    (should (>= (romacs-color-contrast (romacs-color-band (car pair) (cdr pair)) (car pair)) 1.15)))
  ;; None that would hide the text.
  (should-not (romacs-color-band "#000000" "#141414"))
  ;; Apart, but the text would not read on it.
  (should-not (romacs-color-band "#000000" "#505050"))
  ;; Rounded mixes: a nearby one that meets both floors is found.
  (let ((band (romacs-color-band "#3f51c7" "#16e827" 1.3)))
    (should band)
    (should (>= (romacs-color-contrast band "#3f51c7") 1.3))
    (should (>= (romacs-color-contrast "#16e827" band) 3.0)))
  ;; Below the first step: the least mix, not the step's.
  (should (< (romacs-color-contrast (romacs-color-band "#ffffff" "#000000" 1.01) "#ffffff") 1.05))
  ;; The least band that stands apart, found between steps.
  (let ((band (romacs-color-band "#090909" "#696969" 1.2)))
    (should band)
    (should (>= (romacs-color-contrast band "#090909") 1.2))
    (should (>= (romacs-color-contrast "#696969" band) 3.0)))
  ;; Each line of the user's message behind its bar, blank ones too.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "first" :request 1)
    (draw :type 'input :text "second\n\nthird" :request 2)
    (goto-char (point-min))
    (search-forward "second")
    (forward-line 0)
    (should (looking-at "▌ ❯ second\n▌ \n▌ third\n"))))

;;; From elsewhere, quoted

(ert-deftest romacs-agent-log-rounds-quote-what-came-from-elsewhere ()
  ;; Its head, its lines behind a bar, a blank line after; the bar is no
  ;; part of what one copies.
  (romacs-agent-log-tests--with
    (draw :type 'input :text (concat "<romacs-message id=\"m-1\" kind=\"parent\" from=\"lead\">\nGo on\nand on\n</romacs-message>\n"
                                     "Reply to this message with romacs_task_reply(messageId=\"m-1\", text=...)."))
    (draw :type 'delta :text "On it.")
    (let ((text (romacs-agent-log-tests--transcript)))
      (should (string-match-p "┃ ← lead · parent\n┃ Go on\n┃ and on\n\nOn it." text)))
    ;; Two in a row: each a round, the air a round opens after between them.
    (draw :type 'input :text (concat "<romacs-message id=\"m-2\" kind=\"parent\" from=\"lead\">\nAgain\n</romacs-message>\n"
                                     "Reply to this message with romacs_task_reply(messageId=\"m-2\", text=...)."))
    (draw :type 'input :text (concat "<romacs-message id=\"m-3\" kind=\"parent\" from=\"lead\">\nThird\n</romacs-message>\n"
                                     "Reply to this message with romacs_task_reply(messageId=\"m-3\", text=...)."))
    (should (string-match-p "┃ Again\n\n\n┃ ← lead · parent\n┃ Third" (romacs-agent-log-tests--transcript)))
    ;; A line cut short: RET shows the rest.
    (let ((romacs-agent-log-output-width 20) (truncate-string-ellipsis "…"))
      (draw :type 'input :text (concat "<romacs-message id=\"m-4\" kind=\"parent\" from=\"lead\">\n" (make-string 40 ?x) "\n</romacs-message>\n"
                                       "Reply to this message with romacs_task_reply(messageId=\"m-4\", text=...).")))
    (should (string-match-p "┃ x+…\n┃ … [^\n]* shows all" (romacs-agent-log-tests--transcript)))
    (goto-char (point-min))
    (search-forward "Go on")
    (should (equal (filter-buffer-substring (line-beginning-position) (line-end-position)) "Go on"))))

;;; A stream that starts with line breaks

(ert-deftest romacs-agent-log-stream-marks-its-first-text ()
  ;; Line breaks first, then text: the entry starts at the text, with no
  ;; blank lines before it; a later reply after work stands apart.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "go" :request 1)
    (draw :type 'delta :text "\n\n")
    (draw :type 'delta :text "Hello")
    (should (string-match-p "❯ go\n\nHello" (romacs-agent-log-tests--transcript)))
    (goto-char (point-min))
    (search-forward "Hello")
    (should (eq (get-text-property (match-beginning 0) 'romacs-agent-log-entry) 'text))
    (draw :type 'tool :state 'started :id "t1" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'delta :text "After.")
    (should (string-match-p "\n\nAfter\\." (romacs-agent-log-tests--transcript)))))

;;; Blank lines, one at a time

(ert-deftest romacs-agent-log-blank-lines-are-one ()
  (romacs-agent-log-tests--with
    ;; Two prompts, two rounds: the air a round opens after between them.
    (draw :type 'input :text "one" :request 1)
    (draw :type 'input :text "two" :request 2)
    (should (equal (romacs-agent-log-tests--transcript) "▌ ❯ one\n\n\n▌ ❯ two\n\n"))
    ;; Thinking of line breaks only, a call, then the reply: no trace of
    ;; the thinking, one blank line before the reply.
    (draw :type 'thinking :text "\n\n")
    (draw :type 'tool :state 'started :id "t1" :name "Bash" :input (romacs-agent-log-tests--input "command" "ls"))
    (draw :type 'delta :text "Done.")
    (should (string-match-p "\\`▌ ❯ one\n\n\n▌ ❯ two\n\n  \\$ ls\n\nDone\\.\n\\'" (romacs-agent-log-tests--transcript)))))

(ert-deftest romacs-agent-log-rounds-arrival-without-its-call-is-top-level ()
  ;; Its parent call never drawn: a round of its own at the top level.
  (romacs-agent-log-tests--with
    (draw :type 'input :text "go" :request 1)
    (dolist (event (romacs-agent--claude-injected "<task-notification>\n<status>completed</status>\n<summary>Build done</summary>\n</task-notification>" "missing-call"))
      (romacs-agent-log-draw session event))
    (goto-char (point-min))
    (search-forward "background task")
    (should (text-property-any (point-min) (point) 'romacs-agent-log-round t))
    (let ((round (text-property-any (1+ (text-property-any (point-min) (point-max) 'romacs-agent-log-round t)) (point-max) 'romacs-agent-log-round t)))
      (should round)
      (should (< round (point))))
    (should (string-match-p "Build done\n\n\\'" (romacs-agent-log-tests--transcript)))))

;;; Under a call

(ert-deftest romacs-agent-log-rounds-quote-under-a-call-opens-from-its-indent ()
  ;; A quote under a subagent's call: RET at a row's first column, on the
  ;; call's bar, opens it whole.
  (romacs-agent-log-tests--with
    (draw :type 'tool :state 'started :id "a1" :name "Agent" :input (romacs-agent-log-tests--input "description" "sub"))
    (let ((romacs-agent-log-rounds-other-lines 1))
      (dolist (event (romacs-agent--claude-injected "<task-notification>\n<status>completed</status>\n<summary>one\ntwo\nthree</summary>\n</task-notification>" "a1"))
        (romacs-agent-log-draw session event)))
    (goto-char (point-min))
    (search-forward "shows all")
    (should (string-prefix-p "│ " (buffer-substring-no-properties (line-beginning-position) (line-end-position))))
    (should (get-text-property (line-beginning-position) 'romacs-agent-log-full))))

;;; A streamed batch's faces, in the cycle that draws it

(ert-deftest romacs-agent-log-faces-come-before-the-redisplay ()
  ;; A delta's faces wait for the redisplay that draws it, on the chat's
  ;; own pre-redisplay-functions, never on a timer of their own.
  (romacs-agent-log-tests--with
    (let ((timers (length timer-list))
          (strong (lambda ()
                    (goto-char (point-min))
                    (search-forward "bold")
                    (memq 'romacs-markdown-strong (ensure-list (get-text-property (1- (point)) 'face))))))
      (draw :type 'delta :text "Some **bold** words.")
      (should (= (length timer-list) timers))
      (should (memq #'romacs-agent-log-faces--before-redisplay pre-redisplay-functions))
      (should-not (funcall strong))
      (run-hook-with-args 'pre-redisplay-functions (selected-window))
      (should (funcall strong))
      (should-not (memq #'romacs-agent-log-faces--before-redisplay pre-redisplay-functions)))))

;; Plain words after plain text: nothing to face, no batch waits.
(ert-deftest romacs-agent-log-faces-plain-words-need-no-batch ()
  (romacs-agent-log-tests--with
    (draw :type 'delta :text "Some **bold** words")
    (run-hook-with-args 'pre-redisplay-functions (selected-window))
    (draw :type 'delta :text " and more plain words, as most of a stream is")
    (should-not (memq #'romacs-agent-log-faces--before-redisplay pre-redisplay-functions))
    (draw :type 'delta :text " then `code`")
    (should (memq #'romacs-agent-log-faces--before-redisplay pre-redisplay-functions))
    (run-hook-with-args 'pre-redisplay-functions (selected-window))
    (goto-char (point-min))
    (search-forward "code")
    (should (get-text-property (match-beginning 0) 'face))))

;; What plain characters may still change: a batch waits for each.
(ert-deftest romacs-agent-log-faces-plain-words-that-make-a-block-wait ()
  (dolist (case '(("1" ". item")                  ; an ordered list
                  ("Intro\n\n    " "echo")       ; indented code
                  ("[id]\n\n[id]:" " target")   ; a link definition
                  ("see www" ".example.com")     ; an autolink
                  ("see " "www.example.com")
                  ("> hello\n " "world")         ; a quote's lazy line
                  ("[id]\n\n[id]:\n " "target")))  ; a definition's next line
    (romacs-agent-log-tests--with
      (draw :type 'delta :text (car case))
      (run-hook-with-args 'pre-redisplay-functions (selected-window))
      (draw :type 'delta :text (cadr case))
      (should (memq #'romacs-agent-log-faces--before-redisplay pre-redisplay-functions)))))

(ert-deftest romacs-agent-log-faces-a-definition-on-its-next-line-faces-its-reference ()
  (romacs-agent-log-tests--with
    (draw :type 'delta :text "[id]\n\n[id]:\n ")
    (run-hook-with-args 'pre-redisplay-functions (selected-window))
    (draw :type 'delta :text "target")
    (run-hook-with-args 'pre-redisplay-functions (selected-window))
    (goto-char (point-min))
    (search-forward "[id]")
    (should (get-text-property (1+ (match-beginning 0)) 'face))))

(provide 'romacs-agent-log-tests)
;;; romacs-agent-log-tests.el ends here

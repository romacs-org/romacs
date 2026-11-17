;;; romacs-agent-restored-chat-tests.el --- A chat a start brings back -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-agent-test-stored)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))

(defconst romacs-agent-restored-chat-tests--id "7e570000-1111-4222-8333-444455556666")

(defun romacs-agent-restored-chat-tests--record (seq key value)
  (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq)))

(ert-deftest romacs-agent-restored-chat-replays-when-first-shown ()
  ;; A start brings back every chat that was open: none reads its journal
  ;; until a window shows it, then it has its whole transcript, its draft
  ;; kept below it.
  (let* ((root (make-temp-file "romacs-restored-" t))
         ;; Its sockets' directory is short: TMPDIR may not be.
         (run (romacs-test-socket-directory "restored-"))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" run))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (dir (expand-file-name romacs-agent-restored-chat-tests--id (romacs-agent--state-root)))
         (replays 0))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" romacs-agent-restored-chat-tests--id "harness" "claude"
                                                        "provider" "claude" "cwd" root "title" "restored" "titleAuto" :false
                                                        "argv" [] "options" (romacs-mcp--object) "seenSeq" 0
                                                        "hostSocket" (expand-file-name "none/host.sock" root)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n"))
                                     (list (romacs-agent-restored-chat-tests--record
                                            1 "in" (romacs-mcp--object "type" "user" "message" (romacs-mcp--object "content" "first question")))
                                           (romacs-agent-restored-chat-tests--record
                                            2 "out" (romacs-mcp--object
                                                     "type" "assistant" "parent_tool_use_id" :null
                                                     "message" (romacs-mcp--object
                                                                "id" "m1" "content"
                                                                (vector (romacs-mcp--object "type" "text" "text" "first answer")))))
                                           (romacs-agent-restored-chat-tests--record
                                            3 "out" (romacs-mcp--object "type" "result" "result" "first answer")))
                                     "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          ;; As a chat finds it: its records in SQLite, kept as drawn.
          (remhash romacs-agent-restored-chat-tests--id romacs-agent-events--imported)
          (cl-letf (((symbol-function 'romacs-agent--keep-models) #'ignore))
            (romacs-agent-test-stored (romacs-agent-load-session romacs-agent-restored-chat-tests--id)))
          ;; As a start finds it: not loaded, its row alone known.
          (remhash romacs-agent-restored-chat-tests--id romacs-agent--sessions)
          (cl-letf* ((replay (symbol-function 'romacs-agent-ui--replay))
                     ((symbol-function 'romacs-agent-ui--replay)
                      (lambda (session) (cl-incf replays) (funcall replay session)))
                     ((symbol-function 'romacs-agent--keep-models) #'ignore))
            (let* ((buffer (romacs-agent-ui-desktop-restore
                            nil nil (list :session romacs-agent-restored-chat-tests--id :draft "half a question")))
                   ;; Its row alone, no session loaded.
                   (session (buffer-local-value 'romacs-agent-ui--session buffer)))
              (should-not (romacs-agent-loaded romacs-agent-restored-chat-tests--id))
              ;; Restored: its draft, no journal read for its transcript.
              (should (= replays 0))
              (should (equal (with-current-buffer buffer (romacs-agent-ui--composer-text)) "half a question"))
              ;; An event while not yet shown is not drawn: its replay will.
              (romacs-agent-ui--render session (list :type 'input :text "second question" :seq 4 :last t))
              (should-not (string-search "second question" (with-current-buffer buffer (buffer-string))))
              ;; Shown: replayed once, the transcript above the draft.
              (with-current-buffer buffer (romacs-agent-ui--shown-again))
              (romacs-agent-ui--shown-again (progn (switch-to-buffer buffer) (selected-window)))
              (should (= replays 1))
              (with-current-buffer buffer
                (should (string-search "first question" (buffer-string)))
                (should (string-search "first answer" (buffer-string)))
                (should (equal (romacs-agent-ui--composer-text) "half a question")))
              ;; Its first use loads it, the chat its live session's, drawn
              ;; again from its records (no supervisor replays them).
              (with-current-buffer buffer
                (let ((this-command 'romacs-agent-chat-send)) (romacs-agent-ui--ensure-live)))
              (setq session (romacs-agent-get romacs-agent-restored-chat-tests--id))
              (should (eq (buffer-local-value 'romacs-agent-ui--session buffer) session))
              (should (= replays 2))
              ;; Live events draw as before.
              (romacs-agent-ui--render session (list :type 'input :text "third question" :seq 5 :last t))
              (should (string-search "third question" (with-current-buffer buffer (buffer-string))))
              ;; Shown again: nothing replays again.
              (romacs-agent-show session)
              (should (= replays 2))
              (kill-buffer buffer)
              ;; Its supervisor never came up while it was not shown (said
              ;; by no journal record): its chat says so once shown.
              (let ((again (romacs-agent-ui--make-chat session)))
                (romacs-agent--raise session (list :type 'unstarted :seq 9 :text "its supervisor never started"))
                (romacs-agent-show session)
                (should (string-match-p "^! its supervisor never started$" (with-current-buffer again (buffer-string))))
                (kill-buffer again)))))
      (delete-directory root t)
      (delete-directory run t))))

(ert-deftest romacs-agent-restored-chat-shows-what-came-before-its-load ()
  ;; Shown from its row, its session then loaded by a watcher with no
  ;; supervisor listening (no replay to wait for): the chat is drawn again
  ;; at once, a record written after it was drawn included.
  (let* ((root (make-temp-file "romacs-restored-" t))
         (run (romacs-test-socket-directory "restored-"))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" run))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent-ui--row-chats (make-hash-table :test #'equal))
         (id romacs-agent-restored-chat-tests--id))
    (unwind-protect
        (romacs-agent-test-with-store
         (let ((dir (expand-file-name id (romacs-agent--state-root))))
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root
                                                        "title" "restored" "titleAuto" :false "argv" []
                                                        "options" (romacs-mcp--object) "seenSeq" 0
                                                        "hostSocket" (expand-file-name "none/host.sock" root)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n"))
                                     (list (romacs-agent-restored-chat-tests--record
                                            1 "in" (romacs-mcp--object "type" "user" "message" (romacs-mcp--object "content" "first question")))
                                           (romacs-agent-restored-chat-tests--record
                                            2 "out" (romacs-mcp--object "type" "result" "result" "first answer")))
                                     "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (cl-letf (((symbol-function 'romacs-agent--keep-models) #'ignore))
            (romacs-agent-test-stored (romacs-agent-load-session id))
            (remhash id romacs-agent--sessions)
            (let ((buffer (romacs-agent-ui-desktop-restore nil nil (list :session id :draft ""))))
              (unwind-protect
                  (progn
                    (romacs-agent-ui--fill buffer)
                    (should (string-search "first question" (with-current-buffer buffer (buffer-string))))
                    ;; Said while the chat showed its row: in the store only.
                    (romacs-agent-events-append (buffer-local-value 'romacs-agent-ui--session buffer)
                                                (romacs-agent-restored-chat-tests--record
                                                 3 "in" (romacs-mcp--object "type" "user" "message"
                                                                            (romacs-mcp--object "content" "said meanwhile"))))
                    (should-not (string-search "said meanwhile" (with-current-buffer buffer (buffer-string))))
                    ;; A watcher's load, not a command: no hook fired by hand.
                    (let ((live (romacs-agent-load-session id)))
                      (should (eq (buffer-local-value 'romacs-agent-ui--session buffer) live))
                      (should (string-search "said meanwhile" (with-current-buffer buffer (buffer-string))))))
                (kill-buffer buffer))))))
      (delete-directory root t)
      (delete-directory run t))))

;;; romacs-agent-restored-chat-tests.el ends here

;;; romacs-agent-rollback-tests.el --- A session back to before one of its turns -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-ui))

(defun romacs-agent-rollback-tests--git (directory &rest args)
  (with-temp-buffer
    (let ((default-directory (file-name-as-directory directory))
          (process-environment (append romacs-agent-git-variables process-environment)))
      (unless (zerop (apply #'call-process "git" nil t nil args)) (error "git %S: %s" args (buffer-string)))
      (string-trim (buffer-string)))))

(defun romacs-agent-rollback-tests--read (file)
  (and (file-exists-p file) (with-temp-buffer (insert-file-contents file) (buffer-string))))

(defun romacs-agent-rollback-tests--journal (session &rest records)
  "Write RECORDS, objects without their seq, as SESSION's journal."
  (let ((seq 0) (coding-system-for-write 'utf-8-unix))
    (write-region (mapconcat (lambda (record)
                               (puthash "seq" (cl-incf seq) record)
                               (puthash "t" (float seq) record)
                               (concat (romacs-agent--json record) "\n"))
                             records "")
                  nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) nil 'silent)
    (setf (romacs-agent-seq session) seq)))

(defun romacs-agent-rollback-tests--prompt (request text)
  (romacs-mcp--object "in" (romacs-mcp--object "type" "user" "romacsRequest" request
                                               "message" (romacs-mcp--object "role" "user" "content" text))))

(defun romacs-agent-rollback-tests--assistant (uuid &optional parent)
  (romacs-mcp--object "out" (romacs-mcp--object "type" "assistant" "uuid" uuid "parent_tool_use_id" (or parent :null)
                                                "message" (romacs-mcp--object "id" uuid "content" []))))

(defmacro romacs-agent-rollback-tests--with (fields &rest body)
  "BODY with `session', an idle Claude session of FIELDS recorded in a
private state root, its writes to its harness in `written'."
  (declare (indent 1))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (written nil)
          (session (romacs-agent--create ,@fields :id (romacs-agent--uuid) :harness 'claude :provider "claude"
                                         :title "t" :state 'idle :turns 2)))
     (romacs-agent--manifest session nil)
     ;; Its harness answers a rollback at once, as told: (refuse . WHY)
     ;; refuses it.
     (cl-letf (((symbol-function 'romacs-agent--line)
                (lambda (s object &rest _)
                  (push object written)
                  (when (hash-table-p object)
                    (romacs-agent--raise s (if (gethash "refuse" object) (list :type 'reverted :error (gethash "refuse" object))
                                             '(:type reverted)))))))
       ,@body)))

(defun romacs-agent-rollback-tests--run (session request &rest options)
  "`romacs-agent-rollback''s outcome, waited for."
  (let ((outcome nil) (end (+ (float-time) 30)))
    (apply #'romacs-agent-rollback session request (lambda (result) (setq outcome result)) options)
    (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05))
    outcome))

(defun romacs-agent-rollback-tests--checkpoint (session turn edge)
  "Checkpoint SESSION's project as TURN's EDGE, waited for."
  (let ((done nil) (end (+ (float-time) 30)))
    (romacs-agent-diff--snapshot session turn edge (lambda (&rest _) (setq done t)))
    (while (and (not done) (< (float-time) end)) (accept-process-output nil 0.05))))

(defmacro romacs-agent-rollback-tests--project (&rest body)
  "BODY with `session' working in `repo', a git project of a.txt and
b.txt, through two turns, 10 and 20: turn 10 changes a.txt and makes
new.txt, turn 20 changes a.txt again; each checkpointed.  Its harness
rolls back by the line (revert REQUEST)."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-rollback-" t)) (repo (expand-file-name "p" root)))
     (unwind-protect
         (romacs-agent-rollback-tests--with (:cwd repo)
           (make-directory repo)
           (cl-flet ((put (name text) (write-region text nil (expand-file-name name repo) nil 'silent))
                     (file (name) (romacs-agent-rollback-tests--read (expand-file-name name repo))))
             (ignore #'file)
             (put "a.txt" "one\n") (put "b.txt" "keep\n")
             (romacs-agent-rollback-tests--git repo "init" "-q")
             (romacs-agent-rollback-tests--git repo "add" ".")
             (romacs-agent-rollback-tests--git repo "-c" "commit.gpgsign=false" "-c" "user.name=t" "-c" "user.email=t@t"
                                               "commit" "-q" "-m" "first")
             (romacs-agent-rollback-tests--journal
              session (romacs-agent-rollback-tests--prompt 10 "make it two") (romacs-agent-rollback-tests--assistant "u1")
              (romacs-agent-rollback-tests--prompt 20 "make it three") (romacs-agent-rollback-tests--assistant "u2"))
             (romacs-agent-rollback-tests--checkpoint session 10 'start)
             (put "a.txt" "two\n") (put "new.txt" "made\n")
             (romacs-agent-rollback-tests--checkpoint session 10 'end)
             (romacs-agent-rollback-tests--checkpoint session 20 'start)
             (put "a.txt" "three\n")
             (romacs-agent-rollback-tests--checkpoint session 20 'end)
             (cl-letf (((symbol-function 'romacs-agent--claude-rollback)
                        (lambda (_session _records request) (romacs-mcp--object "revert" request))))
               ,@body)))
       (delete-directory root t))))

(ert-deftest romacs-agent-rollback-restores-what-the-turns-changed ()
  (romacs-agent-rollback-tests--project
    ;; Something else changed b.txt since: not the turns', not restored.
    (put "b.txt" "mine\n")
    (should (equal (romacs-agent-rollback-tests--run session 10 :files t) '(:done 2)))
    (should (equal (file "a.txt") "one\n"))
    (should-not (file "new.txt"))
    (should (equal (file "b.txt") "mine\n"))
    ;; The harness was told; the user's index and branch were not touched.
    (should (equal (gethash "revert" (car written)) 10))
    (should (equal (romacs-agent-rollback-tests--git repo "status" "--porcelain" "--" "a.txt") ""))
    ;; Those turns are no longer the conversation's.
    (should-error (romacs-agent-rollback-tests--run session 20 :files t) :type 'user-error)))

(ert-deftest romacs-agent-rollback-names-files-changed-meanwhile ()
  (romacs-agent-rollback-tests--project
    ;; The user edited a file the turns changed, after them.
    (put "a.txt" "the user's\n")
    (should (equal (romacs-agent-rollback-tests--run session 20 :files t) '(:clobbered ("a.txt"))))
    ;; Nothing done: the file, the journal, the harness.
    (should (equal (file "a.txt") "the user's\n"))
    (should-not written)
    (should-not (cl-some #'romacs-agent--rollback-note-p (romacs-agent--journal-records session)))
    ;; Said so, it goes.
    (should (equal (romacs-agent-rollback-tests--run session 20 :files t :force t) '(:done 1)))
    (should (equal (file "a.txt") "two\n"))
    (should (equal (file "new.txt") "made\n"))))

(ert-deftest romacs-agent-rollback-refusals ()
  (romacs-agent-rollback-tests--with (:state 'running)
    (romacs-agent-rollback-tests--journal session (romacs-agent-rollback-tests--prompt 3 "x"))
    (should-error (romacs-agent-rollback session 3 #'ignore) :type 'user-error))
  ;; No checkpoint (a project git does not keep): the files cannot go back.
  (romacs-agent-rollback-tests--with (:cwd "/")
    (romacs-agent-rollback-tests--journal session (romacs-agent-rollback-tests--prompt 3 "x"))
    (should (string-match-p "No checkpoint"
                            (cadr (should-error (romacs-agent-rollback session 3 #'ignore :files t) :type 'user-error))))
    (should-not written)
    (should-error (romacs-agent-rollback session 99 #'ignore) :type 'user-error)))

(ert-deftest romacs-agent-rollback-live-records ()
  ;; A rollback note drops its prompt and what follows, up to the note.
  (let* ((records (list (romacs-agent-rollback-tests--prompt 1 "a") (romacs-agent-rollback-tests--assistant "u1")
                        (romacs-agent-rollback-tests--prompt 2 "b") (romacs-agent-rollback-tests--assistant "u2")
                        (romacs-mcp--object "note" (romacs-mcp--object "type" "rollback" "to" 2))
                        (romacs-agent-rollback-tests--prompt 3 "c")))
         (live (romacs-agent--live-records records)))
    (should (equal (delq nil (mapcar #'romacs-agent--record-request live)) '(1 3)))
    (should (romacs-agent--rollback-note-p (nth 2 live)))))

(ert-deftest romacs-agent-rollback-claude-resumes-before-the-turn ()
  (romacs-agent-rollback-tests--with ()
    (romacs-agent-rollback-tests--journal
     session (romacs-agent-rollback-tests--prompt 5 "a") (romacs-agent-rollback-tests--assistant "u1")
     (romacs-agent-rollback-tests--prompt 9 "b") (romacs-agent-rollback-tests--assistant "sub" "toolu_1")
     (romacs-agent-rollback-tests--assistant "u2")
     (romacs-mcp--object "note" (romacs-mcp--object "type" "rollback" "to" 9)))
    (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
      (let ((argv (romacs-agent--claude-argv session t)))
        (should (equal (cadr (member "--resume" argv)) (romacs-agent-id session)))
        (should (equal (cadr (member "--resume-session-at" argv)) "u1")))
      ;; Spoken since: the conversation goes on from there by itself.
      (romacs-agent-rollback-tests--journal
       session (romacs-agent-rollback-tests--prompt 5 "a") (romacs-agent-rollback-tests--assistant "u1")
       (romacs-mcp--object "note" (romacs-mcp--object "type" "rollback" "to" 5))
       (romacs-agent-rollback-tests--prompt 12 "c"))
      (should-not (member "--resume-session-at" (romacs-agent--claude-argv session t))))))

(ert-deftest romacs-agent-rollback-claude-to-its-start-is-a-conversation-anew ()
  (romacs-agent-rollback-tests--with ()
    (romacs-agent-rollback-tests--journal session (romacs-agent-rollback-tests--prompt 5 "a")
                                          (romacs-agent-rollback-tests--assistant "u1"))
    (let ((records (romacs-agent--journal-records session)))
      (let ((how (romacs-agent--claude-rollback session records 5)))
        (should (eq (car how) 'relaunch))
        ;; Its new name is taken only once the rollback is journaled.
        (should-not (romacs-agent-harness-session session))
        (funcall (cadr how))))
    (let ((native (romacs-agent-harness-session session)))
      (should-not (equal native (romacs-agent-id session)))
      (should (equal (gethash "harnessSession" (romacs-agent--read-json (expand-file-name "manifest.json" (romacs-agent-dir session))))
                     native))
      (romacs-agent-rollback-tests--journal session (romacs-agent-rollback-tests--prompt 5 "a")
                                            (romacs-agent-rollback-tests--assistant "u1")
                                            (romacs-mcp--object "note" (romacs-mcp--object "type" "rollback" "to" 5)))
      (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
        (let ((argv (romacs-agent--claude-argv session t)))
          (should (equal (cadr (member "--session-id" argv)) native))
          (should-not (member "--resume" argv)))))))

(ert-deftest romacs-agent-rollback-codex-reverts-the-turn ()
  (let* ((session (romacs-agent--make :id "c" :harness 'codex :harness-session "th"))
         (records (list (romacs-mcp--object "in" (romacs-mcp--object "id" "turn/start:4" "method" "turn/start" "romacsRequest" 7))
                        (romacs-mcp--object "out" (romacs-mcp--object "id" "turn/start:4"
                                                                      "result" (romacs-mcp--object "turn" (romacs-mcp--object "id" "T7"))))
                        (romacs-mcp--object "in" (romacs-mcp--object "id" "turn/steer:5" "method" "turn/steer" "romacsRequest" 8)))))
    (cl-letf (((symbol-function 'romacs-agent--harness-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent--ready-p) (lambda (_) t)))
      (let ((line (romacs-agent--codex-rollback session records 7)))
        (should (equal (gethash "method" line) "thread/revert"))
        (should (equal (gethash "threadId" (gethash "params" line)) "th"))
        (should (equal (gethash "beforeTurnId" (gethash "params" line)) "T7")))
      (should (string-match-p "steered" (cadr (should-error (romacs-agent--codex-rollback session records 8) :type 'user-error)))))
    (cl-letf (((symbol-function 'romacs-agent--harness-live-p) #'ignore))
      (should-error (romacs-agent--codex-rollback session records 7) :type 'user-error))))

(ert-deftest romacs-agent-rollback-is-drawn-from-the-journal ()
  (romacs-agent-rollback-tests--with ()
    (romacs-agent-rollback-tests--journal
     session (romacs-agent-rollback-tests--prompt 5 "kept prompt") (romacs-agent-rollback-tests--assistant "u1")
     (romacs-agent-rollback-tests--prompt 9 "dropped prompt") (romacs-agent-rollback-tests--assistant "u2")
     (romacs-mcp--object "note" (romacs-mcp--object "type" "rollback" "to" 9 "turns" 1 "prompt" "dropped prompt"
                                                    "files" t)))
    (let ((buffer (save-window-excursion (romacs-agent-show session))))
      (unwind-protect
          (with-current-buffer buffer
            (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
              (should (string-match-p "↶ rolled back 1 turn and their files to before: dropped prompt" text))
              ;; Kept, dimmed; the turn before it is not.
              (let ((dropped (+ (point-min) (string-search "dropped prompt" text)))
                    (kept (+ (point-min) (string-search "kept prompt" text))))
                (should (memq 'romacs-agent-dim (ensure-list (get-text-property dropped 'face))))
                (should-not (memq 'romacs-agent-dim (ensure-list (get-text-property kept 'face)))))))
        (kill-buffer buffer)))))

(ert-deftest romacs-agent-rollback-keys ()
  (should (eq (lookup-key romacs-agent-chat-actions-map "u") 'romacs-agent-chat-rollback))
  (should (equal (command-modes 'romacs-agent-chat-rollback) '(romacs-agent-chat-mode))))

(ert-deftest romacs-agent-rollback-holds-messages-sent-meanwhile ()
  ;; A message sent while the rollback reads the project waits in the
  ;; queue: it is never accepted and then rolled back.  The turns' end
  ;; checkpoints are read once the snapshots before it are done.
  (romacs-agent-rollback-tests--project
    (let ((outcome nil) (end (+ (float-time) 30)) (released nil))
      (romacs-agent-rollback session 20 (lambda (result) (setq outcome result)) :files t)
      (should (plist-get (romacs-agent-driver-state session) :rolling-back))
      (romacs-agent-send session "meanwhile")
      (should (equal (car (car (romacs-agent-queue session))) "meanwhile"))
      (should-not written)
      ;; Once released, what waited goes on as usual.
      (cl-letf (((symbol-function 'romacs-agent--send-next) (lambda (s &rest _) (push s released))))
        (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05)))
      (should (equal outcome '(:done 1)))
      (should-not (plist-get (romacs-agent-driver-state session) :rolling-back))
      (should (equal released (list session)))
      (should (equal (file "a.txt") "two\n")))))

(ert-deftest romacs-agent-rollback-a-failure-leaves-it-to-try-again ()
  ;; Files that will not come back: nothing journaled, the harness untold.
  (romacs-agent-rollback-tests--project
    (cl-letf (((symbol-function 'romacs-agent-rollback--restore) (lambda (&rest _) (error "checkout-index failed"))))
      (should (equal (romacs-agent-rollback-tests--run session 20 :files t) '(:error "checkout-index failed"))))
    (should-not written)
    (should-not (cl-some #'romacs-agent--rollback-note-p (romacs-agent--journal-records session)))
    ;; The harness refusing: no note either.
    (cl-letf (((symbol-function 'romacs-agent--claude-rollback)
               (lambda (&rest _) (romacs-mcp--object "refuse" "no such turn"))))
      (should (equal (romacs-agent-rollback-tests--run session 20) '(:error "no such turn"))))
    (should-not (cl-some #'romacs-agent--rollback-note-p (romacs-agent--journal-records session)))
    ;; A note that will not be written: no waiter left, and a second try
    ;; goes once.
    (let ((hooks romacs-agent-event-functions))
      (cl-letf (((symbol-function 'romacs-agent--note) (lambda (&rest _) (error "journal full"))))
        (should (equal (romacs-agent-rollback-tests--run session 20) '(:error "journal full"))))
      (romacs-agent--raise session '(:type status :text "x"))
      (should (equal romacs-agent-event-functions hooks)))
    ;; One at a time, the first's left alone.
    (let ((token (list 20)))
      (setf (romacs-agent-driver-state session) (list :rolling-back token))
      (should-error (romacs-agent-rollback-tests--run session 20) :type 'user-error)
      (should (eq (plist-get (romacs-agent-driver-state session) :rolling-back) token))
      (setf (romacs-agent-driver-state session) nil))
    (setq written nil)
    (should (equal (romacs-agent-rollback-tests--run session 20) '(:done 1)))
    (should (= (length written) 1))))

(ert-deftest romacs-agent-rollback-an-inherited-start-is-refused ()
  ;; Before a fork's first prompt here is its source's conversation.
  (romacs-agent-rollback-tests--with (:options '(:fork "src"))
    (romacs-agent-rollback-tests--journal session (romacs-agent-rollback-tests--prompt 3 "x"))
    (should (string-match-p "began before"
                            (cadr (should-error (romacs-agent--claude-rollback session (romacs-agent--journal-records session) 3)
                                                :type 'user-error))))))

(provide 'romacs-agent-rollback-tests)
;;; romacs-agent-rollback-tests.el ends here

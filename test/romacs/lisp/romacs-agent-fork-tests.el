;;; romacs-agent-fork-tests.el --- A session going on from another's conversation -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

;; These tests start no supervisor or harness: the build's own (beside the
;; binary) are not needed to make a session.
(advice-add 'romacs-agent--check-companions :override #'ignore)

(defun romacs-agent-fork-tests--files (directory)
  "Each file under DIRECTORY with its contents."
  (mapcar (lambda (file) (cons file (with-temp-buffer (insert-file-contents-literally file) (buffer-string))))
          (directory-files-recursively directory "")))

(defmacro romacs-agent-fork-tests--with (harness fields &rest body)
  "BODY with `source', an idle HARNESS session of FIELDS recorded under the
state root, its files kept in `before'; `launched' each (SESSION ARGV) a
start launched, its manifest written."
  (declare (indent 2))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-providers '(("claude") ("codex")))
          (launched nil)
          ;; FIELDS first: the first of a key given wins.
          (source (romacs-agent--create ,@fields :id (romacs-agent--uuid) :harness ,harness
                                        :provider (symbol-name ,harness) :cwd "/work/p" :title "Plan"
                                        :state 'idle :turns 1 :options '(:model "m" :effort "high"))))
     (romacs-agent--manifest source nil)
     (write-region (concat "{\"seq\":1,\"t\":1,\"in\":{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello source\"}}}\n"
                           "{\"seq\":2,\"t\":2,\"in\":{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"after the fork\"}}}\n")
                   nil (expand-file-name "journal.jsonl" (romacs-agent-dir source)) nil 'silent)
     (setf (romacs-agent-seq source) 1)
     (let ((before (romacs-agent-fork-tests--files (romacs-agent-dir source))))
       (cl-letf (((symbol-function 'romacs-agent--launch)
                  (lambda (session argv &rest _)
                    (push (list session argv) launched)
                    (romacs-agent--manifest session argv)))
                 ((symbol-function 'romacs-agent--attach) #'ignore)
                 ((symbol-function 'romacs-agent--when-attached) #'ignore))
         ,@body))))

(ert-deftest romacs-agent-fork-claude ()
  (romacs-agent-fork-tests--with 'claude ()
    (let* ((fork (romacs-agent-fork source))
           (argv (cadr (car launched)))
           (manifest (romacs-agent--read-json (expand-file-name "manifest.json" (romacs-agent-dir fork)))))
      (should-not (equal (romacs-agent-id fork) (romacs-agent-id source)))
      ;; Claude Code's own fork: the source's conversation, copied under the new id.
      (should (equal (cadr (member "--resume" argv)) (romacs-agent-id source)))
      (should (member "--fork-session" argv))
      (should (equal (cadr (member "--session-id" argv)) (romacs-agent-id fork)))
      (should (equal (cadr (member "--model" argv)) "m"))
      (should (equal (cadr (member "--effort" argv)) "high"))
      (should (equal (romacs-agent-cwd fork) "/work/p"))
      (should (equal (romacs-agent-title fork) "Plan (fork)"))
      (should (equal (gethash "provider" manifest) "claude"))
      (should (equal (gethash "forkedFrom" (gethash "options" manifest)) (romacs-agent-id source)))
      ;; The source's own record is as it was.
      (should (equal (romacs-agent-fork-tests--files (romacs-agent-dir source)) before)))))

(ert-deftest romacs-agent-fork-of-an-imported-conversation ()
  ;; An imported session's conversation is the harness's own id.
  (romacs-agent-fork-tests--with 'claude (:turns 0 :harness-session "11111111-2222-4333-8444-555555555555"
                                         :imported '(:path "/x.jsonl" :size 10))
    (romacs-agent-fork source)
    (should (equal (cadr (member "--resume" (cadr (car launched)))) "11111111-2222-4333-8444-555555555555"))))

(ert-deftest romacs-agent-fork-codex ()
  (romacs-agent-fork-tests--with 'codex (:harness-session "01aa-source")
    (let* ((fork (romacs-agent-fork source))
           (step (romacs-agent--codex-thread fork 3)))
      (should (equal (gethash "method" step) "thread/fork"))
      (should (equal (gethash "threadId" (gethash "params" step)) "01aa-source"))
      (should (equal (gethash "model" (gethash "params" step)) "m"))
      ;; Its thread named, it goes on with that one.
      (romacs-agent--raise fork (list :type 'session :harness-session "01bb-fork"))
      (let ((again (romacs-agent--codex-thread fork 4)))
        (should (equal (gethash "method" again) "thread/resume"))
        (should (equal (gethash "threadId" (gethash "params" again)) "01bb-fork")))
      (should (equal (romacs-agent-harness-session source) "01aa-source"))
      (should (equal (romacs-agent-fork-tests--files (romacs-agent-dir source)) before)))))

(ert-deftest romacs-agent-fork-codex-answer-names-the-new-thread ()
  (let ((session (romacs-agent--make :id "c" :harness 'codex :options '(:fork "01aa"))))
    (let ((events (romacs-agent--codex-answer
                   session "thread/fork:3"
                   (romacs-mcp--object "id" "thread/fork:3"
                                       "result" (romacs-mcp--object "thread" (romacs-mcp--object "id" "01cc"))))))
      (should (equal (plist-get (cadr events) :harness-session) "01cc"))
      (should (equal (car events) '(:type handshake :state (:thread "01cc")))))
    (should (equal (romacs-agent--codex-input-events
                    session (romacs-mcp--object "id" "thread/fork:3" "method" "thread/fork"))
                   '((:type handshake :state (:sent "thread")))))))

(ert-deftest romacs-agent-fork-refusals ()
  (dolist (state '(starting running waiting-approval))
    (romacs-agent-fork-tests--with 'claude (:state state)
      (should (string-match-p "fork it when it is idle"
                              (cadr (should-error (romacs-agent-fork source) :type 'user-error))))
      (should-not launched)))
  ;; It ended in the middle of a turn.
  (romacs-agent-fork-tests--with 'claude (:state 'ended :interrupted t)
    (should (string-match-p "middle of a turn" (cadr (should-error (romacs-agent-fork source) :type 'user-error))))
    (should-not launched))
  (romacs-agent-fork-tests--with 'claude (:turns 0)
    (should (string-match-p "no conversation" (cadr (should-error (romacs-agent-fork source) :type 'user-error)))))
  ;; Its instance now keeps conversations elsewhere: the harness would find none.
  (romacs-agent-fork-tests--with 'claude (:config-dir "/elsewhere")
    (should-error (romacs-agent-fork source) :type 'user-error)
    (should-not launched))
  (let ((romacs-agent--drivers (copy-alist romacs-agent--drivers))
        (romacs-agent--driver-table (copy-hash-table romacs-agent--driver-table)))
    (romacs-agent-define-harness 'noforks :lifetime 'persistent :display "No forks" :config-variable "NOFORKS_HOME")
    (should-error (romacs-agent-fork (romacs-agent--make :id "n" :harness 'noforks :state 'idle :turns 1))
                  :type 'user-error)))

(ert-deftest romacs-agent-fork-is-in-the-session-menu ()
  (require 'romacs-agent-ui)
  (should (eq (lookup-key romacs-agent-chat-actions-map "f") 'romacs-agent-chat-fork))
  (should-not (commandp 'romacs-agent-fork))
  (should (equal (command-modes 'romacs-agent-chat-fork) '(romacs-agent-chat-mode))))

(ert-deftest romacs-agent-fork-claude-forks-until-it-has-spoken ()
  ;; Claude Code writes a fork's transcript with its first message: a run
  ;; that ended before one forks again.
  (romacs-agent-fork-tests--with 'claude ()
    (let ((fork (romacs-agent-fork source)))
      (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
        (let ((again (romacs-agent--claude-argv fork t)))
          (should (member "--fork-session" again))
          (should (equal (cadr (member "--resume" again)) (romacs-agent-id source))))
        (setf (romacs-agent-input-seq fork) 3)
        (let ((spoken (romacs-agent--claude-argv fork t)))
          (should-not (member "--fork-session" spoken))
          (should (equal (cadr (member "--resume" spoken)) (romacs-agent-id fork))))))))

(ert-deftest romacs-agent-fork-of-a-session-just-imported ()
  ;; Imported, its harness up and caught up: it is free, though `starting'.
  (romacs-agent-fork-tests--with 'claude (:state 'starting :caught-up t :turns 0 :connection 'live
                                         :imported '(:path "/x.jsonl" :size 10))
    (cl-letf (((symbol-function 'process-live-p) (lambda (process) (eq process 'live))))
      (should (romacs-agent-fork source)))
    (should launched)))

(ert-deftest romacs-agent-fork-says-where-it-came-from ()
  (require 'romacs-agent-ui)
  (romacs-agent-fork-tests--with 'claude ()
    (let* ((fork (romacs-agent-fork source))
           (first (with-temp-buffer
                    (insert-file-contents (expand-file-name "journal.jsonl" (romacs-agent-dir fork)))
                    (json-parse-string (buffer-substring (point-min) (line-end-position)) :object-type 'hash-table)))
           (note (gethash "note" first)))
      (should (eql (gethash "seq" first) 1))
      (should (equal (gethash "type" note) "fork"))
      (should (equal (gethash "from" note) (romacs-agent-id source)))
      (should (eql (gethash "seq" note) 1))
      (should (eql (gethash "turns" note) 1))
      ;; Its chat shows the source's transcript to the fork, then the rule.
      (let ((buffer (save-window-excursion (romacs-agent-show fork))))
        (unwind-protect
            (with-current-buffer buffer
              (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
                (should (string-match-p "hello source" text))
                (should-not (string-match-p "after the fork" text))
                (should (string-match-p "Forked from Plan at its turn 1" text))
                (should (< (string-search "hello source" text) (string-search "Forked from" text)))))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-fork-names ()
  ;; A fork's fork is no "(fork) (fork)"; one whose only turn was cut has
  ;; a conversation.
  (romacs-agent-fork-tests--with 'claude (:title "Plan (fork)" :turns 0 :input-seq 4)
    (should (equal (romacs-agent-title (romacs-agent-fork source)) "Plan (fork)"))))

(defun romacs-agent-fork-tests--notes (session)
  "The fork notes in SESSION's journal, as (SEQ AFTER)."
  (let ((notes nil))
    (romacs-agent--read-journal
     session (lambda (record)
               (let ((note (gethash "note" record)))
                 (when (and (hash-table-p note) (equal (gethash "type" note) "fork"))
                   (push (list (gethash "seq" note) (gethash "after" note)) notes)))))
    (nreverse notes)))

(ert-deftest romacs-agent-fork-forked-again-says-how-far ()
  ;; Not yet spoken, it forks its source anew at each run: when the source
  ;; went on meanwhile, the fork's journal and chat say so.
  (require 'romacs-agent-ui)
  (romacs-agent-fork-tests--with 'claude ()
    (let ((fork (romacs-agent-fork source)))
      (setf (romacs-agent-seq source) 2 (romacs-agent-turns source) 2 (romacs-agent-state fork) 'ended)
      (romacs-agent-resume fork)
      (should (equal (romacs-agent-fork-tests--notes fork) '((1 0) (2 1))))
      (should (member "--fork-session" (cadr (car launched))))
      ;; Unchanged since: no note more.
      (setf (romacs-agent-state fork) 'ended)
      (romacs-agent-resume fork)
      (should (= (length (romacs-agent-fork-tests--notes fork)) 2))
      (let ((buffer (save-window-excursion (romacs-agent-show fork))))
        (unwind-protect
            (with-current-buffer buffer
              (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
                (should (= (how-many "hello source" 0 (length text)) 1))
                (should (string-match-p (concat "hello source\\(.\\|\n\\)*Forked from Plan at its turn 1"
                                                "\\(.\\|\n\\)*after the fork\\(.\\|\n\\)*"
                                                "Forked again from Plan, now at its turn 2")
                                        text))))
          (kill-buffer buffer)))
      ;; Spoken, it has its own conversation: no re-fork, no note.
      (setf (romacs-agent-seq source) 3 (romacs-agent-input-seq fork) 9 (romacs-agent-state fork) 'ended)
      (romacs-agent-resume fork)
      (should (= (length (romacs-agent-fork-tests--notes fork)) 2))
      (should-not (member "--fork-session" (cadr (car launched)))))))

(ert-deftest romacs-agent-fork-draws-an-imported-source-as-its-own ()
  (require 'romacs-agent-ui)
  ;; Loaded before it is stubbed: the draw's own require would replace the stub.
  (require 'romacs-agent-history)
  (romacs-agent-fork-tests--with 'claude (:imported '(:path "/x.jsonl" :size 10))
    (let* ((fork (romacs-agent-fork source)) (drawn nil))
      (cl-letf (((symbol-function 'romacs-agent-history-draw)
                 (lambda (session imported) (push (list session imported) drawn))))
        (kill-buffer (save-window-excursion (romacs-agent-show fork))))
      (should (equal drawn (list (list fork '(:path "/x.jsonl" :size 10))))))))

(ert-deftest romacs-agent-fork-draws-the-source-s-rows-with-its-state ()
  (require 'romacs-agent-ui)
  (let* ((fork (romacs-agent--make :id "f" :turn-start 100.0))
         (source (romacs-agent--make :id "s" :turn-start 7.0))
         (own (romacs-agent-tools fork)) (seen nil))
    (puthash "call" "parent" (romacs-agent-tools source))
    (cl-letf (((symbol-function 'romacs-agent-ui--draw)
               (lambda (session event _buffer)
                 (push (list (romacs-agent-turn-start session) (gethash "call" (romacs-agent-tools session))
                             (plist-get event :seq))
                       seen))))
      (romacs-agent-ui--draw-as fork source '(:type done :seq 5)))
    (should (equal seen '((7.0 "parent" 5))))
    (should (eql (romacs-agent-turn-start fork) 100.0))
    (should (eq (romacs-agent-tools fork) own))))

(ert-deftest romacs-agent-fork-of-an-unspoken-fork ()
  (romacs-agent-fork-tests--with 'claude (:turns 0 :options '(:fork "x"))
    (should (string-match-p "send it a message first"
                            (cadr (should-error (romacs-agent-fork source) :type 'user-error))))))

(provide 'romacs-agent-fork-tests)
;;; romacs-agent-fork-tests.el ends here

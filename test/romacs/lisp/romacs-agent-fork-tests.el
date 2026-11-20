;;; romacs-agent-fork-tests.el --- A session going on from another's conversation -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-agent-test-stored)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-notes)

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

(defun romacs-agent-fork-tests--turns (session from to)
  "Write turns FROM to TO, a prompt and its end each, as SESSION's journal
after LEAD, a list of objects written first (their seqs from 1)."
  (write-region (mapconcat
                 (lambda (n)
                   (concat (json-serialize
                            (romacs-mcp--object
                             "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                                      "romacsRequest" n "type" "user")
                             "seq" (1- (* 2 n)) "t" n))
                           "\n"
                           (json-serialize (romacs-mcp--object "out" (romacs-mcp--object "result" "ok" "type" "result")
                                                               "seq" (* 2 n) "t" n))
                           "\n"))
                 (number-sequence from to) "")
                nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) nil 'silent))

(defun romacs-agent-fork-tests--plain (title)
  "A Claude session TITLE recorded under the state root, nothing written
in it yet: its journal is the test's to write, as a fork's moved later."
  (let ((session (romacs-agent--create :id (romacs-agent--uuid) :harness 'claude :provider "claude" :cwd "/work/p"
                                       :title title :state 'idle :turns 1 :options '(:model "m" :effort "high"))))
    (romacs-agent--manifest session nil)
    session))

(defun romacs-agent-fork-tests--note-then (fork note-objects lines)
  "Write FORK's journal: NOTE-OBJECTS as its notes, seqs from 1, then
LINES, objects with their own seqs."
  (let ((seq 0))
    (write-region (concat (mapconcat (lambda (note) (concat (json-serialize (romacs-mcp--object "note" note "seq" (cl-incf seq) "t" seq)) "\n"))
                                     note-objects "")
                          (mapconcat (lambda (line) (concat (json-serialize line) "\n")) lines ""))
                  nil (expand-file-name "journal.jsonl" (romacs-agent-dir fork)) nil 'silent)))

;; A fork's turns taken of its source are rows of its own (seqs 1..N, its
;; note at N+1), each naming its source, kept drawn under the fork's id.

(defun romacs-agent-fork-tests--form (text)
  "A drawn form of one turn saying TEXT."
  (with-temp-buffer
    (insert (propertize "> " 'face 'romacs-agent-dim)
            (propertize text 'romacs-agent-input t 'face 'romacs-agent-prompt) "\n")
    (romacs-agent-drawn-form (point-min) (point-max))))

(defun romacs-agent-fork-tests--borrowed (fork source texts &optional drawn)
  "FORK as a fork of SOURCE that took TEXTS, its first turns: its note at
seq N+1 and a turn of its own after, its records stored; `turns-page'
then answers with a borrowed row before its own for each of TEXTS,
seqs 1..N, kept drawn under FORK's id when DRAWN (all by default) holds
its seq."
  (let* ((n (length texts)) (id (romacs-agent-id fork)) (seq n))
    (romacs-agent-fork-tests--note-then
     fork nil
     (list (romacs-mcp--object "note" (romacs-mcp--object "from" (romacs-agent-id source) "seq" n "title" "Plan"
                                                          "turns" n "type" "fork")
                               "seq" (cl-incf seq) "t" seq)
           (romacs-mcp--object "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "fork's own" "role" "user")
                                                        "romacsRequest" 1 "type" "user")
                               "seq" (cl-incf seq) "t" seq)
           (romacs-mcp--object "out" (romacs-mcp--object "result" "ok" "type" "result") "seq" (cl-incf seq) "t" seq)))
    (setf (romacs-agent-seq fork) seq)
    (romacs-agent-test-stored fork)
    (cl-loop for text in texts for first from 1
             when (or (null drawn) (memq first drawn))
             do (romacs-agent-drawn-put (romacs-agent-turns--db) id first (romacs-agent-fork-tests--form text)))
    (cl-loop for first from n downto 1
             collect (list :first-seq first :last-seq first :source-session (romacs-agent-id source)
                           :source-first-seq (* 2 first)))))

(defun romacs-agent-fork-tests--paged (borrowed)
  "`romacs-agent-turns-page' with BORROWED, rows newest first, under the
session's own."
  (let ((page (symbol-function 'romacs-agent-turns-page)))
    (lambda (session &optional before limit)
      (let* ((limit (or limit 20))
             (own (funcall page session before most-positive-fixnum))
             (rows (append (plist-get own :rows)
                           (seq-filter (lambda (row) (or (null before) (< (plist-get row :first-seq) before))) borrowed))))
        (list :rows (seq-take rows limit) :after (plist-get own :after) :more (> (length rows) limit))))))

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

(ert-deftest romacs-agent-fork-of-an-unspoken-fork ()
  (romacs-agent-fork-tests--with 'claude (:turns 0 :options '(:fork "x"))
    (should (string-match-p "send it a message first"
                            (cadr (should-error (romacs-agent-fork source) :type 'user-error))))))

(ert-deftest romacs-agent-fork-a-source-folding-aside-is-folded-before-forking-again ()
  ;; The source a start left to a worker to fold has no seq of its records
  ;; yet: folded here first, its fork note is written all the same.
  (let ((source (romacs-agent--make :id "src" :seq 0)) (noted nil)
        (session (romacs-agent--make :id "fork" :options (list :forked (list "src" 5)))))
    (cl-letf (((symbol-function 'romacs-agent--harness-get) (lambda (_s key) (and (eq key :forks) (lambda (_) t))))
              ((symbol-function 'romacs-agent-get) (lambda (id) (and (equal id "src") source)))
              ((symbol-function 'romacs-agent--fold-now) (lambda (s) (setf (romacs-agent-seq s) 9)))
              ((symbol-function 'romacs-agent--fork-note) (lambda (_s after) (list 'fork after)))
              ((symbol-function 'romacs-agent--note) (lambda (_s note) (push note noted))))
      (romacs-agent--refork session)
      (should (equal noted '((fork 5)))))))

(provide 'romacs-agent-fork-tests)
;;; romacs-agent-fork-tests.el ends here

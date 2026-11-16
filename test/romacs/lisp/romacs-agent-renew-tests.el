;;; romacs-agent-renew-tests.el --- A session going on in a new conversation -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-ui))

(defun romacs-agent-renew-tests--journal (session &rest records)
  "Make RECORDS, objects without their seq, SESSION's records: in SQLite
when its records are there, else its journal."
  (let ((seq 0) (coding-system-for-write 'utf-8-unix))
    (dolist (record records)
      (puthash "seq" (cl-incf seq) record)
      (puthash "t" (float seq) record))
    (if (romacs-agent-events-p session)
        (progn (sqlite-execute (romacs-agent-events--db) "delete from events where session = ?" (list (romacs-agent-id session)))
               (dolist (record records) (romacs-agent-events-append session record)))
      (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n")) records "")
                    nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) nil 'silent))
    (setf (romacs-agent-seq session) seq)))

(defun romacs-agent-renew-tests--append (session &rest records)
  "Add RECORDS, with their seqs, to SESSION's records: in SQLite when they
are there, else its journal."
  (if (romacs-agent-events-p session)
      (dolist (record records) (romacs-agent-events-append session record))
    (let ((coding-system-for-write 'utf-8-unix))
      (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n")) records "")
                    nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) t 'silent))))

(defun romacs-agent-renew-tests--prompt (request text)
  (romacs-mcp--object "in" (romacs-mcp--object "type" "user" "romacsRequest" request
                                               "message" (romacs-mcp--object "role" "user" "content" text))))

(defun romacs-agent-renew-tests--reply (uuid text)
  (romacs-mcp--object "out" (romacs-mcp--object "type" "result" "uuid" uuid "result" text)))

(defmacro romacs-agent-renew-tests--with (harness &rest body)
  "BODY with `session', an idle session of HARNESS recorded in a private
state root that has said two things, its writes to its harness in
`written' (LINE . RECORD) and its relaunches counted in `relaunched'."
  (declare (indent 1))
  `(let* ((root (make-temp-file "romacs-renew-" t))
          ;; Its sockets' directory is short: TMPDIR may not be.
          (run (romacs-test-socket-directory "renew-"))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" run))
                                       process-environment))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          ;; Its timers its own: a renewal it leaves mid-read ends with it,
          ;; not in the next test.
          (timer-list nil)
          ;; Its database its own, its state root's.
          (romacs-agent-store--db nil)
          (written nil) (relaunched 0)
          (session (romacs-agent--create :id (romacs-agent--uuid) :harness ,harness
                                         :provider (symbol-name ,harness) :cwd root
                                         :title "t" :state 'idle :turns 2)))
     (unwind-protect
         (progn
           (romacs-agent--manifest session nil)
           (romacs-agent-renew-tests--journal
            session (romacs-agent-renew-tests--prompt 5 "build the parser")
            (romacs-agent-renew-tests--reply "u1" "the parser is built")
            (romacs-agent-renew-tests--prompt 9 "now its tests")
            (romacs-agent-renew-tests--reply "u2" "its tests pass"))
           (cl-letf (((symbol-function 'romacs-agent--line)
                      (lambda (_s object &optional record &rest _) (push (cons object record) written)))
                     ((symbol-function 'romacs-agent--relaunch)
                      (lambda (_s &optional before) (when before (funcall before)) (cl-incf relaunched))))
             ,@body))
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory root t)
       (delete-directory run t))))

(defun romacs-agent-renew-tests--run (session &optional why)
  "`romacs-agent-renew''s outcome, waited for."
  (let ((outcome nil) (end (+ (float-time) 30)))
    (romacs-agent-renew session why (lambda (result) (setq outcome result)))
    (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05))
    outcome))

(defun romacs-agent-renew-tests--ran (session)
  "SESSION's harness ran anew: its new run's note is read."
  (romacs-agent--raise session (list :type 'run :seq (1+ (romacs-agent-seq session)))))

(defun romacs-agent-renew-tests--notes (session)
  (delq nil (mapcar (lambda (record) (and (romacs-agent--note-p record "renew") (gethash "note" record)))
                    (romacs-agent--journal-records session))))

(defun romacs-agent-renew-tests--manifest (session)
  (romacs-agent--read-json (expand-file-name "manifest.json" (romacs-agent-dir session))))

(ert-deftest romacs-agent-renew-claude-same-session-new-conversation ()
  ;; The same session goes on in a new conversation: its id stays, its
  ;; native id is new, journaled with the brief; its next run begins the
  ;; new conversation under that id.
  (romacs-agent-renew-tests--with 'claude
    (let* ((id (romacs-agent-id session))
           (outcome (romacs-agent-renew-tests--run session "context full"))
           (native (cadr outcome))
           (note (car (romacs-agent-renew-tests--notes session))))
      (should (eq (car outcome) :done))
      (should (romacs-agent--uuid-p native))
      (should-not (equal native id))
      (should (eq (romacs-agent-get id) session))
      (should (equal (romacs-agent-harness-session session) native))
      (should (equal (gethash "harnessSession" (romacs-agent-renew-tests--manifest session)) native))
      (should (equal (gethash "from" note) id))
      (should (equal (gethash "to" note) native))
      (should (equal (gethash "why" note) "context full"))
      ;; The brief: the first request and the latest turns, then the line.
      (let ((brief (gethash "brief" note)))
        (should (string-prefix-p "Renewed conversation" brief))
        (should (string-search "User: build the parser" brief))
        (should (string-search "Assistant: its tests pass" brief))
        (should (string-suffix-p "---\n\n" brief))
        (should (equal (romacs-agent-brief session) brief)))
      (should (= relaunched 1))
      ;; Held until its new run starts.
      (should (romacs-agent--held-p session))
      (romacs-agent--raise session (list :type 'run :seq 90))
      (should-not (romacs-agent--held-p session))
      (should-not written)
      ;; Its next run begins the new conversation; one said in goes on.
      (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
        (let ((argv (romacs-agent--claude-argv session t)))
          (should (equal (cadr (member "--session-id" argv)) native))
          (should-not (member "--resume" argv))))
      ;; The chat draws it.
      (let ((buffer (save-window-excursion (romacs-agent-show session))))
        (unwind-protect
            (should (string-search "── renewed: a new conversation (context full) ──"
                                   (with-current-buffer buffer (buffer-string))))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-renew-brief-goes-with-the-next-message ()
  ;; Messages sent while it renews wait; the first to go carries the
  ;; brief.  The journal keeps the message as written, marked.
  (romacs-agent-renew-tests--with 'claude
    (let ((outcome nil) (end (+ (float-time) 30))
          (read (symbol-function 'romacs-agent-events-async)))
      ;; Its journal read between keystrokes, as a long one is.
      (cl-letf (((symbol-function 'romacs-agent-events-async)
                 (lambda (&rest args) (run-at-time 0.05 nil (lambda () (apply read args))))))
        (romacs-agent-renew session nil (lambda (result) (setq outcome result))))
      (should (romacs-agent--held-p session))
      (romacs-agent-send session "meanwhile")
      (should (equal (car (car (romacs-agent-queue session))) "meanwhile"))
      (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05))
      (should (eq (car outcome) :done))
      ;; Still waiting: the new run has not started.
      (romacs-agent-send session "and this")
      (should (equal (mapcar #'car (romacs-agent-queue session)) '("meanwhile" "and this")))
      (romacs-agent--raise session (list :type 'run :seq 90))
      (let ((brief (romacs-agent-brief session)))
        (romacs-agent--input session "meanwhile")
        (let* ((line (car (car written))) (record (cdr (car written))))
          (should (equal (romacs-agent--text-of (gethash "content" (gethash "message" line))) (concat brief "meanwhile")))
          (should (equal (romacs-agent--text-of (gethash "content" (gethash "message" record))) "meanwhile"))
          (should (eq (gethash "romacsBrief" record) t)))
        (should-not (romacs-agent-brief session))
        (setq written nil)
        (romacs-agent--input session "then")
        (should (equal (romacs-agent--text-of (gethash "content" (gethash "message" (car (car written))))) "then"))))))

(ert-deftest romacs-agent-renew-folds-from-the-journal ()
  ;; A restart reads it back: the new native id, and the brief until a
  ;; message has gone.
  (romacs-agent-renew-tests--with 'claude
    (romacs-agent-renew-tests--run session)
    (let ((native (romacs-agent-harness-session session))
          (brief (romacs-agent-brief session))
          (copy (romacs-agent--make :id (romacs-agent-id session) :harness 'claude :dir (romacs-agent-dir session))))
      (romacs-agent--read-journal copy (lambda (record) (romacs-agent--record copy record)))
      (should (equal (romacs-agent-harness-session copy) native))
      (should (equal (romacs-agent-brief copy) brief))
      ;; No run since: still renewing, until one starts.
      (should (romacs-agent-renewing copy))
      (romacs-agent--record copy (romacs-mcp--object "seq" 98 "t" 98.0 "note" (romacs-mcp--object "type" "run")))
      (should-not (romacs-agent-renewing copy))
      (romacs-agent--record copy (romacs-mcp--object "seq" 99 "t" 99.0 "in"
                                                     (romacs-mcp--object "type" "user" "romacsRequest" 12 "romacsBrief" t
                                                                         "message" (romacs-mcp--object "role" "user" "content" "go"))))
      (should-not (romacs-agent-brief copy)))))

(ert-deftest romacs-agent-renew-begins-the-live-records ()
  ;; Its conversation begins at the note: a rollback reaches no further
  ;; back, nor to the prompt that carried the brief.
  (romacs-agent-renew-tests--with 'claude
    (romacs-agent-renew-tests--run session)
    (romacs-agent-renew-tests--ran session)
    (let ((records (romacs-agent--journal-records session)))
      (should (romacs-agent--note-p (car (romacs-agent--live-records records)) "renew"))
      (should-not (cl-some #'romacs-agent--record-request (romacs-agent--live-records records))))
    (romacs-agent-renew-tests--journal
     session (romacs-agent-renew-tests--prompt 5 "old")
     (romacs-mcp--object "note" (romacs-mcp--object "type" "renew" "from" "a" "to" "b" "brief" "B"))
     (romacs-agent-renew-tests--prompt 20 "first after") (romacs-agent-renew-tests--reply "u3" "ok")
     (romacs-agent-renew-tests--prompt 21 "second after") (romacs-agent-renew-tests--reply "u4" "ok"))
    (should (string-match-p "no longer" (cadr (should-error (romacs-agent-rollback session 5 #'ignore) :type 'user-error))))
    (should (string-match-p "began this renewed" (cadr (should-error (romacs-agent-rollback session 20 #'ignore)
                                                                     :type 'user-error))))))

(ert-deftest romacs-agent-renew-again-carries-the-last-brief ()
  ;; Renewed twice: the second brief carries the first as it was, then
  ;; what was said since; nothing from before the first renewal besides.
  (romacs-agent-renew-tests--with 'claude
    (romacs-agent-renew-tests--run session)
    (romacs-agent-renew-tests--ran session)
    (let ((first (romacs-agent-brief session)))
      (romacs-agent--input session "write the docs")
      (let ((seq (romacs-agent-seq session)))
        (romacs-agent-renew-tests--append
         session (romacs-mcp--object "seq" (+ seq 1) "t" 50.0 "in" (cdr (car written)))
         (romacs-mcp--object "seq" (+ seq 2) "t" 51.0 "out" (romacs-mcp--object "type" "result" "result" "docs written")))
        (setf (romacs-agent-seq session) (+ seq 2))
        ;; Read back, as its journal now says.
        (remhash session romacs-agent-renew--written))
      (romacs-agent-renew-tests--run session)
      (let ((second (romacs-agent-brief session)))
        (should (string-search (string-trim first) second))
        (should (string-search "User: write the docs" second))
        (should (string-search "Assistant: docs written" second))
        (should (= 1 (cl-loop with start = 0 while (string-search "User: build the parser" second start)
                              count t do (setq start (1+ (string-search "User: build the parser" second start))))))))))

(ert-deftest romacs-agent-renew-codex-starts-a-new-thread ()
  ;; Codex names the new thread as it starts it; a fork is one no more.
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-harness-session session) "thread-old"
          (romacs-agent-options session) (list :fork "thread-source" :model "m"))
    (let ((outcome (romacs-agent-renew-tests--run session)))
      (should (equal outcome '(:done nil)))
      (should-not (romacs-agent-harness-session session))
      (should-not (plist-member (romacs-agent-options session) :fork))
      (should (equal (plist-get (romacs-agent-options session) :model) "m"))
      (should (null (gethash "harnessSession" (romacs-agent-renew-tests--manifest session) :absent)))
      (should (equal (gethash "from" (car (romacs-agent-renew-tests--notes session))) "thread-old"))
      (should (equal (gethash "method" (romacs-agent--codex-thread session 3)) "thread/start")))))

(ert-deftest romacs-agent-renew-refusals ()
  (romacs-agent-renew-tests--with 'claude
    (setf (romacs-agent-state session) 'running)
    (should (string-match-p "busy" (cadr (should-error (romacs-agent-renew session) :type 'user-error))))
    (setf (romacs-agent-state session) 'idle)
    (romacs-agent-renew-tests--run session)
    ;; Held until its new run starts.
    (should (string-match-p "already" (cadr (should-error (romacs-agent-renew session) :type 'user-error))))
    (romacs-agent-renew-tests--ran session)
    (cl-letf (((symbol-function 'romacs-agent-events-async) #'ignore))
      (romacs-agent-renew session)
      (should (string-match-p "already" (cadr (should-error (romacs-agent-renew session) :type 'user-error))))))
  (let ((session (romacs-agent--make :id "x" :harness 'claude)))
    (cl-letf (((symbol-function 'romacs-agent--harness-get) (lambda (_ key) (and (eq key :display) "Fake"))))
      (should (string-match-p "cannot renew" (cadr (should-error (romacs-agent-renew session) :type 'user-error)))))))

(ert-deftest romacs-agent-renew-soon-waits-for-the-turn-end ()
  ;; Asked mid-turn (an agent renewing itself), at its turn's end.
  (romacs-agent-renew-tests--with 'claude
    (setf (romacs-agent-state session) 'running)
    (cl-letf (((symbol-function 'romacs-agent--live-p) (lambda (_) t)))
      (should (eq (romacs-agent-renew-soon session "its own") 'soon))
      (should-not (romacs-agent-renew-tests--notes session))
      (setf (romacs-agent-state session) 'idle)
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (let ((end (+ (float-time) 30)))
        (while (and (= relaunched 0) (< (float-time) end)) (accept-process-output nil 0.05)))
      (should (= relaunched 1))
      (should (equal (gethash "why" (car (romacs-agent-renew-tests--notes session))) "its own"))
      ;; Once: a later end does not renew again.
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (accept-process-output nil 0.2)
      (should (= relaunched 1)))))

(ert-deftest romacs-agent-renew-keys-and-tool ()
  (should (eq (lookup-key romacs-agent-chat-actions-map "F") 'romacs-agent-chat-renew))
  (should (equal (command-modes 'romacs-agent-chat-renew) '(romacs-agent-chat-mode)))
  (should (functionp (plist-get (cdr (assoc "romacs_task_renew" (let ((romacs-mcp-tools nil)) (load "romacs-agent-tools" nil t) romacs-mcp-tools))) :handler))))

(ert-deftest romacs-agent-renew-a-failed-write-keeps-the-brief ()
  (romacs-agent-renew-tests--with 'claude
    (romacs-agent-renew-tests--run session)
    (let ((brief (romacs-agent-brief session)))
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (user-error "Session t has ended"))))
        (should-error (romacs-agent--input session "request") :type 'user-error))
      (should (equal (romacs-agent-brief session) brief))
      (romacs-agent--input session "request")
      (should (string-prefix-p brief (romacs-agent--text-of (gethash "content" (gethash "message" (car (car written))))))))))

(ert-deftest romacs-agent-renew-at-a-turn-end-holds-before-what-waited ()
  ;; Wanted at a turn's end: held at once, so what waited for that end
  ;; goes to the new conversation, not the old.
  (romacs-agent-renew-tests--with 'claude
    (setf (romacs-agent-state session) 'running)
    (cl-letf (((symbol-function 'romacs-agent--live-p) (lambda (_) t)))
      (romacs-agent-renew-soon session "full")
      (romacs-agent-send-later session "next")
      (should (= (romacs-agent-queue-length session) 1))
      (setf (romacs-agent-state session) 'idle)
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (should (romacs-agent--held-p session))
      (romacs-agent--send-next session t)
      (should-not written))))

(ert-deftest romacs-agent-renew-unfinished-is-finished-on-restore ()
  ;; Journaled, then Romacs ended before its harness ran anew: a restore
  ;; finishes it, its manifest made to say so, and holds it till then.
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-options session) (list :fork "thread-source"))
    (romacs-agent--manifest-put session "options" (romacs-agent--options-object (romacs-agent-options session)))
    (romacs-agent--record session (romacs-mcp--object "seq" 50 "t" 50.0 "note"
                                                      (romacs-mcp--object "type" "renew" "from" "old" "to" :null
                                                                          "brief" "B")))
    (should (romacs-agent--held-p session))
    (romacs-agent-renew--restored)
    (should (= relaunched 1))
    (should-not (plist-member (romacs-agent-options session) :fork))
    (should (null (gethash "harnessSession" (romacs-agent-renew-tests--manifest session) :absent)))
    ;; Renewed and run since: nothing to finish.
    (romacs-agent--raise session (list :type 'run :seq 51))
    (romacs-agent-renew--restored)
    (should (= relaunched 1))))

(ert-deftest romacs-agent-renew-refuses-a-rollback-meanwhile ()
  (romacs-agent-renew-tests--with 'claude
    (cl-letf (((symbol-function 'romacs-agent-events-async) #'ignore))
      (romacs-agent-renew session)
      (should (string-match-p "renewing already"
                              (cadr (should-error (romacs-agent-rollback session 9 #'ignore) :type 'user-error)))))))

(ert-deftest romacs-agent-renew-unfinished-is-finished-once-replayed ()
  ;; Its supervisor still runs: the renewal is found by its replay, and
  ;; finished once that is over, once.
  (romacs-agent-renew-tests--with 'claude
    (let ((connection (make-process :name "romacs-renew-old" :command '("cat") :noquery t)))
      (unwind-protect
          (progn
            (setf (romacs-agent-connection session) connection)
            (romacs-agent-renew--restored)
            (should (= relaunched 0))
            (romacs-agent--record session (romacs-mcp--object "seq" 50 "t" 50.0 "note"
                                                              (romacs-mcp--object "type" "renew" "from" "a"
                                                                                  "to" (romacs-agent--uuid) "brief" "B")))
            (run-hook-with-args 'romacs-agent-caught-up-functions session)
            (run-hook-with-args 'romacs-agent-caught-up-functions session)
            (should (= relaunched 1))
            ;; Its new run: done with, a later renewal finishes again.
            (romacs-agent--raise session (list :type 'run :seq 51))
            (should-not (gethash session romacs-agent-renew--finishing)))
        (delete-process connection)))))

(ert-deftest romacs-agent-renew-survives-a-restart-before-its-note ()
  ;; Its harness run anew (a selection's restart) before the journal is
  ;; read: the renewal still holds it, and goes on.  A selection is
  ;; refused while it holds.
  (romacs-agent-renew-tests--with 'claude
    (let ((outcome nil) (end (+ (float-time) 30)))
      (romacs-agent-renew session nil (lambda (result) (setq outcome result)))
      (should (string-match-p "renewing" (cadr (should-error (romacs-agent-select session "m" nil) :type 'user-error))))
      ;; What a resume does to its driver's state.
      (setf (romacs-agent-driver-state session) nil)
      (should (romacs-agent--held-p session))
      (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05))
      (should (eq (car outcome) :done)))))

(defun romacs-agent-renew-tests--noted (session asked live outcome-cell reply)
  "Answer SESSION's note turn with REPLY (a done's plist), once ASKED (a
function) says it was asked: LIVE (a cell) says its harness is there
until then."
  (let ((end (+ (float-time) 30)))
    (while (and (not (funcall asked)) (< (float-time) end))
      (accept-process-output nil 0.05)))
  (setcar live nil)
  (romacs-agent--raise session (list :type 'input :text "note?" :own "renew-note" :seq 20))
  (romacs-agent--raise session (append (list :type 'done :seq 21) reply))
  (let ((end (+ (float-time) 30)))
    (while (and (not (car outcome-cell)) (< (float-time) end)) (accept-process-output nil 0.05))))

(defmacro romacs-agent-renew-tests--asking (&rest body)
  "BODY with `session' (a delegated task) asked for its note: its harness
there (`live') and ready; `outcome' the renewal's outcome cell."
  (declare (indent 0))
  `(romacs-agent-renew-tests--with 'claude
     (let ((live (list t)) (outcome (list nil)))
       (setf (romacs-agent-options session) (list :parent "pppppppp-0000-4000-8000-000000000000")
             (romacs-agent-driver-state session) (list :ready t))
       (cl-letf (((symbol-function 'romacs-agent--harness-live-p) (lambda (_) (car live))))
         (romacs-agent-renew session "full" (lambda (result) (setcar outcome result)))
         ,@body))))

(ert-deftest romacs-agent-renew-leads-with-the-sessions-own-note ()
  ;; One more turn, Romacs's own: the session writes its note, which the
  ;; brief leads with; that turn's end goes to no parent.
  (romacs-agent-renew-tests--asking
    (romacs-agent-renew-tests--noted session (lambda () (cl-find-if (lambda (w) (hash-table-p (cdr w))) written))
                                     live outcome '(:text "Goal: the parser. Next: its tests."))
    (should (eq (car (car outcome)) :done))
    (let ((asked (cdr (cl-find-if (lambda (w) (hash-table-p (cdr w))) written))))
      (should (equal (gethash "romacsOwn" asked) "renew-note")))
    (should-not (romacs-agent-ends session))
    (let ((note (car (romacs-agent-renew-tests--notes session))))
      (should (equal (gethash "summary" note) "note"))
      (should (string-search "Handoff note, written by this session" (gethash "brief" note)))
      (should (string-search "Goal: the parser. Next: its tests." (gethash "brief" note))))))

(ert-deftest romacs-agent-renew-without-a-note-when-it-fails ()
  (romacs-agent-renew-tests--asking
    (romacs-agent-renew-tests--noted session (lambda () (cl-find-if (lambda (w) (hash-table-p (cdr w))) written))
                                     live outcome '(:text "x" :error t :reason "API error"))
    (should (eq (car (car outcome)) :done))
    (let ((note (car (romacs-agent-renew-tests--notes session))))
      (should (equal (gethash "summary" note) "selection"))
      (should-not (string-search "Handoff note" (gethash "brief" note))))))

(ert-deftest romacs-agent-renew-takes-a-compaction-summary-or-the-turns-alone ()
  ;; A compaction since its last turn: its summary is the note, no turn
  ;; asked.  With selection chosen: no turn asked either.
  (romacs-agent-renew-tests--with 'claude
    (setf (romacs-agent-driver-state session) (list :ready t))
    (cl-letf (((symbol-function 'romacs-agent--harness-live-p) (lambda (_) nil)))
      (let ((events (list '(:type done :text "ok") '(:type compacted :text "Summary: all of it."))))
        (let ((got nil))
          (romacs-agent-renew--with-note session nil events (lambda (note source) (setq got (list note source))))
          (should (equal got '("Summary: all of it." "compaction"))))
        (let ((got nil))
          (romacs-agent-renew--with-note session nil (reverse events) (lambda (note source) (setq got (list note source))))
          (should (equal got '(nil "selection"))))))
    (let ((romacs-agent-renew-summary 'selection) (got nil))
      (cl-letf (((symbol-function 'romacs-agent--harness-live-p) (lambda (_) t)))
        (romacs-agent-renew--with-note session nil nil (lambda (note source) (setq got (list note source)))))
      (should (equal got '(nil "selection")))
      (should-not written))))

(ert-deftest romacs-agent-renew-by-itself-past-its-threshold ()
  ;; Past the threshold at a turn's end: renewed by itself, once a
  ;; conversation; not below it, not at its own note turn's end.
  (romacs-agent-renew-tests--with 'claude
    (cl-letf (((symbol-function 'romacs-agent--live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-renew) (lambda (s why &rest _) (push (cons s why) relaunched))))
      (setq relaunched nil)
      (setf (romacs-agent-context-window session) 200000 (romacs-agent-context-used session) 100000)
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (should-not relaunched)
      (setf (romacs-agent-context-used session) 170000 (romacs-agent-own-turn session) "renew-note")
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (should-not relaunched)
      (setf (romacs-agent-own-turn session) nil)
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (should (equal relaunched (list (cons session "context 85%"))))
      (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
      (should (= (length relaunched) 1))
      ;; Off: never.
      (let ((romacs-agent-renew-at nil))
        (setf (romacs-agent-harness-session session) (romacs-agent--uuid))
        (run-hook-with-args 'romacs-agent-event-functions session '(:type done))
        (should (= (length relaunched) 1))))))

(ert-deftest romacs-agent-renew-a-journal-that-cannot-be-read-lets-go ()
  ;; Its journal unreadable, in its first read or a later batch: the
  ;; renewal fails, said so, and holds nothing.
  (romacs-agent-renew-tests--with 'claude
    (let ((outcome nil) (end (+ (float-time) 30)))
      (cl-letf (((symbol-function 'insert-file-contents) (lambda (&rest _) (signal 'file-error '("Opening input file" "gone")))))
        (romacs-agent-renew session nil (lambda (result) (setq outcome result)))
        (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05)))
      (should (eq (car outcome) :error))
      (should-not (romacs-agent--held-p session)))
    (let ((outcome nil) (end (+ (float-time) 30)) (steps 0))
      (cl-letf* ((record (symbol-function 'romacs-agent--record))
                 ;; The replay's copy fails at its second record, a later batch.
                 ((symbol-function 'romacs-agent--record)
                  (lambda (target &rest args)
                    (when (and (not (eq target session)) (> (cl-incf steps) 1)) (error "Bad batch"))
                    (apply record target args))))
        (romacs-agent-renew-tests--journal
         session (romacs-agent-renew-tests--prompt 5 "a") (romacs-agent-renew-tests--reply "u1" "b")
         (romacs-agent-renew-tests--prompt 9 "c") (romacs-agent-renew-tests--reply "u2" "d"))
        (cl-letf (((symbol-function 'run-at-time)
                   (lambda (_time _repeat function &rest args)
                     ;; Batches of one record: a later batch fails.
                     (if (eq function #'romacs-agent-events-async)
                         (apply function (append (seq-take args 2) (list 1 (nth 3 args))))
                       (apply function args)))))
          (romacs-agent-renew session nil (lambda (result) (setq outcome result))))
        (while (and (not outcome) (< (float-time) end)) (accept-process-output nil 0.05)))
      (should (equal outcome '(:error "Bad batch")))
      (should-not (romacs-agent--held-p session)))))

(ert-deftest romacs-agent-renew-a-failed-finish-may-be-tried-again ()
  (romacs-agent-renew-tests--with 'codex
    (romacs-agent--record session (romacs-mcp--object "seq" 50 "t" 50.0 "note"
                                                      (romacs-mcp--object "type" "renew" "from" "old" "to" :null "brief" "B")))
    (cl-letf (((symbol-function 'romacs-agent--manifest-put) (lambda (&rest _) (error "Disk full"))))
      (romacs-agent-renew--restored))
    (should (= relaunched 0))
    (romacs-agent-renew--restored)
    (should (= relaunched 1))))

(ert-deftest romacs-agent-renew-waits-for-a-prompt-on-its-way ()
  ;; A prompt written and not read back yet: its turn is on its way, the
  ;; renewal waits.  Read back, it may go.
  (romacs-agent-renew-tests--with 'claude
    (run-hook-with-args 'romacs-agent-input-functions session 7)
    (should (string-match-p "busy" (cadr (should-error (romacs-agent-renew session) :type 'user-error))))
    ;; One its harness ended without reading back is on its way no more.
    (run-hook-with-args 'romacs-agent-input-functions session 6)
    (romacs-agent--raise session (list :type 'exit :status 1 :seq 29))
    (should-not (romacs-agent-renew--unechoed-p session))
    (run-hook-with-args 'romacs-agent-input-functions session 7)
    (romacs-agent--raise session (list :type 'input :text "go" :request 7 :seq 30))
    (setf (romacs-agent-state session) 'idle (romacs-agent-done-seq session) 31)
    (should (eq (car (romacs-agent-renew-tests--run session)) :done))))

(ert-deftest romacs-agent-renew-takes-only-its-own-turns-end ()
  ;; Another turn's end while the note is asked (one that was on its way
  ;; regardless) is not the note.
  (romacs-agent-renew-tests--asking
    (let ((end (+ (float-time) 30)))
      (while (and (not (cl-find-if (lambda (w) (hash-table-p (cdr w))) written)) (< (float-time) end))
        (accept-process-output nil 0.05)))
    (setcar live nil)
    (romacs-agent--raise session (list :type 'done :seq 19 :text "The follow-up was handled"))
    (accept-process-output nil 0.1)
    (should-not (car outcome))
    (romacs-agent--raise session (list :type 'input :text "note?" :own "renew-note" :seq 20))
    (romacs-agent--raise session (list :type 'done :seq 21 :text "Goal: X."))
    (let ((end (+ (float-time) 30)))
      (while (and (not (car outcome)) (< (float-time) end)) (accept-process-output nil 0.05)))
    (let ((note (car (romacs-agent-renew-tests--notes session))))
      (should (string-search "Goal: X." (gethash "brief" note)))
      (should-not (string-search "The follow-up was handled" (gethash "brief" note))))))

(ert-deftest romacs-agent-renew-keeps-a-missed-steer-for-the-new-conversation ()
  ;; Codex owes a missed steer a turn of its own: not written while a
  ;; renewal holds the session (a refusal still is), and sent to the new
  ;; conversation first.
  (require 'romacs-agent-codex)
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-driver-state session)
          (list :thread "th" :ready t :owed (list (list :turn "turn/steer:4" "late words" nil)
                                                (list :reply 9 "item/tool/call"))))
    (romacs-agent-renew--hold session (list 'test))
    (should (eq (car (romacs-agent--codex-owed-now session)) :reply))
    (romacs-agent-renew--hold session nil)
    (should (eq (car (romacs-agent--codex-owed-now session)) :turn))
    (romacs-agent--enqueue session (list "waiting" nil nil nil))
    (romacs-agent-renew--finish session nil)
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("late words" "waiting")))))

(ert-deftest romacs-agent-renew-keeps-a-missed-steer-through-an-exit ()
  ;; Owed while held, then its harness exits (its run's state cleared)
  ;; before the renewal finishes: the turn still goes to the new one.
  (require 'romacs-agent-codex)
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-driver-state session) (list :thread "th" :ready t))
    (romacs-agent-renew--hold session (list 'test))
    (romacs-agent--raise session (list :type 'handshake
                                       :state (list :owed (list (list :turn "turn/steer:4" "late words" nil)))))
    (romacs-agent--raise session (list :type 'exit :status 0 :seq 60))
    (should-not (plist-get (romacs-agent-driver-state session) :owed))
    (romacs-agent-renew--hold session nil)
    (romacs-agent-renew--finish session nil)
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("late words")))))

(ert-deftest romacs-agent-renew-takes-the-debt-it-starts-with ()
  ;; Owed before the renewal begins, its harness exiting during the note:
  ;; the turn goes to the new conversation.  A rollback's hold takes no
  ;; debt along, and a renewal given up leaves none behind.
  (require 'romacs-agent-codex)
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-driver-state session)
          (list :thread "th" :ready t :owed (list (list :turn "turn/steer:4" "late words" nil))))
    (cl-letf (((symbol-function 'romacs-agent-events-async) #'ignore))
      (romacs-agent-renew session))
    (romacs-agent--raise session (list :type 'exit :status 0 :seq 60))
    (romacs-agent-renew--release session (romacs-agent-renew--holder session))
    ;; Given up before its note: no debt kept for a later one.
    (should-not (gethash session romacs-agent-renew--owed))
    (setf (romacs-agent-driver-state session)
          (list :thread "th" :ready t :owed (list (list :turn "turn/steer:4" "late words" nil))))
    (cl-letf (((symbol-function 'romacs-agent-events-async) #'ignore))
      (romacs-agent-renew session))
    (romacs-agent--raise session (list :type 'exit :status 0 :seq 61))
    (romacs-agent-renew--hold session nil)
    (romacs-agent-renew--finish session nil)
    (should (equal (mapcar #'car (romacs-agent-queue session)) '("late words")))
    (should-not (gethash session romacs-agent-renew--owed)))
  (romacs-agent-renew-tests--with 'codex
    (setf (romacs-agent-driver-state session) (list :thread "th" :ready t :rolling-back (list 'r)))
    (romacs-agent--raise session (list :type 'handshake
                                       :state (list :owed (list (list :turn "turn/steer:4" "late" nil)))))
    (should-not (gethash session romacs-agent-renew--owed))))

(ert-deftest romacs-agent-renew-threshold-for-views ()
  (romacs-agent-renew-tests--with 'claude
    (should (= (romacs-agent-renew-threshold session) 0.8))
    (let ((romacs-agent-renew-at nil)) (should-not (romacs-agent-renew-threshold session)))
    ;; Renewed by itself from this conversation: no more.
    (puthash session (romacs-agent--native-id session) romacs-agent-renew--automatic)
    (should-not (romacs-agent-renew-threshold session)))
  (let ((session (romacs-agent--make :id "x" :harness 'claude)))
    (cl-letf (((symbol-function 'romacs-agent--harness-get) (lambda (_ _key) nil)))
      (should-not (romacs-agent-renew-threshold session)))))

;;; romacs-agent-renew-tests.el ends here

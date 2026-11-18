;;; romacs-agent-events-tests.el --- A session's records in SQLite -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-events))

(require 'romacs-agent-claude)
(require 'romacs-test-notes)

(defun romacs-agent-events-tests--line (seq key value)
  "Record SEQ's line as the supervisor writes it: keys sorted."
  (concat (json-serialize (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq))) "\n"))

(defun romacs-agent-events-tests--turn (n seq)
  "Turn N's lines from SEQ: its prompt, its answer, its result."
  (concat (romacs-agent-events-tests--line
           seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (romacs-agent-events-tests--line
           (+ seq 1) "out" (romacs-mcp--object
                            "message" (romacs-mcp--object
                                       "content" (vector (romacs-mcp--object "text" (format "answer %d" n) "type" "text"))
                                       "id" (format "m%d" n))
                            "parent_tool_use_id" :null "type" "assistant"))
          (romacs-agent-events-tests--line
           (+ seq 2) "out" (romacs-mcp--object "result" (format "answer %d" n) "type" "result"))))

(defun romacs-agent-events-tests--with-session (turns body)
  "Call BODY with a session whose journal holds a run note and TURNS
turns, and its journal's file; a state root of its own."
  (let* ((root (make-temp-file "romacs-events-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--launched (make-hash-table :test #'equal))
         (romacs-agent--held-notes (make-hash-table :test #'equal))
         (id "e7e70000-2222-4333-8444-555566667777")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (journal (expand-file-name "journal.jsonl" dir)))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root
                                                        "title" "events" "titleAuto" :false "argv" []
                                                        "options" (romacs-mcp--object)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (concat (romacs-agent-events-tests--line 1 "note" (romacs-mcp--object "type" "run"))
                                  (mapconcat (lambda (n) (romacs-agent-events-tests--turn n (+ 2 (* 3 (1- n)))))
                                             (number-sequence 1 turns) ""))
                          nil journal nil 'silent))
          (romacs-agent--load dir)
          (funcall body (romacs-agent-get id) journal))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(defun romacs-agent-events-tests--seqs (session &optional from end)
  "The seqs of SESSION's records, as `romacs-agent--read-journal' reads them."
  (let ((seqs nil))
    (romacs-agent--read-journal session (lambda (record) (push (gethash "seq" record) seqs)) from end)
    (nreverse seqs)))

(defun romacs-agent-events-tests--at (session place)
  "The seq of the record SESSION's records hold at PLACE."
  (car (romacs-agent-events-tests--seqs session place)))

(defun romacs-agent-events-tests--hold (id)
  "A process holding the lock of the session ID, as its host does, once held."
  (let* ((lock (expand-file-name "host.lock" (expand-file-name id (romacs-agent--state-root))))
         (process (start-process "romacs-test-host-lock" nil "flock" "-o" lock "sleep" "60")))
    (process-put process 'lock lock)
    (with-timeout (10 (error "The lock was never taken"))
      (while (eql 0 (call-process "flock" nil nil nil "-n" lock "true"))
        (accept-process-output nil 0.05)))
    process))

(defun romacs-agent-events-tests--let-go (host)
  "End HOST, a process holding a session's lock (`romacs-agent-events-tests--hold'),
and wait until the lock is free: its process killed, it may hold it a
moment more under load."
  (delete-process host)
  (with-timeout (10 (error "The lock was never let go"))
    (while (not (eql 0 (call-process "flock" nil nil nil "-n" (process-get host 'lock) "true")))
      (accept-process-output nil 0.05))))

(ert-deftest romacs-agent-events-read-as-the-journal-was ()
  ;; Imported: the same records, the same tail, notes and places, read
  ;; from SQLite; the journal moved to the backup directory; a note
  ;; written with no supervisor goes to SQLite.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     12
     (lambda (session journal)
       (let* ((seqs (romacs-agent-events-tests--seqs session))
              (tail (romacs-agent--journal-tail session 3))
              (tail-seq (romacs-agent-events-tests--at session (car tail)))
              (after (romacs-agent-events-tests--seqs session (romacs-agent--journal-after session 20)))
              (notes (length (romacs-agent--journal-notes session "run")))
              (recorded (romacs-agent--recorded-p session))
              (last (gethash "seq" (romacs-agent--journal-last session '("romacsRequest") #'romacs-agent--record-request)))
              (head (romacs-agent-events-tests--at session (romacs-agent--journal-head session))))
         (should (romacs-agent-events-import session))
         (should (romacs-agent-events-p session))
         (should-not (file-exists-p journal))
         (should (directory-files (romacs-agent--state-root) nil "\\`journal-backup-"))
         (should (equal (romacs-agent-events-tests--seqs session) seqs))
         (let ((tail2 (romacs-agent--journal-tail session 3)))
           (should (equal (cdr tail2) (cdr tail)))
           (should (equal (romacs-agent-events-tests--at session (car tail2)) tail-seq)))
         (should (equal (romacs-agent-events-tests--seqs session (romacs-agent--journal-after session 20)) after))
         (should-not (romacs-agent--journal-after session 999))
         (should (= (length (romacs-agent--journal-notes session "run")) notes))
         (should (eq (romacs-agent--recorded-p session) recorded))
         ;; Read from their ends as the journal's were.
         (should (= (gethash "seq" (romacs-agent--journal-last session '("romacsRequest") #'romacs-agent--record-request)) last))
         (should (= (romacs-agent-events-tests--at session (romacs-agent--journal-head session)) head))
         (should (= (romacs-agent--journal-head session 2) 2))
         ;; Their size in UTF-8 bytes, from a seq on, against a limit.
         (should (romacs-agent-events-over-p session 0 100))
         (should-not (romacs-agent-events-over-p session 0 1000000))
         (should-not (romacs-agent-events-over-p session 999 0))
         ;; With no supervisor, Romacs's own note goes to SQLite.
         (romacs-agent--note session (romacs-mcp--object "type" "selection"))
         (should (= (length (romacs-agent--journal-notes session "selection")) 1))
         (should (= (car (last (romacs-agent-events-tests--seqs session))) (1+ (car (last seqs)))))
         ;; Importing again: nothing.
         (should (romacs-agent-events-import session))
         (should (= (length (romacs-agent-events-tests--seqs session)) (1+ (length seqs))))
         ;; Bytes, not characters: 1,000 three-byte characters are over 2,500.
         (romacs-agent--note session (romacs-mcp--object "type" "wide" "text" (make-string 1000 ?界)))
         (let ((wide (caar (sqlite-select (romacs-agent-events--db) "select max(seq) from events where session = ?"
                                          (list (romacs-agent-id session))))))
           (should (romacs-agent-events-over-p session wide 2500))
           (should-not (romacs-agent-events-over-p session wide 3200))))))))

(ert-deftest romacs-agent-events-replay-in-steps-from-sqlite ()
  ;; A replay between keystrokes reads its pages from one snapshot: a
  ;; record added meanwhile is not waited for.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--page 4))
    (romacs-agent-events-tests--with-session
     5
     (lambda (session _journal)
       (romacs-agent-events-import session)
       (let ((result :none) (steps 0))
         (cl-letf (((symbol-function 'run-at-time)
                    (lambda (_time _repeat f &rest args)
                      (when (= (cl-incf steps) 1)
                        (romacs-agent-events-append session (romacs-mcp--object "seq" 999 "t" 1.0 "note" (romacs-mcp--object "type" "late"))))
                      (apply f args))))
           (romacs-agent-events-async session (lambda (events) (setq result events)) 3))
         (should (listp result))
         (should (equal (mapcar (lambda (e) (plist-get e :text))
                                (seq-filter (lambda (e) (eq (plist-get e :type) 'input)) result))
                        '("question 1" "question 2" "question 3" "question 4" "question 5"))))))))

(ert-deftest romacs-agent-events-replay-a-page-of-seqs-at-a-time ()
  ;; A whole replay from SQLite (a renewal's worker's) reads a page of seqs
  ;; at a time, each a short read, and gives what its journal gave.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     5
     (lambda (session _journal)
       (let ((from-journal (romacs-agent-events session)) (reads 0))
         (romacs-agent-events-import session)
         (should (romacs-agent-events-p session))
         (cl-letf* ((romacs-agent--events-page 3)
                    (read (symbol-function 'romacs-agent-events-read))
                    ((symbol-function 'romacs-agent-events-read) (lambda (&rest args) (cl-incf reads) (apply read args))))
           (should (equal (romacs-agent-events session) from-journal)))
         (should (> reads 1)))))))

(ert-deftest romacs-agent-events-thinking-counters-are-not-kept ()
  (should (romacs-agent-events--counter-p
           (romacs-mcp--object "seq" 1 "out" (romacs-mcp--object "type" "system" "subtype" "thinking_tokens"))))
  (should-not (romacs-agent-events--counter-p
               (romacs-mcp--object "seq" 1 "out" (romacs-mcp--object "type" "system" "subtype" "init")))))

(ert-deftest romacs-agent-events-a-launch-imports-first ()
  ;; With the store on, a supervisor started for a journal's session
  ;; writes to SQLite: the session is imported before it starts, and its
  ;; command names the database.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events-store t) (command nil))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session journal)
       (cl-letf (((symbol-function 'call-process)
                  (lambda (_program &rest args) (setq command args) 0))
                 ((symbol-function 'romacs-agent--route-here) #'ignore))
         (romacs-agent--launch session '("harness")))
       (should (romacs-agent-events-p session))
       (should-not (file-exists-p journal))
       (should (member "--store" command))
       (should (member (romacs-agent-store--file) command))))))

(ert-deftest romacs-agent-events-two-records-of-one-seq-are-kept ()
  ;; A journal holding two records of one seq (a note written beside its
  ;; supervisor's record): both imported, read in their order, a page's
  ;; end never between them.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--page 2))
    (romacs-agent-events-tests--with-session
     1
     (lambda (session journal)
       (write-region (romacs-agent-events-tests--line 4 "note" (romacs-mcp--object "type" "selection")) nil journal t 'silent)
       (write-region (romacs-agent-events-tests--line 4 "note" (romacs-mcp--object "type" "beside")) nil journal t 'silent)
       (should (romacs-agent-events-import session))
       (should (equal (romacs-agent-events-tests--seqs session) '(1 2 3 4 4 4)))
       (let ((pager (romacs-agent-events-pager session)) (seen nil) (at 0))
         (while at
           (pcase (funcall (car pager) at)
             (`(,lines . ,next) (setq seen (append seen lines) at next))
             (_ (setq at nil))))
         (funcall (cdr pager))
         (should (= (length seen) 6)))))))

(ert-deftest romacs-agent-events-a-per-turn-start-is-where-records-go ()
  ;; A per-turn harness's turn, the store on: imported first, so where its
  ;; run's records start is a place of SQLite's, after its last record.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events-store t))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session _journal)
       (cl-letf (((symbol-function 'romacs-agent--launch) #'ignore)
                 ((symbol-function 'romacs-agent--attach) #'ignore)
                 ((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                 ((symbol-function 'romacs-agent--argv) (lambda (&rest _) '("harness"))))
         (romacs-agent--turn session "next"))
       (should (romacs-agent-events-p session))
       (should (equal (romacs-agent-journal-start session) (romacs-agent-events-end session)))
       (should (= (romacs-agent-journal-start session) 8))))))

(ert-deftest romacs-agent-events-a-failed-replay-lets-its-pages-go ()
  ;; A replay that fails in a step lets go of its snapshot; one that is done
  ;; lets go of it too, once.
  (let ((closed 0))
    (cl-letf (((symbol-function 'romacs-agent--journal-pager)
               (lambda (_s) (cons (lambda (_from) (cons (list "{\"seq\":1,\"t\":1.0,\"in\":{\"prompt\":\"p\"}}") nil))
                                  (lambda () (cl-incf closed)))))
              ((symbol-function 'run-at-time) (lambda (_t _r f &rest args) (apply f args))))
      (let ((session (romacs-agent--make :id "f" :harness 'claude :tools (make-hash-table :test #'equal)
                                         :streamed (make-hash-table :test #'equal))))
        (romacs-agent-events-async session #'ignore)
        (should (= closed 1))
        (cl-letf (((symbol-function 'romacs-agent--record) (lambda (&rest _) (error "Broken"))))
          (should-error (romacs-agent-events-async session #'ignore)))
        (should (= closed 2))))))

(defun romacs-agent-events-tests--delta (seq id text)
  (romacs-agent-events-tests--line
   seq "out" (romacs-mcp--object "api_message_id" id "type" "stream_event"
                                 "event" (romacs-mcp--object "type" "content_block_delta" "index" 0
                                                             "delta" (romacs-mcp--object "type" "text_delta" "text" text)))))

(defun romacs-agent-events-tests--reply (session)
  "SESSION's reply as a fold of its records leaves it."
  (let ((scratch (romacs-agent--scratch session)) (romacs-agent-event-functions nil))
    (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)))
    (romacs-agent-reply scratch)))

(ert-deftest romacs-agent-events-compaction-keeps-what-a-fold-needs ()
  ;; Compacted: a completed message's deltas are gone, an interrupted
  ;; one's kept, and a fold leaves the same reply.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (write-region (concat (romacs-agent-events-tests--line 2 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "q" "role" "user")
                                                                                          "romacsRequest" 1 "type" "user"))
                             (romacs-agent-events-tests--delta 3 "m1" "Hel")
                             (romacs-agent-events-tests--delta 4 "m1" "lo")
                             (romacs-agent-events-tests--line
                              5 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                          "message" (romacs-mcp--object "id" "m1" "content"
                                                                                        (vector (romacs-mcp--object "type" "text" "text" "Hello")))))
                             (romacs-agent-events-tests--line 6 "out" (romacs-mcp--object "type" "result" "result" "Hello"))
                             (romacs-agent-events-tests--line 7 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "q2" "role" "user")
                                                                                          "romacsRequest" 2 "type" "user"))
                             (romacs-agent-events-tests--delta 8 "m2" "Interrupt")
                             (romacs-agent-events-tests--delta 9 "m2" "ed"))
                     nil journal t 'silent)
       (romacs-agent-events-import session)
       (let ((before (romacs-agent-events-tests--reply session)))
         (should (= (romacs-agent-events-compact (romacs-agent-id session)) 2))
         (should (equal (romacs-agent-events-tests--seqs session) '(1 2 5 6 7 8 9)))
         (should (equal (romacs-agent-events-tests--reply session) before))
         ;; Compacted to its last record: not compacted again until it has more.
         (should-not (member (romacs-agent-id session) (romacs-agent-events--compactable))))))))

(ert-deftest romacs-agent-events-compaction-drops-a-codex-commands-streamed-output ()
  ;; A Codex command whose completed item came: its streamed output is gone
  ;; (the item holds it); one never completed keeps its own.  Removed in
  ;; windows of two seqs, and imported two records a transaction.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--compact-window 2))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (cl-flet ((codex (seq method params)
                   (romacs-agent-events-tests--line seq "out" (romacs-mcp--object "method" method "params" params))))
         (write-region (concat (codex 2 "item/started" (romacs-mcp--object "item" (romacs-mcp--object "id" "c1" "type" "commandExecution")))
                               (codex 3 "item/commandExecution/outputDelta" (romacs-mcp--object "itemId" "c1" "delta" "a"))
                               (codex 4 "item/commandExecution/outputDelta" (romacs-mcp--object "itemId" "c1" "delta" "b"))
                               (codex 5 "item/fileChange/outputDelta" (romacs-mcp--object "itemId" "c1" "delta" "c"))
                               (codex 6 "item/completed" (romacs-mcp--object "item" (romacs-mcp--object "id" "c1" "type" "commandExecution"
                                                                                                        "aggregatedOutput" "abc")))
                               (codex 7 "item/started" (romacs-mcp--object "item" (romacs-mcp--object "id" "c2" "type" "commandExecution")))
                               (codex 8 "item/commandExecution/outputDelta" (romacs-mcp--object "itemId" "c2" "delta" "cut")))
                       nil journal t 'silent))
       (romacs-agent-events-import session)
       (should (= (romacs-agent-events-compact (romacs-agent-id session)) 3))
       (should (equal (romacs-agent-events-tests--seqs session) '(1 2 6 7 8)))))))

(ert-deftest romacs-agent-events-a-migration-job-writes-and-the-editor-makes-them-its-own ()
  ;; A job writes a session's records, compacted, and its turn rows, but
  ;; they are not its own until the editor flips them, and not while it
  ;; has the session loaded; the queue leaves one loaded here or run by a
  ;; supervisor, and a launch leaves a journal too long to move here, or
  ;; one a worker is writing.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-turns)
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal))
        (romacs-agent-events--flip-queue nil))
    (romacs-agent-events-tests--with-session
     3
     (lambda (session journal)
       (let ((dir (romacs-agent-dir session)) (id (romacs-agent-id session)))
         ;; Loaded here: not queued.
         (should-not (member dir (romacs-agent-events--journaled)))
         (let ((romacs-agent-events-store t))
           ;; A launch: too long to move here, or being written by a worker.
           (let ((romacs-agent-fold-bytes 10)) (romacs-agent-events-own session))
           (puthash id t romacs-agent-events--migrating)
           (romacs-agent-events-own session)
           (remhash id romacs-agent-events--migrating)
           (should (file-exists-p journal))
           (should-not (romacs-agent-events-p session)))
         (remhash id romacs-agent--sessions)
         (should (member dir (romacs-agent-events--journaled)))
         ;; Its supervisor runs: not queued, not written.
         (let ((host (romacs-agent-events-tests--hold id)))
           (should-not (member dir (romacs-agent-events--journaled)))
           (should-not (romacs-agent-events--migrate-job (list dir)))
           (romacs-agent-events-tests--let-go host))
         (should (equal (romacs-agent-events--migrate-job (list dir)) (list id)))
         ;; Written, not its own: read from its journal still, not queued again.
         (should-not (romacs-agent-events-p session))
         (should (file-exists-p journal))
         (should-not (member dir (romacs-agent-events--journaled)))
         (should (= (caar (sqlite-select (romacs-agent-events--db) "select count(*) from turns where session = ? and first_seq > 0" (list id))) 3))
         ;; Loaded here: left written.
         (puthash id session romacs-agent--sessions)
         (cl-letf (((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
           (romacs-agent-events--flip-soon (list id))
           (should-not (romacs-agent-events-p session))
           (should (file-exists-p journal))
           ;; Unloaded: made its own, its journal backed up.
           (remhash id romacs-agent--sessions)
           (romacs-agent-events--flip-soon (list id)))
         (should (romacs-agent-events-p session))
         (should-not (file-exists-p journal))
         (should-not (sqlite-select (romacs-agent-events--db) "select 1 from staged where session = ?" (list id))))))))

(defmacro romacs-agent-events-tests--workers (&rest body)
  "Run BODY with worker VMs that run each job here, at once."
  `(cl-letf (((symbol-function 'isolate-main-p) (lambda () t))
             ((symbol-function 'isolate-spawn) (lambda (function arg &rest _) (list 'done (funcall function arg))))
             ((symbol-function 'isolate-on-done) (lambda (future then) (funcall then future)))
             ((symbol-function 'isolate-await) #'cadr)
             ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
     ,@body))

(ert-deftest romacs-agent-events-a-loaded-session-waits-for-its-move ()
  ;; A launch or a note on a loaded session too long to move here waits
  ;; for a worker to move it, ahead of the rest, and then goes to SQLite:
  ;; the notes held, their host started once it is moved.  One a chat
  ;; shows goes on in its journal at once.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p) (fboundp 'isolate-spawn)))
  (require 'romacs-agent-turns)
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal))
        (romacs-agent-events--waiting (make-hash-table :test #'equal))
        (romacs-agent-events--flip-queue nil) (romacs-agent-events--migrate-queue (list "/elsewhere"))
        (romacs-agent-events--migrate-running 1) (romacs-agent-events-migrate-jobs 1)
        (romacs-agent-events-store t))
    (romacs-agent-events-tests--with-session
     3
     (lambda (session journal)
       (let ((id (romacs-agent-id session)) (size (file-attribute-size (file-attributes journal))) (ran nil)
             ;; Loaded whole; too long to move here from now on.
             (romacs-agent-fold-bytes 10))
         ;; Shown in a chat: in its journal, at once.
         (let ((romacs-agent-events-held-functions (list (lambda (s) (eq s session)))))
           (romacs-agent-events-then session (lambda () (push 'shown ran)))
           (should (equal ran '(shown)))
           (should-not (romacs-agent-events-p session)))
         ;; A launch, then two notes, run as they came; the notes are
         ;; numbered by the host that writes them.  Its job starts at once
         ;; though the one job allowed runs: alone, ahead of the rest, its
         ;; records only.  Until it ends, all wait.
         (setf (romacs-agent-journal-start session) size)
         (let ((ended nil))
           (romacs-agent-events-tests--workers
            (cl-letf (((symbol-function 'isolate-on-done) (lambda (future then) (setq ended (lambda () (funcall then future))))))
              (romacs-agent-events-then
               session (lambda () (push (list (romacs-agent-events-p session) (1+ (romacs-agent-seq session))) ran)))
              (romacs-agent--note session (romacs-mcp--object "type" "selection"))
              (romacs-agent--note session (romacs-mcp--object "type" "plan"))
              (should (equal romacs-agent-events--migrate-queue '("/elsewhere")))
              (should (= romacs-agent-events--migrate-running 2))
              (should (equal ran '(shown)))
              (should (romacs-agent-events-waiting-p id))
              (should (= (romacs-agent-seq session) 10))
              (should (= (romacs-agent-held-notes-count) 2))
              (should (= (file-attribute-size (file-attributes journal)) size))
              ;; It ends: flipped though loaded, then what waited; the
              ;; queue's next waits for the job allowed.
              (funcall ended)
              (should (equal romacs-agent-events--migrate-queue '("/elsewhere"))))))
         (should (equal ran '((t 11) shown)))
         ;; Not compacted, no turn rows: those come after.
         (dolist (table '("compacted" "turns"))
           (should-not (and (sqlite-select (romacs-agent-events--db) "select 1 from sqlite_master where name = ?" (list table))
                            (sqlite-select (romacs-agent-events--db) (format "select 1 from %s where session = ?" table) (list id)))))
         (should (zerop (romacs-agent-held-notes-count)))
         (should-not (romacs-agent-events-waiting-p id))
         (should (romacs-agent-events-p session))
         (should (equal (sqlite-select (romacs-agent-events--db) "select seq, kind from events where session = ? and seq > 10 order by seq" (list id))
                        '((11 "note") (12 "note"))))
         ;; Where its next supervisor starts: a seq, no longer the journal's byte.
         (should (= (romacs-agent-journal-start session) 11))
         (should (= (file-attribute-size (file-attributes journal)) size)))))))

(ert-deftest romacs-agent-events-a-session-a-job-moves-is-waited-for-under-its-lock ()
  ;; A job moving it holds its lock: a launch waits for that job all the
  ;; same, not taken for a session a host runs.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p) (fboundp 'isolate-spawn)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal))
        (romacs-agent-events--waiting (make-hash-table :test #'equal))
        (romacs-agent-events--migrate-queue nil)
        (romacs-agent-events-store t))
    (romacs-agent-events-tests--with-session
     3
     (lambda (session _journal)
       (let ((id (romacs-agent-id session)) (ran nil) (romacs-agent-fold-bytes 10))
         (puthash id t romacs-agent-events--migrating)
         (let ((host (romacs-agent-events-tests--hold id)))
           (unwind-protect
               (cl-letf (((symbol-function 'isolate-main-p) (lambda () t)))
                 (romacs-agent-events-then session (lambda () (setq ran t)))
                 (should-not ran)
                 (should (romacs-agent-events-waiting-p id))
                 ;; Its job moves it: not queued again.
                 (should-not romacs-agent-events--migrate-queue))
             (romacs-agent-events-tests--let-go host))))))))

;; Sent to twice while its move waits: one resume, one launch.
(ert-deftest romacs-agent-events-a-resume-waiting-for-its-move-is-one ()
  (let ((romacs-agent--resuming (make-hash-table :test #'equal))
        (session (romacs-agent--make :id "r" :harness 'claude :state 'ended))
        (waiting nil) (launched 0) (reforks 0))
    (cl-letf (((symbol-function 'romacs-agent--per-turn-p) #'ignore)
              ((symbol-function 'romacs-agent--check-config-dir) #'ignore)
              ((symbol-function 'romacs-agent--check-companions) #'ignore)
              ((symbol-function 'romacs-agent--refork) (lambda (_) (cl-incf reforks)))
              ((symbol-function 'romacs-agent--changed) #'ignore)
              ((symbol-function 'romacs-agent--argv) #'ignore)
              ((symbol-function 'romacs-agent--launch) (lambda (&rest _) (cl-incf launched)))
              ((symbol-function 'romacs-agent--attach) #'ignore)
              ((symbol-function 'romacs-agent-events-then) (lambda (_s then &optional _) (push then waiting))))
      (romacs-agent-resume session)
      (romacs-agent-resume session)
      (should (= (length waiting) 1))
      (should (= reforks 1))
      (mapc #'funcall waiting)
      (should (= launched 1))
      (should-not (gethash "r" romacs-agent--resuming)))))

(ert-deftest romacs-agent-events-a-failed-move-lets-its-waiters-go-on ()
  ;; A job that moves nothing: what waited goes on, in the journal.
  (let ((romacs-agent-events--waiting (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal))
        (romacs-agent-events--migrate-running 1) (romacs-agent-events--migrate-queue nil)
        (ran nil))
    (puthash "s" (list (lambda () (push 1 ran)) (lambda () (push 2 ran))) romacs-agent-events--waiting)
    (cl-letf (((symbol-function 'romacs-agent-events--flip-soon) #'ignore)
              ((symbol-function 'isolate-await) (lambda (_) nil)))
      (romacs-agent-events--migrated (list "/root/s") 'done))
    (should (equal ran '(2 1)))
    (should-not (romacs-agent-events-waiting-p "s"))))

(ert-deftest romacs-agent-events-a-background-fill-moves-nothing ()
  ;; A turn fill in its batch process loads a session whose load writes a
  ;; note (a pending selection settled): the note goes to its journal; the
  ;; session is not moved to SQLite, which the editor alone does.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-turns)
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events-store t))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session journal)
       (let ((id (romacs-agent-id session)) (size (file-attribute-size (file-attributes journal))))
         (remhash id romacs-agent--sessions)
         (cl-letf (((symbol-function 'romacs-agent--load)
                    (lambda (_dir)
                      (puthash id session romacs-agent--sessions)
                      (romacs-agent--note session (romacs-mcp--object "type" "selection"))))
                   ((symbol-function 'princ) #'ignore))
           (let ((command-line-args-left (list id))) (romacs-agent-turns-fill-batch)))
         (should-not (romacs-agent-events-p session))
         (should (file-exists-p journal))
         (should (> (file-attribute-size (file-attributes journal)) size)))))))

(ert-deftest romacs-agent-events-a-flip-the-lock-holds-off-is-tried-again ()
  ;; A worker holding the lock: the flip is tried again a moment later,
  ;; not lost, and what waits for it waits on.
  (let ((romacs-agent-events--flip-queue nil) (flips 0) (later nil) (waited nil)
        (romacs-agent--sessions (make-hash-table :test #'equal)))
    (cl-letf (((symbol-function 'romacs-agent-events--db) (lambda () 'db))
              ((symbol-function 'sqlite-select) (lambda (&rest _) '((10 3))))
              ((symbol-function 'romacs-agent-events--back-up-moved) #'ignore)
              ((symbol-function 'romacs-agent-events--waited) (lambda (id) (push id waited)))
              ((symbol-function 'romacs-agent-events--flip)
               (lambda (&rest _) (if (= (cl-incf flips) 1) (signal 'sqlite-locked-error '("database is locked")) t)))
              ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args)))
              ((symbol-function 'run-with-timer) (lambda (_time _repeat f &rest args) (setq later (cons f args)))))
      (romacs-agent-events--flip-soon '("s"))
      (should (= flips 1))
      (should later)
      (should-not waited)
      (apply (car later) (cdr later))
      (should (= flips 2))
      (should (equal waited '("s")))
      (should-not romacs-agent-events--flip-queue))))

(ert-deftest romacs-agent-events-a-flip-finds-the-journal-as-written ()
  ;; Made its own only if its journal is as it was written from and no
  ;; supervisor runs; else what was written is removed, the session in its
  ;; journal.  Its last line unended: written when whole, nothing written
  ;; when cut short.  Flipped once: a second flip leaves its records be.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session journal)
       (let ((id (romacs-agent-id session)) (db (romacs-agent-events--db)))
         (cl-flet ((events () (caar (sqlite-select db "select count(*) from events where session = ?" (list id)))))
           ;; Grown after it was written.
           (pcase-let ((`(,size . ,count) (romacs-agent-events--stage session)))
             (write-region (romacs-agent-events-tests--line 99 "note" (romacs-mcp--object "type" "late")) nil journal t 'silent)
             (should-not (romacs-agent-events--flip id size count)))
           (should-not (romacs-agent-events-p session))
           (should (= (events) 0))
           ;; A supervisor started after it was written.
           (pcase-let ((`(,size . ,count) (romacs-agent-events--stage session))
                       (host (romacs-agent-events-tests--hold id)))
             (should-not (romacs-agent-events--flip id size count))
             (romacs-agent-events-tests--let-go host))
           (should (= (events) 0))
           ;; Its last line cut short: nothing written.
           (write-region "{\"note\":{\"type\":\"cut\"" nil journal t 'silent)
           (should-error (romacs-agent-events--stage session))
           (should (= (events) 0))
           ;; Whole, unended: written, and made its own.
           (let ((text (with-temp-buffer (insert-file-contents journal) (buffer-string))))
             (with-temp-file journal
               (insert (substring text 0 (string-search "{\"note\":{\"type\":\"cut\"" text))
                       (string-trim-right (romacs-agent-events-tests--line 100 "note" (romacs-mcp--object "type" "last"))))))
           (pcase-let ((`(,size . ,count) (romacs-agent-events--stage session)))
             (should (romacs-agent-events--flip id size count))
             (should (member 100 (romacs-agent-events-tests--seqs session)))
             ;; Flipped again: its records stay.
             (let ((before (events)))
               (should-not (romacs-agent-events--flip id size count))
               (should (= (events) before))))
           ;; Imported, it is not written again.
           (should-error (romacs-agent-events--stage session))))))))

(ert-deftest romacs-agent-events-a-running-session-is-not-compacted ()
  ;; Its supervisor holding its lock when compacting holds the write lock:
  ;; nothing is removed.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (write-region (concat (romacs-agent-events-tests--delta 2 "m1" "Hi")
                             (romacs-agent-events-tests--line
                              3 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                          "message" (romacs-mcp--object "id" "m1" "content" []))))
                     nil journal t 'silent)
       (romacs-agent-events-import session)
       (let ((host (romacs-agent-events-tests--hold (romacs-agent-id session))))
         (should-error (romacs-agent-events-compact (romacs-agent-id session)))
         (should (equal (romacs-agent-events-tests--seqs session) '(1 2 3)))
         (romacs-agent-events-tests--let-go host))))))

(ert-deftest romacs-agent-events-a-large-database-is-compacted-as-it-is ()
  ;; Too large to be made to give pages back: compacted all the same, its
  ;; freed pages left for reuse, and done.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--vacuum-limit 0))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (write-region (concat (romacs-agent-events-tests--delta 2 "m1" (make-string 20000 ?x))
                             (romacs-agent-events-tests--line
                              3 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                          "message" (romacs-mcp--object "id" "m1" "content" []))))
                     nil journal t 'silent)
       (romacs-agent-events-import session)
       (should (= (romacs-agent-events-compact-all) 1))
       (should (equal (caar (sqlite-select (romacs-agent-events--db) "pragma auto_vacuum")) 0))))))

(ert-deftest romacs-agent-events-a-new-database-gives-pages-back ()
  ;; Its first opening for records, before any is in it, makes it one that
  ;; gives back its freed pages.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil))
    (romacs-agent-events-tests--with-session
     0 (lambda (_session _journal)
         (should (equal (caar (sqlite-select (romacs-agent-events--db) "pragma auto_vacuum")) 2))))))

;;; romacs-agent-events-tests.el ends here

(ert-deftest romacs-agent-events-a-session-started-before-the-store-goes-to-it ()
  ;; In the editor, the store not made yet (the writer makes it as the
  ;; editor starts): a new session's records go to it all the same, not to
  ;; a journal no store-drawn chat shows while it runs.
  (let* ((root (make-temp-file "romacs-first-" t))
         (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
         (romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-events-store t)
         (romacs-agent-events--imported (make-hash-table :test #'equal))
         (romacs-agent-writer-queued t) (asked nil) (done nil)
         (session (romacs-agent--make :id "first" :dir (expand-file-name "first" root))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-write)
                   (lambda (function args &optional then &rest _) (push (cons function args) asked) (setq done then)))
                  ((symbol-function 'romacs-agent-store--make-soon) #'ignore))
          (make-directory (romacs-agent-dir session) t)
          (should-not (romacs-agent-events--db))
          ;; Its launch waits for the writer to make the store and mark it:
          ;; nothing reads its records here before they can be read.
          (let ((launched nil) (romacs-agent-events--waiting (make-hash-table :test #'equal))
                (romacs-agent-events--importing (make-hash-table :test #'equal)))
            (romacs-agent-events-then session (lambda () (setq launched t)))
            (should-not launched)
            (should-not (romacs-agent-events-p session))
            (pcase-let ((`(,function . ,args) (car asked)))
              (should (equal (cons function args) '(romacs-agent-events--mark "first"))))
            (funcall done '(ok t))
            (should launched)
            (should (romacs-agent-events-p session))))
      (delete-directory root t))))

(ert-deftest romacs-agent-events-a-long-journal-waits-for-the-store-to-be-made ()
  ;; Early in a fresh start, the store not readable here yet: a session
  ;; whose journal only a worker moves waits for the writer to make the
  ;; store, then for the worker, never taken for unmovable meanwhile.  A
  ;; store still unreadable once the writer answered lets it go.
  ;; Every module whose tables the writer makes, as in the editor.
  (require 'romacs-agent-turns)
  (require 'romacs-agent-history)
  (dolist (made '(t nil unasked))
    (let* ((root (make-temp-file "romacs-long-" t))
           (romacs-agent-store--making nil)
           (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
           (romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-events-store t)
           (romacs-agent-events--imported (make-hash-table :test #'equal))
           (romacs-agent-events--waiting (make-hash-table :test #'equal))
           (romacs-agent-events--importing (make-hash-table :test #'equal))
           (romacs-agent-events--migrating (make-hash-table :test #'equal))
           (romacs-agent-events--migrate-queue nil) (romacs-agent-events--until-store nil)
           (romacs-agent-writer-queued t) (romacs-agent-fold-bytes 10) (ran nil)
           (session (romacs-agent--make :id "long" :dir (expand-file-name "romacs/agents/long" root)
                                        :host-socket (expand-file-name "none.sock" root))))
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent-write) #'ignore)
                    ;; Unreadable with no asking to make it (an open that
                    ;; failed otherwise): nothing would let it go.
                    ((symbol-function 'romacs-agent-store--make-soon)
                     (if (eq made 'unasked) #'ignore (symbol-function 'romacs-agent-store--make-soon)))
                    ((symbol-function 'isolate-main-p) (lambda () t))
                    ((symbol-function 'romacs-agent-events--migrate-next) #'ignore))
            (make-directory (romacs-agent-dir session) t)
            (with-temp-file (expand-file-name "journal.jsonl" (romacs-agent-dir session))
              (insert "{\"seq\":1,\"t\":1.0,\"in\":{\"prompt\":\"a long journal\"}}\n"))
            (romacs-agent-events-then session (lambda () (setq ran t)))
            (when (eq made 'unasked)
              (should ran)
              (should-not (romacs-agent-events-waiting-p "long")))
            (unless (eq made 'unasked)
              (should-not ran)
              (should (romacs-agent-events-waiting-p "long")))
            ;; The writer made the store, as it does, or not.
            (when (eq made t)
              (let ((romacs-agent-writer-queued nil))
                (romacs-agent-store--made)
                (romacs-agent-events--db))
              (setq romacs-agent-store--db nil))
            (run-hooks 'romacs-agent-store-readable-hook)
            (if (not (eq made t))
                (should ran)
              ;; A worker's now: queued for it, THEN once it is moved.
              (should-not ran)
              (should (member (romacs-agent-dir session) romacs-agent-events--migrate-queue))
              (romacs-agent-events--waited "long")
              (should ran)))
        (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
        (delete-directory root t)))))

(ert-deftest romacs-agent-events-a-start-asks-the-host-once-however-many-sessions ()
  ;; A start backs up moved journals and lists those still to move: which
  ;; a supervisor holds is asked of the host's program once, not once a
  ;; session: its cost stays flat however many sessions there are.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     1
     (lambda (_session _journal)
       (let ((ids (mapcar (lambda (n) (format "a0a00000-0000-4000-8000-%012d" n)) (number-sequence 1 40)))
             (calls 0) (backed 0))
         (dolist (id ids)
           (let ((dir (expand-file-name id (romacs-agent--state-root))))
             (make-directory dir t)
             (write-region "" nil (expand-file-name "host.lock" dir) nil 'silent)
             (write-region "{}\n" nil (expand-file-name "journal.jsonl" dir) nil 'silent)))
         (cl-letf (((symbol-function 'romacs-agent-events--host)
                    (lambda (&rest _) (cl-incf calls) (cons 0 "")))
                   ((symbol-function 'romacs-agent-events--back-up) (lambda (_) (cl-incf backed))))
           (romacs-agent-events--back-up-moved ids)
           (should (= calls 1))
           (should (= backed 40))
           (setq calls 0)
           (should (>= (length (romacs-agent-events--journaled)) 40))
           (should (= calls 1))
           ;; A great many: a call a batch, each command line bounded.
           (let ((romacs-agent-events--held-batch 15))
             (setq calls 0)
             (romacs-agent-events--journaled)
             (should (= calls 3)))))))))

(ert-deftest romacs-agent-events-a-socket-left-behind-supervises-nothing ()
  ;; One holds a session while its lock is held (a host running, a move);
  ;; a socket and a lock file a host left behind (its unit stopped, a
  ;; crash) are none, so the session is moved.  A host older than the lock
  ;; left no lock file: its socket says so then.
  (let* ((root (make-temp-file "romacs-stale-" t))
         (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
         (id "aaaaaaaa-0000-4000-8000-00000000050c")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (lock (expand-file-name "host.lock" dir))
         (socket (expand-file-name "host.sock" root)))
    (unwind-protect
        (progn
          (make-directory dir t)
          (should-not (romacs-agent-events--supervised-p id socket))
          ;; An older host: its socket alone.
          (write-region "" nil socket nil 'silent)
          (should (romacs-agent-events--supervised-p id socket))
          ;; A host gone: its lock file and socket left, held by none.
          (write-region "" nil lock nil 'silent)
          (should-not (romacs-agent-events--supervised-p id socket))
          (should-not (gethash dir (romacs-agent-events--held (list dir))))
          ;; Held (a host starting, running, or a move): one holds it.
          (let ((locker (start-process "romacs-test-lock" nil "flock" "-o" lock "sleep" "30")))
            (unwind-protect
                (progn
                  (with-timeout (10 (error "The lock was never taken"))
                    (while (eql 0 (call-process "flock" nil nil nil "-n" lock "true"))
                      (accept-process-output nil 0.05)))
                  (should (romacs-agent-events--supervised-p id socket))
                  (should (gethash dir (romacs-agent-events--held (list dir)))))
              (delete-process locker))))
      (delete-directory root t))))

(ert-deftest romacs-agent-events-an-import-that-fails-says-how ()
  ;; The host's program killed by a signal reads as that signal, negated;
  ;; an import that fails says how its stage ended and what it said last.
  (cl-letf (((symbol-function 'romacs-agent-events--host-program) (lambda () "sh")))
    (should (equal (romacs-agent-events--host "-c" "kill -KILL $$") '(-9 . "")))
    (should (equal (romacs-agent-events--host "-c" "echo boom; exit 3") '(3 . "boom\n")))
    ;; What it wrote just before it ended, all of it.
    (should (equal (cdr (romacs-agent-events--host "-c" "head -c 200000 /dev/zero | tr '\\0' x; echo"))
                   (concat (make-string 200000 ?x) "\n"))))
  (cl-letf (((symbol-function 'romacs-agent-events--host-program) (lambda () "/nonexistent/romacs-agent-host")))
    (should-not (car (romacs-agent-events--host "--stage"))))
  (let ((session (romacs-agent--make :id "f0f0f0f0-1111-4222-8333-444455556666" :dir "/nonexistent")))
    (cl-letf (((symbol-function 'romacs-agent-store--file) (lambda () "/nonexistent/sessions.sqlite")))
      (cl-letf (((symbol-function 'romacs-agent-events--host) (lambda (&rest _) '(-9 . ""))))
        (should (string-match-p "killed by signal 9, saying nothing"
                                (error-message-string (should-error (romacs-agent-events--stage session))))))
      (cl-letf (((symbol-function 'romacs-agent-events--host) (lambda (&rest _) '(nil . "Searching for program: No such file"))))
        (should (string-match-p "did not run: Searching for program"
                                (error-message-string (should-error (romacs-agent-events--stage session))))))
      (cl-letf (((symbol-function 'romacs-agent-events--host) (lambda (&rest _) '(2 . "the store is locked\n"))))
        (should (string-match-p "exited 2: the store is locked\\'"
                                (error-message-string (should-error (romacs-agent-events--stage session)))))))))

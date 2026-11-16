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

(ert-deftest romacs-agent-events-a-line-is-kept-as-written ()
  ;; As the supervisor writes it: its row read from its ends, the line as
  ;; it is; a counter left out; another order parsed.
  (let ((line "{\"out\":{\"type\":\"assistant\",\"text\":\"thinking_tokens are words\"},\"seq\":7,\"t\":12.5}"))
    (should (equal (romacs-agent-events--line-row "s" line) (list "s" 7 12.5 "out" line))))
  (should-not (romacs-agent-events--line-row
               "s" "{\"out\":{\"subtype\":\"thinking_tokens\",\"type\":\"system\"},\"seq\":8,\"t\":1.0}"))
  (let ((row (romacs-agent-events--line-row "s" "{\"seq\":9,\"t\":2.0,\"note\":{\"type\":\"run\"}}")))
    (should (equal (seq-take row 4) '("s" 9 2.0 "note"))))
  (should-not (romacs-agent-events--line-row "s" "not json"))
  ;; A line of megabytes: its ends read, no pattern across it.
  (let ((line (concat "{\"out\":{\"text\":\"" (make-string 4000000 ?a) "\"},\"seq\":10,\"t\":3.0}")))
    (should (equal (seq-take (romacs-agent-events--line-row "s" line) 4) '("s" 10 3.0 "out"))))
  ;; A nested seq and time are not the line's.
  (let ((row (romacs-agent-events--line-row "s" "{\"t\":4.0,\"seq\":11,\"note\":{\"seq\":1,\"t\":2}}")))
    (should (equal (seq-take row 4) '("s" 11 4.0 "note")))))

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
        (romacs-agent-events--compact-window 2) (romacs-agent-events--chunk 2))
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

(ert-deftest romacs-agent-events-a-migration-job-moves-what-is-left ()
  ;; A job moves a session in a journal to SQLite, compacted, its turns
  ;; rows; the queue leaves one loaded here or run by a supervisor, and a
  ;; launch leaves a journal too long to move on the editor.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-turns)
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--migrating (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     3
     (lambda (session journal)
       (let ((dir (romacs-agent-dir session)) (id (romacs-agent-id session)))
         ;; Loaded here: not queued.
         (should-not (member dir (romacs-agent-events--journaled)))
         ;; A launch: too long to move here, left in its journal.
         (let ((romacs-agent-events-store t) (romacs-agent-fold-bytes 10))
           (romacs-agent-events-own session)
           (should (file-exists-p journal))
           (should-not (romacs-agent-events-p session)))
         (remhash id romacs-agent--sessions)
         (should (member dir (romacs-agent-events--journaled)))
         ;; Its supervisor runs: not queued.
         (let ((socket (romacs-agent--host-socket id)))
           (make-directory (file-name-directory socket) t)
           (write-region "" nil socket nil 'silent)
           (should-not (member dir (romacs-agent-events--journaled)))
           (should-not (romacs-agent-events--migrate-job (list dir)))
           (delete-file socket))
         (should (equal (romacs-agent-events--migrate-job (list dir)) (list id)))
         ;; Its journal kept until the editor knows no reader has it.
         (should (file-exists-p journal))
         (should (romacs-agent-events-p session))
         (puthash id session romacs-agent--sessions)
         (romacs-agent-events--back-up-moved (list id))
         (should (file-exists-p journal))
         (remhash id romacs-agent--sessions)
         (romacs-agent-events--back-up-moved (list id))
         (should-not (file-exists-p journal))
         (should (= (caar (sqlite-select (romacs-agent-events--db) "select count(*) from turns where session = ? and first_seq > 0" (list id))) 3))
         (should-not (member dir (romacs-agent-events--journaled))))))))

(ert-deftest romacs-agent-events-a-move-is-one-processs-and-checked-at-its-end ()
  ;; Claimed by another live process: left.  A dead one's claim: taken.
  ;; The journal grown, or a supervisor started, as it moved: undone, the
  ;; session in its journal.  Its last line unended: kept when whole, no
  ;; move when cut short.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session journal)
       (let ((id (romacs-agent-id session)) (db (romacs-agent-events--db)) (dead 999999999))
         (cl-flet ((claimed (pid) (sqlite-execute db "insert or replace into moving values (?, ?, ?)" (list id pid 1.0)))
                   (events () (caar (sqlite-select db "select count(*) from events where session = ?" (list id)))))
           ;; pid 1 lives as long as the system does.
           (claimed 1)
           (should-not (romacs-agent-events-import session))
           (should (file-exists-p journal))
           (should (= (events) 0))
           (should-not (process-attributes dead))
           (claimed dead)
           ;; Grown as it moved: undone.
           (let ((lines (symbol-function 'romacs-agent--journal-lines)) (once nil))
             (cl-letf (((symbol-function 'romacs-agent--journal-lines)
                        (lambda (&rest args)
                          (unless once
                            (setq once t)
                            (write-region (romacs-agent-events-tests--line 99 "note" (romacs-mcp--object "type" "late")) nil journal t 'silent))
                          (apply lines args))))
               (should-error (romacs-agent-events-import session))))
           (should-not (romacs-agent-events-p session))
           (should (= (events) 0))
           (should (file-exists-p journal))
           (should-not (sqlite-select db "select 1 from moving where session = ?" (list id)))
           ;; A supervisor started as it moved: undone.
           (let ((socket (romacs-agent--host-socket id)) (lines (symbol-function 'romacs-agent--journal-lines)))
             (make-directory (file-name-directory socket) t)
             (cl-letf (((symbol-function 'romacs-agent--journal-lines)
                        (lambda (&rest args) (write-region "" nil socket nil 'silent) (apply lines args))))
               (should-error (romacs-agent-events-import session)))
             (delete-file socket))
           (should-not (romacs-agent-events-p session))
           ;; Loaded by an editor as it moved (its mark replaces the claim): undone.
           (let ((lines (symbol-function 'romacs-agent--journal-lines)))
             (cl-letf (((symbol-function 'romacs-agent--journal-lines)
                        (lambda (&rest args) (claimed 1) (apply lines args))))
               (should-error (romacs-agent-events-import session))))
           (should-not (romacs-agent-events-p session))
           (should (= (events) 0))
           ;; The editor's mark stays; a move waits for a start that leaves it unloaded.
           (should-not (romacs-agent-events-import session))
           (claimed dead)
           ;; Its last line cut short: no move.
           (write-region "{\"note\":{\"type\":\"cut\"" nil journal t 'silent)
           (should-error (romacs-agent-events-import session))
           (should-not (romacs-agent-events-p session))
           ;; Whole, unended: moved with it.
           (let ((text (with-temp-buffer (insert-file-contents journal) (buffer-string))))
             (with-temp-file journal
               (insert (substring text 0 (string-search "{\"note\":{\"type\":\"cut\"" text))
                       (string-trim-right (romacs-agent-events-tests--line 100 "note" (romacs-mcp--object "type" "last"))))))
           (should (romacs-agent-events-import session))
           (should (member 100 (romacs-agent-events-tests--seqs session)))
           ;; Another that saw it not moved claims it after: it finds it moved,
           ;; and its records stay.
           (let ((before (events)) (romacs-agent-events--imported (make-hash-table :test #'equal)))
             (cl-letf (((symbol-function 'romacs-agent-events-p) #'ignore))
               (should (romacs-agent-events-import session)))
             (should (= (events) before)))))))))

(ert-deftest romacs-agent-events-a-running-session-is-not-compacted ()
  ;; Its supervisor's socket there when compacting holds the write lock:
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
       (let ((socket (romacs-agent--host-socket (romacs-agent-id session))))
         (make-directory (file-name-directory socket) t)
         (write-region "" nil socket nil 'silent)
         (should-error (romacs-agent-events-compact (romacs-agent-id session)))
         (should (equal (romacs-agent-events-tests--seqs session) '(1 2 3)))
         (delete-file socket))))))

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

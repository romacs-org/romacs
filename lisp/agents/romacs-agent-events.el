;;; romacs-agent-events.el --- A session's records in SQLite  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A session's records (what its journal.jsonl holds: the harness's
;; output, what was written to it, Romacs's notes, its exits) can live in
;; the `events' table of the agents' sessions.sqlite instead, a row a
;; record: its session, seq, time, kind and the record itself as JSON.
;; Unlike the `sessions' table, which is a cache, these rows are the
;; records: they are never dropped to be made again.
;;
;; A session is read from SQLite once it has been imported (a row in
;; `imported', written in the transaction that imports its journal):
;; until then its journal is read as before.  The editor moves no journal
;; itself: the installer stages them offline and a start makes them the
;; sessions' own (`romacs-agent-events-migrate').  The journal reader
;; stays only for the sessions an install found running, and goes once
;; none is left (romacs#317).  Readers name places in a
;; session's records the same way for both: for a journal a byte, for
;; SQLite a seq, FROM inclusive and END exclusive, and pass them on
;; without looking inside.  A read that spans more than one statement is
;; one read transaction, so what it reads does not change under it.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-store)

(defvar romacs-agent-turns-draw-function)

(defconst romacs-agent-events--schema
  '("create table if not exists events (
       session text not null, seq integer not null, t real, kind text, record text not null)"
    ;; Not unique: a journal can hold two records of one seq (a note
    ;; written beside its supervisor's record), read in the order kept.
    "create index if not exists events_seq on events (session, seq)"
    "create index if not exists events_kind on events (session, kind, seq)"
    "create table if not exists imported (session text primary key, at real) without rowid"
    ;; A session's records the installer staged, not its records yet: the
    ;; journal's size and the count they were written from
    ;; (`romacs-agent-events--flip').
    "create table if not exists staged (session text primary key, size integer, count integer) without rowid")
  "The records' tables, made with the database: never dropped.")

(defvar romacs-agent-events--made nil "The database the tables were made in.")

(defun romacs-agent-events--db ()
  "The database, its records' tables made; nil without SQLite."
  (when-let* ((db (romacs-agent-store--open)))
    ;; The editor reads: the writer made them (`romacs-agent-store--made').
    (unless (or (eq romacs-agent-events--made db) (romacs-agent-store-readonly-p))
      ;; Before any record is in it: a database made to give back its
      ;; freed pages while it is small, as it is when first opened here.
      (romacs-agent-events--compact-schema db)
      (dolist (statement romacs-agent-events--schema) (sqlite-execute db statement))
      (setq romacs-agent-events--made db))
    db))

(add-hook 'romacs-agent-store-made-functions #'romacs-agent-events--db)

(defvar romacs-agent-events--imported (make-hash-table :test #'equal)
  "Each session id known imported, by this process: once it is, it stays.")

(defun romacs-agent-events--supervised-p (id &optional socket)
  "Whether one holds the session ID: a host running for it, or a move of
its records, holding its lock (`romacs-agent-host --supervised').  A host
older than the lock left no host.lock: its socket (SOCKET, else ID's)
says so then."
  (let ((dir (expand-file-name id (romacs-agent--state-root))))
    (if (file-exists-p (expand-file-name "host.lock" dir))
        (and (gethash dir (romacs-agent-events--held (list dir))) t)
      (let ((socket (or socket (romacs-agent--host-socket id))))
        (and socket (file-exists-p socket))))))

(defun romacs-agent-events--supervised (dirs)
  "The session directories of DIRS one holds, as a table, as
`romacs-agent-events--supervised-p' says of one: every lock asked of
the host's program at once (`romacs-agent-events--held'), never a call
a session; one with no host.lock (a host older than the lock) by its
socket."
  (let ((locked nil) (unlocked nil))
    (dolist (dir dirs)
      (if (file-exists-p (expand-file-name "host.lock" dir)) (push dir locked) (push dir unlocked)))
    (let ((held (romacs-agent-events--held locked)))
      (dolist (dir unlocked)
        (let ((socket (romacs-agent--host-socket (file-name-nondirectory dir))))
          (when (and socket (file-exists-p socket)) (puthash dir t held))))
      held)))

(defconst romacs-agent-events--held-batch 1000
  "The most session directories one `romacs-agent-host --supervised' is
asked about: its command line stays far inside the system's limit.")

(defun romacs-agent-events--held (dirs)
  "The session directories of DIRS whose lock one holds, as a table: one
`romacs-agent-host --supervised' for them all, a call for each
`romacs-agent-events--held-batch' of them."
  (let ((held (make-hash-table :test #'equal)))
    (while dirs
      (pcase-let ((`(,status . ,output) (apply #'romacs-agent-events--host "--supervised"
                                               (seq-take dirs romacs-agent-events--held-batch))))
        ;; Not started: as `make-process' would have, not its words for lines.
        (unless status (error "romacs-agent-host --supervised: %s" output))
        (dolist (line (split-string output "\n" t))
          (puthash line t held)))
      (setq dirs (nthcdr romacs-agent-events--held-batch dirs)))
    held))

(defun romacs-agent-events-p (session)
  "Whether SESSION's records are read from SQLite: it has been imported."
  (let ((id (romacs-agent-id session)))
    (or (gethash id romacs-agent-events--imported)
        (when-let* ((db (romacs-agent-events--db))
                    ((sqlite-select db "select 1 from imported where session = ?" (list id))))
          (puthash id t romacs-agent-events--imported)))))

(defun romacs-agent-events--kind (record)
  "RECORD's kind: its key other than seq and time."
  (catch 'kind
    (maphash (lambda (key _) (unless (member key '("seq" "t")) (throw 'kind key))) record)
    nil))

(defun romacs-agent-events--parse (text)
  (ignore-errors (json-parse-string text :object-type 'hash-table :null-object :null :false-object :false)))

(defmacro romacs-agent-events--reading (db &rest body)
  "BODY in one snapshot of DB's records: a savepoint, so a read within a
read nests."
  (declare (indent 1))
  `(progn (sqlite-execute ,db "savepoint reading")
          (unwind-protect (progn ,@body) (ignore-errors (sqlite-execute ,db "release reading")))))

(defconst romacs-agent-events--page 500 "The records a page of `romacs-agent-events-pager' holds.")

(defun romacs-agent-events-pager (session)
  "SESSION's records a page at a time, all from one snapshot: (FETCH
. CLOSE) as `romacs-agent--journal-pager' says, FETCH's place 0 for the
first page.  The snapshot is a read
transaction on a connection of its own, so the editor's writes meanwhile
are not part of it; CLOSE ends it."
  (let ((db (ignore-errors (sqlite-open (romacs-agent-store--file)))) (id (romacs-agent-id session)))
    (if (not db) (cons #'ignore #'ignore)
      (sqlite-pragma db "busy_timeout = 200")
      (sqlite-execute db "begin")
      ;; Its cursor is the last row read, (SEQ . ROWID): two records of
      ;; one seq are never split from each other by a page's end.
      (cons (lambda (from)
              (let* ((seq (if (consp from) (car from) -1)) (rowid (if (consp from) (cdr from) -1))
                     (rows (sqlite-select db (concat "select seq, record, rowid from events where session = ?"
                                                     " and (seq > ? or (seq = ? and rowid > ?)) order by seq, rowid limit ?")
                                          (list id seq seq rowid romacs-agent-events--page))))
                (and rows (cons (mapcar #'cadr rows)
                                (and (= (length rows) romacs-agent-events--page)
                                     (let ((last (car (last rows)))) (cons (nth 0 last) (nth 2 last))))))))
            (lambda () (ignore-errors (sqlite-execute db "commit")) (ignore-errors (sqlite-close db)))))))

;;; Reading

(defun romacs-agent-events-read (session function &optional from end)
  "Call FUNCTION with each of SESSION's records, in order, from seq FROM
\(inclusive) to seq END (exclusive) when given."
  (when-let* ((db (romacs-agent-events--db)))
    (romacs-agent-events--reading db
      (let ((rows (sqlite-select db "select record from events where session = ? and seq >= ? and seq < ? order by seq, rowid"
                                 (list (romacs-agent-id session) (or from 0) (or end most-positive-fixnum)) 'set)))
        (unwind-protect
            (let (row)
              (while (setq row (sqlite-next rows))
                (when-let* ((record (romacs-agent-events--parse (car row))))
                  (when (hash-table-p record) (funcall function record)))))
          (sqlite-finalize rows))))))

(defun romacs-agent-events-after (session seq)
  "The place after SESSION's record SEQ, or nil when it has none."
  (when-let* ((db (romacs-agent-events--db))
              ((sqlite-select db "select 1 from events where session = ? and seq = ?" (list (romacs-agent-id session) seq))))
    (1+ seq)))

(defun romacs-agent-events-end (session)
  "The place after SESSION's last record (1 when it has none)."
  (if-let* ((db (romacs-agent-events--db))
            (last (caar (sqlite-select db "select max(seq) from events where session = ?" (list (romacs-agent-id session))))))
      (1+ last)
    1))

(defun romacs-agent-events-notes (session type &optional end)
  "SESSION's notes of TYPE (a string) before place END, oldest first."
  (when-let* ((db (romacs-agent-events--db)))
    (delq nil (mapcar (lambda (row) (let ((record (romacs-agent-events--parse (car row))))
                                      (and (hash-table-p record) (gethash "note" record))))
                      (sqlite-select db (concat "select record from events where session = ? and kind = 'note' and seq < ?"
                                                " and json_extract(record, '$.note.type') = ? order by seq, rowid")
                                     (list (romacs-agent-id session) (or end most-positive-fixnum) type))))))

(defun romacs-agent-events-tail (session turns &optional end begin)
  "As `romacs-agent--journal-tail', in places that are seqs: (FROM .
PROMPTS), FROM the seq of a prompt that started its turn with none under
way, at least TURNS back between BEGIN and END, or nil to replay all of
them, which hold PROMPTS prompts."
  (if-let* ((db (romacs-agent-events--db)))
      (romacs-agent-events--reading db
        (let* ((id (romacs-agent-id session)) (probe (romacs-agent--scratch session))
               (end (or end most-positive-fixnum)) (begin (or begin 0))
               (prompts (mapcar #'car (sqlite-select
                                       db (concat "select seq from events where session = ? and kind = 'in' and seq >= ? and seq < ?"
                                                  " and json_extract(record, '$.in.romacsRequest') is not null order by seq desc")
                                       (list id begin end))))
               (seen 0) (found nil))
          (while (and prompts (not found))
            (let ((seq (pop prompts)))
              (setq seen (1+ seen))
              (when (and (>= seen turns)
                         (let ((before (mapcar (lambda (row) (romacs-agent-events--parse (car row)))
                                               (sqlite-select db (concat "select record from events where session = ?"
                                                                         " and seq >= ? and seq < ? order by seq desc limit ?")
                                                              (list id begin seq romacs-agent--turn-free-reach)))))
                           ;; Fewer than it looks at: it reached their start.
                           (romacs-agent--turn-free-records-p
                            session probe before (< (length before) romacs-agent--turn-free-reach))))
                ;; With no prompt before it, it is no cut.
                (setq found (and prompts (cons seq seen))))))
          (or found (cons nil (+ seen (length prompts))))))
    (cons nil 0)))

;; What a journal's readers find from its ends, from SQLite.

(defun romacs-agent-events-last (session needles predicate)
  "As `romacs-agent--journal-last': SESSION's last record that PREDICATE
holds for, only a record holding one of NEEDLES (strings) parsed."
  (when-let* ((db (romacs-agent-events--db)))
    (romacs-agent-events--reading db
      (let ((rows (sqlite-select db (concat "select record from events where session = ? and ("
                                            (mapconcat (lambda (_) "instr(record, ?) > 0") needles " or ")
                                            ") order by seq desc, rowid desc")
                                 (cons (romacs-agent-id session) needles) 'set))
            (found nil))
        (unwind-protect
            (let (row)
              (while (and (not found) (setq row (sqlite-next rows)))
                (let ((record (romacs-agent-events--parse (car row))))
                  (when (and (hash-table-p record) (funcall predicate record)) (setq found record)))))
          (sqlite-finalize rows))
        found))))

(defun romacs-agent-events-head (session &optional end)
  "As `romacs-agent--journal-head', in seqs: the seq of SESSION's first
prompt before END, or END (its end by default) when it has none."
  (let ((end (or end (romacs-agent-events-end session))))
    (or (when-let* ((db (romacs-agent-events--db)))
          (caar (sqlite-select db (concat "select min(seq) from events where session = ? and kind = 'in' and seq < ?"
                                          " and json_extract(record, '$.in.romacsRequest') is not null")
                               (list (romacs-agent-id session) end))))
        end)))

(defun romacs-agent-events-ends (ids)
  "How far the records of the sessions IDS names go, in one read: each
\(ID . PLACE), PLACE the seq after its last (as `romacs-agent-events-end')."
  (when-let* ((db (romacs-agent-events--db)))
    ;; Each session's last seq by its own look into the index, not every
    ;; seq of it (a `group by' reads them all); a query for each 200, well
    ;; inside SQLite's limit on a compound select.
    (romacs-agent-events--reading db
      (mapcan (lambda (chunk)
              (delq nil (mapcar (lambda (row) (and (cadr row) (cons (car row) (1+ (cadr row)))))
                                (sqlite-select db (mapconcat (lambda (_) "select ?, (select max(seq) from events where session = ?)")
                                                             chunk " union all ")
                                               (mapcan (lambda (id) (list id id)) chunk)))))
              (seq-partition ids 200)))))

(defun romacs-agent-events-over-p (session from limit)
  "Whether SESSION's records from seq FROM on hold more than LIMIT bytes
of UTF-8, as a journal's size says for a journal: added up a record at a
time, stopping once past LIMIT."
  (when-let* ((db (romacs-agent-events--db)))
    (let ((rows (sqlite-select db (concat "select length(cast(record as blob)) from events"
                                          " where session = ? and seq >= ? order by seq")
                               (list (romacs-agent-id session) (or from 0)) 'set))
          (bytes 0) row)
      (unwind-protect
          (while (and (<= bytes limit) (setq row (sqlite-next rows)))
            (setq bytes (+ bytes (or (car row) 0))))
        (sqlite-finalize rows))
      (> bytes limit))))

;;; Writing, with no supervisor running

(defun romacs-agent-events-append (session record)
  "Add RECORD (a table with its seq and time) to SESSION's records: by the
writer, from the editor (`romacs-agent-write')."
  (romacs-agent-write 'romacs-agent-events--insert
                      (list (list (romacs-agent-id session) (gethash "seq" record) (gethash "t" record)
                                  (romacs-agent-events--kind record) (romacs-agent--json record)))))

(defun romacs-agent-events--insert (row)
  "Insert ROW, a record's (SESSION SEQ T KIND RECORD), into `events',
waiting a while for a database another writer holds (a compaction)
rather than losing it.  No store, or a write that fails, signals."
  (let ((db (or (romacs-agent-events--db) (error "No agents store to keep a record in"))))
    (let ((tries 0) (done nil))
      (while (not done)
        (condition-case err
            (progn (sqlite-execute db "insert into events values (?, ?, ?, ?, ?)" row)
                   (setq done t))
          (error (if (and (< (cl-incf tries) 50) (string-match-p "locked\\|busy" (error-message-string err)))
                     (sleep-for 0.1)
                   (signal (car err) (cdr err)))))))))

;;; Importing a journal

(defcustom romacs-agent-events-store t
  "Whether sessions' records move to SQLite: each as a supervisor is
started for it or Romacs adds to its records, small enough to import
here, and the rest as the installer stages them
\(`romacs-agent-events-migrate').  Off, journals are written as before; a
session already imported is read from SQLite either way."
  :type 'boolean :group 'romacs-agent)

(defun romacs-agent-events-own (session)
  "Import SESSION before anything more is added to its records, when
`romacs-agent-events-store' is on, no supervisor (an earlier build's,
writing its journal) runs for it, and its journal is small enough to move
here (`romacs-agent-fold-bytes'): a longer one goes on in its journal
until an install stages it (`romacs-agent-events-migrate').  From the
editor the import is the writer's (`romacs-agent-events--import-queued'):
then `queued', and what waits for it (`romacs-agent-events-then') runs
once it is done."
  (when (and romacs-agent-events-store (romacs-agent-dir session) (not (romacs-agent-events-p session))
             ;; From the editor the writer makes the store as it imports:
             ;; a session started before the store is made goes to it too
             ;; (its supervisor makes its records' tables), not to a journal
             ;; no store-drawn chat shows while it runs.
             (or (romacs-agent-writer-queued-p) (romacs-agent-events--db))
             (not (romacs-agent-events--supervised-p (romacs-agent-id session) (romacs-agent-host-socket session)))
             (<= (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (romacs-agent-dir session)))) 0)
                 (or romacs-agent-fold-bytes most-positive-fixnum)))
    (if (romacs-agent-writer-queued-p) (romacs-agent-events--import-queued session)
      (condition-case err (romacs-agent-events-import session)
        (error (message "romacs-agent-events: %s: %s" (romacs-agent-id session) (error-message-string err)))))))

(defvar romacs-agent-events--importing (make-hash-table :test #'equal)
  "The ids of the sessions whose import the writer has been asked for.")

(defun romacs-agent-events--import-queued (session)
  "Ask the writer to import SESSION; `queued'.  One with no records is
known imported at once, and only marked so (`romacs-agent-events--mark'):
nothing of it is read meanwhile, and the mark clears nothing a host
writes.  While the store cannot be read here yet (the writer has still
to make it, early in a fresh start) it is `queued' until marked, the
store then made: nothing reads its records here before it can.  One
staged as its journal now is (by the installer, or a worker before this
start) is made its own by the flip, not written again.  Its journal
stays where it is, a backup no reader reads."
  (let ((id (romacs-agent-id session)) (dir (romacs-agent-dir session)))
    (cond
     ((and (not (file-exists-p (expand-file-name "journal.jsonl" dir))) (romacs-agent-events--db))
      (puthash id t romacs-agent-events--imported)
      (romacs-agent-write 'romacs-agent-events--mark (list id))
      t)
     ((not (file-exists-p (expand-file-name "journal.jsonl" dir)))
      (unless (gethash id romacs-agent-events--importing)
        (puthash id t romacs-agent-events--importing)
        (romacs-agent-write 'romacs-agent-events--mark (list id)
                            (lambda (answer)
                              (remhash id romacs-agent-events--importing)
                              (when (eq (car-safe answer) 'ok) (puthash id t romacs-agent-events--imported))
                              (romacs-agent-events--waited id))))
      'queued)
     ((romacs-agent-events--staged-p id dir)
      (romacs-agent-events--flip-soon (list id))
      'queued)
     (t
      (unless (gethash id romacs-agent-events--importing)
        (puthash id t romacs-agent-events--importing)
        (romacs-agent-write
         'romacs-agent-events--import-dir (list id dir)
         (lambda (answer)
           (remhash id romacs-agent-events--importing)
           (when (equal answer '(ok t)) (puthash id t romacs-agent-events--imported))
           (romacs-agent-events--waited id))))
      'queued))))

(defun romacs-agent-events--staged-p (id dir)
  "Whether the session ID names, recorded in DIR, is staged as its journal
now is: its size at staging its journal's size."
  (when-let* ((db (romacs-agent-events--db))
              (size (caar (sqlite-select db "select size from staged where session = ?" (list id)))))
    (eql size (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" dir))))))

(defun romacs-agent-events--mark (id)
  "The writer's: mark the session ID, which has no journal, imported: its
records are SQLite's from the first, and none is removed."
  (sqlite-execute (romacs-agent-events--db) "insert or ignore into imported values (?, ?)" (list id (float-time)))
  t)

(defun romacs-agent-events--import-dir (id dir)
  "The writer's: import the session ID recorded in DIR, its journal kept
for the editor to back up; t when imported."
  (romacs-agent-events-import (romacs-agent--make :id id :dir dir) t))

(defun romacs-agent-events--write (db statement rows)
  "Run STATEMENT on DB for each of ROWS in one transaction."
  (sqlite-execute db "begin immediate")
  (condition-case err
      (progn (dolist (row rows) (sqlite-execute db statement row))
             (sqlite-execute db "commit"))
    (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err)))))

(defconst romacs-agent-events--insert "insert into events values (?, ?, ?, ?, ?)"
  "How an import writes a record.")

(defun romacs-agent-events--host-program ()
  "The supervisor's program, which moves and holds a session's records."
  (romacs-agent--program "romacs-agent-host"))

(defun romacs-agent-events--host (&rest args)
  "Run the supervisor's program with ARGS to its end: (STATUS . OUTPUT),
its standard output and error together, all of it; STATUS its exit
code, or the signal that ended it, negated, or nil when it could not be
started, OUTPUT then why."
  (let ((buffer (generate-new-buffer " *romacs-agent-host*" t)))
    (unwind-protect
        ;; No sentinel's line in what it said: its end is the status.
        (pcase (condition-case err
                   (make-process :name "romacs-agent-host" :noquery t :connection-type 'pipe :buffer buffer
                                 :sentinel #'ignore :command (cons (romacs-agent-events--host-program) args))
                 (error (cons nil (error-message-string err))))
          ((and `(nil . ,_) not-started) not-started)
          (process
           (while (process-live-p process) (accept-process-output process 0.05))
           ;; Ended, what it wrote last may still be in its pipe.
           (while (accept-process-output process 0))
           (cons (if (eq (process-status process) 'signal) (- (process-exit-status process)) (process-exit-status process))
                 (with-current-buffer buffer (buffer-string)))))
      (kill-buffer buffer))))

(defun romacs-agent-events--stage (session)
  "Write SESSION's journal into SQLite as records that are not yet its
own (no `imported' row: no reader reads them), by its supervisor's
program (`romacs-agent-host --stage'), off this VM: every record but
`thinking_tokens' counters, up to the journal's size as this began, its
last line too when whole and unended, what an earlier stage left removed
first.  It holds the session's lock as it runs, so no host starts on the
journal meanwhile, and refuses one a host holds.  (SIZE . COUNT), the
journal's size and the records written; an error, nothing kept, when its
last line is cut short or it cannot run."
  (let ((id (romacs-agent-id session))
        (journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
    (pcase-let ((`(,status . ,output)
                 (romacs-agent-events--host "--stage" journal "--store" (romacs-agent-store--file) "--session" id)))
      (if (and (eql status 0) (string-match "\\`\\([0-9]+\\) \\([0-9]+\\)$" output))
          (cons (string-to-number (match-string 1 output)) (string-to-number (match-string 2 output)))
        (error "Importing %s: the host's --stage %s%s" id
               (cond ((not (integerp status)) "did not run")
                     ((< status 0) (format "was killed by signal %d" (- status)))
                     ((eql status 0) "said nothing it could read")
                     (t (format "exited %d" status)))
               (let ((said (string-trim output)))
                 (if (string-empty-p said) ", saying nothing"
                   (concat ": " (substring said (max 0 (- (length said) 500)))))))))))

(defun romacs-agent-events--holding (dir function)
  "Call FUNCTION holding the lock of the session recorded in DIR, so no
host starts on its journal meanwhile (`romacs-agent-host --hold'); nil,
FUNCTION not called, when a host holds it."
  (let ((process (make-process :name "romacs-agent-hold" :noquery t :connection-type 'pipe
                                :buffer (generate-new-buffer " *romacs-agent-hold*" t)
                                :command (list (romacs-agent-events--host-program) "--hold" dir))))
    (unwind-protect
        (progn
          ;; A deadline of its own, no timer: a timer may not run here.
          (let ((end (+ (float-time) 10)))
            (while (and (process-live-p process) (< (float-time) end)
                        (not (with-current-buffer (process-buffer process)
                               (string-search "held" (buffer-string)))))
              (accept-process-output process 0.05)))
          (and (with-current-buffer (process-buffer process) (string-search "held" (buffer-string)))
               (funcall function)))
      (when (process-live-p process) (process-send-eof process))
      (let ((end (+ (float-time) 5)))
        (while (and (process-live-p process) (< (float-time) end)) (accept-process-output process 0.05)))
      (delete-process process)
      (when (buffer-live-p (process-buffer process)) (kill-buffer (process-buffer process))))))

(defun romacs-agent-events--flip (id size count &optional dir)
  "Make the records written for session ID its own, holding its lock so
no host starts on its journal meanwhile (`romacs-agent-events--holding'):
in one transaction, holding the write lock, only if its journal is still
SIZE bytes, they are still COUNT and it has none of its own yet; a host
holding the lock, never.  Run by the editor alone, for a session it does
not have loaded: no reader here switches from journal to SQLite midway.
DIR, where the session is recorded, when not under the state root.
Non-nil when made its own; else the records written, and their `staged'
row, are removed."
  ;; Two holds, the stage's and this one, not one across both: the flip
  ;; waits while the session is loaded here (`romacs-agent-events--flip-step'),
  ;; and a lock held that long would refuse its every launch meanwhile.  A
  ;; host started between them writes its journal on, which the size check
  ;; below sees: the stage is dropped, and moved again later.
  (let ((held nil) (ok nil))
    (romacs-agent-events--holding (or dir (expand-file-name id (romacs-agent--state-root)))
                                  (lambda () (setq held t ok (romacs-agent-events--flip-held id size count t))))
    (unless held (romacs-agent-events--flip-held id size count nil))
    ok))

(defun romacs-agent-events--flip-held (id size count free)
  "`romacs-agent-events--flip''s transaction: FREE, no host holds the session."
  (let* ((db (romacs-agent-events--db))
         (journal (expand-file-name "journal.jsonl" (expand-file-name id (romacs-agent--state-root))))
         (ok nil))
    (sqlite-execute db "begin immediate")
    (condition-case err
        (progn
          (setq ok (and (eql (or (file-attribute-size (file-attributes journal)) 0) size)
                        free
                        (not (sqlite-select db "select 1 from imported where session = ?" (list id)))
                        (eql (caar (sqlite-select db "select count(*) from events where session = ?" (list id))) count)))
          (if ok (sqlite-execute db "insert into imported values (?, ?)" (list id (float-time)))
            (unless (sqlite-select db "select 1 from imported where session = ?" (list id))
              (sqlite-execute db "delete from events where session = ?" (list id))))
          (sqlite-execute db "delete from staged where session = ?" (list id))
          (sqlite-execute db "commit"))
      (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err))))
    (when ok (puthash id t romacs-agent-events--imported))
    ok))

(defun romacs-agent-events-import (session &optional keep)
  "Move SESSION's journal into SQLite, here, at once: its records written
\(`romacs-agent-events--stage') and made its own (`romacs-agent-events--flip');
then the journal goes to the backup directory, unless KEEP.  Non-nil
when SESSION is imported."
  (or (romacs-agent-events-p session)
      (when (romacs-agent-events--db)
        (pcase-let ((`(,size . ,count) (romacs-agent-events--stage session)))
          (unless (romacs-agent-events--flip (romacs-agent-id session) size count (romacs-agent-dir session))
            (error "Importing %s: its journal changed meanwhile" (romacs-agent-id session))))
        (unless keep (romacs-agent-events--back-up session))
        t)))

(defun romacs-agent-events--counter-p (record)
  "Whether RECORD is a `thinking_tokens' counter, which no reader folds."
  (let ((out (gethash "out" record)))
    (and (hash-table-p out) (equal (gethash "type" out) "system") (equal (gethash "subtype" out) "thinking_tokens"))))

(defun romacs-agent-events--back-up (session)
  "Move an imported SESSION's journal to the backup directory of today."
  (let ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
    (when (file-exists-p journal)
      (let ((dir (expand-file-name (format-time-string "journal-backup-%Y%m%d/") (romacs-agent--state-root))))
        (make-directory dir t)
        (set-file-modes dir #o700)
        (rename-file journal (expand-file-name (concat (romacs-agent-id session) ".jsonl") dir) t)))))

;;; Compaction

;; A harness streams a message's text as deltas, then sends the whole
;; message: once that has come, its deltas say nothing it does not.  A
;; session no supervisor runs has them removed, here, off the editor; a
;; message that never came whole (an interrupted turn) keeps its deltas.
;; Claude tags a delta's stream event with its message's id
;; (`api_message_id'), the whole message an `assistant' record of that id;
;; Codex a delta with its item's id, the whole item `item/completed'.

(defconst romacs-agent-events--compact-statements
  '("create temp table if not exists whole (id text primary key, seq integer) without rowid"
    "delete from temp.whole"
    ;; Each message that came whole, and the last of its whole records.
    "insert or replace into temp.whole
       select json_extract(record, '$.out.message.id'), max(seq) from events
       where session = ?1 and kind = 'out' and seq < ?2 and json_extract(record, '$.out.type') = 'assistant'
         and json_extract(record, '$.out.message.id') is not null
       group by 1"
    "insert or replace into temp.whole
       select json_extract(record, '$.out.params.item.id'), max(seq) from events
       where session = ?1 and kind = 'out' and seq < ?2 and json_extract(record, '$.out.method') = 'item/completed'
         and json_extract(record, '$.out.params.item.id') is not null
       group by 1"
    ;; The deltas of those messages, from before their whole record, of
    ;; seqs ?3 to ?4 (`romacs-agent-events-compact' goes a window at a time).
    "delete from events where session = ?1 and kind = 'out' and seq < ?2 and seq >= ?3 and seq < ?4
       and ((json_extract(record, '$.out.type') = 'stream_event'
             and json_extract(record, '$.out.event.type') = 'content_block_delta'
             and seq < (select seq from temp.whole where id = json_extract(events.record, '$.out.api_message_id')))
            or (json_extract(record, '$.out.method') in ('item/agentMessage/delta', 'item/commandExecution/outputDelta',
                                                         'item/fileChange/outputDelta')
                and seq < (select seq from temp.whole where id = json_extract(events.record, '$.out.params.itemId'))))")
  "What compacting a session removes, its records before seq ?2: the
deltas of each message whose whole message came after them, and the
streamed output of each Codex command or file change whose completed item
came after it (that item holds the output a chat shows).  The last
statement's count is the records removed.")

(defconst romacs-agent-events--vacuum-limit (* 64 1024 1024)
  "The largest database made to give back its freed pages by itself: making
it so rewrites it once, holding every writer meanwhile.")

(defun romacs-agent-events--compact-schema (db)
  "Make DB's tables for compaction: what each session is compacted to, and,
while it is still small (`romacs-agent-events--vacuum-limit'), its pages
given back as they free (auto_vacuum incremental, set once, one rewrite)."
  (sqlite-execute db "create table if not exists compacted (session text primary key, seq integer) without rowid")
  (unless (or (equal (caar (sqlite-select db "pragma auto_vacuum")) 2)
              (> (* (caar (sqlite-select db "pragma page_count")) (caar (sqlite-select db "pragma page_size")))
                 romacs-agent-events--vacuum-limit))
    (sqlite-pragma db "auto_vacuum = incremental")
    (sqlite-execute db "vacuum")))

(defconst romacs-agent-events--compact-window 2000
  "The seqs compaction removes deltas of a transaction: other writers wait
for no more than that.")

(defun romacs-agent-events-compact (id)
  "Compact the session ID names, its records up to its last: the whole
messages found first, then their deltas removed a window of seqs a
transaction (`romacs-agent-events--compact-window').  The number of
records removed."
  (when-let* ((db (romacs-agent-events--db)))
    (romacs-agent-events--compact-schema db)
    (let* ((bounds (car (sqlite-select db "select min(seq), max(seq) from events where session = ?" (list id))))
           (end (1+ (or (cadr bounds) 0)))
           (statements romacs-agent-events--compact-statements)
           (removed 0))
      ;; The whole messages, in this connection's temporary table: read, no
      ;; write lock taken.
      (dolist (statement (butlast statements))
        (sqlite-execute db statement (and (string-search "?" statement) (list id end))))
      (cl-loop for low from (or (car bounds) 0) below end by romacs-agent-events--compact-window
               do (sqlite-execute db "begin immediate")
               (condition-case err
                   (progn
                     ;; Checked holding the write lock: a supervisor started
                     ;; after it writes nothing until this commits, and what
                     ;; this removes is from before it.
                     (when (romacs-agent-events--supervised-p id)
                       (error "Its supervisor runs"))
                     (setq removed (+ removed (sqlite-execute db (car (last statements))
                                                              (list id end low (+ low romacs-agent-events--compact-window)))))
                     (sqlite-execute db "commit"))
                 (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err)))))
      (romacs-agent-events--write db "insert or replace into compacted values (?, ?)" (list (list id (1- end))))
      removed)))

(defun romacs-agent-events--compactable ()
  "The ids of imported sessions with records since they were last
compacted and no supervisor running: none is writing to them."
  (when-let* ((db (romacs-agent-events--db)))
    (romacs-agent-events--compact-schema db)
    (let* ((ids (mapcar #'car (sqlite-select
                               db (concat "select imported.session from imported"
                                          " left join compacted on compacted.session = imported.session"
                                          " where coalesce(compacted.seq, 0) <"
                                          " (select max(seq) from events where events.session = imported.session)"))))
           (held (romacs-agent-events--supervised
                  (mapcar (lambda (id) (expand-file-name id (romacs-agent--state-root))) ids))))
      (cl-remove-if (lambda (id) (gethash (expand-file-name id (romacs-agent--state-root)) held)) ids))))

(defun romacs-agent-events-compact-all ()
  "Compact every compactable session, then give back the freed pages when
over a quarter of the database is free; the records removed."
  (let ((removed 0))
    (dolist (id (romacs-agent-events--compactable))
      (setq removed (+ removed (condition-case err (romacs-agent-events-compact id)
                                (error (message "romacs-agent-events: compacting %s: %s" id (error-message-string err)) 0)))))
    (when-let* ((db (romacs-agent-events--db))
                ;; Only a database made to give its pages back can: one left
                ;; as it was reuses them.
                ((equal (caar (sqlite-select db "pragma auto_vacuum")) 2))
                (free (caar (sqlite-select db "pragma freelist_count")))
                (pages (caar (sqlite-select db "pragma page_count")))
                ((> (* 4 free) pages)))
      ;; In steps, outside any session's transaction: others write meanwhile.
      (while (> (caar (sqlite-select db "pragma freelist_count")) 0)
        (sqlite-execute db "pragma incremental_vacuum(2000)")))
    removed))

(defun romacs-agent-events-compact-batch ()
  "`romacs-agent-events-compact-all' in a batch process, its count printed.
It moves no session's records to SQLite: the editor alone does."
  (let ((romacs-agent-events-store nil))
    (princ (format "romacs-agent-events: compacted, %d records removed\n" (romacs-agent-events-compact-all)))))

;;; Migration, at install

;; The installer stages every session still in a journal into SQLite,
;; offline, before any editor starts (`romacs-agent-events-migrate'): its
;; records written, compacted and its turn rows filled, recorded `staged':
;; not yet the session's own, so an editor still running reads its
;; journal as before.  The next editor makes them its own
;; (`romacs-agent-events--flip'), at idle, for a session it does not have
;; loaded, in one short transaction that finds its journal unchanged: no
;; reader here ever switches from journal to SQLite midway.  A moved
;; session's journal stays where it is, a backup no reader reads.

(defun romacs-agent-events-migrate-soon ()
  "Make what the installer staged the sessions' own once the editor is
idle (`romacs-agent-events--migrate-start'), with the store on."
  (when (and romacs-agent-events-store (not noninteractive) (fboundp 'isolate-spawn) (isolate-main-p)
             (romacs-agent-events--db))
    (run-with-idle-timer 5 nil #'romacs-agent-events--migrate-start)))

(defun romacs-agent-events-migrate-batch ()
  "`romacs-agent-events-migrate' in a batch process before any editor
starts (the installer's, on every install: done ones are passed over),
then the rows of sessions that have none filled.  Prints `skipped ID'
and `failed ID: WHY' as it goes, then `migrated N' and `rows filled N';
exits 1 when one failed."
  (let ((report (romacs-agent-events-migrate (lambda (line) (princ (concat line "\n"))))))
    (princ (format "migrated %d\n" (plist-get report :migrated)))
    (princ (format "rows filled %d\n" (romacs-agent-store-fill)))
    (kill-emacs (if (plist-get report :failed) 1 0))))

(defun romacs-agent-events-migrate (&optional say)
  "Write every recorded session still in a journal into SQLite, here
\(`romacs-agent-events--migrate-one': its records staged, compacted, its
turn rows filled), but not yet its own: (:migrated N :skipped IDS
:failed IDS).  An editor still running reads its journal as before; the
next editor's start makes the staged records the sessions' own
\(`romacs-agent-events--flip-soon').  A session whose supervisor runs
\(its lock held) is left in its journal, skipped.  SAY, when given, is
called with a line for each skipped or failed one."
  (require 'romacs-agent-turns)
  (require 'romacs-agent-ui)
  (romacs-agent-store--made)
  (let* ((db (or (romacs-agent-events--db) (error "No agents store to migrate to")))
         (say (or say #'ignore))
         (imported (make-hash-table :test #'equal)) (staged (make-hash-table :test #'equal))
         (dirs nil) (migrated 0) (skipped nil) (failed nil))
    (dolist (row (sqlite-select db "select session from imported")) (puthash (car row) t imported))
    (dolist (row (sqlite-select db "select session, size from staged")) (puthash (car row) (cadr row) staged))
    (dolist (dir (and (file-directory-p (romacs-agent--state-root))
                      (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))
      (let ((id (file-name-nondirectory dir))
            (size (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" dir)))))
        ;; Not its own yet, nor staged as its journal is now (a journal an
        ;; editor wrote to since is staged again).
        (when (and size (not (gethash id imported)) (not (eql (gethash id staged) size)))
          (push dir dirs))))
    (let ((held (romacs-agent-events--supervised dirs)))
      (dolist (dir (nreverse dirs))
        (let ((id (file-name-nondirectory dir)))
          (if (gethash dir held)
              (progn (push id skipped) (funcall say (format "skipped %s" id)))
            (if (romacs-agent-events--migrate-job (list dir))
                (cl-incf migrated)
              (push id failed)
              (funcall say (format "failed %s" id)))))))
    (list :migrated migrated :skipped (nreverse skipped) :failed (nreverse failed))))

(defun romacs-agent-events--migrate-start ()
  "Make the moves the installer staged the sessions' own: one query of
the staged ones, then one short flip each.  A moved session's journal
stays where it is, a backup no reader reads."
  (romacs-agent-events--flip-soon (mapcar #'car (sqlite-select (romacs-agent-events--db) "select session from staged"))))

(defun romacs-agent-events--migrate-job (dirs)
  "Stage the sessions recorded in DIRS in SQLite, compact them and fill
their turn rows.  The ids staged."
  (let ((romacs-agent-store-patience romacs-agent-store-patient))
    (when (romacs-agent-events--db)
      ;; It writes records; it never makes them a session's own.  The
      ;; turn rows it writes are drawn here, off the editor.
      (let ((romacs-agent-events-store nil)
            (romacs-agent-turns-draw-function #'romacs-agent-ui-draw-turn))
        (delq nil (mapcar (lambda (dir) (romacs-agent-events--migrate-one dir)) dirs))))))

(defun romacs-agent-events--migrate-one (dir)
  "Stage the session recorded in DIR in SQLite, compacted and its turn
rows filled; its id, or nil when it is left (its supervisor
runs, it failed)."
  (let ((romacs-agent--sessions (make-hash-table :test #'equal))
        (romacs-agent-event-functions nil))
    (condition-case err
        (when-let* ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir)))
                    (session (romacs-agent--of-manifest dir manifest))
                    ((not (romacs-agent-events--supervised-p (romacs-agent-id session)))))
          (puthash (romacs-agent-id session) session romacs-agent--sessions)
          ;; Its records written, not its own: the editor makes them so.
          (pcase-let* ((id (romacs-agent-id session))
                       (`(,size . ,count) (romacs-agent-events--stage session))
                       (removed (romacs-agent-events-compact id)))
            (when (fboundp 'romacs-agent-turns-fill) (romacs-agent-turns-fill session))
            (romacs-agent-events--write (romacs-agent-events--db) "insert or replace into staged values (?, ?, ?)"
                                        (list (list id size (- count removed))))
            id))
      (error (message "romacs-agent-events: moving %s: %s" dir (error-message-string err)) nil))))

(defvar romacs-agent-events--flip-queue nil "The ids of the sessions whose written records wait to be made theirs.")

(defun romacs-agent-events--flip-soon (ids)
  "Make the records the installer staged for the sessions IDS names
theirs, from the command loop (`romacs-agent-events--flip-step')."
  (let ((idle (null romacs-agent-events--flip-queue)))
    (setq romacs-agent-events--flip-queue (append romacs-agent-events--flip-queue (cl-remove-if (lambda (id) (member id romacs-agent-events--flip-queue)) ids)))
    (when (and idle romacs-agent-events--flip-queue) (run-at-time 0 nil #'romacs-agent-events--flip-step))))

(defun romacs-agent-events--flip-step ()
  "Make the waiting sessions' written records theirs, each in one short
transaction (`romacs-agent-events--flip') the writer makes, its journal
left where it is.  One loaded here is left as it is, written, for a start
that leaves it unloaded, unless something waits for it
\(`romacs-agent-events-then'), which is then called, moved or not; one
the lock held off is tried again a moment later."
  (dolist (id (prog1 romacs-agent-events--flip-queue (setq romacs-agent-events--flip-queue nil)))
    (let ((row (and (romacs-agent-events--flippable-p id)
                    ;; Unreadable: let go, read from its journal.
                    (ignore-errors
                      (car (sqlite-select (romacs-agent-events--db) "select size, count from staged where session = ?" (list id)))))))
      (if (not row) (romacs-agent-events--waited id)
        (romacs-agent-write
         'romacs-agent-events--flip (list id (car row) (cadr row))
         (lambda (answer)
           (pcase (if (and (equal answer '(ok nil)) (romacs-agent-events--imported-row-p id))
                      ;; Made before a writer that ended, sent again.
                      '(ok t)
                    answer)
             (`(ok t)
              ;; Loaded: where its next supervisor's records start is a
              ;; seq now, not a journal's byte.
              (when-let* ((session (gethash id romacs-agent--sessions)))
                (setf (romacs-agent-journal-start session) (romacs-agent-events-end session)))
              (romacs-agent-events--waited id))
             ;; The writer held off: tried again a moment later.
             (`(error (sqlite-locked-error . ,_))
              (run-with-timer 1 nil #'romacs-agent-events--flip-soon (list id)))
             (`(error ,err)
              (message "romacs-agent-events: flipping %s: %s" id (error-message-string err))
              (romacs-agent-events--waited id))
             (_ (romacs-agent-events--waited id)))))))))

;;; Waiting for an import or a flip

(defvar romacs-agent-events--waiting (make-hash-table :test #'equal)
  "Each session id to what waits for its records to be in SQLite, oldest
first: functions called once its import or flip ends, done or failed.")

(defvar romacs-agent-events--until-store nil
  "What waits for the store to be read here, each (SESSION . THEN), in the
order given (`romacs-agent-events-then').")

(defvar romacs-agent-events--store-answered nil
  "Non-nil while what waited for the store goes on: the writer has
answered, so none waits for it again.")

(defun romacs-agent-events-then (session then)
  "Call THEN once where SESSION's records are is settled.  At once when
they are in SQLite, or stay in its journal, read there until an install
moves it (romacs#317): the editor moves no journal itself.  After its own
import (`romacs-agent-events-own'), or the flip of what the installer
staged for it, when one is under way; early in a fresh start, after the
writer makes the store.  Several are called in the order given."
  (let ((id (romacs-agent-id session)) (own (romacs-agent-events-own session)))
    (cond
     ;; The store not readable here yet, early in a fresh start: what
     ;; settles it waits for the writer to make the store
     ;; (`romacs-agent-events--store-readable').
     ((and (not (eq own 'queued)) (not romacs-agent-events--store-answered)
           (romacs-agent-events--store-pending-p session))
      (setq romacs-agent-events--until-store (append romacs-agent-events--until-store (list (cons session then)))))
     ;; Its import or flip under way: called as it ends.
     ((or (eq own 'queued) (gethash id romacs-agent-events--waiting))
      (puthash id (append (gethash id romacs-agent-events--waiting) (list then)) romacs-agent-events--waiting))
     ;; Staged as its journal now is (by the installer): made its own by
     ;; the flip, not written again.
     ((and romacs-agent-events-store (not (romacs-agent-events-p session)) (romacs-agent-dir session)
           (romacs-agent-events--staged-p id (romacs-agent-dir session)))
      (puthash id (list then) romacs-agent-events--waiting)
      (romacs-agent-events--flip-soon (list id)))
     (t (funcall then)))))

(defun romacs-agent-events--store-pending-p (session)
  "Whether where SESSION's records are waits for the store, which cannot
be read here yet and the writer is asked to make (a stage of it may be
there): the store on, the writer the editor's, and not imported."
  (and romacs-agent-events-store (romacs-agent-writer-queued-p) (romacs-agent-dir session)
       (not (romacs-agent-events-p session))
       (not (romacs-agent-events--db))
       ;; The writer asked to make it (by that look): its answer lets the
       ;; wait go.  One not readable for another reason is not waited for.
       romacs-agent-store--making))

(defun romacs-agent-events--store-readable ()
  "The writer answered the asking to make the store: what waited for it
goes on, in order, as it would have (`romacs-agent-events-then'); once
only, so a store still unreadable lets it go, read from its journal."
  (let ((waiting romacs-agent-events--until-store) (romacs-agent-events--store-answered t))
    (setq romacs-agent-events--until-store nil)
    (pcase-dolist (`(,session . ,then) waiting)
      (with-demoted-errors "romacs-agent-events: after the store was made: %S"
        (romacs-agent-events-then session then)))))

(add-hook 'romacs-agent-store-readable-hook #'romacs-agent-events--store-readable)

(defun romacs-agent-events-waiting-p (id)
  "Whether something waits for the session ID names to move to SQLite,
or for the store it moves to (`romacs-agent-events--until-store')."
  (or (and (gethash id romacs-agent-events--waiting) t)
      (and (cl-find id romacs-agent-events--until-store :key (lambda (entry) (romacs-agent-id (car entry))) :test #'equal) t)))

(defun romacs-agent-events--waited (id)
  "Call what waits for the session ID names to move, moved or not, in the
order it came."
  (when-let* ((thens (gethash id romacs-agent-events--waiting)))
    (remhash id romacs-agent-events--waiting)
    (dolist (then thens)
      (with-demoted-errors "romacs-agent-events: after a move: %S" (funcall then)))))


(defun romacs-agent-events--imported-row-p (id)
  "Whether the store says the session ID names is imported, read now."
  (and (sqlite-select (romacs-agent-events--db) "select 1 from imported where session = ?" (list id)) t))

(defun romacs-agent-events--flippable-p (id)
  "Whether the editor may make the session ID names its records' own now:
not loaded here, or loaded and waiting for it."
  (or (null (gethash id romacs-agent--sessions)) (and (gethash id romacs-agent-events--waiting) t)))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-events-migrate-soon)

(provide 'romacs-agent-events)
;;; romacs-agent-events.el ends here

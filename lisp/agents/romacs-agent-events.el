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
;; until then its journal is read as before.  Readers name places in a
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
    ;; A session's records written by a worker, not its records yet: the
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

(defun romacs-agent-events--held (dirs)
  "The session directories of DIRS whose lock one holds, as a table: one
`romacs-agent-host --supervised' for them all."
  (let ((held (make-hash-table :test #'equal)))
    (when dirs
      (dolist (line (split-string (cdr (apply #'romacs-agent-events--host "--supervised" dirs)) "\n" t))
        (puthash line t held)))
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
  "How far the records of the sessions IDS names go, in one query: each
\(ID . PLACE), PLACE the seq after its last (as `romacs-agent-events-end')."
  (when-let* ((db (romacs-agent-events--db)))
    (mapcar (lambda (row) (cons (car row) (1+ (cadr row))))
            (sqlite-select db (format "select session, max(seq) from events where session in (%s) group by session"
                                      (mapconcat (lambda (_) "?") ids ","))
                           ids))))

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
started for it or Romacs adds to its records, and the rest in worker VMs
after a start (`romacs-agent-events-migrate-soon').  Off, journals are
written as before; a session already imported is read from SQLite either
way."
  :type 'boolean :group 'romacs-agent)

(defun romacs-agent-events-own (session)
  "Import SESSION before anything more is added to its records, when
`romacs-agent-events-store' is on, no supervisor (an earlier build's,
writing its journal) runs for it, and its journal is small enough to move
here (`romacs-agent-fold-bytes'): a longer one goes on in its journal
until a worker moves it (`romacs-agent-events-migrate-soon').  From the
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
             ;; A worker writing it meanwhile: it stays in its journal, and
             ;; the editor finds that journal grown when it would flip it.
             (not (gethash (romacs-agent-id session) romacs-agent-events--migrating))
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
store then made: nothing reads its records here before it can.  Its
journal goes to the backup directory here, once imported, so no reader
here finds it gone before it reads SQLite."
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
     (t
      (unless (gethash id romacs-agent-events--importing)
        (puthash id t romacs-agent-events--importing)
        (romacs-agent-write
         'romacs-agent-events--import-dir (list id dir)
         (lambda (answer)
           (remhash id romacs-agent-events--importing)
           (when (equal answer '(ok t))
             (puthash id t romacs-agent-events--imported)
             (with-demoted-errors "romacs-agent-events: backing up: %S" (romacs-agent-events--back-up session)))
           (romacs-agent-events--waited id))))
      'queued))))

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
its standard output and error together."
  (let* ((buffer (generate-new-buffer " *romacs-agent-host*" t))
         (process (make-process :name "romacs-agent-host" :noquery t :connection-type 'pipe :buffer buffer
                                :command (cons (romacs-agent-events--host-program) args))))
    (unwind-protect
        (progn
          (while (process-live-p process) (accept-process-output process 0.05))
          (cons (process-exit-status process) (with-current-buffer buffer (buffer-string))))
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
        (error "Importing %s: %s" id (string-trim output))))))

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
    (cl-remove-if #'romacs-agent-events--supervised-p
                  (mapcar #'car (sqlite-select
                                 db (concat "select imported.session from imported"
                                            " left join compacted on compacted.session = imported.session"
                                            " where coalesce(compacted.seq, 0) <"
                                            " (select max(seq) from events where events.session = imported.session)"))))))

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

;;; Migration, in worker VMs

;; With the store on, every session still in a journal moves to SQLite:
;; after a start's restore, once the editor is idle, a few sessions a job
;; go to worker VMs, `romacs-agent-events-migrate-jobs' jobs at once (one
;; database takes one writer at a time: more jobs only wait on it).  A job
;; writes each session's records, compacts them and fills its turn rows
;; (one something waits for, a resume, only its records: the rest would
;; hold that wait up, and its rows are filled once it is moved), in short
;; transactions on its own connection, and records them `staged':
;; written, not yet the session's own, so no reader reads them.  The
;; editor alone makes them its own (`romacs-agent-events--flip'), at idle,
;; for a session it does not have loaded, in one short transaction that
;; finds its journal unchanged: no reader here ever switches from journal
;; to SQLite midway, and no two processes decide it.  Its journal then
;; goes to the backup directory (Romacs's own journals only, never a
;; harness's transcripts).  A session loaded here, or whose supervisor
;; runs, waits for a later start.

(defcustom romacs-agent-events-migrate-jobs 2
  "How many worker VMs move sessions to SQLite at once.  The database takes
one writer at a time, so more jobs only wait for it, and hold the editor's
own writes up as they do."
  :type 'natnum :group 'romacs-agent)

(defvar romacs-agent-events--migrate-queue nil "The session directories still to move to SQLite.")
(defvar romacs-agent-events--migrate-running 0 "How many migration jobs run.")
(defvar romacs-agent-events--migrating (make-hash-table :test #'equal)
  "The ids of the sessions a running job moves.")

(defvar romacs-agent-events-migrated-functions nil
  "Called with the ids a migration job moved to SQLite, once it ends.")

(defconst romacs-agent-events--migrate-batch '(8 . 67108864)
  "At most how many sessions, and how many bytes of journals, a migration
job moves; a journal past the bytes goes alone.")

(defun romacs-agent-events-migrate-soon ()
  "Move every session still in a journal to SQLite, in worker VMs, once the
editor is idle: with the store on, in an editor whose worker VMs run."
  (when (and romacs-agent-events-store (not noninteractive) (fboundp 'isolate-spawn) (isolate-main-p)
             (romacs-agent-events--db))
    (run-with-idle-timer 5 nil #'romacs-agent-events--migrate-start)))

(defun romacs-agent-events--journaled ()
  "The directories of the recorded sessions still in a journal, neither
imported nor written for, none of them loaded here, nor run by a
supervisor."
  (let ((imported (make-hash-table :test #'equal)))
    ;; Moved, or written and waiting to be made theirs.
    (dolist (row (sqlite-select (romacs-agent-events--db) "select session from imported union select session from staged"))
      (puthash (car row) t imported))
    (let* ((candidates
            (cl-remove-if-not
             (lambda (dir)
               (let ((id (file-name-nondirectory dir)))
                 (and (not (gethash id imported)) (not (gethash id romacs-agent--sessions))
                      (not (gethash id romacs-agent-events--migrating))
                      (file-exists-p (expand-file-name "journal.jsonl" dir)))))
             (and (file-directory-p (romacs-agent--state-root))
                  (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-"))))
           ;; Which a supervisor holds: one asking for them all.
           (locked (seq-filter (lambda (dir) (file-exists-p (expand-file-name "host.lock" dir))) candidates))
           (held (romacs-agent-events--held locked)))
      (cl-remove-if (lambda (dir)
                      (if (member dir locked) (gethash dir held)
                        (romacs-agent-events--supervised-p (file-name-nondirectory dir))))
                    candidates))))

(defun romacs-agent-events--back-up-moved (ids)
  "Move to the backup directory the journals of the sessions IDS names
that are imported, none of them loaded here, nor run by a supervisor:
no reader has them.  A loaded one's goes at a later start."
  (dolist (id ids)
    (let ((dir (expand-file-name id (romacs-agent--state-root))))
      (when (and (not (gethash id romacs-agent--sessions))
                 (not (romacs-agent-events--supervised-p id))
                 (file-exists-p (expand-file-name "journal.jsonl" dir)))
        (with-demoted-errors "romacs-agent-events: backing up: %S"
          (romacs-agent-events--back-up (romacs-agent--make :id id :dir dir)))))))

(defun romacs-agent-events--migrate-start ()
  "Back up the journals of sessions moved before, queue every session
still in a journal, and start the jobs."
  (romacs-agent-events--back-up-moved
   (mapcar #'car (sqlite-select (romacs-agent-events--db) "select session from imported")))
  ;; Written by an earlier start's workers: made theirs first.
  (romacs-agent-events--flip-soon (mapcar #'car (sqlite-select (romacs-agent-events--db) "select session from staged")))
  (setq romacs-agent-events--migrate-queue
        (append romacs-agent-events--migrate-queue
                (cl-remove-if (lambda (dir) (member dir romacs-agent-events--migrate-queue)) (romacs-agent-events--journaled))))
  (romacs-agent-events--migrate-next))

(defun romacs-agent-events-migrate-first (id)
  "Move the session ID names to SQLite before the others waiting: a chat
opened before its turn came."
  (let ((dir (expand-file-name id (romacs-agent--state-root))))
    (when (member dir romacs-agent-events--migrate-queue)
      (setq romacs-agent-events--migrate-queue (cons dir (delete dir romacs-agent-events--migrate-queue))))))

(defun romacs-agent-events-migrating-p (id)
  "Whether a running job is moving the session ID names to SQLite."
  (and (gethash id romacs-agent-events--migrating) t))

(defun romacs-agent-events--waited-dir-p (dir)
  "Whether something waits for the session recorded in DIR to move."
  (and (gethash (file-name-nondirectory dir) romacs-agent-events--waiting) t))

(defun romacs-agent-events--migrate-next ()
  "Give the next waiting sessions to a worker VM when fewer than
`romacs-agent-events-migrate-jobs' jobs run, a job a turn of the command
loop: a worker's start is the editor's time.  One something waits for
goes alone, at once, however many run: only its import is waited for."
  (when (and romacs-agent-events--migrate-queue
             (or (< romacs-agent-events--migrate-running (max 1 romacs-agent-events-migrate-jobs))
                 (romacs-agent-events--waited-dir-p (car romacs-agent-events--migrate-queue))))
    (let ((batch nil) (waited nil) (bytes 0))
      (while (and romacs-agent-events--migrate-queue (< (length batch) (car romacs-agent-events--migrate-batch))
                  (not waited)
                  (not (and batch (romacs-agent-events--waited-dir-p (car romacs-agent-events--migrate-queue))))
                  (let ((size (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (car romacs-agent-events--migrate-queue))))
                                  0)))
                    (when (or (null batch) (<= (+ bytes size) (cdr romacs-agent-events--migrate-batch)))
                      (cl-incf bytes size))))
        (let* ((dir (pop romacs-agent-events--migrate-queue)) (id (file-name-nondirectory dir)))
          ;; Loaded here since it was queued: left for its next launch,
          ;; unless that launch waits for it.
          (unless (and (gethash id romacs-agent--sessions) (not (gethash id romacs-agent-events--waiting)))
            (puthash id t romacs-agent-events--migrating)
            (when (gethash id romacs-agent-events--waiting) (push id waited))
            (push dir batch))))
      (when batch
        (setq batch (nreverse batch))
        (cl-incf romacs-agent-events--migrate-running)
        (let ((future (condition-case err
                          (isolate-spawn (if waited 'romacs-agent-events--stage-job 'romacs-agent-events--migrate-job) batch
                                         :name "romacs-agent migrate"
                                         :require '(romacs-agent romacs-agent-claude romacs-agent-codex romacs-agent-events
                                                                 romacs-agent-turns romacs-agent-ui cl-extra help-mode)
                                         :bindings (romacs-agent--fold-bindings))
                        (error (list 'error err)))))
          (if (eq (car-safe future) 'error) (romacs-agent-events--migrated batch future)
            (isolate-on-done future (lambda (done) (romacs-agent-events--migrated batch done))))))
      (run-at-time 0 nil #'romacs-agent-events--migrate-next))))

(defun romacs-agent-events--stage-job (dirs)
  "`romacs-agent-events--migrate-job' of sessions something waits for, a
resume say: their records only, so it has them at once; their turn rows
are filled once they are theirs (`romacs-agent-events--flip-step'), their
compaction left for later."
  (romacs-agent-events--migrate-job dirs t))

(defun romacs-agent-events--migrate-job (dirs &optional staged-only)
  "In a worker VM: move the sessions recorded in DIRS to SQLite, compact
them and fill their turn rows, but STAGED-ONLY.  The ids moved."
  (let ((romacs-agent-store-patience romacs-agent-store-patient))
    (when (romacs-agent-events--db)
      ;; It writes records; it never makes them a session's own.  The
      ;; turn rows it writes are drawn here, off the editor.
      (let ((romacs-agent-events-store nil)
            (romacs-agent-turns-draw-function #'romacs-agent-ui-draw-turn))
        (delq nil (mapcar (lambda (dir) (romacs-agent-events--migrate-one dir staged-only)) dirs))))))

(defun romacs-agent-events--migrate-one (dir &optional staged-only)
  "Move the session recorded in DIR to SQLite, compacted and its turn rows
filled, but STAGED-ONLY; its id, or nil when it is left (its supervisor
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
                       (removed (if staged-only 0 (romacs-agent-events-compact id))))
            (when (and (not staged-only) (fboundp 'romacs-agent-turns-fill)) (romacs-agent-turns-fill session))
            (romacs-agent-events--write (romacs-agent-events--db) "insert or replace into staged values (?, ?, ?)"
                                        (list (list id size (- count removed))))
            id))
      (error (message "romacs-agent-events: moving %s: %s" dir (error-message-string err)) nil))))

(defun romacs-agent-events--migrated (batch done)
  "A migration job of BATCH (directories) ended, DONE its future (or (error
ERR) when none started): the ones it moved are told
\(`romacs-agent-events-migrated-functions'), and the next jobs start."
  (cl-decf romacs-agent-events--migrate-running)
  (dolist (dir batch) (remhash (file-name-nondirectory dir) romacs-agent-events--migrating))
  (let ((moved (unless (eq (car-safe done) 'error)
                 (condition-case err (isolate-await done)
                   (error (message "romacs-agent-events: a migration job: %s" (error-message-string err)) nil)))))
    (romacs-agent-events--flip-soon moved)
    ;; Not moved: what waits for it goes on with its journal.
    (dolist (dir batch)
      (unless (member (file-name-nondirectory dir) moved) (romacs-agent-events--waited (file-name-nondirectory dir)))))
  (romacs-agent-events--migrate-next))

(defvar romacs-agent-events--flip-queue nil "The ids of the sessions whose written records wait to be made theirs.")

(defun romacs-agent-events--flip-soon (ids)
  "Make the records workers wrote for the sessions IDS names theirs, from
the command loop (`romacs-agent-events--flip-step')."
  (let ((idle (null romacs-agent-events--flip-queue)))
    (setq romacs-agent-events--flip-queue (append romacs-agent-events--flip-queue (cl-remove-if (lambda (id) (member id romacs-agent-events--flip-queue)) ids)))
    (when (and idle romacs-agent-events--flip-queue) (run-at-time 0 nil #'romacs-agent-events--flip-step))))

(defun romacs-agent-events--flip-step ()
  "Make the waiting sessions' written records theirs, each in one short
transaction (`romacs-agent-events--flip') the writer makes, its journal
then backed up.  One loaded here is left as it is, written, for a start
that leaves it unloaded, unless something waits for it
\(`romacs-agent-events-then'), which is then called, moved or not.  The
ones made theirs are told at once, when every flip is answered
\(`romacs-agent-events-migrated-functions'); one the lock held off is
tried again a moment later."
  (let ((ids (prog1 romacs-agent-events--flip-queue (setq romacs-agent-events--flip-queue nil)))
        (asked 0) (moved nil) (all nil))
    (cl-flet ((answered ()
                (when (and all (zerop asked))
                  (romacs-agent-events--back-up-moved moved)
                  (when moved (run-hook-with-args 'romacs-agent-events-migrated-functions (nreverse moved))))))
      (dolist (id ids)
        (let ((row (and (romacs-agent-events--flippable-p id)
                        (car (sqlite-select (romacs-agent-events--db) "select size, count from staged where session = ?" (list id))))))
          (if (not row) (romacs-agent-events--waited id)
            (cl-incf asked)
            (romacs-agent-write
             'romacs-agent-events--flip (list id (car row) (cadr row))
             (lambda (answer)
               (cl-decf asked)
               (pcase (if (and (equal answer '(ok nil)) (romacs-agent-events--imported-row-p id))
                          ;; Made before a writer that ended, sent again.
                          '(ok t)
                        answer)
                 (`(ok t)
                  (push id moved)
                  ;; Loaded: where its next supervisor's records start is a
                  ;; seq now, not a journal's byte.
                  (when-let* ((session (gethash id romacs-agent--sessions)))
                    (setf (romacs-agent-journal-start session) (romacs-agent-events-end session)))
                  ;; Waited for: moved without its turn rows, filled now,
                  ;; off the editor, as a live turn's are.
                  (when (and (gethash id romacs-agent-events--waiting) (fboundp 'romacs-agent-turns-fill-soon))
                    (romacs-agent-turns-fill-soon (list id)))
                  (romacs-agent-events--waited id))
                 ;; A worker writing: tried again a moment later.
                 (`(error (sqlite-locked-error . ,_))
                  (run-with-timer 1 nil #'romacs-agent-events--flip-soon (list id)))
                 (`(error ,err)
                  (message "romacs-agent-events: flipping %s: %s" id (error-message-string err))
                  (romacs-agent-events--waited id))
                 (_ (romacs-agent-events--waited id)))
               (answered))))))
      (setq all t)
      (answered))))

;;; Waiting for a move

(defvar romacs-agent-events--waiting (make-hash-table :test #'equal)
  "Each session id to what waits for its records to be in SQLite, oldest
first: functions called once a worker has moved it, or its move failed.")

(defvar romacs-agent-events-held-functions nil
  "Functions called with a loaded session still in its journal; non-nil
from one when a view of it holds places in that journal: the session
goes on in its journal rather than move under that view.")

(defvar romacs-agent-events--until-store nil
  "What waits for the store to be read here, each (SESSION . THEN), in the
order given (`romacs-agent-events-then').")

(defvar romacs-agent-events--store-answered nil
  "Non-nil while what waited for the store goes on: the writer has
answered, so none waits for it again.")

(defun romacs-agent-events-then (session then)
  "Call THEN once SESSION's records are in SQLite.  At once when they are,
or are made so here (`romacs-agent-events-own'), or cannot be: the store
off, a supervisor running for it, an import here failing, or a view
holding places in its journal (`romacs-agent-events-held-functions').
A journal too long to move here, or one a worker moves now, waits for
that worker VM, ahead of every other session, and THEN is called once it
is moved or its move failed; early in a fresh start it first waits for
the writer to make the store it moves to.  Several are called in the
order given."
  (let ((id (romacs-agent-id session)) (own (romacs-agent-events-own session)))
    (cond
     ;; The store not readable here yet, early in a fresh start: a worker
     ;; moves it once the writer has made the store, so it waits for that
     ;; (`romacs-agent-events--store-readable'), not taken for unmovable.
     ((and (not (eq own 'queued)) (not romacs-agent-events--store-answered)
           (romacs-agent-events--store-pending-p session))
      (setq romacs-agent-events--until-store (append romacs-agent-events--until-store (list (cons session then)))))
     ((and (not (gethash id romacs-agent-events--waiting))
             (not (eq own 'queued))
             (not (and (not (romacs-agent-events-p session))
                       ;; A job moving it holds its lock, so it reads as
                       ;; unmovable then: waited for all the same.
                       (or (gethash id romacs-agent-events--migrating)
                           (and (romacs-agent-events--movable-p session)
                                (> (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (romacs-agent-dir session)))) 0)
                                   (or romacs-agent-fold-bytes most-positive-fixnum))))
                       (not (run-hook-with-args-until-success 'romacs-agent-events-held-functions session)))))
      (funcall then))
     (t
      (puthash id (append (gethash id romacs-agent-events--waiting) (list then)) romacs-agent-events--waiting)
      (let ((dir (romacs-agent-dir session)))
        ;; A job moving it already: it is flipped as that job ends; the
        ;; writer importing it: it is done as that import is; moved, its
        ;; waiters are let go behind the writes before them.
        (unless (or (gethash id romacs-agent-events--migrating) (gethash id romacs-agent-events--importing)
                    (romacs-agent-events-p session))
          (setq romacs-agent-events--migrate-queue (cons dir (delete dir romacs-agent-events--migrate-queue)))))
      (romacs-agent-events--migrate-next)))))

(defun romacs-agent-events--store-pending-p (session)
  "Whether SESSION's records could be moved by a worker but for the store,
which cannot be read here yet and the writer is asked to make: the store
on, the writer the editor's, a worker VM here, no supervisor writing its
journal, and not imported."
  (and romacs-agent-events-store (romacs-agent-writer-queued-p) (romacs-agent-dir session)
       (fboundp 'isolate-spawn) (isolate-main-p)
       (not (romacs-agent-events--supervised-p (romacs-agent-id session) (romacs-agent-host-socket session)))
       (not (romacs-agent-events-p session))
       (not (romacs-agent-events--db))
       ;; The writer asked to make it (by that look): its answer lets the
       ;; wait go.  One not readable for another reason is not waited for.
       romacs-agent-store--making))

(defun romacs-agent-events--store-readable ()
  "The writer answered the asking to make the store: what waited for it
goes on, in order, as it would have (`romacs-agent-events-then'); once
only, so a store still unreadable lets it go as unmovable."
  (let ((waiting romacs-agent-events--until-store) (romacs-agent-events--store-answered t))
    (setq romacs-agent-events--until-store nil)
    (pcase-dolist (`(,session . ,then) waiting)
      (with-demoted-errors "romacs-agent-events: after the store was made: %S"
        (romacs-agent-events-then session then)))))

(add-hook 'romacs-agent-store-readable-hook #'romacs-agent-events--store-readable)

(defun romacs-agent-events--movable-p (session)
  "Whether a worker VM can move SESSION to SQLite: the store on, worker VMs
here, and no supervisor writing its journal."
  (and romacs-agent-events-store (romacs-agent-dir session) (fboundp 'isolate-spawn) (isolate-main-p)
       (romacs-agent-events--db)
       (not (romacs-agent-events--supervised-p (romacs-agent-id session) (romacs-agent-host-socket session)))))

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
not loaded here, or loaded and waiting for it with no view holding
places in its journal."
  (let ((session (gethash id romacs-agent--sessions)))
    (or (null session)
        (and (gethash id romacs-agent-events--waiting)
             (not (run-hook-with-args-until-success 'romacs-agent-events-held-functions session))))))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-events-migrate-soon)

(provide 'romacs-agent-events)
;;; romacs-agent-events.el ends here

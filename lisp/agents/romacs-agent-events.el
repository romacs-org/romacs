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

(defconst romacs-agent-events--schema
  '("create table if not exists events (
       session text not null, seq integer not null, t real, kind text, record text not null)"
    ;; Not unique: a journal can hold two records of one seq (a note
    ;; written beside its supervisor's record), read in the order kept.
    "create index if not exists events_seq on events (session, seq)"
    "create index if not exists events_kind on events (session, kind, seq)"
    "create table if not exists imported (session text primary key, at real) without rowid"
    ;; Who moves a session's journal in, while it does (`romacs-agent-events--claim').
    "create table if not exists moving (session text primary key, pid integer, at real) without rowid")
  "The records' tables, made with the database: never dropped.")

(defvar romacs-agent-events--made nil "The database the tables were made in.")

(defun romacs-agent-events--db ()
  "The database, its records' tables made; nil without SQLite."
  (when-let* ((db (romacs-agent-store--open)))
    (unless (eq romacs-agent-events--made db)
      ;; Before any record is in it: a database made to give back its
      ;; freed pages while it is small, as it is when first opened here.
      (romacs-agent-events--compact-schema db)
      (dolist (statement romacs-agent-events--schema) (sqlite-execute db statement))
      (setq romacs-agent-events--made db))
    db))

(defvar romacs-agent-events--imported (make-hash-table :test #'equal)
  "Each session id known imported, by this process: once it is, it stays.")

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
  "Add RECORD (a table with its seq and time) to SESSION's records, waiting
a while for a database another writer holds (a compaction) rather than
losing it."
  (when-let* ((db (romacs-agent-events--db)))
    (let ((tries 0) (done nil))
      (while (not done)
        (condition-case err
            (progn (sqlite-execute db "insert into events values (?, ?, ?, ?, ?)"
                                   (list (romacs-agent-id session) (gethash "seq" record) (gethash "t" record)
                                         (romacs-agent-events--kind record) (romacs-agent--json record)))
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
until a worker moves it (`romacs-agent-events-migrate-soon')."
  (when (and romacs-agent-events-store (romacs-agent-dir session) (not (romacs-agent-events-p session))
             (romacs-agent-events--db)
             (not (let ((socket (romacs-agent-host-socket session))) (and socket (file-exists-p socket))))
             (<= (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (romacs-agent-dir session)))) 0)
                 (or romacs-agent-fold-bytes most-positive-fixnum)))
    (condition-case err (romacs-agent-events-import session)
      (error (message "romacs-agent-events: %s: %s" (romacs-agent-id session) (error-message-string err))))))


(defconst romacs-agent-events--chunk 500
  "The records an import writes a transaction: other writers (the editor's
own) wait for no more than that.")

(defun romacs-agent-events--write (db statement rows)
  "Run STATEMENT on DB for each of ROWS in one transaction."
  (sqlite-execute db "begin immediate")
  (condition-case err
      (progn (dolist (row rows) (sqlite-execute db statement row))
             (sqlite-execute db "commit"))
    (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err)))))

(defconst romacs-agent-events--line-head "\\`{\"\\([a-z]+\\)\":"
  "How a record's line begins as the supervisor writes it, its keys
sorted: its kind first.")

(defconst romacs-agent-events--line-tail ",\"seq\":\\([0-9]+\\),\"t\":\\([-+.0-9eE]+\\)}\\'"
  "How it ends: its seq and time last.  Matched against the line's last
characters alone: a pattern across a line of megabytes overflows.")

(defun romacs-agent-events--line-row (id line)
  "Session ID's journal LINE as its `events' row, the line kept as it is;
nil for a `thinking_tokens' counter, which no reader folds, or a line
that is no record.  A line not as the supervisor writes them is parsed."
  (cond ((and (string-search "thinking_tokens" line)
              (let ((record (romacs-agent-events--parse line)))
                (or (not (hash-table-p record)) (romacs-agent-events--counter-p record))))
         nil)
        ((let ((tail (substring line (max 0 (- (length line) 80)))))
           (and (string-match romacs-agent-events--line-tail tail)
                (let ((seq (string-to-number (match-string 1 tail))) (time (string-to-number (match-string 2 tail))))
                  (and (string-match romacs-agent-events--line-head line)
                       (list id seq time (match-string 1 line) line))))))
        (t (let ((record (romacs-agent-events--parse line)))
             (and (hash-table-p record) (integerp (gethash "seq" record))
                  (list id (gethash "seq" record) (gethash "t" record)
                        (romacs-agent-events--kind record) (romacs-agent--json record)))))))

(defconst romacs-agent-events--insert "insert into events values (?, ?, ?, ?, ?)"
  "How an import writes a record.")

(defun romacs-agent-events--claim (db id)
  "Claim moving session ID's journal in for this process: non-nil when
it is this process's to move, no other live process holding it."
  (sqlite-execute db "begin immediate")
  (condition-case err
      (let* ((pid (caar (sqlite-select db "select pid from moving where session = ?" (list id))))
             (free (or (null pid) (= pid (emacs-pid)) (null (process-attributes pid)))))
        (when free (sqlite-execute db "insert or replace into moving values (?, ?, ?)" (list id (emacs-pid) (float-time))))
        (sqlite-execute db "commit")
        free)
    (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err)))))

(defun romacs-agent-events--release (db id)
  "Let go of this process's claim on session ID."
  (ignore-errors (romacs-agent-events--write db "delete from moving where session = ? and pid = ?" (list (list id (emacs-pid))))))

(defun romacs-agent-events-import (session &optional keep)
  "Move SESSION's journal into SQLite, once: its records (but for
`thinking_tokens' counters, which no reader folds), `romacs-agent-events--chunk'
a transaction, then its `imported' row, which makes them its records;
then the journal goes to the backup directory, unless KEEP (a worker's
move: the editor backs it up when it knows no reader has it).

Only the process that claims the session moves it
\(`romacs-agent-events--claim'); another's live claim leaves it, nil.
The records are its journal's up to its size as the move began, its
last line too when whole and unended; the move is made theirs only if
the journal is still that size and no supervisor has started, else it
is undone and the session stays in its journal.  A move cut short,
what it wrote, is removed by the next.  Non-nil when SESSION is
imported."
  (or (romacs-agent-events-p session)
      (when-let* ((db (romacs-agent-events--db))
                  (id (romacs-agent-id session))
                  ((romacs-agent-events--claim db id)))
        (unwind-protect
            (let* ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session)))
                   (size (or (file-attribute-size (file-attributes journal)) 0))
                   (count 0) (rows nil) (offset 0))
              (cl-flet ((add (row)
                          (push row rows)
                          (cl-incf count)
                          (when (>= (length rows) romacs-agent-events--chunk)
                            (romacs-agent-events--write db romacs-agent-events--insert (nreverse rows))
                            (setq rows nil))))
                (romacs-agent-events--write db "delete from events where session = ?" (list (list id)))
                (condition-case err
                    (progn
                      ;; A slice of lines at a time, each kept as it is: only
                      ;; one not as the supervisor writes them is parsed.
                      (let (slice)
                        (while (setq slice (and offset (< offset size) (romacs-agent--journal-lines journal offset size)))
                          (dolist (line (car slice))
                            (when-let* ((row (romacs-agent-events--line-row id line))) (add row)))
                          (setq offset (cdr slice))))
                      ;; Its last line, unended: a whole record, or no move.
                      (when (and offset (< offset size))
                        (let ((rest (with-temp-buffer
                                      (set-buffer-multibyte nil)
                                      (insert-file-contents-literally journal nil offset size)
                                      (string-trim (decode-coding-string (buffer-string) 'utf-8)))))
                          (unless (string-empty-p rest)
                            (unless (hash-table-p (romacs-agent-events--parse rest))
                              (error "Importing %s: its last line is cut short" id))
                            (when-let* ((row (romacs-agent-events--line-row id rest))) (add row)))))
                      (when rows (romacs-agent-events--write db romacs-agent-events--insert (nreverse rows)))
                      (sqlite-execute db "begin immediate")
                      ;; Checked holding the write lock: nothing more came, and
                      ;; no supervisor writes to the journal now.
                      (unless (eql (or (file-attribute-size (file-attributes journal)) 0) size)
                        (error "Importing %s: its journal grew meanwhile" id))
                      (when (file-exists-p (romacs-agent--host-socket id))
                        (error "Importing %s: its supervisor started meanwhile" id))
                      (unless (= count (or (caar (sqlite-select db "select count(*) from events where session = ?" (list id))) 0))
                        (error "Importing %s: %d records read, another count written" id count))
                      (sqlite-execute db "insert into imported values (?, ?)" (list id (float-time)))
                      (sqlite-execute db "commit"))
                  (error (ignore-errors (sqlite-execute db "rollback"))
                         ;; Its own, claimed: no other process wrote them.
                         (ignore-errors (romacs-agent-events--write db "delete from events where session = ?" (list (list id))))
                         (signal (car err) (cdr err)))))
              (puthash id t romacs-agent-events--imported)
              (unless keep (romacs-agent-events--back-up session))
              t)
          (romacs-agent-events--release db id)))))

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
                     (when (file-exists-p (romacs-agent--host-socket id))
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
    (cl-remove-if (lambda (id) (file-exists-p (romacs-agent--host-socket id)))
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
  "`romacs-agent-events-compact-all' in a batch process, its count printed."
  (princ (format "romacs-agent-events: compacted, %d records removed\n" (romacs-agent-events-compact-all))))

;;; Migration, in worker VMs

;; With the store on, every session still in a journal moves to SQLite:
;; after a start's restore, once the editor is idle, a few sessions a job
;; go to worker VMs, `romacs-agent-events-migrate-jobs' jobs at once (one
;; database takes one writer at a time: more jobs only wait on it).  A job imports
;; each (its journal to the backup directory: Romacs's own journals only,
;; never a harness's transcripts), compacts it and fills its turn rows,
;; each in short transactions on its own connection.  The editor only
;; hears which are done.  A session loaded here, or whose supervisor runs,
;; is left for its next launch or the next start.

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

(defconst romacs-agent-events--worker-busy 60000
  "The milliseconds a worker's writes wait for the database's lock: a
worker waits for the editor, never the editor for long on a worker.")

(defun romacs-agent-events-migrate-soon ()
  "Move every session still in a journal to SQLite, in worker VMs, once the
editor is idle: with the store on, in an editor whose worker VMs run."
  (when (and romacs-agent-events-store (not noninteractive) (fboundp 'isolate-spawn) (isolate-main-p)
             (romacs-agent-events--db))
    (run-with-idle-timer 5 nil #'romacs-agent-events--migrate-start)))

(defun romacs-agent-events--journaled ()
  "The directories of the recorded sessions still in a journal, not
imported, none of them loaded here, nor run by a supervisor."
  (let ((imported (make-hash-table :test #'equal)))
    (dolist (row (sqlite-select (romacs-agent-events--db) "select session from imported")) (puthash (car row) t imported))
    (cl-remove-if-not
     (lambda (dir)
       (let ((id (file-name-nondirectory dir)))
         (and (not (gethash id imported)) (not (gethash id romacs-agent--sessions))
              (not (gethash id romacs-agent-events--migrating))
              (file-exists-p (expand-file-name "journal.jsonl" dir))
              (not (file-exists-p (romacs-agent--host-socket id))))))
     (and (file-directory-p (romacs-agent--state-root))
          (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))))

(defun romacs-agent-events--back-up-moved (ids)
  "Move to the backup directory the journals of the sessions IDS names
that are imported, none of them loaded here, nor run by a supervisor:
no reader has them.  A loaded one's goes at a later start."
  (dolist (id ids)
    (let ((dir (expand-file-name id (romacs-agent--state-root))))
      (when (and (not (gethash id romacs-agent--sessions))
                 (not (file-exists-p (romacs-agent--host-socket id)))
                 (file-exists-p (expand-file-name "journal.jsonl" dir)))
        (with-demoted-errors "romacs-agent-events: backing up: %S"
          (romacs-agent-events--back-up (romacs-agent--make :id id :dir dir)))))))

(defun romacs-agent-events--migrate-start ()
  "Back up the journals of sessions moved before, queue every session
still in a journal, and start the jobs."
  (romacs-agent-events--back-up-moved
   (mapcar #'car (sqlite-select (romacs-agent-events--db) "select session from imported")))
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

(defun romacs-agent-events--migrate-next ()
  "Give the next waiting sessions to a worker VM when fewer than
`romacs-agent-events-migrate-jobs' jobs run, a job a turn of the command
loop: a worker's start is the editor's time."
  (when (and romacs-agent-events--migrate-queue
             (< romacs-agent-events--migrate-running (max 1 romacs-agent-events-migrate-jobs)))
    (let ((batch nil) (bytes 0))
      (while (and romacs-agent-events--migrate-queue (< (length batch) (car romacs-agent-events--migrate-batch))
                  (let ((size (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (car romacs-agent-events--migrate-queue))))
                                  0)))
                    (when (or (null batch) (<= (+ bytes size) (cdr romacs-agent-events--migrate-batch)))
                      (cl-incf bytes size))))
        (let* ((dir (pop romacs-agent-events--migrate-queue)) (id (file-name-nondirectory dir)))
          ;; Loaded here since it was queued: left for its next launch.
          (unless (gethash id romacs-agent--sessions)
            (puthash id t romacs-agent-events--migrating)
            (push dir batch))))
      (when batch
        (setq batch (nreverse batch))
        (cl-incf romacs-agent-events--migrate-running)
        (let ((future (condition-case err
                          (isolate-spawn 'romacs-agent-events--migrate-job batch
                                         :name "romacs-agent migrate"
                                         :require '(romacs-agent romacs-agent-claude romacs-agent-codex romacs-agent-events
                                                                 romacs-agent-turns cl-extra help-mode)
                                         :bindings (romacs-agent--fold-bindings))
                        (error (list 'error err)))))
          (if (eq (car-safe future) 'error) (romacs-agent-events--migrated batch future)
            (isolate-on-done future (lambda (done) (romacs-agent-events--migrated batch done))))))
      (run-at-time 0 nil #'romacs-agent-events--migrate-next))))

(defun romacs-agent-events--migrate-job (dirs)
  "In a worker VM: move the sessions recorded in DIRS to SQLite, compact
them and fill their turn rows.  The ids moved."
  (when-let* ((db (romacs-agent-events--db)))
    (sqlite-pragma db (format "busy_timeout = %d" romacs-agent-events--worker-busy))
    (delq nil (mapcar #'romacs-agent-events--migrate-one dirs))))

(defun romacs-agent-events--migrate-one (dir)
  "Move the session recorded in DIR to SQLite; its id, or nil when it is
left (its supervisor runs, it failed)."
  (let ((romacs-agent--sessions (make-hash-table :test #'equal))
        (romacs-agent-event-functions nil))
    (condition-case err
        (when-let* ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir)))
                    (session (romacs-agent--of-manifest dir manifest))
                    ((not (file-exists-p (romacs-agent--host-socket (romacs-agent-id session))))))
          (puthash (romacs-agent-id session) session romacs-agent--sessions)
          ;; Its journal kept: the editor moves it once no reader has it.
          (when (romacs-agent-events-import session t)
            (romacs-agent-events-compact (romacs-agent-id session))
            (when (fboundp 'romacs-agent-turns-fill) (romacs-agent-turns-fill session))
            (romacs-agent-id session)))
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
    (dolist (id moved) (puthash id t romacs-agent-events--imported))
    (romacs-agent-events--back-up-moved moved)
    (when moved (run-hook-with-args 'romacs-agent-events-migrated-functions moved)))
  (romacs-agent-events--migrate-next))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-events-migrate-soon)

(provide 'romacs-agent-events)
;;; romacs-agent-events.el ends here

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
    "create table if not exists imported (session text primary key, at real) without rowid")
  "The records' tables, made with the database: never dropped.")

(defvar romacs-agent-events--made nil "The database the tables were made in.")

(defun romacs-agent-events--db ()
  "The database, its records' tables made; nil without SQLite."
  (when-let* ((db (romacs-agent-store--open)))
    (unless (eq romacs-agent-events--made db)
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

;;; Writing, with no supervisor running

(defun romacs-agent-events-append (session record)
  "Add RECORD (a table with its seq and time) to SESSION's records."
  (when-let* ((db (romacs-agent-events--db)))
    (sqlite-execute db "insert into events values (?, ?, ?, ?, ?)"
                    (list (romacs-agent-id session) (gethash "seq" record) (gethash "t" record)
                          (romacs-agent-events--kind record) (romacs-agent--json record)))))

;;; Importing a journal

(defcustom romacs-agent-events-store nil
  "Whether sessions' records move to SQLite: each as a supervisor is
started for it or Romacs adds to its records.  Off, journals are written
as before; a session already imported is read from SQLite either way."
  :type 'boolean :group 'romacs-agent)

(defun romacs-agent-events-own (session)
  "Import SESSION before anything more is added to its records, when
`romacs-agent-events-store' is on and no supervisor (an earlier build's,
writing its journal) runs for it."
  (when (and romacs-agent-events-store (romacs-agent-dir session) (not (romacs-agent-events-p session))
             (romacs-agent-events--db)
             (not (let ((socket (romacs-agent-host-socket session))) (and socket (file-exists-p socket)))))
    (condition-case err (romacs-agent-events-import session)
      (error (message "romacs-agent-events: %s: %s" (romacs-agent-id session) (error-message-string err))))))


(defun romacs-agent-events-import (session)
  "Move SESSION's journal into SQLite, once: its records (but for
`thinking_tokens' counters, which no reader folds), each checked as
written, and its `imported' row, in one transaction; then the journal
goes to the backup directory.  Non-nil when SESSION is imported."
  (or (romacs-agent-events-p session)
      (when-let* ((db (romacs-agent-events--db)))
        (let* ((id (romacs-agent-id session))
               (journal (expand-file-name "journal.jsonl" (romacs-agent-dir session)))
               (count 0) (ok nil))
          (sqlite-execute db "begin immediate")
          (unwind-protect
              (progn
                (when (file-readable-p journal)
                  (romacs-agent--read-journal
                   session
                   (lambda (record)
                     (unless (romacs-agent-events--counter-p record)
                       (sqlite-execute db "insert into events values (?, ?, ?, ?, ?)"
                                       (list id (gethash "seq" record) (gethash "t" record)
                                             (romacs-agent-events--kind record) (romacs-agent--json record)))
                       (cl-incf count)))))
                (unless (= count (or (caar (sqlite-select db "select count(*) from events where session = ?" (list id))) 0))
                  (error "Importing %s: %d records read, another count written" id count))
                (sqlite-execute db "insert into imported values (?, ?)" (list id (float-time)))
                (setq ok t))
            (sqlite-execute db (if ok "commit" "rollback")))
          (puthash id t romacs-agent-events--imported)
          (romacs-agent-events--back-up session)
          t))))

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

(provide 'romacs-agent-events)
;;; romacs-agent-events.el ends here

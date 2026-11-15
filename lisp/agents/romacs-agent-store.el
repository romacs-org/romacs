;;; romacs-agent-store.el --- One table of the agent sessions, for listing them  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; sessions.sqlite, in the agents' state root, has one row a session: what
;; a list of the sessions shows (its harness, provider and model, title,
;; directory, parent, state, last activity, cost) and the path of the
;; harness's own transcript.  It is a cache: made again from the sessions'
;; records whenever it is gone, deleting it loses nothing but speed.
;; Each row is replaced in place, one statement at a time, in WAL mode.
;;
;; A loaded session's row is written each time its state is kept
;; (`romacs-agent--keep', through `romacs-agent-kept-functions').  A
;; session no process has loaded gets its row from its manifest and kept
;; state (`romacs-agent-store-fill'), no journal read: a batch process
;; does it once (`romacs-agent-store-fill-soon').

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)

(declare-function romacs-agent-history--get "romacs-agent-history" (harness key))

(defconst romacs-agent-store--columns
  '(id dir harness provider model title cwd parent state last_time cost transcript
       input_seq done_seq seen_seq settled_seq)
  "The table's columns, in order.")

(defconst romacs-agent-store--schema
  '("create table if not exists sessions (
       id text primary key, dir text not null, harness text, provider text, model text,
       title text, cwd text, parent text, state text, last_time real, cost real, transcript text,
       input_seq integer, done_seq integer, seen_seq integer, settled_seq integer)"
    "create index if not exists sessions_state on sessions (state)"
    "create index if not exists sessions_recent on sessions (last_time desc)"
    "create index if not exists sessions_cwd on sessions (cwd)"
    "create index if not exists sessions_parent on sessions (parent)")
  "The table and its indexes, made when the database is opened.")

(defconst romacs-agent-store--version 2
  "The table's form (`user_version'): a database of another is made again,
as a cache is.")

(defvar romacs-agent-store--db nil
  "The open database, as (FILE . DB), or nil.")

(defun romacs-agent-store--file ()
  (expand-file-name "sessions.sqlite" (romacs-agent--state-root)))

(defun romacs-agent-store--open ()
  "The database of the current state root, opened and made once; nil when
this build has no SQLite or it cannot be opened."
  (let ((file (romacs-agent-store--file)))
    (if (equal (car romacs-agent-store--db) file) (cdr romacs-agent-store--db)
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (setq romacs-agent-store--db nil)
      (when (and (fboundp 'sqlite-available-p) (sqlite-available-p)
                 (file-directory-p (file-name-directory file)))
        (condition-case err
            (let ((db (sqlite-open file)))
              (sqlite-pragma db "journal_mode = WAL")
              (sqlite-pragma db "busy_timeout = 200")
              (unless (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version)
                (sqlite-execute db "drop table if exists sessions")
                (sqlite-pragma db (format "user_version = %d" romacs-agent-store--version)))
              (dolist (statement romacs-agent-store--schema) (sqlite-execute db statement))
              (set-file-modes file #o600)
              (setq romacs-agent-store--db (cons file db))
              db)
          (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))))

(defvar romacs-agent-store--transcripts nil
  "While a fill runs, each harness home's transcripts, listed once: a table
from (HARNESS . HOME) to a table from conversation id to path.")

(defun romacs-agent-store--listed (harness home)
  "HOME's transcripts of HARNESS by conversation id, listed once a fill."
  (let ((key (cons harness home)))
    (or (gethash key romacs-agent-store--transcripts)
        (puthash key (let ((table (make-hash-table :test #'equal)))
                       (pcase-dolist (`(,path . ,_) (ignore-errors (funcall (romacs-agent-history--get harness :files)
                                                                           home most-positive-fixnum)))
                         ;; Claude's file is named by the id; Codex's rollout ends with it.
                         (when (string-match "\\([0-9a-f]\\{8\\}-[0-9a-f-]\\{27\\}\\)\\.jsonl\\'" path)
                           (puthash (match-string 1 path) path table)))
                       table)
                 romacs-agent-store--transcripts))))

(defun romacs-agent-store--transcript (harness config-dir native)
  "The path of HARNESS's own transcript of conversation NATIVE, kept in
CONFIG-DIR (nil: its default), or nil."
  (when (stringp native)
    (require 'romacs-agent-history)
    (let ((home (romacs-agent--home harness config-dir)))
      (ignore-errors
        (if romacs-agent-store--transcripts
            (gethash native (romacs-agent-store--listed harness home))
          (funcall (romacs-agent-history--get harness :find) home native))))))

(defun romacs-agent-store-write (row &optional absent)
  "Put ROW, a plist of the columns' values (`romacs-agent-store--columns',
as keywords), in place of its session's, or only when it has none with
ABSENT; non-nil when it was written, quietly nil when the database cannot
be written."
  (when-let* ((db (romacs-agent-store--open)))
    (condition-case err
        (> (sqlite-execute db (format "insert or %s into sessions values (%s)" (if absent "ignore" "replace")
                                      (mapconcat (lambda (_) "?") romacs-agent-store--columns ","))
                           (mapcar (lambda (column) (plist-get row (intern (format ":%s" column))))
                                   romacs-agent-store--columns))
           0)
      (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))

(defun romacs-agent-store-put (session &optional state again)
  "Write SESSION's row, its state STATE when given (as its records left it);
one that cannot be written now (the database busy) is tried once more a
moment later, unless AGAIN says this is that try."
  (unless (or (romacs-agent-store--put session state) again (not (romacs-agent-store--open)))
    (run-with-idle-timer 1 nil #'romacs-agent-store-put session state t)))

(defun romacs-agent-store--put (session state)
  "Write SESSION's row, its state STATE when given; non-nil when written."
  (romacs-agent-store-write
   (list :id (romacs-agent-id session) :dir (romacs-agent-dir session)
         :harness (symbol-name (romacs-agent-harness session)) :provider (romacs-agent-provider session)
         :model (romacs-agent-model session) :title (romacs-agent-title session) :cwd (romacs-agent-cwd session)
         :parent (plist-get (romacs-agent-options session) :parent)
         :state (symbol-name (or state (romacs-agent-state session) 'detached))
         :last_time (romacs-agent-last-time session) :cost (romacs-agent-cost session)
         :transcript (romacs-agent-store--transcript (romacs-agent-harness session) (romacs-agent-config-dir session)
                                                     (romacs-agent-harness-session session))
         :input_seq (romacs-agent-input-seq session) :done_seq (romacs-agent-done-seq session)
         :seen_seq (romacs-agent-seen-seq session) :settled_seq (romacs-agent-settled-seq session))))

;;; Filling it

(defun romacs-agent-store--last-seq (journal)
  "The seq of JOURNAL's last record, read from its last bytes alone, or nil."
  (let ((size (file-attribute-size (file-attributes journal))))
    (when (and size (> size 0))
      (with-temp-buffer
        (set-buffer-multibyte nil)
        (insert-file-contents-literally journal nil (max 0 (- size 65536)) size)
        (goto-char (point-max))
        ;; A record's line ends with its seq and time, or starts with its seq.
        (and (or (re-search-backward "\"seq\":\\([0-9]+\\),\"t\":[-+.0-9eE]+}$" nil t)
                 (re-search-backward "^{\"seq\":\\([0-9]+\\)," nil t))
             (string-to-number (match-string 1)))))))

(defun romacs-agent-store--recorded-row (dir)
  "The row of the session recorded in DIR, from its manifest and its kept
state (`romacs-agent--kept-text'); no journal is read.  Nil with no
manifest."
  (when-let* ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
    (let* ((kept (let ((file (expand-file-name "state.eld" dir)))
                   (and (file-readable-p file)
                        (with-temp-buffer
                          (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
                          (ignore-errors (plist-get (read (current-buffer)) :fields))))))
           (field (lambda (slot) (cdr (assq slot kept))))
           (options (gethash "options" manifest))
           (harness (gethash "harness" manifest))
           (transcript (romacs-agent-store--transcript (intern harness) (gethash "configDir" manifest)
                                                       (gethash "harnessSession" manifest)))
           (journal (expand-file-name "journal.jsonl" dir)))
      (list :id (gethash "id" manifest) :dir dir :harness harness
            :provider (or (gethash "provider" manifest) harness)
            :model (or (funcall field 'model) (and (hash-table-p options) (romacs-agent--value (gethash "model" options))))
            :title (or (funcall field 'title) (gethash "title" manifest)) :cwd (gethash "cwd" manifest)
            :parent (and (hash-table-p options) (romacs-agent--value (gethash "parent" options)))
            ;; A turn under way when it was kept, with no supervisor now
            ;; to go on with it: detached.
            :state (let ((state (funcall field 'state)))
                     (cond ((file-exists-p (expand-file-name "cancelled" dir)) "ended")
                           ((and state (not (memq state '(running starting waiting-approval))))
                            (symbol-name state))
                           ((and state (file-exists-p (romacs-agent--host-socket (gethash "id" manifest))))
                            (symbol-name state))
                           (t "detached")))
            :last_time (or (funcall field 'last-time)
                           (let ((file (or transcript journal)))
                             (and (file-exists-p file)
                                  (float-time (file-attribute-modification-time (file-attributes file))))))
            :cost (funcall field 'cost) :transcript transcript
            ;; With no kept state, its activity is at most its last record:
            ;; settled only when settled since that.
            :input_seq (if kept (funcall field 'input-seq) (romacs-agent-store--last-seq journal))
            :done_seq (if kept (funcall field 'done-seq) (romacs-agent-store--last-seq journal))
            :seen_seq (let ((seen (gethash "seenSeq" manifest))) (and (integerp seen) seen))
            :settled_seq (let ((settled (gethash "settledSeq" manifest))) (and (integerp settled) settled))))))

(defun romacs-agent-store--missing ()
  "The directories of recorded sessions that have no row."
  (let ((rows (make-hash-table :test #'equal)))
    (when-let* ((db (romacs-agent-store--open)))
      (dolist (row (sqlite-select db "select id from sessions")) (puthash (car row) t rows)))
    (cl-remove-if (lambda (dir) (gethash (file-name-nondirectory dir) rows))
                  (and (file-directory-p (romacs-agent--state-root))
                       (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))))

(defun romacs-agent-store-fill ()
  "Write the row of each recorded session that has none, from its records
(`romacs-agent-store--recorded-row'); return how many were written."
  (let ((count 0) (db (romacs-agent-store--open))
        (romacs-agent-store--transcripts (make-hash-table :test #'equal)))
    ;; Written only where no row is (a loaded session's own, written
    ;; meanwhile, is newer), fifty rows a short transaction that takes the
    ;; write lock as it begins.
    (when db
      (dolist (dirs (seq-partition (romacs-agent-store--missing) 50))
        ;; Made first, the files read: the lock is held for the writes alone.
        (let ((rows (delq nil (mapcar (lambda (dir)
                                        (condition-case err (romacs-agent-store--recorded-row dir)
                                          (error (message "romacs-agent-store: %s: %s" dir (error-message-string err)) nil)))
                                      dirs))))
          (when (and rows (ignore-errors (sqlite-execute db "begin immediate") t))
            (unwind-protect
                (dolist (row rows) (when (romacs-agent-store-write row t) (cl-incf count)))
              (ignore-errors (sqlite-execute db "commit"))))))
      count)))

(defun romacs-agent-store-fill-batch ()
  "`romacs-agent-store-fill' in a batch process, its count printed."
  (princ (format "romacs-agent-store: filled %d\n" (romacs-agent-store-fill))))

(defvar romacs-agent-store--filling nil "The batch process filling the database, while it runs.")

(defvar romacs-agent-store--filled nil "Non-nil once a batch process has filled the database.")

(defvar romacs-agent-store-filled-hook nil
  "Run once a batch process has filled the database.")

(defun romacs-agent-store-fill-soon ()
  "Fill the database in a batch process of this build, off the editor, when
no fill runs; `romacs-agent-store-filled-hook' runs once it ends."
  (when (and (not noninteractive) (not (process-live-p romacs-agent-store--filling)) (romacs-agent-store--open))
    (setq romacs-agent-store--filling
          (make-process
           :name "romacs-agent-store-fill" :noquery t :connection-type 'pipe
           :buffer (generate-new-buffer " *romacs-agent-store-fill*" t)
           :command (list (expand-file-name invocation-name invocation-directory) "--batch"
                          "-l" "romacs-agent-store" "-f" "romacs-agent-store-fill-batch")
           :sentinel (lambda (process _event)
                       (unless (process-live-p process)
                         (when (buffer-live-p (process-buffer process)) (kill-buffer (process-buffer process)))
                         (setq romacs-agent-store--filled t)
                         (run-hooks 'romacs-agent-store-filled-hook)))))))

;;; Listing

(defun romacs-agent-store-listed-p (dir)
  "Whether the session recorded in DIR is listed from its row: it has one,
or, as an editor's start restores (`romacs-agent-restore-soon'), one is
coming, a batch fill running (`romacs-agent-store-fill-soon', started
here the first time a row is missing).  A start with no rows then loads
only the live and waiting sessions, the others shown as their rows land;
one still missing a row after a fill is loaded as it is."
  (when-let* ((db (romacs-agent-store--open)))
    (or (and (sqlite-select db "select 1 from sessions where id = ?" (list (file-name-nondirectory dir))) t)
        (and romacs-agent--restoring (not noninteractive)
             (progn (unless (or romacs-agent-store--filled (process-live-p romacs-agent-store--filling))
                      (with-demoted-errors "romacs-agent-store: filling: %S" (romacs-agent-store-fill-soon)))
                    (process-live-p romacs-agent-store--filling))))))

(defun romacs-agent-store-unloaded ()
  "The sessions that have a row but are not loaded, newest first, each a
session record made from its row alone: to show, not to act on (acting
on one loads it, `romacs-agent-find')."
  (when-let* ((db (romacs-agent-store--open)))
    (delq nil
          (mapcar (lambda (row)
                    (pcase-let ((`(,id ,dir ,harness ,provider ,model ,title ,cwd ,parent ,state ,last ,cost
                                   ,input ,done ,seen ,settled)
                                 row))
                      (unless (romacs-agent-get id)
                        (romacs-agent--make :id id :dir dir :harness (intern harness) :provider provider :model model
                                            :title title :cwd cwd :options (and parent (list :parent parent))
                                            :state (intern (or state "detached")) :last-time last :cost cost
                                            :input-seq input :done-seq (or done 0) :seen-seq seen :settled-seq settled))))
                  (sqlite-select db (concat "select id, dir, harness, provider, model, title, cwd, parent, state,"
                                            " last_time, cost, input_seq, done_seq, seen_seq, settled_seq"
                                            " from sessions order by last_time desc"))))))

(add-hook 'romacs-agent-kept-functions #'romacs-agent-store-put)
;; Restored: the rows of sessions recorded with none are made, off the editor.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-store-fill-soon)
(setq romacs-agent-listed-p #'romacs-agent-store-listed-p)

(provide 'romacs-agent-store)
;;; romacs-agent-store.el ends here

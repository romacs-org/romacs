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
;;
;; It also keeps which projects the board shows folded (`board_folds'),
;; the one thing in it no record holds: a table of its own, kept when the
;; sessions' table is made again; lost with the file, every project shows.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-writer)

(declare-function romacs-agent-history--get "romacs-agent-history" (harness key))

(defconst romacs-agent-store--columns
  '(id dir harness provider model title cwd parent state last_time cost transcript
       input_seq done_seq seen_seq settled_seq attention
       effort plan permission_mode context_used context_window resume_time seq request)
  "The table's columns, in order.")

(defconst romacs-agent-store--schema
  '("create table if not exists sessions (
       id text primary key, dir text not null, harness text, provider text, model text,
       title text, cwd text, parent text, state text, last_time real, cost real, transcript text,
       input_seq integer, done_seq integer, seen_seq integer, settled_seq integer,
       attention integer, effort text, plan integer, permission_mode text, context_used integer,
       context_window integer, resume_time real, seq integer, request text)"
    "create index if not exists sessions_state on sessions (state)"
    "create index if not exists sessions_recent on sessions (last_time desc)"
    "create index if not exists sessions_cwd on sessions (cwd)"
    "create index if not exists sessions_parent on sessions (parent, request)"
    "create table if not exists board_folds (project text primary key)"
    "create table if not exists queued (session text not null, id text not null, position real not null,
       item text not null, primary key (session, id))")
  "The table and its indexes, made when the database is opened.")

(defconst romacs-agent-store--version 5
  "The table's form (`user_version'): a database of another has its table
made in this form, its rows kept (`romacs-agent-store--reform').")

(defvar romacs-agent-store--db nil
  "The open database, as ((FILE . QUEUED) . DB), or nil: QUEUED when it
was opened to read, its writes queued to the writer.")

(defun romacs-agent-store--file ()
  (expand-file-name "sessions.sqlite" (romacs-agent--state-root)))

(defvar romacs-agent-store--readonly nil
  "Whether this process's connection only reads: the editor's, whose
writes go to the writer (`romacs-agent-writer-queued-p').  Its tables are
the writer's to make.")

(defun romacs-agent-store-readonly-p ()
  "Whether this process only reads the store (`romacs-agent-store--readonly')."
  romacs-agent-store--readonly)

(defun romacs-agent-store--open ()
  "The database of the current state root, opened and made once; nil when
this build has no SQLite or it cannot be opened.  In the editor, whose
writes are queued to the writer, it is opened to read, once the writer
has made it."
  (let* ((file (romacs-agent-store--file)) (queued (romacs-agent-writer-queued-p))
         ;; Opened as this process writes: one opened to write before its
         ;; writes were queued (early in a start) is opened again, to read.
         (key (cons file queued)))
    (if (equal (car romacs-agent-store--db) key) (cdr romacs-agent-store--db)
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (setq romacs-agent-store--db nil)
      (if queued (romacs-agent-store--open-to-read file) (romacs-agent-store--open-to-write file)))))

(defconst romacs-agent-store-patient 60000
  "The milliseconds a process off the editor waits for the store's lock:
the writer, a migration worker, a batch fill wait for each other rather
than lose a write.")

(defvar romacs-agent-store-patience 200
  "The milliseconds this process's connection waits for the store's lock,
from its first statement: short in the editor, which must not stall,
`romacs-agent-store-patient' off it.  A reader waits too: a WAL read is
refused a moment while another connection checkpoints or vacuums.")

(defun romacs-agent-store--open-to-read (file)
  "FILE opened to read, or nil while the writer has not made it as this
build reads it: not there yet, or of another version.  The writer is
asked to then, once (`romacs-agent-store--made').  A WAL snapshot read
never waits on a writer."
  (if (not (file-exists-p file))
      (progn (romacs-agent-store--make-soon) nil)
    (condition-case err
        (let ((db (sqlite-open file t)))
          (sqlite-pragma db (format "busy_timeout = %d" romacs-agent-store-patience))
          (if (not (and (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version)
                        ;; Every module's tables made: a store the writer is
                        ;; making this moment is not read yet.
                        (= (caar (sqlite-select db "select count(*) from sqlite_master where name in ('sessions', 'events', 'turns', 'turn_drawn', 'transcripts')"))
                           5)))
              (progn (sqlite-close db) (romacs-agent-store--make-soon) nil)
            (setq romacs-agent-store--readonly t)
            (setq romacs-agent-store--db (cons (cons file t) db))
            db))
      (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))

(defvar romacs-agent-store--making nil "Whether the writer is making the store.")

(defvar romacs-agent-store-readable-hook nil
  "Run in the editor once the writer has answered its asking to make the
store (`romacs-agent-store--make-soon'): what waited for the store to be
read here goes on.")

(defun romacs-agent-store--make-soon ()
  "Ask the writer to make the store, once until it has."
  (unless romacs-agent-store--making
    (setq romacs-agent-store--making t)
    (romacs-agent-write 'romacs-agent-store--made nil
                        (lambda (_answer)
                          (setq romacs-agent-store--making nil)
                          (run-hooks 'romacs-agent-store-readable-hook)))))

(defvar romacs-agent-store-made-functions nil
  "Functions that make the tables of the store's other modules, called by
the writer once it has opened it (`romacs-agent-store--made').")

(defun romacs-agent-store--writer-open ()
  "In a writer: the store opened, its directory made first.  The writer
makes the store, in a home where no agent has run yet as well: nothing
else may have made the state directory."
  (make-directory (file-name-directory (romacs-agent-store--file)) t)
  (romacs-agent-store--open))

(defun romacs-agent-store--made ()
  "In a writer: the store opened, every module's tables made."
  (when (romacs-agent-store--writer-open) (run-hooks 'romacs-agent-store-made-functions) t))

(defun romacs-agent-store--writer-start ()
  "The writer's start: the store opened, its tables made, its waits long,
its commits not synced one by one (`synchronous=NORMAL', as a supervisor's
are): what a crash may lose of them is the writer's queue's window."
  ;; Its connection waits long; the editor, which runs this when it ends
  ;; with no writer (`romacs-agent-writer--here'), keeps its own short.
  (when-let* ((db (let ((romacs-agent-store-patience romacs-agent-store-patient)) (romacs-agent-store--writer-open))))
    (sqlite-pragma db "synchronous = NORMAL")
    (romacs-agent-store--made)))

(add-hook 'romacs-agent-writer-start-functions #'romacs-agent-store--writer-start)
(defun romacs-agent-store--writer-bindings ()
  "What the writer starts with: the editor's agents' options, and the long
wait for the store's lock, every connection of its life."
  (cons (cons 'romacs-agent-store-patience romacs-agent-store-patient) (romacs-agent--fold-bindings)))

(setq romacs-agent-writer-bindings-function #'romacs-agent-store--writer-bindings)

(defun romacs-agent-store--reform (db)
  "Make DB's sessions table, of another version, in this one's form, every
row kept with the columns both forms have, in one transaction: a row lost
hides its session until a fill writes it again.  A row the new form
refuses (a column it needs, missing) is left out."
  (sqlite-execute db "begin immediate")
  (condition-case err
      (progn
        (when (sqlite-select db "select 1 from sqlite_master where type = 'table' and name = 'sessions'")
          (sqlite-execute db "drop table if exists sessions_reformed")
          (sqlite-execute db (string-replace "create table if not exists sessions ("
                                             "create table sessions_reformed ("
                                             (car romacs-agent-store--schema)))
          (let* ((new (mapcar #'cadr (sqlite-select db "pragma table_info(sessions_reformed)")))
                 (old (mapcar #'cadr (sqlite-select db "pragma table_info(sessions)")))
                 (both (string-join (seq-filter (lambda (column) (member column old)) new) ", ")))
            (sqlite-execute db (format "insert or ignore into sessions_reformed (%s) select %s from sessions" both both)))
          (sqlite-execute db "drop table sessions")
          (sqlite-execute db "alter table sessions_reformed rename to sessions"))
        (sqlite-pragma db (format "user_version = %d" romacs-agent-store--version))
        (sqlite-execute db "commit"))
    (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err)))))

(defun romacs-agent-store--open-to-write (file)
  "FILE opened to write and its tables made; nil when it cannot be."
  (setq romacs-agent-store--readonly nil)
  (when (and (fboundp 'sqlite-available-p) (sqlite-available-p)
             (file-directory-p (file-name-directory file)))
    (condition-case err
        (let ((db (sqlite-open file)))
          (sqlite-pragma db (format "busy_timeout = %d" romacs-agent-store-patience))
          (sqlite-pragma db "journal_mode = WAL")
          (unless (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version)
            (romacs-agent-store--reform db))
          (dolist (statement romacs-agent-store--schema) (sqlite-execute db statement))
          (set-file-modes file #o600)
          (setq romacs-agent-store--db (cons (cons file nil) db))
          db)
      ;; Off the editor a lock is waited out by the caller, never read
      ;; as no store.
      (sqlite-locked-error
       (if (>= romacs-agent-store-patience romacs-agent-store-patient) (signal (car err) (cdr err))
         (message "romacs-agent-store: %s" (error-message-string err))
         nil))
      (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))

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
ABSENT; non-nil when it was written or queued to the writer, quietly nil
when the database cannot be written."
  (romacs-agent-written-p
   (romacs-agent-write 'romacs-agent-store--write-values (list (romacs-agent-store--row-values row) absent)
                       nil (list 'row (plist-get row :id)))))

(defun romacs-agent-store--row-values (row)
  "ROW, a plist of the columns' values, as the columns' values in order."
  (mapcar (lambda (column) (plist-get row (intern (format ":%s" column)))) romacs-agent-store--columns))

(defun romacs-agent-store--write-values (values absent)
  "Put VALUES, a row's columns in order, in place of its session's, or only
when it has none with ABSENT; non-nil when it was written.  A failure
signals: the writer sees it."
  (let ((db (or (romacs-agent-store--open) (error "No agents store to write a row in"))))
    (> (sqlite-execute db (format "insert or %s into sessions values (%s)" (if absent "ignore" "replace")
                                  (mapconcat (lambda (_) "?") romacs-agent-store--columns ","))
                       values)
       0)))

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
         :seen_seq (romacs-agent-seen-seq session) :settled_seq (romacs-agent-settled-seq session)
         ;; Wanting the user or its parent: its last turn failed, or a task
         ;; holding ends its parent may not have had yet.
         :attention (if (or (romacs-agent-failed session)
                            (and (plist-get (romacs-agent-options session) :parent) (romacs-agent-ends session)))
                        1 0)
         ;; What its chat's header says of it, from its row alone.
         :effort (plist-get (romacs-agent-options session) :effort)
         :plan (if (plist-get (romacs-agent-options session) :plan) 1 0)
         :permission_mode (and (romacs-agent-permission-mode session) (format "%s" (romacs-agent-permission-mode session)))
         :context_used (romacs-agent-context-used session) :context_window (romacs-agent-context-window session)
         :resume_time (and (fboundp 'romacs-agent-resume-time) (romacs-agent-resume-time session))
         ;; The last record its kept state folded.
         :seq (romacs-agent-seq session)
         :request (plist-get (romacs-agent-options session) :request))))

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
    (let* ((state (let ((text (or (and (fboundp 'romacs-agent-turns-fold) (romacs-agent-turns-fold (gethash "id" manifest)))
                                  (let ((file (expand-file-name "state.eld" dir)))
                                    (and (file-readable-p file)
                                         (with-temp-buffer
                                           (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
                                           (buffer-string)))))))
                    (and text (ignore-errors (car (read-from-string text))))))
           (kept (plist-get state :fields))
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
            :request (and (hash-table-p options) (romacs-agent--value (gethash "request" options)))
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
            :settled_seq (let ((settled (gethash "settledSeq" manifest))) (and (integerp settled) settled))
            ;; Its last turn failed, or a task's ends for its parent, as
            ;; kept; unknown with no kept state, so wanted.
            :attention (if (or (funcall field 'failed)
                               (and (hash-table-p options) (romacs-agent--value (gethash "parent" options))
                                    (or (not kept) (funcall field 'ends))))
                           1 0)
            :effort (and (hash-table-p options) (romacs-agent--value (gethash "effort" options)))
            :plan (if (and (hash-table-p options) (eq (gethash "plan" options) t)) 1 0)
            :permission_mode (let ((mode (funcall field 'permission-mode))) (and mode (format "%s" mode)))
            :context_used (funcall field 'context-used) :context_window (funcall field 'context-window)
            :resume_time (let ((at (gethash "resumeAt" manifest))) (and (numberp at) at))
            :seq (let ((seq (plist-get state :seq))) (and (integerp seq) seq))))))

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
  (let ((count 0) (db nil) (missing nil)
        (romacs-agent-store--transcripts (make-hash-table :test #'equal)))
    ;; Opened and asked what is missing until the lock lets it, as a chunk
    ;; is written (`romacs-agent-store--fill-chunk').
    (while (condition-case nil
               (progn (setq db (romacs-agent-store--open) missing (and db (romacs-agent-store--missing))) nil)
             (sqlite-locked-error t)))
    ;; Written only where no row is (a loaded session's own, written
    ;; meanwhile, is newer), fifty rows a short transaction that takes the
    ;; write lock as it begins.
    (when db
      (dolist (dirs (seq-partition missing 50))
        (cl-incf count (romacs-agent-store--fill-chunk db dirs)))
      count)))

(defun romacs-agent-store--fill-chunk (db dirs)
  "Write in DB the rows of the sessions recorded in DIRS, where none is;
how many were written.  The lock held by another past this connection's
wait, anywhere in it, it is all tried again until done: a row missing
hides its session from the board.  A session whose row cannot be made
otherwise is said and left out; a chunk that fails otherwise is said and
left to the next fill, the rows being a cache of records."
  (catch 'done
    (while t
      (condition-case err
          ;; Made first, the files read: the lock is held for the writes alone.
          (let ((rows (delq nil (mapcar (lambda (dir)
                                          (condition-case err (romacs-agent-store--recorded-row dir)
                                            (sqlite-locked-error (signal (car err) (cdr err)))
                                            (error (message "romacs-agent-store: %s: %s" dir (error-message-string err)) nil)))
                                        dirs)))
                (written 0))
            (when rows
              (sqlite-execute db "begin immediate")
              ;; Written here, in this transaction: a row that fails fails
              ;; the chunk.
              (dolist (row rows)
                (when (romacs-agent-store--write-values (romacs-agent-store--row-values row) t) (cl-incf written)))
              (sqlite-execute db "commit"))
            (throw 'done written))
        (sqlite-locked-error (ignore-errors (sqlite-execute db "rollback")))
        (error (ignore-errors (sqlite-execute db "rollback"))
               (message "romacs-agent-store: %d rows not written, left to the next fill: %s"
                        (length dirs) (error-message-string err))
               (throw 'done 0))))))

(defvar romacs-agent-events-store)

(declare-function romacs-agent--marks-reconcile "romacs-agent" ())
(declare-function romacs-agent--marked "romacs-agent" (&optional any))

(defun romacs-agent-store-fill-batch ()
  "`romacs-agent-store-fill' in a batch process, its count printed, and
every session's marks made what its files say
\(`romacs-agent--marks-reconcile'), each newly marked id printed for the
editor to load.  It moves no session's records to SQLite: the editor
alone does."
  ;; Every kind of what can wait on a session, its files asked.
  (require 'romacs-agent-watch)
  (let ((romacs-agent-events-store nil) (romacs-agent-store-patience romacs-agent-store-patient))
    (princ (format "romacs-agent-store: filled %d\n" (romacs-agent-store-fill)))
    ;; Still with no row: the editor loads them, as it does the marked.
    (dolist (dir (romacs-agent-store--missing))
      (princ (format "romacs-agent-store: missing %s\n" (file-name-nondirectory dir))))
    (dolist (id (romacs-agent--marks-reconcile))
      (princ (format "romacs-agent-store: marked %s\n" id)))
    ;; Its report whole: the editor takes it for all there is.
    (princ "romacs-agent-store: done\n")))

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
                         (romacs-agent-store--filled
                          (when (buffer-live-p (process-buffer process))
                            (prog1 (with-current-buffer (process-buffer process) (buffer-string))
                              (kill-buffer (process-buffer process))))
                          (process-exit-status process))
                         (run-hooks 'romacs-agent-store-filled-hook)))))))

(defun romacs-agent-store--filled (output &optional status)
  "A fill ended with STATUS (0 by default), saying OUTPUT: the sessions it
left with no row and those it newly marked as waited on are loaded, none
of the rows asked here, when it ended well and its report is whole;
else every session with no row and every marked one, asked here."
  (setq romacs-agent-store--filled t)
  (if (and output (eql (or status 0) 0) (string-match-p "^romacs-agent-store: done$" output))
      (romacs-agent-store--load-missing (romacs-agent-store--said-ids "missing\\|marked" output) t)
    ;; What it said lost or cut short: asked here, the rows missing and
    ;; every marked session, one it marked before it stopped among them.
    (romacs-agent-store--load-missing
     (append (mapcar #'file-name-nondirectory (romacs-agent-store--missing))
             ;; Its marks so far, known whole or not.
             (when-let* ((marked (romacs-agent--marked t))) (hash-table-keys marked)))
     t)))

(defun romacs-agent-store--said-ids (what output)
  "The ids a fill's OUTPUT names on its lines of WHAT (a regexp)."
  (let ((ids nil) (start 0) (line (format "^romacs-agent-store: \\(?:%s\\) \\([0-9a-f][^ \n]*\\)$" what)))
    (while (string-match line output start)
      (push (match-string 1 output) ids)
      (setq start (match-end 0)))
    ids))

(defun romacs-agent-store--marked-ids (output)
  "The ids a fill's OUTPUT says it newly marked."
  (romacs-agent-store--said-ids "marked" output))

(defun romacs-agent-store--load-missing (&optional ids said)
  "Load, a turn at a time as a start's restore does, each session IDS
names that this process does not know: as a fill says, those it left
with no row (a start left them to it, `romacs-agent-store-listed-p', and
with none one is shown only once loaded) and those it newly marked as
waited on (`romacs-agent--marks-reconcile').  Not SAID, every recorded
session with no row, asked here."
  (let ((idle (null romacs-agent--restore-pending)))
    (dolist (dir (if said (mapcar (lambda (id) (expand-file-name id (romacs-agent--state-root))) ids)
                   (romacs-agent-store--missing)))
      ;; One gone (a mark it left): nothing to load, nor to know as loaded.
      (unless (or (gethash (file-name-nondirectory dir) romacs-agent--sessions)
                  (not (file-directory-p dir))
                  (member dir romacs-agent--restore-pending))
        (setq romacs-agent--restore-pending (append romacs-agent--restore-pending (list dir)))))
    (when (and idle romacs-agent--restore-pending)
      ;; After a start's restore: known as it knows sessions once loaded.
      (unless romacs-agent--restoring (setq romacs-agent--late-loads t))
      (run-at-time 0 nil #'romacs-agent--restore-step))))

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

(defconst romacs-agent-store--quiet-states '("idle" "detached" "unstarted")
  "The states a session's row may be in for it to want nothing now.")

(declare-function romacs-agent-events--db "romacs-agent-events" ())

(defun romacs-agent-store-quiet-p (dir)
  "Whether the session recorded in DIR wants nothing now, as its row says:
idle (or detached, unstarted), so nothing asked of the user, and no
error nor anything owed to its parent (`romacs-agent-quiet-p'); and
nothing came since its state was kept: its records in SQLite go no
further than the seq that state folded.  One still in a journal is not
quiet: whether more came cannot be told there without reading it.
Quiet, the place its records were found to end at (the seq after the
kept one): what its supervisor writes from there is new."
  (when-let* (((fboundp 'romacs-agent-events--db))
              ;; Its records' tables, made with the database.
              (db (romacs-agent-events--db))
              (id (file-name-nondirectory dir))
              (row (car (sqlite-select db (concat "select state, attention, seq,"
                                                  " (select 1 from imported where imported.session = sessions.id),"
                                                  " (select max(seq) from events where events.session = sessions.id)"
                                                  " from sessions where id = ?")
                                       (list id)))))
    (pcase-let ((`(,state ,attention ,seq ,imported ,last) row))
      (and (member state romacs-agent-store--quiet-states) (eql attention 0)
           imported (integerp seq) (or (null last) (<= last seq))
           (1+ seq)))))

(defun romacs-agent-store-row-session (id)
  "The session ID names, made from its row alone (none of its kept state
read, nor its records): what a chat shows of it before it is used, its
header's facts too.  Nil with no row.  Acting on it loads it
\(`romacs-agent-load-session')."
  (when-let* ((db (romacs-agent-store--open))
              (row (car (sqlite-select db (concat "select dir, harness, provider, model, title, cwd, parent, state,"
                                                  " last_time, cost, input_seq, done_seq, seen_seq, settled_seq,"
                                                  " effort, plan, permission_mode, context_used, context_window, resume_time"
                                                  " from sessions where id = ?")
                                       (list id)))))
    (pcase-let ((`(,dir ,harness ,provider ,model ,title ,cwd ,parent ,state ,last ,cost ,input ,done ,seen ,settled
                   ,effort ,plan ,mode ,used ,window ,resume)
                 row))
      (romacs-agent--make :id id :dir dir :harness (intern harness) :provider provider :model model
                          :title title :cwd cwd
                          :options (append (and parent (list :parent parent)) (and effort (list :effort effort))
                                           (and (eql plan 1) (list :plan t)) (and resume (list :resume-at resume)))
                          :state (intern (or state "detached")) :last-time last :cost cost
                          :input-seq input :done-seq (or done 0) :seen-seq seen :settled-seq settled
                          :permission-mode mode :context-used used :context-window window))))

(defun romacs-agent-store-unloaded ()
  "The sessions that have a row but are not loaded, newest first, each a
session record made from its row alone: to show, not to act on (acting
on one loads it, `romacs-agent-get')."
  (when-let* ((db (romacs-agent-store--open)))
    (delq nil
          (mapcar (lambda (row)
                    (pcase-let ((`(,id ,dir ,harness ,provider ,model ,title ,cwd ,parent ,state ,last ,cost
                                   ,input ,done ,seen ,settled)
                                 row))
                      (unless (romacs-agent-loaded id)
                        (romacs-agent--make :id id :dir dir :harness (intern harness) :provider provider :model model
                                            :title title :cwd cwd :options (and parent (list :parent parent))
                                            :state (intern (or state "detached")) :last-time last :cost cost
                                            :input-seq input :done-seq (or done 0) :seen-seq seen :settled-seq settled))))
                  (sqlite-select db (concat "select id, dir, harness, provider, model, title, cwd, parent, state,"
                                            " last_time, cost, input_seq, done_seq, seen_seq, settled_seq"
                                            " from sessions order by last_time desc"))))))

;;; Messages waiting in a session's queue (romacs#287)

(defun romacs-agent-store--item-json (item)
  "ITEM, a queued message all data, as the JSON its row keeps."
  (json-serialize
   (romacs-mcp--object "text" (romacs-agent-item-text item)
                       "images" (vconcat (romacs-agent-item-images item))
                       "messages" (vconcat (mapcar #'romacs-agent--message-json (romacs-agent-item-messages item)))
                       "own" (or (romacs-agent-item-own item) :null))))

(defun romacs-agent-store--item-of (id position json)
  "The queued message a row keeps: ID, POSITION and its JSON.  A steer
comes back as a turn of its own: the turn it steered is no next run's."
  (let ((object (json-parse-string json)))
    (romacs-agent-item (gethash "text" object)
                       :images (append (gethash "images" object) nil)
                       :messages (mapcar #'romacs-agent--message-of (gethash "messages" object))
                       :kind 'next :own (romacs-agent--value (gethash "own" object))
                       :id id :position position :kept t)))

(defun romacs-agent-store-queued-put (id item)
  "Keep ITEM, waiting in the queue of the session ID names, by the writer."
  (romacs-agent-write 'romacs-agent-store--queued-row
                      (list id (romacs-agent-item-id item) (romacs-agent-item-position item)
                            (romacs-agent-store--item-json item))
                      nil (list 'queued id (romacs-agent-item-id item))))

(defun romacs-agent-store-queued-delete (id item-id)
  "Let go of the kept message ITEM-ID of the session ID names, by the writer."
  (romacs-agent-write 'romacs-agent-store--queued-row (list id item-id nil nil)
                      nil (list 'queued id item-id)))

(defun romacs-agent-store--queued-row (id item-id position json)
  "Keep the message ITEM-ID of session ID at POSITION as JSON, or with
none let it go, here.  A failure signals."
  (let ((db (or (romacs-agent-store--open) (error "No agents store to keep a queued message in"))))
    (if json
        (sqlite-execute db "insert or replace into queued values (?, ?, ?, ?)" (list id item-id position json))
      (sqlite-execute db "delete from queued where session = ? and id = ?" (list id item-id)))
    t))

(defun romacs-agent-store-queued (id)
  "The messages kept waiting for the session ID names, in order."
  (when-let* ((db (romacs-agent-store--open)))
    (condition-case err
        (mapcar (lambda (row) (apply #'romacs-agent-store--item-of row))
                (sqlite-select db "select id, position, item from queued where session = ? order by position"
                               (list id)))
      (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))

(defun romacs-agent-store--requeue (session _manifest)
  "SESSION, loaded: what was kept waiting in its queue waits there again,
sent once its harness may take it."
  (unless (romacs-agent-queue session)
    (dolist (item (romacs-agent-store-queued (romacs-agent-id session)))
      (romacs-agent--enqueue session item))
    (when (romacs-agent-queue session) (romacs-agent--dispatch session))))

(add-hook 'romacs-agent-loaded-functions #'romacs-agent-store--requeue)
;; A session with messages kept waiting is loaded as a restore runs.
(add-to-list 'romacs-agent-mark-kinds
             (cons 'queued (lambda (dir _manifest)
                             (and (romacs-agent-store-queued (file-name-nondirectory (directory-file-name dir))) t))))

;;; The board's folds

(defun romacs-agent-store-folded ()
  "The projects the board shows folded, their directories; nil when
there is no database."
  (when-let* ((db (romacs-agent-store--open)))
    (condition-case err (mapcar #'car (sqlite-select db "select project from board_folds"))
      (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))

(defun romacs-agent-store-fold (project folded)
  "Keep PROJECT, a directory, FOLDED on the board or not, by the writer;
quietly nothing when the database cannot be written."
  (romacs-agent-write 'romacs-agent-store--fold-row (list project folded) nil (list 'fold project)))

(defun romacs-agent-store--fold-row (project folded)
  "Keep PROJECT FOLDED on the board or not, here.  A failure signals."
  (let ((db (or (romacs-agent-store--open) (error "No agents store to keep the board's folds in"))))
    (sqlite-execute db (if folded "insert or ignore into board_folds values (?)"
                         "delete from board_folds where project = ?")
                    (list project))
    t))

(defun romacs-agent-store-requested (parent request)
  "The id of the session PARENT (nil: no session) started with client
request id REQUEST: from its row; while a fill still writes the rows (a
start just after the table was made again), from the manifests of the
sessions not loaded, which a start may have left as rows to come."
  (or (when-let* ((db (romacs-agent-store--open)))
        (caar (sqlite-select db "select id from sessions where parent is ? and request = ? limit 1" (list parent request))))
      (and (process-live-p romacs-agent-store--filling)
           (romacs-agent-store--requested-recorded parent request))))

(defun romacs-agent-store--requested-recorded (parent request)
  "The id of the session not loaded whose manifest says PARENT started it
with REQUEST, or nil."
  (catch 'found
    (dolist (dir (and (file-directory-p (romacs-agent--state-root))
                      (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))
      (let ((id (file-name-nondirectory dir)))
        (unless (romacs-agent-loaded id)
          (when-let* ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir)))
                      (options (gethash "options" manifest))
                      ((hash-table-p options))
                      ((equal (romacs-agent--value (gethash "request" options)) request))
                      ((equal (romacs-agent--value (gethash "parent" options)) parent)))
            (throw 'found id)))))
    nil))

(add-hook 'romacs-agent-kept-functions #'romacs-agent-store-put)
(add-hook 'romacs-agent-requested-functions #'romacs-agent-store-requested)
;; Restored: the rows of sessions recorded with none are made, off the editor.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-store-fill-soon)
(defun romacs-agent-store-listed-ids ()
  "The ids of the sessions with a row, as a table: one query, however many."
  (let ((table (make-hash-table :test #'equal)))
    (when-let* ((db (romacs-agent-store--open)))
      (dolist (row (sqlite-select db "select id from sessions")) (puthash (car row) t table)))
    table))

(defun romacs-agent-store-prefixed-ids (short)
  "The ids with a row that begin with SHORT: the primary key's range."
  (when-let* ((db (romacs-agent-store--open)))
    (mapcar #'car (sqlite-select db "select id from sessions where id >= ? and id < ?"
                                 (list short (concat short "\uffff"))))))

(defun romacs-agent-store-rows-since (place)
  "(IDS . PLACE): the ids of the sessions whose rows were written after
PLACE (a rowid: a row written again takes a new one), nil for none yet,
and the place after them; nil with no listing to ask."
  (when-let* ((db (romacs-agent-store--open)))
    (if (null place)
        (cons nil (or (caar (sqlite-select db "select max(rowid) from sessions")) 0))
      (let ((rows (sqlite-select db "select id, rowid from sessions where rowid > ? order by rowid" (list place))))
        (cons (mapcar #'car rows) (if rows (cadr (car (last rows))) place))))))

(setq romacs-agent-listed-p #'romacs-agent-store-listed-p)
(setq romacs-agent-prefixed-ids #'romacs-agent-store-prefixed-ids)
(setq romacs-agent-rows-since #'romacs-agent-store-rows-since)
(setq romacs-agent-listed-ids #'romacs-agent-store-listed-ids)
(setq romacs-agent-quiet-p #'romacs-agent-store-quiet-p)

(provide 'romacs-agent-store)
;;; romacs-agent-store.el ends here

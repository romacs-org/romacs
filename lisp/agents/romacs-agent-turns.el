;;; romacs-agent-turns.el --- A session's turns, a row each, in SQLite  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each turn of a session is a row of the `turns' table of the agents'
;; sessions.sqlite: where it begins and ends in the session's records
;; (seqs), when, its prompt, its reply as the chat shows it, a line a
;; tool call, its cost.  The rows are written from a session's records,
;; replayed after its latest row into a copy, from the events every
;; harness's adapter gives (input, tool, done), whatever holds the records
;; (a journal or SQLite): by a batch process, a moment after a turn ends
;; and at a restore, never on the editor; a turn becomes a row only once
;; it ends.  So where a chat's last turns begin is one indexed query, and
;; text can be searched (`turns_fts').

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-store)

(defconst romacs-agent-turns--schema
  '("create table if not exists turns (
       session text not null, first_seq integer not null, last_seq integer, t real,
       prompt text, reply text, tools text, cost real, primary key (session, first_seq)) without rowid")
  "The turns' table, made with the database.")

(defconst romacs-agent-turns--search-schema
  '("create virtual table if not exists turns_fts using fts5 (session unindexed, first_seq unindexed, prompt, reply, tools)")
  "The turns' text, searchable: made where SQLite has FTS5.")

(defvar romacs-agent-turns--made nil "The database the turns' tables were made in.")

(defun romacs-agent-turns--db ()
  "The database, its turns' tables made; nil without SQLite."
  (when-let* ((db (romacs-agent-store--open)))
    (unless (eq romacs-agent-turns--made db)
      (dolist (statement romacs-agent-turns--schema) (sqlite-execute db statement))
      (ignore-errors (dolist (statement romacs-agent-turns--search-schema) (sqlite-execute db statement)))
      (setq romacs-agent-turns--made db))
    db))

;;; Writing

(defvar romacs-agent-turns--open (make-hash-table :test #'eq :weakness 'key)
  "Each session's turn under way: a plist of :first, :prompt, :tools (each
\(ID NAME TEXT STATE), the latest first).")

(defun romacs-agent-turns--summary (input)
  "A tool call's INPUT (a table), as one short line: its first text."
  (let ((text (and (hash-table-p input)
                   (catch 'found
                     (maphash (lambda (_ value) (when (and (stringp value) (not (string-empty-p value))) (throw 'found value)))
                              input)
                     nil))))
    (if text (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " text) 60 nil nil "…") "")))

(defun romacs-agent-turns--tools-text (tools)
  "TOOLS, a turn's calls (the latest first), a line each: `NAME · TEXT · ok'."
  (mapconcat (lambda (tool)
               (pcase-let ((`(,_ ,name ,text ,state) tool))
                 (string-join (delq nil (list name (and (not (string-empty-p text)) text)
                                              (pcase state ('completed "ok") ('failed "failed") (_ "…"))))
                              " · ")))
             (reverse tools) "\n"))

(defun romacs-agent-turns--write (session last)
  "Write SESSION's turn under way, ended at seq LAST, as its row."
  (when-let* ((turn (gethash session romacs-agent-turns--open))
              (db (romacs-agent-turns--db)))
    (remhash session romacs-agent-turns--open)
    (let ((row (list (romacs-agent-id session) (plist-get turn :first) last (or (plist-get turn :time) (float-time))
                     (plist-get turn :prompt) (romacs-agent-reply session)
                     (romacs-agent-turns--tools-text (plist-get turn :tools)) (romacs-agent-cost session))))
      ;; The row and its text's index in one transaction: never two of it.
      (condition-case err
          (progn
            (sqlite-execute db "begin immediate")
            (sqlite-execute db "insert or replace into turns values (?, ?, ?, ?, ?, ?, ?, ?)" row)
            (when (sqlite-select db "select 1 from sqlite_master where name = 'turns_fts'")
              (sqlite-execute db "delete from turns_fts where session = ? and first_seq = ?" (list (nth 0 row) (nth 1 row)))
              (sqlite-execute db "insert into turns_fts values (?, ?, ?, ?, ?)"
                              (list (nth 0 row) (nth 1 row) (nth 4 row) (nth 5 row) (nth 6 row))))
            (sqlite-execute db "commit"))
        (error (ignore-errors (sqlite-execute db "rollback"))
               (message "romacs-agent-turns: %s" (error-message-string err)))))))

(defun romacs-agent-turns--event (session event)
  "Fold EVENT into SESSION's turn under way; a turn's end writes it."
  (when (romacs-agent-dir session)
    (pcase (plist-get event :type)
      ;; A prompt begins a turn; a steer is its turn's.
      ('input (unless (or (plist-get event :steer) (gethash session romacs-agent-turns--open))
                (puthash session (list :first (plist-get event :seq) :prompt (plist-get event :text) :tools nil
                                       :time (plist-get event :time))
                         romacs-agent-turns--open)))
      ('tool (when-let* ((turn (gethash session romacs-agent-turns--open)))
               (let* ((tools (plist-get turn :tools)) (known (assoc (plist-get event :id) tools)))
                 (if known (setf (nth 3 known) (plist-get event :state))
                   (push (list (plist-get event :id) (plist-get event :name)
                               (romacs-agent-turns--summary (plist-get event :input)) (plist-get event :state))
                         tools))
                 (puthash session (plist-put turn :tools tools) romacs-agent-turns--open))))
      ;; Only its end makes a turn a row: one cut by an exit may yet be
      ;; resumed and end, its row written then.
      ('done (romacs-agent-turns--write session (or (plist-get event :seq) (romacs-agent-seq session)))))))

(defvar romacs-agent-turns--due nil "The ids of sessions whose rows are to be caught up, by a timer.")

(defun romacs-agent-turns--ended (session event)
  "A live turn's end: SESSION's rows are caught up a moment later, by a
batch process (`romacs-agent-turns-fill-soon'), not here."
  (when (and (eq (plist-get event :type) 'done) (romacs-agent-dir session)
             (eq (romacs-agent-get (romacs-agent-id session)) session))
    (unless romacs-agent-turns--due
      (run-with-timer 2 nil (lambda () (let ((ids romacs-agent-turns--due))
                                         (setq romacs-agent-turns--due nil)
                                         (romacs-agent-turns-fill-soon ids)))))
    (cl-pushnew (romacs-agent-id session) romacs-agent-turns--due :test #'equal)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-turns--ended)

;;; Filling

(defun romacs-agent-turns--last (session)
  "The seq SESSION's latest row ends at, or nil with no row."
  (when-let* ((db (romacs-agent-turns--db)))
    (caar (sqlite-select db "select max(last_seq) from turns where session = ?" (list (romacs-agent-id session))))))

(defun romacs-agent-turns-fill (session)
  "Write the rows of SESSION's turns its rows do not have yet: its records
after its latest row (all of them with none) replayed into a copy, as
they came.  The number written."
  (let* ((last (romacs-agent-turns--last session))
         (from (if last (romacs-agent--journal-after session last) 0)))
    (if (not from) 0
      (let ((scratch (romacs-agent--scratch session)) (before (romacs-agent-turns-count session)))
        (let ((romacs-agent-event-functions (list #'romacs-agent-turns--event)))
          (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)) from))
        ;; A turn under way at the records' end is not one yet.
        (remhash scratch romacs-agent-turns--open)
        (- (romacs-agent-turns-count session) before)))))

(defun romacs-agent-turns-fill-batch ()
  "`romacs-agent-turns-fill' for the sessions the command line names (all
recorded ones, with none), in a batch process: each fills from where its
rows end, nothing when its records end there too."
  (let ((written 0) (t0 (float-time))
        (ids (prog1 command-line-args-left (setq command-line-args-left nil))))
    (dolist (dir (if ids (mapcar (lambda (id) (expand-file-name id (romacs-agent--state-root))) ids)
                   (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))
      (when (file-exists-p (expand-file-name "manifest.json" dir))
        (cl-letf (((symbol-function 'romacs-agent--attach) #'ignore))
          (romacs-agent--load dir))
        (when-let* ((session (romacs-agent-get (file-name-nondirectory dir))))
          ;; Whether records came since its rows is the records' to say: a
          ;; session loaded beside its running supervisor has an older seq.
          (setq written (+ written (condition-case err (romacs-agent-turns-fill session)
                                     (error (message "romacs-agent-turns: %s: %s" dir (error-message-string err)) 0))))
          (remhash (romacs-agent-id session) romacs-agent--sessions))))
    (princ (format "romacs-agent-turns: %d turns written in %.0f s\n" written (- (float-time) t0)))))

(defvar romacs-agent-turns--filling nil "The batch process catching the rows up, while it runs.")

(defvar romacs-agent-turns--waiting nil
  "The ids a fill is still to catch up once the one running ends: t for
all of them.")

(defun romacs-agent-turns-fill-soon (&optional ids)
  "Catch the rows of the sessions IDS names (all of them, with none) up with
their records in a batch process of this build, off the editor; while one
runs, once it ends."
  (when (and (not noninteractive) (romacs-agent-turns--db))
    (if (process-live-p romacs-agent-turns--filling)
        (setq romacs-agent-turns--waiting
              (if (or (null ids) (eq romacs-agent-turns--waiting t)) t
                (cl-union ids romacs-agent-turns--waiting :test #'equal)))
      (setq romacs-agent-turns--filling
            (make-process
             :name "romacs-agent-turns-fill" :noquery t :connection-type 'pipe
             :buffer (generate-new-buffer " *romacs-agent-turns-fill*" t)
             :command (append (list (expand-file-name invocation-name invocation-directory) "--batch"
                                    "-l" "romacs-agent" "-f" "romacs-agent-turns-fill-batch")
                              ids)
             :sentinel (lambda (process _event)
                         (unless (process-live-p process)
                           (when (buffer-live-p (process-buffer process)) (kill-buffer (process-buffer process)))
                           (when-let* ((waiting romacs-agent-turns--waiting))
                             (setq romacs-agent-turns--waiting nil)
                             (romacs-agent-turns-fill-soon (unless (eq waiting t) waiting))))))))))

;; Restored: what ran while no editor was there gets its rows, off the editor.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-turns-fill-soon)

;;; Reading

(defun romacs-agent-turns-first-seqs (session n &optional before)
  "The first seqs of SESSION's latest N turns (before seq BEFORE when
given), the latest first."
  (when-let* ((db (romacs-agent-turns--db)))
    (mapcar #'car (sqlite-select db (concat "select first_seq from turns where session = ?"
                                            (if before " and first_seq < ?" "") " order by first_seq desc limit ?")
                                 (append (list (romacs-agent-id session)) (and before (list before)) (list n))))))

(defun romacs-agent-turns-count (session)
  "How many turns SESSION has rows for."
  (or (and-let* ((db (romacs-agent-turns--db)))
        (caar (sqlite-select db "select count(*) from turns where session = ?" (list (romacs-agent-id session)))))
      0))

(defun romacs-agent-turns-search (query &optional limit)
  "The turns whose text matches QUERY (FTS5 syntax), the best first: each
\(SESSION FIRST-SEQ SNIPPET), at most LIMIT (20)."
  (when-let* ((db (romacs-agent-turns--db)))
    (ignore-errors
      (sqlite-select db (concat "select session, first_seq, snippet(turns_fts, -1, '[', ']', '…', 12) from turns_fts"
                                " where turns_fts match ? order by rank limit ?")
                     (list query (or limit 20))))))

(provide 'romacs-agent-turns)
;;; romacs-agent-turns.el ends here

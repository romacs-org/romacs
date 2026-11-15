;;; romacs-agent-store.el --- Agent sessions listed from one SQLite database  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Every recorded agent session has a row in sessions.sqlite, in the
;; agents' state root: what a list of the sessions shows (title, project,
;; harness, state, parent, last activity, its reply's start), so that
;; listing them reads no journal.  The database is in WAL mode, each
;; write one short transaction: several Romacs processes, and a batch
;; process filling it, share it.
;;
;; A session's row is written as its state is kept (`romacs-agent--keep',
;; through `romacs-agent-kept-functions'), so the row says what the kept
;; state says.  `romacs-agent-store-fill' writes the rows, and the kept
;; state, of the sessions that have none: a batch process runs it
;; (`romacs-agent-store-fill-soon'), off the editor, as their journals
;; are long.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)

(defconst romacs-agent-store--schema
  '("create table if not exists sessions (
       id text primary key, dir text not null, title text, cwd text,
       harness text, provider text, model text, state text, parent text,
       last_time real, seq integer, done_seq integer, input_seq integer,
       failed integer, reply text, written real)"
    "create index if not exists sessions_state on sessions (state)"
    "create index if not exists sessions_recent on sessions (last_time desc)"
    "create index if not exists sessions_cwd on sessions (cwd)"
    "create index if not exists sessions_parent on sessions (parent)")
  "The database's tables and indexes, made when it is opened.")

(defconst romacs-agent-store--reply-chars 2000
  "The most of a session's reply its row keeps: what a preview shows.")

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
              (dolist (statement romacs-agent-store--schema) (sqlite-execute db statement))
              (set-file-modes file #o600)
              (setq romacs-agent-store--db (cons file db))
              db)
          (error (message "romacs-agent-store: %s" (error-message-string err)) nil))))))

(defun romacs-agent-store--row (session)
  "SESSION's row, its values in the table's column order."
  (let ((reply (romacs-agent-reply session)))
    (list (romacs-agent-id session) (romacs-agent-dir session) (romacs-agent-title session)
          (romacs-agent-cwd session) (symbol-name (romacs-agent-harness session))
          (romacs-agent-provider session) (romacs-agent-model session)
          (symbol-name (or (romacs-agent-state session) 'detached))
          (plist-get (romacs-agent-options session) :parent)
          (romacs-agent-last-time session) (romacs-agent-seq session) (romacs-agent-done-seq session)
          (romacs-agent-input-seq session) (if (romacs-agent-failed session) 1 0)
          (and (stringp reply) (substring reply 0 (min (length reply) romacs-agent-store--reply-chars)))
          (float-time))))

(defun romacs-agent-store-put (session &optional state)
  "Write SESSION's row, its state STATE when given (as its records left it);
quietly nothing when the database cannot be written."
  (when-let* ((db (romacs-agent-store--open)))
    (let ((row (romacs-agent-store--row session)))
      (when state (setf (nth 7 row) (symbol-name state)))
      (condition-case err
          (sqlite-execute db (concat "insert or replace into sessions values ("
                                     (mapconcat #'identity (make-list (length row) "?") ",") ")")
                          row)
        (error (message "romacs-agent-store: %s" (error-message-string err)))))))

(defun romacs-agent-store-ids ()
  "The ids of the sessions that have a row."
  (when-let* ((db (romacs-agent-store--open)))
    (mapcar #'car (sqlite-select db "select id from sessions"))))

;;; Filling it

(defun romacs-agent-store--unsaved ()
  "The directories of recorded sessions with no kept state or no row, whose
supervisor is not running: what `romacs-agent-store-fill' folds."
  (let ((rows (make-hash-table :test #'equal)))
    (dolist (id (romacs-agent-store-ids)) (puthash id t rows))
    (cl-remove-if (lambda (dir)
                    (let ((id (file-name-nondirectory dir)))
                      (or (not (file-exists-p (expand-file-name "manifest.json" dir)))
                          (file-exists-p (romacs-agent--host-socket id))
                          (and (gethash id rows) (file-exists-p (expand-file-name "state.eld" dir))))))
                  (and (file-directory-p (romacs-agent--state-root))
                       (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))))

(defun romacs-agent-store-fill ()
  "Fold each unsaved session's journal (`romacs-agent-store--unsaved') once,
and keep its state and row: a batch process's work, as journals are long.
Return how many were filled."
  (let ((count 0))
    (dolist (dir (romacs-agent-store--unsaved) count)
      (condition-case err
          (progn
            (romacs-agent--load dir)
            (when-let* ((session (romacs-agent-get (file-name-nondirectory dir))))
              (romacs-agent--keep session)
              (romacs-agent-store-put session)
              (remhash (romacs-agent-id session) romacs-agent--sessions)
              (cl-incf count)))
        (error (message "romacs-agent-store: %s: %s" dir (error-message-string err)))))))

(defun romacs-agent-store-fill-batch ()
  "`romacs-agent-store-fill' in a batch process, its count printed."
  (let ((romacs-agent-event-functions nil))
    (princ (format "romacs-agent-store: filled %d\n" (romacs-agent-store-fill)))))

(defvar romacs-agent-store--filling nil "The batch process filling the database, while it runs.")

(defvar romacs-agent-store-filled-hook nil
  "Run once a batch process has filled the database.")

(defun romacs-agent-store-fill-soon ()
  "Fill the database in a batch process of this build, when any session is
unsaved and no fill runs; `romacs-agent-store-filled-hook' runs once it is."
  (when (and (not (process-live-p romacs-agent-store--filling)) (romacs-agent-store--open)
             (romacs-agent-store--unsaved))
    (setq romacs-agent-store--filling
          (make-process
           :name "romacs-agent-store-fill" :noquery t :connection-type 'pipe
           :buffer (generate-new-buffer " *romacs-agent-store-fill*" t)
           :command (list (expand-file-name invocation-name invocation-directory) "--batch"
                          "-l" "romacs-agent-store" "-f" "romacs-agent-store-fill-batch")
           :sentinel (lambda (process _event)
                       (unless (process-live-p process)
                         (when (buffer-live-p (process-buffer process)) (kill-buffer (process-buffer process)))
                         (run-hooks 'romacs-agent-store-filled-hook)))))))

(add-hook 'romacs-agent-kept-functions #'romacs-agent-store-put)

(provide 'romacs-agent-store)
;;; romacs-agent-store.el ends here

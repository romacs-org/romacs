;;; romacs-agent-store-tests.el --- Agent sessions listed from SQLite -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-store))

(defun romacs-agent-store-tests--live (id)
  "A socket where the supervisor of session ID listens, as one running has."
  (let ((socket (romacs-agent--host-socket id)))
    (make-directory (file-name-directory socket) t)
    (write-region "" nil socket nil 'silent)))

(defmacro romacs-agent-store-tests--with (&rest body)
  "BODY with a state root of its own (`root'), a registry of its own, and
`record' recording a session there (ID TITLE &optional RUNNING), its
journal one turn long."
  (declare (indent 0))
  `(let* ((root (file-name-as-directory (make-temp-file "romacs-store-" t)))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (romacs-agent-store--db nil))
     (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root))
               ((symbol-function 'romacs-agent--runtime-root) (lambda () (file-name-as-directory (expand-file-name "run" root)))))
       (cl-flet ((record (id title &optional running)
                   (let ((dir (expand-file-name id root)))
                     (make-directory dir t)
                     (write-region (json-serialize (romacs-mcp--object "id" id "harness" "claude" "title" title "cwd" "/p/one/"))
                                   nil (expand-file-name "manifest.json" dir) nil 'silent)
                     (with-temp-file (expand-file-name "journal.jsonl" dir)
                       (insert (json-serialize (romacs-mcp--object "seq" 1 "t" 100.0 "in" (romacs-mcp--object "prompt" "hello"))) "\n"
                               (json-serialize (romacs-mcp--object "seq" 2 "t" 101.0 "ev" (romacs-mcp--object "type" "result" "result" "done it")))
                               "\n"))
                     (when running (romacs-agent-store-tests--live id)))))
         (unwind-protect (progn ,@body)
           (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))
           (delete-directory root t))))))

(ert-deftest romacs-agent-store-fills-from-the-records ()
  ;; A session with no row gets one from its manifest, and its kept state
  ;; when it has one, its journal never read; filled, none is missing.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (record "aaaaaaaa-0000-4000-8000-000000000001" "one")
    (record "aaaaaaaa-0000-4000-8000-000000000002" "two")
    (with-temp-file (expand-file-name "aaaaaaaa-0000-4000-8000-000000000002/state.eld" root)
      (prin1 (list :version 1 :layout nil :seq 2 :fields '((state . idle) (cost . 0.5) (last-time . 101.0) (title . "two kept")))
             (current-buffer)))
    (cl-letf (((symbol-function 'romacs-agent--read-journal) (lambda (&rest _) (error "A journal read"))))
      (should (= (romacs-agent-store-fill) 2)))
    (should-not (romacs-agent-store--missing))
    (should (= (hash-table-count romacs-agent--sessions) 0))
    (should (equal (sqlite-select (romacs-agent-store--open)
                                  "select id, title, cwd, state, cost, last_time from sessions order by id")
                   `(("aaaaaaaa-0000-4000-8000-000000000001" "one" "/p/one/" "detached" nil
                      ,(float-time (file-attribute-modification-time
                                    (file-attributes (expand-file-name "aaaaaaaa-0000-4000-8000-000000000001/journal.jsonl" root)))))
                     ("aaaaaaaa-0000-4000-8000-000000000002" "two kept" "/p/one/" "idle" 0.5 101.0))))
    ;; Filled again: nothing to do.
    (should (= (romacs-agent-store-fill) 0))))

(ert-deftest romacs-agent-store-row-follows-the-kept-state ()
  ;; A kept state writes its session's row, a later one replacing it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (ignore #'record)
    (let ((session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000001" :harness 'claude
                                       :title "first" :cwd "/p/two/" :dir (expand-file-name "b" root) :seq 3)))
      (make-directory (romacs-agent-dir session))
      (puthash (romacs-agent-id session) session romacs-agent--sessions)
      (romacs-agent--keep session 'idle)
      (setf (romacs-agent-title session) "second" (romacs-agent-seq session) 4)
      (romacs-agent--keep session 'idle)
      (should (equal (sqlite-select (romacs-agent-store--open) "select title, state from sessions")
                     '(("second" "idle")))))))

(ert-deftest romacs-agent-store-a-rename-reaches-the-row ()
  ;; A rename at a state already kept rewrites no row by its keep: its
  ;; title alone is written to the row, the rest of it as it was.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (ignore #'record)
    (let ((session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000002" :harness 'claude
                                       :title "first" :cwd "/p/two/" :dir (expand-file-name "b" root) :seq 3)))
      (make-directory (romacs-agent-dir session))
      (puthash (romacs-agent-id session) session romacs-agent--sessions)
      (romacs-agent--keep session 'idle)
      (setf (romacs-agent-title session) "renamed")
      (let ((events nil) (romacs-agent-event-functions nil))
        (add-hook 'romacs-agent-event-functions (lambda (_s e) (push (plist-get e :type) events)))
        (romacs-agent--update-manifest session)
        (should (equal (sqlite-select (romacs-agent-store--open) "select title, state, cwd from sessions")
                       '(("renamed" "idle" "/p/two/"))))
        ;; Its views told once it is written, to read the row again.
        (should (memq 'changed events)))
      ;; Queued, not written yet: its views are told only once it is; a
      ;; write that failed is tried once more.
      (let ((done nil) (told 0) (idle nil))
        (cl-letf (((symbol-function 'romacs-agent-write) (lambda (_f _a d _k) (setq done d) 'queued))
                  ((symbol-function 'run-with-idle-timer) (lambda (&rest args) (push args idle))))
          (romacs-agent-store-title (romacs-agent-id session) "later" (lambda () (cl-incf told)))
          (should (= told 0))
          (funcall done '(error (sqlite-locked-error "locked")))
          (should (= told 0))
          (should (= (length idle) 1))
          (funcall done '(ok t))
          (should (= told 1)))))))

(ert-deftest romacs-agent-store-leaves-the-listed-unloaded ()
  ;; Restored: a listed session with no kept state and no supervisor is
  ;; not loaded (its journal unread), one whose supervisor runs a turn is;
  ;; the board shows both, and asking for the first loads it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-board)
  (romacs-agent-store-tests--with
    (let ((old "aaaaaaaa-0000-4000-8000-000000000001") (live "aaaaaaaa-0000-4000-8000-000000000002")
          (journals nil))
      (record old "old")
      (record live "live" t)
      ;; OLD a task delegated by "parent" with client request "request-1".
      (let* ((file (expand-file-name (concat old "/manifest.json") root))
             (manifest (romacs-agent--read-json file)))
        (puthash "options" (romacs-mcp--object "parent" "parent" "request" "request-1") manifest)
        (romacs-agent--write-json file manifest))
      (romacs-agent-store-fill)
      ;; Its supervisor at work on a turn.
      (sqlite-execute (romacs-agent-store--open) "update sessions set state = 'running' where id = ?" (list live))
      (cl-letf* ((read (symbol-function 'romacs-agent--read-journal))
                 ((symbol-function 'romacs-agent--read-journal)
                  (lambda (session &rest args) (push (romacs-agent-id session) journals) (apply read session args)))
                 ((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                 ((symbol-function 'romacs-agent--endpoint) #'ignore)
                 ((symbol-function 'romacs-agent--sweep) #'ignore)
                 ((symbol-function 'romacs-agent-store-fill-soon) #'ignore))
        (romacs-agent-restore)
        (should-not (romacs-agent-loaded old))
        (should (romacs-agent-get live))
        (should-not (member old journals))
        (let ((shown (mapcar #'romacs-agent-id (romacs-agent-board--sessions))))
          (should (member old shown))
          (should (member live shown)))
        (let ((found (romacs-agent-get old)))
          (should (equal (romacs-agent-title found) "old"))
          (should (eq (romacs-agent-get old) found)))
        ;; A delegation asked again after a start, its task left a row and
        ;; indexed nowhere: found by its row, loaded.
        (remhash old romacs-agent--sessions)
        (let ((romacs-agent--requests (make-hash-table :test #'equal)))
          (should (equal (romacs-agent-title (romacs-agent-requested "parent" "request-1")) "old"))
          (should-not (romacs-agent-requested "parent" "request-2"))
          ;; Asked for before its row lands (a fill still running): by its
          ;; manifest.  And one no session delegated: its NULL parent matched.
          (remhash old romacs-agent--sessions)
          (clrhash romacs-agent--requests)
          (sqlite-execute (romacs-agent-store--open) "delete from sessions where id = ?" (list old))
          (let ((romacs-agent-store--filling (make-process :name "fill" :command '("sleep" "30") :noquery t)))
            (unwind-protect
                (should (equal (romacs-agent-title (romacs-agent-requested "parent" "request-1")) "old"))
              (delete-process romacs-agent-store--filling)))
          (remhash old romacs-agent--sessions)
          (sqlite-execute (romacs-agent-store--open) "update sessions set parent = null, request = 'request-9' where id = ?" (list live))
          (remhash live romacs-agent--sessions)
          (clrhash romacs-agent--requests)
          (should (equal (romacs-agent-id (romacs-agent-requested nil "request-9")) live)))))))

;; Restored a turn at a time from the command loop: the same.
(ert-deftest romacs-agent-store-leaves-the-listed-unloaded-in-steps ()
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((old "aaaaaaaa-0000-4000-8000-000000000001") (live "aaaaaaaa-0000-4000-8000-000000000002")
          (romacs-agent--restore-pending nil) (romacs-agent--restoring nil)
          (romacs-agent--restored nil) (romacs-agent--restore-count 0))
      (record old "old")
      (record live "live" t)
      (romacs-agent-store-fill)
      ;; Its supervisor at work on a turn.
      (sqlite-execute (romacs-agent-store--open) "update sessions set state = 'running' where id = ?" (list live))
      (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                ((symbol-function 'romacs-agent--endpoint) #'ignore)
                ((symbol-function 'romacs-agent--sweep) #'ignore)
                ((symbol-function 'romacs-agent-store-fill-soon) #'ignore)
                ((symbol-function 'romacs-startup-progress) #'ignore)
                ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
        (romacs-agent-restore-soon)
        (should-not romacs-agent--restoring)
        (should-not (romacs-agent-loaded old))
        (should (romacs-agent-get live))))))

;; A first start, no rows yet: their fill runs aside, only the live
;; session is loaded; once the fill has run, one still with no row is
;; loaded as it is.
(ert-deftest romacs-agent-store-no-rows-yet-loads-the-live-alone ()
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((old "aaaaaaaa-0000-4000-8000-000000000001") (live "aaaaaaaa-0000-4000-8000-000000000002")
          (romacs-agent-store--filling nil) (romacs-agent-store--filled nil) (fills 0)
          (romacs-agent--restore-pending nil) (romacs-agent--restoring nil)
          (romacs-agent--restored nil) (romacs-agent--restore-count 0)
          ;; An editor's start, not a batch run's; its writes made here.
          (noninteractive nil) (romacs-agent-writer-queued nil))
      (record old "old")
      (record live "live" t)
      (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                ((symbol-function 'romacs-agent--endpoint) #'ignore)
                ((symbol-function 'romacs-agent--sweep) #'ignore)
                ((symbol-function 'romacs-startup-progress) #'ignore)
                ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args)))
                ((symbol-function 'romacs-agent-store-fill-soon)
                 (lambda () (cl-incf fills)
                   (unless (process-live-p romacs-agent-store--filling)
                     (setq romacs-agent-store--filling (make-process :name "fill" :command '("sleep" "30") :noquery t))))))
        (unwind-protect
            (progn
              (romacs-agent-restore-soon)
              (should (> fills 0))
              (should-not (romacs-agent-loaded old))
              (should (romacs-agent-get live)))
          (delete-process romacs-agent-store--filling))
        ;; Filled, or no start restoring: loaded as it is.
        (setq fills 0 romacs-agent-store--filled t romacs-agent--restoring t)
        (should-not (romacs-agent--unloaded-p (expand-file-name old root)))
        (setq romacs-agent-store--filled nil romacs-agent--restoring nil)
        (should-not (romacs-agent--unloaded-p (expand-file-name old root)))
        (should (= fills 0))
        ;; The restore over, a fill that ends without its row: loaded then.
        (setq romacs-agent-store--filled t)
        (should-not (romacs-agent-loaded old))
        (romacs-agent-store--load-missing)
        (should (romacs-agent-get old))
        (should-not romacs-agent--restore-pending)))))

(ert-deftest romacs-agent-store-a-quiet-live-session-stays-a-row-until-it-writes ()
  ;; Its supervisor running, its row quiet (idle, nothing wanted) and its
  ;; records in SQLite no further than its kept state: a start leaves it a
  ;; row.  When its supervisor writes more (a message come in, a task's
  ;; end), it is loaded, so attached, and reads what came; so too when its
  ;; supervisor wrote and ended between looks.  Loaded at once: a live one
  ;; whose row says a turn runs, or that wants the user or its parent, or
  ;; with records past its kept state (written while no editor was there),
  ;; or still in a journal.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let* ((n 0) (ids (cl-loop for name in '(quiet ending busy wanting behind journal)
                               collect (cons name (format "aaaaaaaa-0000-4000-8000-0000000001%02d" (cl-incf n)))))
           (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil)
           (romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal))
           (attached nil))
      (cl-flet ((id (name) (cdr (assq name ids)))
                (append (name seq) (romacs-agent-events-append
                                    (romacs-agent--quiet-session (expand-file-name (cdr (assq name ids)) root))
                                    (romacs-mcp--object "seq" seq "t" (float seq) "note" (romacs-mcp--object "type" "late")))))
        (pcase-dolist (`(,name . ,id) ids) (record id (symbol-name name)))
        (pcase-dolist (`(,name . ,id) ids)
          (unless (eq name 'journal)
            (romacs-agent-events-import (romacs-agent--quiet-session (expand-file-name id root)))))
        (romacs-agent-store-fill)
        (let ((db (romacs-agent-store--open)))
          (pcase-dolist (`(,name . ,id) ids)
            (sqlite-execute db "update sessions set state = ?, attention = ?, seq = 2 where id = ?"
                            (list (if (eq name 'busy) "running" "idle") (if (eq name 'wanting) 1 0) id))))
        ;; Written while no editor was there: past its kept state.
        (append 'behind 3)
        (pcase-dolist (`(,_ . ,id) ids) (romacs-agent-store-tests--live id))
        (unwind-protect
            (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (session &rest _) (push (romacs-agent-id session) attached)))
                      ((symbol-function 'romacs-agent--endpoint) #'ignore)
                      ((symbol-function 'romacs-agent--sweep) #'ignore)
                      ((symbol-function 'romacs-agent-store-fill-soon) #'ignore))
              (romacs-agent-restore)
              (should-not (romacs-agent-loaded (id 'quiet)))
              (should-not (romacs-agent-loaded (id 'ending)))
              (dolist (name '(busy wanting behind journal)) (should (romacs-agent-loaded (id name))))
              ;; Nothing written: still rows.
              (romacs-agent--quiet-tick)
              (should-not (romacs-agent-loaded (id 'quiet)))
              ;; Its supervisor writes: loaded, attached.  The other's writes
              ;; and ends: loaded all the same.
              (append 'quiet 3)
              (append 'ending 3)
              (delete-file (romacs-agent--host-socket (id 'ending)))
              (romacs-agent--quiet-tick)
              (should (romacs-agent-get (id 'quiet)))
              (should (member (id 'quiet) attached))
              (should (romacs-agent-get (id 'ending)))
              (should-not (gethash (id 'quiet) romacs-agent--quiet))
              ;; None left to look at: the timer stops.
              (should-not romacs-agent--quiet-timer))
          (when (timerp romacs-agent--quiet-timer) (cancel-timer romacs-agent--quiet-timer)))))))

(ert-deftest romacs-agent-store-quiet-sessions-in-sqlite-take-one-query-and-one-watch ()
  ;; Quiet sessions whose records are in SQLite: one timer, each look one
  ;; query for all of them; the one that wrote is loaded; with none left,
  ;; no timer.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((ids (list "aaaaaaaa-0000-4000-8000-00000000000a" "aaaaaaaa-0000-4000-8000-00000000000b"
                     "aaaaaaaa-0000-4000-8000-00000000000c"))
          (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil)
          (romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal))
          (loaded nil))
      (dolist (id ids) (record id id))
      (dolist (id ids) (romacs-agent-events-import (romacs-agent--quiet-session (expand-file-name id root))))
      ;; Their supervisors running since.
      (make-directory (expand-file-name "run" root) t)
      (dolist (id ids) (romacs-agent-store-tests--live id))
      (cl-letf (((symbol-function 'romacs-agent-load-session) (lambda (id) (push id loaded))))
        (dolist (id ids)
          (let ((dir (expand-file-name id root)))
            (romacs-agent--quiet-watch dir (romacs-agent--journal-end (romacs-agent--quiet-session dir)))))
        (should (timerp romacs-agent--quiet-timer))
        (let ((queries 0) (select (symbol-function 'sqlite-select)))
          (cl-letf (((symbol-function 'sqlite-select)
                     (lambda (db query &rest args) (when (string-search "max(seq)" query) (cl-incf queries)) (apply select db query args))))
            (romacs-agent--quiet-tick)
            (should (= queries 1))
            (should-not loaded)
            (romacs-agent-events-append (romacs-agent--quiet-session (expand-file-name (cadr ids) root))
                                        (romacs-mcp--object "seq" 9 "t" 9.0 "note" (romacs-mcp--object "type" "late")))
            (romacs-agent--quiet-tick)
            (should (= queries 2))
            (should (equal loaded (list (cadr ids))))))
        (clrhash romacs-agent--quiet)
        (romacs-agent--quiet-tick)
        (should-not romacs-agent--quiet-timer)))))

(ert-deftest romacs-agent-store-a-quiet-session-misses-no-record-and-no-exit ()
  ;; A record written between the look that found it quiet and its watch
  ;; is new all the same (the watch starts where the look found its end);
  ;; a supervisor that ended with nothing new loads it (its exit is read).
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((late "aaaaaaaa-0000-4000-8000-000000000201") (gone "aaaaaaaa-0000-4000-8000-000000000202")
          (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil)
          (romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal))
          (loaded nil))
      (record late "late") (record gone "gone")
      (dolist (id (list late gone)) (romacs-agent-events-import (romacs-agent--quiet-session (expand-file-name id root))))
      (romacs-agent-store-fill)
      (sqlite-execute (romacs-agent-store--open) "update sessions set state = 'idle', attention = 0, seq = 2")
      (make-directory (expand-file-name "run" root) t)
      (dolist (id (list late gone)) (romacs-agent-store-tests--live id))
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent-load-session) (lambda (id) (push id loaded))))
            (let ((marks (mapcar (lambda (id) (cons id (romacs-agent-store-quiet-p (expand-file-name id root)))) (list late gone))))
              (should (equal (mapcar #'cdr marks) '(3 3)))
              ;; Written after the look, before the watch.
              (romacs-agent-events-append (romacs-agent--quiet-session (expand-file-name late root))
                                          (romacs-mcp--object "seq" 3 "t" 3.0 "note" (romacs-mcp--object "type" "late")))
              (pcase-dolist (`(,id . ,mark) marks) (romacs-agent--quiet-watch (expand-file-name id root) mark)))
            ;; Its supervisor ends with nothing new.
            (delete-file (romacs-agent--host-socket gone))
            (romacs-agent--quiet-tick)
            (should (member late loaded))
            (should (member gone loaded)))
        (when (timerp romacs-agent--quiet-timer) (cancel-timer romacs-agent--quiet-timer))))))

(ert-deftest romacs-agent-store-an-older-table-is-made-again-and-the-rest-kept ()
  ;; A database whose sessions table an earlier build made: made again with
  ;; this build's columns, its rows, records and the board's folds kept.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (ignore #'record)
    (let ((file (romacs-agent-store--file)))
      (make-directory (file-name-directory file) t)
      (let ((db (sqlite-open file)))
        (sqlite-execute db "create table sessions (id text primary key, dir text, state text)")
        (sqlite-execute db "insert into sessions values ('old', '/x', 'idle')")
        (sqlite-execute db "create table events (session text not null, seq integer not null, t real, kind text, record text not null)")
        (sqlite-execute db "insert into events values ('old', 1, 1.0, 'note', '{}')")
        (sqlite-execute db "create table board_folds (project text primary key)")
        (sqlite-execute db "insert into board_folds values ('/p')")
        (sqlite-pragma db "user_version = 3")
        (sqlite-close db))
      (let ((db (romacs-agent-store--open)))
        (should (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version))
        (should (member "seq" (mapcar #'cadr (sqlite-select db "pragma table_info(sessions)"))))
        (should (equal (sqlite-select db "select id, dir, state, seq from sessions") '(("old" "/x" "idle" nil))))
        (should (equal (sqlite-select db "select session, seq from events") '(("old" 1))))
        (should (equal (sqlite-select db "select project from board_folds") '(("/p"))))))))

(defconst romacs-agent-store-tests--older-open
  '("drop table if exists sessions"
    "pragma user_version = 2"
    "create table if not exists sessions (
       id text primary key, dir text not null, harness text, provider text, model text,
       title text, cwd text, parent text, state text, last_time real, cost real, transcript text,
       input_seq integer, done_seq integer, seen_seq integer, settled_seq integer)"
    "insert into sessions (id, dir, state) values ('older', '/o', 'idle')")
  "What a build of the store's version 2 (before sessions moved to SQLite
by default) does opening a database of another version, then a row it
writes: the sessions table made again in its form, the rest left.")

(defun romacs-agent-store-tests--tables (db)
  "Each table of DB but the sessions cache, with its rows, sorted."
  (mapcar (lambda (row)
            (cons (car row) (sort (mapcar #'prin1-to-string (sqlite-select db (format "select * from %s" (car row))))
                                  #'string<)))
          (sqlite-select db "select name from sqlite_master where type = 'table' and name != 'sessions' order by name")))

(ert-deftest romacs-agent-store-a-downgrade-and-back-keep-every-record ()
  ;; A store this build migrated, opened by an earlier build (a rollback),
  ;; then by this one again: the earlier one makes its sessions table
  ;; again, this one reforms it with the rows it holds; the records, what
  ;; is imported, the turns and the board's folds are kept, and an
  ;; imported session reads as it did.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-events)
  (romacs-agent-store-tests--with
    (let ((romacs-agent-events-store t) (romacs-agent-events--made nil)
          (romacs-agent-events--imported (make-hash-table :test #'equal)))
      (record "imp" "imported")
      (make-directory (file-name-directory (romacs-agent-store--file)) t)
      (let ((session (romacs-agent--make :id "imp" :dir (expand-file-name "imp" root))))
        (should (romacs-agent-events-import session))
        (romacs-agent-store-fold "/p/one/" t)
        (let* ((db (romacs-agent-store--open))
               (kept (romacs-agent-store-tests--tables db))
               (seqs (lambda ()
                       (let ((seqs nil))
                         (romacs-agent--read-journal (romacs-agent--make :id "imp" :dir (expand-file-name "imp" root))
                                                     (lambda (record) (push (gethash "seq" record) seqs)))
                         (nreverse seqs)))))
          (should (equal (funcall seqs) '(1 2)))
          ;; The earlier build opens it, and writes a row of its own.
          (sqlite-close db)
          (setq romacs-agent-store--db nil)
          (let ((older (sqlite-open (romacs-agent-store--file))))
            (dolist (statement romacs-agent-store-tests--older-open) (sqlite-execute older statement))
            (should (equal (romacs-agent-store-tests--tables older) kept))
            (sqlite-close older))
          ;; This build again: its sessions table, everything else as it was.
          (clrhash romacs-agent-events--imported)
          (setq romacs-agent-events--made nil)
          (let ((db (romacs-agent-store--open)))
            (should (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version))
            (should (sqlite-select db "select 1 from sessions where id = 'older'"))
            (should (equal (romacs-agent-store-tests--tables db) kept))
            (should (equal (funcall seqs) '(1 2)))))))))

(ert-deftest romacs-agent-store-a-fill-chunk-fails-whole ()
  ;; A row of a fill's chunk that fails to be written fails the chunk:
  ;; none of it is kept, it is said, and the next fill has it again.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (ignore #'record)
    (let ((said nil) (execute (symbol-function 'sqlite-execute)) (romacs-agent-writer-queued nil))
      (cl-letf (((symbol-function 'romacs-agent-store--missing) (lambda () '("/r/a" "/r/b" "/r/c")))
                ((symbol-function 'romacs-agent-store--recorded-row)
                 (lambda (dir) (list :id (file-name-nondirectory dir) :dir dir :title "t")))
                ((symbol-function 'sqlite-execute)
                 (lambda (db query &optional values)
                   (if (and (string-prefix-p "insert or ignore into sessions" query) (equal (car values) "b"))
                       (signal 'sqlite-error '("disk I/O error"))
                     (funcall execute db query values))))
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (should (= (romacs-agent-store-fill) 0))
        (should-not (sqlite-select (romacs-agent-store--open) "select id from sessions"))
        (should (cl-some (lambda (line) (string-search "3 rows not written" line)) said))))))

(ert-deftest romacs-agent-store-a-row-session-has-its-headers-facts ()
  ;; Kept, a session's row has what its chat's header says; the session
  ;; made from the row alone says the same, nothing else read.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (ignore #'record)
    (let ((session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000009" :harness 'claude :provider "claude"
                                       :title "header" :cwd "/p/two/" :dir (expand-file-name "b9" root) :seq 3
                                       :model "claude-x" :options (list :parent "p" :effort "high" :plan t)
                                       :permission-mode "acceptEdits" :context-used 1200 :context-window 200000)))
      (make-directory (romacs-agent-dir session))
      (puthash (romacs-agent-id session) session romacs-agent--sessions)
      (romacs-agent--keep session 'idle)
      (cl-letf (((symbol-function 'romacs-agent--read-kept) (lambda (&rest _) (error "Kept state read")))
                ((symbol-function 'romacs-agent--read-journal) (lambda (&rest _) (error "Records read"))))
        (let ((row (romacs-agent-store-row-session (romacs-agent-id session))))
          (should (equal (romacs-agent-model row) "claude-x"))
          (should (equal (plist-get (romacs-agent-options row) :effort) "high"))
          (should (eq (plist-get (romacs-agent-options row) :plan) t))
          (should (equal (plist-get (romacs-agent-options row) :parent) "p"))
          (should (equal (romacs-agent-permission-mode row) "acceptEdits"))
          (should (= (romacs-agent-context-used row) 1200))
          (should (= (romacs-agent-context-window row) 200000))
          (should (eq (romacs-agent-state row) 'idle))
          (should-not (romacs-agent-pending-selection row))))
      (should-not (romacs-agent-store-row-session "nothing")))))

(ert-deftest romacs-agent-store-what-waits-is-loaded ()
  ;; A session with watches kept, or a resume due, is loaded as a restore
  ;; runs, though listed with no kept state and no supervisor.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-tools)
  (romacs-agent-store-tests--with
    (let ((watched "aaaaaaaa-0000-4000-8000-000000000001") (resuming "aaaaaaaa-0000-4000-8000-000000000002")
          (idle "aaaaaaaa-0000-4000-8000-000000000003"))
      (record watched "w") (record resuming "r") (record idle "i")
      (write-region "[]" nil (expand-file-name (concat watched "/watches.json") root) nil 'silent)
      (let ((file (expand-file-name (concat resuming "/manifest.json") root)))
        (write-region (json-serialize (romacs-mcp--object "id" resuming "harness" "claude" "title" "r" "cwd" "/p/one/"
                                                          "resumeAt" 2000000000.0))
                      nil file nil 'silent))
      (romacs-agent-store-fill)
      (should (romacs-agent--unloaded-p (expand-file-name idle root)))
      (should-not (romacs-agent--unloaded-p (expand-file-name watched root)))
      (should-not (romacs-agent--unloaded-p (expand-file-name resuming root))))))

(ert-deftest romacs-agent-store-unloaded-keep-what-the-board-reads ()
  ;; Shown from its row alone, a settled session is still settled; its
  ;; parent's task list has it; settling the finished acts on it loaded.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-board)
  (require 'romacs-agent-tools)
  (romacs-agent-store-tests--with
    (let* ((child "aaaaaaaa-0000-4000-8000-000000000005") (parent "aaaaaaaa-0000-4000-8000-000000000006"))
      (record child "child")
      (let ((file (expand-file-name (concat child "/manifest.json") root)))
        (write-region (json-serialize (romacs-mcp--object "id" child "harness" "claude" "title" "child" "cwd" "/p/one/"
                                                          "settledSeq" 5 "options" (romacs-mcp--object "parent" parent)))
                      nil file nil 'silent))
      (romacs-agent-store-fill)
      (let ((shown (cl-find child (romacs-agent-store-unloaded) :key #'romacs-agent-id :test #'equal)))
        (should (romacs-agent-settled-p shown))
        (should (equal (plist-get (romacs-agent-options shown) :parent) parent)))
      (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () parent)))
        (should (equal (mapcar (lambda (task) (gethash "taskId" task))
                               (gethash "tasks" (romacs-agent-tools--list (romacs-mcp--object))))
                       (list child))))
      (let ((settled nil))
        (cl-letf (((symbol-function 'romacs-agent-board--finished)
                   (lambda (_rows) (list (gethash child (car romacs-agent-board--unloaded)))))
                  ((symbol-function 'romacs-agent-board--shown-rows) #'ignore)
                  ((symbol-function 'romacs-agent-board--redraw) #'ignore)
                  ((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                  ((symbol-function 'romacs-agent-settle) (lambda (s) (push s settled))))
          (romacs-agent-board--sessions)
          (romacs-agent-board-settle-finished))
        (should (equal (mapcar #'romacs-agent-id settled) (list child)))
        (should (eq (car settled) (romacs-agent-get child)))))))

(ert-deftest romacs-agent-store-settled-before-later-activity ()
  ;; Unloaded with no kept state: settled only when settled since its last
  ;; record, a later turn unsettling it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((early "aaaaaaaa-0000-4000-8000-000000000007") (late "aaaaaaaa-0000-4000-8000-000000000008"))
      (record early "early") (record late "late")
      (dolist (entry (list (cons early 1) (cons late 2)))
        (write-region (json-serialize (romacs-mcp--object "id" (car entry) "harness" "claude" "title" "x" "cwd" "/p/one/"
                                                          "settledSeq" (cdr entry)))
                      nil (expand-file-name (concat (car entry) "/manifest.json") root) nil 'silent))
      (romacs-agent-store-fill)
      (let ((shown (romacs-agent-store-unloaded)))
        (should-not (romacs-agent-settled-p (cl-find early shown :key #'romacs-agent-id :test #'equal)))
        (should (romacs-agent-settled-p (cl-find late shown :key #'romacs-agent-id :test #'equal)))))))

(ert-deftest romacs-agent-store-settle-finished-past-a-gone-one ()
  ;; Settling the finished: one gone meanwhile is skipped, the rest are
  ;; settled, each as loaded.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-board)
  (romacs-agent-store-tests--with
    (let ((gone "aaaaaaaa-0000-4000-8000-000000000010") (kept "aaaaaaaa-0000-4000-8000-000000000011") (settled nil))
      (record gone "gone") (record kept "kept")
      (romacs-agent-store-fill)
      (romacs-agent-board--sessions)
      (let ((finished (list (gethash gone (car romacs-agent-board--unloaded))
                            (gethash kept (car romacs-agent-board--unloaded)))))
        (delete-directory (expand-file-name gone root) t)
        (cl-letf (((symbol-function 'romacs-agent-board--finished) (lambda (_rows) finished))
                  ((symbol-function 'romacs-agent-board--shown-rows) #'ignore)
                  ((symbol-function 'romacs-agent-board--redraw) #'ignore)
                  ((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                  ((symbol-function 'romacs-agent-settle) (lambda (s) (push s settled))))
          (romacs-agent-board-settle-finished)))
      (should (equal (mapcar #'romacs-agent-id settled) (list kept)))
      (should (eq (car settled) (romacs-agent-get kept))))))

(ert-deftest romacs-agent-store-keeps-the-board-s-folds ()
  ;; Folded projects are kept, and kept past the sessions' table made
  ;; again for another form of it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (romacs-agent-store-fold "/p/one" t)
    (romacs-agent-store-fold "/p/two" t)
    (romacs-agent-store-fold "/p/two" nil)
    (should (equal (romacs-agent-store-folded) '("/p/one")))
    (sqlite-pragma (cdr romacs-agent-store--db) "user_version = 0")
    (sqlite-close (cdr romacs-agent-store--db))
    (setq romacs-agent-store--db nil)
    (should (equal (romacs-agent-store-folded) '("/p/one")))))

;;; romacs-agent-store-tests.el ends here

(defun romacs-agent-store-tests--hold-lock (file &optional first)
  "Another Romacs process that holds FILE's write lock for 1.5 s, once it
has it: in a transaction of its own, after the statement FIRST when given."
  (let* ((held nil)
         (holder (make-process
                  :name "romacs-store-lock" :noquery t :connection-type 'pipe
                  :filter (lambda (_ text) (when (string-search "held" text) (setq held t)))
                  :command (list (expand-file-name invocation-name invocation-directory) "--batch" "--eval"
                                 (format "%S" `(let ((db (sqlite-open ,file)))
                                                 (sqlite-pragma db "journal_mode = WAL")
                                                 (sqlite-execute db "begin immediate")
                                                 ,@(and first `((sqlite-execute db ,first)))
                                                 (princ "held\n")
                                                 (sleep-for 1.5)
                                                 (sqlite-execute db "commit")))))))
    (with-timeout (30 (delete-process holder) (error "The lock was never taken"))
      (while (not held) (accept-process-output holder 0.1)))
    holder))

(ert-deftest romacs-agent-store-fill-waits-out-another-process-s-lock ()
  ;; Another process holds the store's write lock longer than a connection
  ;; waits for it (a migration worker's long transaction): the fill tries
  ;; its chunk again until it is written, never leaving a row out.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (should (= (romacs-agent-store-fill) 0))
    (record "aaaaaaaa-0000-4000-8000-000000000001" "one")
    (record "aaaaaaaa-0000-4000-8000-000000000002" "two")
    (let ((holder (romacs-agent-store-tests--hold-lock (romacs-agent-store--file))))
      (unwind-protect
          (progn
            ;; Past the editor's wait, the lock still held.
            (should (= (romacs-agent-store-fill) 2))
            (should-not (romacs-agent-store--missing)))
        (delete-process holder)))))

(ert-deftest romacs-agent-store-fill-opens-a-store-being-made ()
  ;; Off the editor, a store whose making is locked by another process
  ;; past this one's wait is opened once the lock lets it, not taken for
  ;; no store: a batch fill writes every row all the same.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (record "aaaaaaaa-0000-4000-8000-000000000001" "one")
    (let ((holder (romacs-agent-store-tests--hold-lock (romacs-agent-store--file) "create table first (x)"))
          (romacs-agent-store-patient 300))
      (unwind-protect
          (let ((romacs-agent-store-patience romacs-agent-store-patient))
            (should (= (romacs-agent-store-fill) 1)))
        (delete-process holder)))))

(ert-deftest romacs-agent-store-the-writer-s-start-leaves-the-editor-s-wait ()
  ;; The editor runs the writer's start when it ends with no writer: its
  ;; own connections' wait stays short after.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((romacs-agent-store-patience 200))
      (romacs-agent-store--writer-start)
      (should (= romacs-agent-store-patience 200))
      (should (= (caar (sqlite-select (romacs-agent-store--open) "pragma busy_timeout")) romacs-agent-store-patient))
      ;; The writer itself waits long for every connection of its life: one
      ;; opened again after a start that failed too.
      (should (equal (alist-get 'romacs-agent-store-patience (funcall romacs-agent-writer-bindings-function))
                     romacs-agent-store-patient)))))

;; In a home where no agent has run, nothing has made the state directory:
;; the writer, asked to make the store, makes it, and the writer's own
;; marks go to it (a mark once failed on no database at all).
(ert-deftest romacs-agent-store-the-writer-makes-its-directory ()
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-events)
  (let* ((home (make-temp-file "romacs-store-" t))
         (root nil)
         (romacs-agent-writer-queued nil)
         (romacs-agent-store--db nil))
    (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root)))
      (unwind-protect
          (progn
            ;; At its start.
            (setq root (expand-file-name "start/romacs/agents/" home))
            (should-not (file-directory-p root))
            (should (romacs-agent-store--writer-start))
            (should (file-exists-p (romacs-agent-store--file)))
            ;; Asked to make the store, and its mark after.
            (setq root (expand-file-name "asked/romacs/agents/" home))
            (should-not (file-directory-p root))
            (should (romacs-agent-store--made))
            (should (file-exists-p (romacs-agent-store--file)))
            (should (romacs-agent-events--mark "aaaaaaaa-0000-4000-8000-000000000001"))
            (should (sqlite-select (romacs-agent-store--open) "select 1 from imported where session = ?"
                                   '("aaaaaaaa-0000-4000-8000-000000000001"))))
        (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))
        (delete-directory home t)))))

(ert-deftest romacs-agent-store-a-reader-waits-for-the-lock ()
  ;; The editor's read connection waits a moment for the lock: a WAL read
  ;; is refused while another connection checkpoints or vacuums, and a
  ;; refused open reads as no store at all.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (romacs-agent-store-fill)
    ;; Every module's tables, as the writer makes them.
    (dolist (table '("events" "turns" "turn_drawn" "transcripts"))
      (sqlite-execute (romacs-agent-store--open) (format "create table if not exists %s (x)" table)))
    (let ((romacs-agent-store--db nil))
      (let ((db (romacs-agent-store--open-to-read (romacs-agent-store--file))))
        (should db)
        (should (= (caar (sqlite-select db "pragma busy_timeout")) romacs-agent-store-patience))
        (sqlite-close db)))))

(ert-deftest romacs-agent-store-a-session-asking-of-a-restart-is-loaded ()
  ;; A listed session that asked to be told of a restart is loaded as a
  ;; restore runs, so it is told: left a row, it would never be.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-tools)
  (romacs-agent-store-tests--with
    (let ((asked "aaaaaaaa-0000-4000-8000-000000000001") (quiet "aaaaaaaa-0000-4000-8000-000000000002"))
      (record asked "asked")
      (record quiet "quiet")
      (write-region "" nil (expand-file-name (concat asked "/restart-wake") root) nil 'silent)
      (romacs-agent-store-fill)
      (should-not (romacs-agent--unloaded-p (expand-file-name asked root)))
      (should (romacs-agent--unloaded-p (expand-file-name quiet root))))))

(ert-deftest romacs-agent-store-a-version-bump-keeps-every-row ()
  ;; A populated store of another version: its sessions table is made in
  ;; this build's form, every row kept with every column both have, in one
  ;; transaction, its indexes made again.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (dotimes (i 120)
      (record (format "aaaaaaaa-0000-4000-8000-%012d" i) (format "row %d" i)))
    (romacs-agent-store-fill)
    (let* ((db (romacs-agent-store--open))
           (rows (sqlite-select db "select * from sessions order by id")))
      (should (= (length rows) 120))
      (sqlite-pragma db (format "user_version = %d" (1- romacs-agent-store--version)))
      (sqlite-close db)
      (setq romacs-agent-store--db nil)
      (let ((db (romacs-agent-store--open)))
        (should (equal (caar (sqlite-select db "pragma user_version")) romacs-agent-store--version))
        (should (equal (sqlite-select db "select * from sessions order by id") rows))
        (should (sqlite-select db "select 1 from sqlite_master where type = 'index' and name = 'sessions_parent'"))
        (should-not (sqlite-select db "select 1 from sqlite_master where name = 'sessions_reformed'"))))))

(ert-deftest romacs-agent-store-a-start-chooses-from-rows-as-from-files ()
  ;; A start chooses the sessions to load from the rows, the marks and one
  ;; listing of where sockets are: the same as asking each session's own
  ;; files, whether the marks are known yet or not.  Known, no session's
  ;; manifest is read; a mark made or ended changes what is loaded.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let* ((ids (cl-loop for i from 1 to 7 collect (format "aaaaaaaa-0000-4000-8000-00000000030%d" i)))
           (dirs (mapcar (lambda (id) (expand-file-name id root)) ids))
           (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil)
           (romacs-agent--restoring nil))
      (cl-flet ((id (n) (nth (1- n) ids)) (dir (n) (nth (1- n) dirs)))
        (dotimes (i 7) (record (id (1+ i)) (format "s%d" (1+ i)) (memq (1+ i) '(3 4))))
        (romacs-agent-store-fill)
        ;; 1 quiet; 2 due to resume; 3 and 4 live; 5 watched; 6 told of a
        ;; restart; 7 with no row.
        (write-region (json-serialize (romacs-mcp--object "id" (id 2) "harness" "claude" "title" "s2" "cwd" "/p/one/"
                                                          "resumeAt" 1e10))
                      nil (expand-file-name "manifest.json" (dir 2)) nil 'silent)
        (write-region "[]" nil (expand-file-name "watches.json" (dir 5)) nil 'silent)
        (write-region "verdict" nil (expand-file-name "restart-wake" (dir 6)) nil 'silent)
        (sqlite-execute (romacs-agent-store--open) "delete from sessions where id = ?" (list (id 7)))
        (let ((oracle (seq-remove #'romacs-agent--unloaded-p dirs)))
          (should (equal oracle (cdr dirs)))
          (should-not (romacs-agent--marked))
          ;; Not known: each session's files asked, as before.
          (should (equal (romacs-agent--to-load dirs) oracle))
          (should-not (romacs-agent--marked))
          ;; A store's fill makes them known, saying which it marked.
          (should (equal (sort (romacs-agent--marks-reconcile) #'string<) (list (id 2) (id 5) (id 6))))
          (should (romacs-agent--marked))
          ;; Known: from the marks, no manifest read.
          (cl-letf (((symbol-function 'romacs-agent--read-json) (lambda (&rest _) (error "A manifest read"))))
            (should (equal (romacs-agent--to-load dirs) oracle))
            (romacs-agent-mark (id 1) 'watches t)
            (should (member (dir 1) (romacs-agent--to-load dirs)))
            (romacs-agent-mark (id 1) 'watches nil)
            (romacs-agent-mark (id 5) 'watches nil)
            (should-not (member (dir 1) (romacs-agent--to-load dirs)))
            (should-not (member (dir 5) (romacs-agent--to-load dirs)))))))))

(ert-deftest romacs-agent-store-a-fill-makes-the-marks-what-the-files-say ()
  ;; Known marks a fact went unmarked under (an older build wrote it, a
  ;; stop cut its mark short): a store's fill marks it, and names the ones
  ;; newly marked, which a start would have missed.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((one "aaaaaaaa-0000-4000-8000-000000000501") (two "aaaaaaaa-0000-4000-8000-000000000502"))
      (record one "one")
      (record two "two")
      (romacs-agent--marks-known)
      (romacs-agent-mark two 'watches t)
      (write-region "[]" nil (expand-file-name "watches.json" (expand-file-name one root)) nil 'silent)
      (should-not (gethash one (romacs-agent--marked)))
      (should (equal (romacs-agent--marks-reconcile) (list one)))
      (should (gethash one (romacs-agent--marked)))
      ;; A mark too many stays: removing it could race a fact's writing.
      (should (gethash two (romacs-agent--marked)))
      (should-not (romacs-agent--marks-reconcile))
      ;; What a fill printed of them, for the editor to load.
      (should (equal (romacs-agent-store--marked-ids
                      (format "romacs-agent-store: filled 0\nromacs-agent-store: marked %s\n" one))
                     (list one))))))

(ert-deftest romacs-agent-store-a-start-asks-of-the-marked-and-the-live-alone ()
  ;; Its marks known, an editor's start asks whether to load only the
  ;; sessions something waits on and those whose supervisor's socket is
  ;; there, a live one with no row yet too: no session's directory listed
  ;; or read.  A view's later look finds a session by the row written
  ;; for it since.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let* ((ids (cl-loop for i from 1 to 7 collect (format "aaaaaaaa-0000-4000-8000-00000000060%d" i)))
           (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil)
           (romacs-agent--rows-seen nil) (noninteractive nil) (romacs-agent-writer-queued nil))
      (cl-flet ((id (n) (nth (1- n) ids)) (dir (n) (expand-file-name (nth (1- n) ids) root)))
        (dotimes (i 5) (record (id (1+ i)) (format "s%d" (1+ i)) (= i 2)))
        (romacs-agent-store-fill)
        (romacs-agent--marks-known)
        (romacs-agent-mark (id 4) 'watches t)
        ;; Live, its row not written yet.
        (record (id 6) "s6" t)
        (cl-letf (((symbol-function 'romacs-agent--read-json) (lambda (&rest _) (error "A manifest read")))
                  ((symbol-function 'romacs-agent--new-session-dirs) (lambda () (error "Every session listed"))))
          (should (equal (sort (car (romacs-agent--start-choice)) #'string<) (list (dir 3) (dir 4) (dir 6)))))
        ;; The start chose among every one recorded: a view's look finds
        ;; one by the row written for it since, listing none.
        (record (id 7) "s7")
        (romacs-agent-store-fill)
        (cl-letf (((symbol-function 'romacs-agent--new-session-dirs) (lambda () (error "Every session listed"))))
          (should (member (dir 7) (car (romacs-agent--recorded-since))))
          (should-not (member (dir 1) (car (romacs-agent--recorded-since)))))))))

(ert-deftest romacs-agent-store-a-session-marked-late-is-known-as-restored ()
  ;; A fill that finds no row missing but a session newly marked, after
  ;; the start's restore ended: the session is loaded, and known as a
  ;; restore knows one (its watches, a restart to tell), its hook run once.
  ;; With no listing answering, a start asks of every session.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let* ((id "aaaaaaaa-0000-4000-8000-000000000701") (restored 0)
           (romacs-agent--restore-pending nil) (romacs-agent--restoring nil) (romacs-agent--late-loads nil)
           (romacs-agent-restored-hook (list (lambda () (cl-incf restored))))
           (romacs-agent--quiet (make-hash-table :test #'equal)) (romacs-agent--quiet-timer nil))
      (record id "late")
      (romacs-agent-store-fill)
      (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                ((symbol-function 'romacs-startup-progress) #'ignore)
                ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
        ;; A fill that names none: nothing asked of the rows here.
        (cl-letf (((symbol-function 'romacs-agent-store--missing) (lambda () (error "Every row asked"))))
          (romacs-agent-store--filled "romacs-agent-store: filled 0\nromacs-agent-store: done\n")
          (should (= restored 0))
          (romacs-agent-store--filled (format "romacs-agent-store: filled 0\nromacs-agent-store: marked %s\nromacs-agent-store: done\n" id)))
        (should (romacs-agent-loaded id))
        (should (= restored 1))
        (should-not romacs-agent--late-loads)
        ;; One a worker folds still: known once folded, not before.
        (let ((late "aaaaaaaa-0000-4000-8000-000000000702"))
          (record late "folding")
          (puthash late t romacs-agent--folding)
          (romacs-agent-store--filled (format "romacs-agent-store: missing %s\nromacs-agent-store: done\n" late))
          (should (= restored 1))
          (remhash late romacs-agent--folding)
          (romacs-agent--restore-finish)
          (should (= restored 2))
          ;; A restore asked for meanwhile tells them itself, once.
          (setq romacs-agent--late-loads t)
          (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                    ((symbol-function 'romacs-agent--sweep) #'ignore))
            (romacs-agent-restore))
          (should (= restored 3))
          (should-not romacs-agent--late-loads)))
      (let ((noninteractive nil) (listed nil))
        (romacs-agent--marks-known)
        (cl-letf (((symbol-function 'romacs-agent-store-rows-since) #'ignore)
                  ((symbol-function 'romacs-agent--new-session-dirs) (lambda () (setq listed t) nil)))
          (should-not (cdr (romacs-agent--start-choice)))
          (should listed)))
      ;; A fill cut short, or failed: its report not taken for all there
      ;; is, every row asked here.
      (let ((asked 0))
        (cl-letf (((symbol-function 'romacs-agent-store--missing) (lambda () (cl-incf asked) nil)))
          (romacs-agent-store--filled "romacs-agent-store: filled 2\n")
          (romacs-agent-store--filled "romacs-agent-store: filled 0\nromacs-agent-store: done\n" 1)
          (should (= asked 2))))
      ;; One it marked before it stopped is loaded all the same.
      (let ((marked "aaaaaaaa-0000-4000-8000-000000000703"))
        (record marked "marked")
        (romacs-agent-store-fill)
        (romacs-agent-mark marked 'resume t)
        (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                  ((symbol-function 'romacs-startup-progress) #'ignore)
                  ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
          (romacs-agent-store--filled (format "romacs-agent-store: filled 0\nromacs-agent-store: marked %s\n" marked)))
        (should (romacs-agent-loaded marked))
        ;; Marked before its marks were known whole, by a fill that then
        ;; stopped: loaded all the same.  A mark whose session is gone
        ;; loads nothing and starts no restore.
        (let ((first "aaaaaaaa-0000-4000-8000-000000000704"))
          (record first "first")
          (romacs-agent-store-fill)
          (delete-file (expand-file-name ".complete" (romacs-agent--marks-dir)))
          (romacs-agent-mark first 'watches t)
          (romacs-agent-mark "aaaaaaaa-0000-4000-8000-000000000799" 'watches t)
          (setq romacs-agent--late-loads nil)
          (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                    ((symbol-function 'romacs-startup-progress) #'ignore)
                    ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
            (romacs-agent-store--filled "romacs-agent-store: filled 0\n"))
          (should (romacs-agent-loaded first))
          (should-not romacs-agent--restore-pending))
        ;; An orphan mark alone: nothing queued, no restore scheduled.
        (let ((timers 0))
          (setq romacs-agent--late-loads nil)
          (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) (cl-incf timers))))
            (romacs-agent-store--filled "romacs-agent-store: filled 0\n"))
          (should-not romacs-agent--restore-pending)
          (should-not romacs-agent--late-loads)
          (should (= timers 0)))))))

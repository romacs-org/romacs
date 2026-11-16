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
               ((symbol-function 'romacs-agent--host-socket) (lambda (id) (expand-file-name (concat "run/" id ".sock") root))))
       (cl-flet ((record (id title &optional running)
                   (let ((dir (expand-file-name id root)))
                     (make-directory dir t)
                     (write-region (json-serialize (romacs-mcp--object "id" id "harness" "claude" "title" title "cwd" "/p/one/"))
                                   nil (expand-file-name "manifest.json" dir) nil 'silent)
                     (with-temp-file (expand-file-name "journal.jsonl" dir)
                       (insert (json-serialize (romacs-mcp--object "seq" 1 "t" 100.0 "in" (romacs-mcp--object "prompt" "hello"))) "\n"
                               (json-serialize (romacs-mcp--object "seq" 2 "t" 101.0 "ev" (romacs-mcp--object "type" "result" "result" "done it")))
                               "\n"))
                     (when running
                       (make-directory (expand-file-name "run" root) t)
                       (write-region "" nil (romacs-agent--host-socket id) nil 'silent)))))
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
        (should-not (romacs-agent-get old))
        (should (romacs-agent-get live))
        (should-not (member old journals))
        (let ((shown (mapcar #'romacs-agent-id (romacs-agent-board--sessions))))
          (should (member old shown))
          (should (member live shown)))
        (let ((found (romacs-agent-find old)))
          (should (equal (romacs-agent-title found) "old"))
          (should (eq (romacs-agent-get old) found)))))))

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
        (should-not (romacs-agent-get old))
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
              (should-not (romacs-agent-get old))
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
        (should-not (romacs-agent-get old))
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
        (make-directory (expand-file-name "run" root) t)
        (pcase-dolist (`(,_ . ,id) ids) (write-region "" nil (romacs-agent--host-socket id) nil 'silent))
        (unwind-protect
            (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (session &rest _) (push (romacs-agent-id session) attached)))
                      ((symbol-function 'romacs-agent--endpoint) #'ignore)
                      ((symbol-function 'romacs-agent--sweep) #'ignore)
                      ((symbol-function 'romacs-agent-store-fill-soon) #'ignore))
              (romacs-agent-restore)
              (should-not (romacs-agent-get (id 'quiet)))
              (should-not (romacs-agent-get (id 'ending)))
              (dolist (name '(busy wanting behind journal)) (should (romacs-agent-get (id name))))
              ;; Nothing written: still rows.
              (romacs-agent--quiet-tick)
              (should-not (romacs-agent-get (id 'quiet)))
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
      (dolist (id ids) (write-region "" nil (romacs-agent--host-socket id) nil 'silent))
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
      (dolist (id (list late gone)) (write-region "" nil (romacs-agent--host-socket id) nil 'silent))
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
  ;; this build's columns, its records and the board's folds kept.
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
        (should-not (sqlite-select db "select 1 from sessions"))
        (should (equal (sqlite-select db "select session, seq from events") '(("old" 1))))
        (should (equal (sqlite-select db "select project from board_folds") '(("/p"))))))))

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

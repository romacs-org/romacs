;;; romacs-agent-writer.el --- The agents' store written off the editor  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The editor's main VM never commits to the agents' store.  A commit on
;; a loaded disk can stall for seconds (measured: 2.25 s for one write
;; with no other writer, on btrfs at load 80), and a commit waits for
;; every other writer's lock besides.  So each write the editor makes is
;; a call, FUNCTION and its ARGS, queued to one long-lived worker VM, the
;; writer, which runs it against its own connection, in the order
;; queued, and answers with its value or its error.  The editor reads
;; through a read-only connection: a WAL snapshot never waits on a
;; writer.
;;
;; Anywhere else (a batch process, a worker VM) a write runs at once,
;; there: those are off the editor already.
;;
;; What the editor writes it already holds: a note is folded into its
;; session before it is queued, a kept state is the session's own.  No
;; caller reads back a row it has just queued.
;;
;; An orderly end loses nothing: an exit and a restart wait for every
;; queued write (`romacs-agent-writer-drain'), a restart before its
;; successor starts.  A crash loses what is still queued, a write-back
;; window as WAL's `synchronous=NORMAL' has on power loss, kept small by
;; a writer that keeps up.  No queue is kept on disk: that would be a
;; journal again.  What such a crash can lose is Romacs's own record of
;; its last moments (notes, kept states, rows, the last two made again
;; from records); a harness keeps its own transcript of the prompts and
;; replies, and a supervisor commits its records before it sends them.

;;; Code:

(require 'cl-lib)

(declare-function isolate-spawn "isolate")
(declare-function isolate-send "isolate")
(declare-function isolate-receive "isolate")
(declare-function isolate-await "isolate")
(declare-function isolate-on-done "isolate")
(declare-function isolate-live-p "isolate")
(declare-function isolate-main-p "isolate")
(declare-function isolate-future-isolate "isolate")
(declare-function isolate-future-done-p "isolate")

(defvar romacs-agent-writer-queued 'auto
  "Whether this process queues its writes to the writer: `auto', when it
is the editor's main VM and can run worker VMs (`romacs-agent-writer-queued-p').
Nil, a write runs at once, here.")

(defun romacs-agent-writer-queued-p ()
  "Whether this process queues its writes (`romacs-agent-writer-queued'):
asked each time, so a world built in a batch process does not keep a
batch process's answer."
  (if (eq romacs-agent-writer-queued 'auto)
      (and (not noninteractive) (fboundp 'isolate-spawn) (fboundp 'isolate-main-p) (isolate-main-p))
    romacs-agent-writer-queued))

(defvar romacs-agent-writer-features
  '(romacs-agent romacs-agent-claude romacs-agent-codex romacs-agent-events romacs-agent-turns
                 romacs-agent-store romacs-agent-drawn romacs-agent-history romacs-agent-writer cl-extra help-mode)
  "The features the writer loads: those defining what it is asked to run.")

(defvar romacs-agent-writer-bindings-function nil
  "Called with no argument for the variables the writer starts with, as
`isolate-spawn' :bindings: the state root and what it reads.")

(defvar romacs-agent-writer-start-functions nil
  "Functions the writer calls as it starts, before its first write: those
opening what it writes, as a writer off the editor opens it.")

(defvar romacs-agent-writer-held-functions nil
  "Functions that each return how many records they hold that no writer
can make yet (notes for a supervisor not attached here): a requested
restart waits for them, and a drain counts them not saved.")

(defvar romacs-agent-writer-held-live-functions nil
  "Functions that each return non-nil while records they hold are on
their way (a host of theirs starting or attached): a drain waits for
them as for its own writes.")

(defun romacs-agent-writer--held ()
  "How many records are held outside the writer's queue."
  (apply #'+ (mapcar #'funcall romacs-agent-writer-held-functions)))

(defvar romacs-agent-writer--job nil "The writer's future while it runs.")
(defvar romacs-agent-writer--next 0 "The id of the next write.")
(defvar romacs-agent-writer--draining nil
  "Non-nil while the writer is drained: its end sends nothing again.")

(defvar romacs-agent-writer--pending (make-hash-table)
  "The writes not answered, by id: each (ID FUNCTION ARGS DONE TRIES KEY).")

(defvar romacs-agent-writer--unsent nil
  "The writes waiting for room in the writer's queue, oldest first.")
(defvar romacs-agent-writer--unsent-tail nil "The last cons of `romacs-agent-writer--unsent'.")

(defvar romacs-agent-writer--failed (make-hash-table)
  "The writes the writer answered with an error, by id, each (ID FUNCTION
ARGS DONE TRIES): tried again later, never dropped.")

(defconst romacs-agent-writer--failed-tries 5
  "How many times a failed write is sent again in a session; then it waits
for the editor's end, which makes it itself.")

(defun romacs-agent-writer--unanswered ()
  "How many writes are sent or queued to the writer and not answered."
  (hash-table-count romacs-agent-writer--pending))

(defun romacs-agent-writer-pending-count ()
  "How many writes are queued and not yet made: unanswered, or failed."
  (+ (hash-table-count romacs-agent-writer--pending) (hash-table-count romacs-agent-writer--failed)))

(defconst romacs-agent-writer--window 16
  "The batches sent and not answered at most: a worker's queue holds
`isolate-queue-limit' messages.  Writes past them wait here, unsent.")

(defconst romacs-agent-writer--batch 500 "The writes a batch carries at most.")

(defvar romacs-agent-writer--in-flight 0 "The batches sent and not answered.")

(defun romacs-agent-write (function args &optional done key)
  "Run (apply FUNCTION ARGS), a write to the agents' store, off the editor.
FUNCTION is a symbol the writer defines.  DONE, when given, is called on
the editor with (ok VALUE) or (error ERR) once it ran.  KEY names what a
write puts in place of what was there (a session's row): a later write
of the same KEY supersedes an earlier one that failed, never retried
after it.  In a process
that does not queue (`romacs-agent-writer-queued-p') it runs here, now,
and its answer is returned; queued, `queued' is: it never waits, and a
full channel to the writer holds it here until there is room."
  (if (not (romacs-agent-writer-queued-p))
      (let ((answer (condition-case err (list 'ok (apply function args))
                      (error (list 'error err)))))
        (when done (funcall done answer))
        answer)
    (let ((entry (list (cl-incf romacs-agent-writer--next) function args done 0 key)))
      (when key (romacs-agent-writer--supersede key))
      (puthash (car entry) entry romacs-agent-writer--pending)
      (romacs-agent-writer--queue entry))
    (romacs-agent-writer--flush)
    'queued))

(defun romacs-agent-writer--queue (entry)
  "Put ENTRY at the end of the unsent writes."
  (let ((cell (list entry)))
    (if romacs-agent-writer--unsent (setcdr romacs-agent-writer--unsent-tail cell)
      (setq romacs-agent-writer--unsent cell))
    (setq romacs-agent-writer--unsent-tail cell)))

(defun romacs-agent-written-p (answer)
  "Whether ANSWER, `romacs-agent-write''s, says its write was made or is
queued to be: its value when it ran here."
  (pcase answer
    ('queued t)
    (`(ok ,value) value)))

(defun romacs-agent-writer--isolate ()
  "The writer's worker, started when it is not running."
  (unless (and romacs-agent-writer--job (not (isolate-future-done-p romacs-agent-writer--job)))
    (setq romacs-agent-writer--in-flight 0)
    (setq romacs-agent-writer--job
          (isolate-spawn 'romacs-agent-writer--serve
                         :fresh t :name "romacs-agent writer" :require romacs-agent-writer-features
                         :bindings (and romacs-agent-writer-bindings-function
                                        (funcall romacs-agent-writer-bindings-function))))
    (isolate-on-done romacs-agent-writer--job #'romacs-agent-writer--ended))
  (isolate-future-isolate romacs-agent-writer--job))

(defvar romacs-agent-writer--here-running nil
  "Non-nil while the editor makes the writes itself: none is sent then.")

(defun romacs-agent-writer--flush ()
  "Send the unsent writes, in order, a batch at a time, while fewer than
`romacs-agent-writer--window' batches are unanswered; none while the
editor makes them itself (`romacs-agent-writer--here')."
  (let ((isolate (not romacs-agent-writer--here-running)))
    (while (and isolate romacs-agent-writer--unsent (< romacs-agent-writer--in-flight romacs-agent-writer--window))
      (setq isolate (condition-case err (romacs-agent-writer--isolate)
                      ;; None to be had: the rest stays unsent, tried later.
                      (error (romacs-agent-writer--unavailable err) nil)))
      (when isolate
        (let ((batch nil) (n 0))
          (while (and romacs-agent-writer--unsent (< n romacs-agent-writer--batch))
            (pcase-let ((`(,id ,function ,args ,_ ,_ ,_) (pop romacs-agent-writer--unsent)))
              (push (list id function args) batch)
              (cl-incf n)))
          (unless romacs-agent-writer--unsent (setq romacs-agent-writer--unsent-tail nil))
          (isolate-send isolate (list 'romacs-agent-write-batch (nreverse batch)))
          (cl-incf romacs-agent-writer--in-flight))))))

(defconst romacs-agent-writer--retry 5
  "The seconds before a writer that could not be started is tried again.")

(defvar romacs-agent-writer--retry-timer nil "The timer trying a writer again.")

(defun romacs-agent-writer--unavailable (why)
  "No writer could be started (WHY): the writes stay queued, and a writer
is tried again in `romacs-agent-writer--retry' seconds.  The editor still
commits nothing; an exit or a restart makes them here at the last
\(`romacs-agent-writer-drain')."
  (unless (timerp romacs-agent-writer--retry-timer)
    (message "romacs-agent-writer: no writer (%s): %d writes wait" (error-message-string why)
             (romacs-agent-writer-pending-count))
    (setq romacs-agent-writer--retry-timer
          (run-with-timer romacs-agent-writer--retry nil
                          (lambda () (setq romacs-agent-writer--retry-timer nil) (romacs-agent-writer--flush))))))

(defconst romacs-agent-writer--here-tries 3 "How many times a write made here is tried.")

(defun romacs-agent-writer--here ()
  "The editor is ending and no writer answers: make every unanswered write
here, in order, opened as a writer opens the store (long waits for its
lock).  A write is let go only once it is made; one that fails every
try stays queued, and the drain says so.  Only an ending editor commits.
How many were made."
  (let ((entries nil) (romacs-agent-writer--here-running t))
    (let ((romacs-agent-writer-queued nil))
      (with-demoted-errors "romacs-agent-writer: opening here: %S" (run-hooks 'romacs-agent-writer-start-functions)))
    (maphash (lambda (_id entry) (push entry entries)) romacs-agent-writer--pending)
    (setq romacs-agent-writer--unsent nil romacs-agent-writer--unsent-tail nil romacs-agent-writer--in-flight 0)
    ;; Of the writes of one KEY, the last alone: it supersedes the others.
    (let ((last (make-hash-table :test #'equal)))
      (dolist (entry entries)
        (when-let* ((key (nth 5 entry)))
          (when (> (car entry) (car (gethash key last '(0)))) (puthash key entry last))))
      (dolist (entry entries)
        (when-let* ((key (nth 5 entry)))
          (unless (eq entry (gethash key last))
            (remhash (car entry) romacs-agent-writer--pending)
            (setq entries (delq entry entries))))))
    ;; In passes: every write tried, one pause, the failures tried again,
    ;; `romacs-agent-writer--here-tries' passes at most.  A disk that
    ;; fails every write holds an ending editor for those pauses alone,
    ;; however many writes are queued.
    (let ((todo (sort entries (lambda (a b) (< (car a) (car b))))) (pass 0) (last nil) (made 0))
      (while (and todo (< pass romacs-agent-writer--here-tries))
        (when (> pass 0) (sleep-for 1))
        (cl-incf pass)
        (setq todo (cl-remove-if
                    (lambda (entry)
                      (pcase-let* ((`(,id ,function ,args ,done ,_ ,_) entry)
                                   (answer (condition-case err
                                               ;; The write here; what its DONE queues is
                                               ;; queued, and made by the next round.
                                               (list 'ok (let ((romacs-agent-writer-queued nil)) (apply function args)))
                                             (error (list 'error err)))))
                        (if (eq (car answer) 'error)
                            (progn (setq last (cons function (cadr answer))) nil)
                          (remhash id romacs-agent-writer--pending)
                          (cl-incf made)
                          (when done (with-demoted-errors "romacs-agent-writer: after a write: %S" (funcall done answer)))
                          t)))
                    todo)))
      (when todo
        (message "romacs-agent-writer: %d not saved, the last failing: %s: %S" (length todo) (car last) (cdr last)))
      made)))

(defun romacs-agent-writer--unsend ()
  "Mark every unanswered write unsent: its writer is gone."
  (setq romacs-agent-writer--in-flight 0 romacs-agent-writer--unsent nil romacs-agent-writer--unsent-tail nil)
  (let ((entries nil))
    (maphash (lambda (_id entry) (push entry entries)) romacs-agent-writer--pending)
    (mapc #'romacs-agent-writer--queue (sort entries (lambda (a b) (< (car a) (car b)))))))

(defun romacs-agent-writer--ended (future)
  "The writer's job ended (FUTURE): asked to, or it died.  What it did
not answer is sent again to a new one, in order: a write is an upsert,
or a record whose second copy a fold skips."
  (when (eq future romacs-agent-writer--job)
    (setq romacs-agent-writer--job nil)
    (romacs-agent-writer--unsend)
    (when (and (> (romacs-agent-writer-pending-count) 0) (not romacs-agent-writer--draining))
      (message "romacs-agent-writer: the writer ended, %d writes sent again" (romacs-agent-writer-pending-count))
      (romacs-agent-writer--flush))))

(defun romacs-agent-writer--receive (_sender message)
  "The writer's answers to a batch, MESSAGE: what waited on each is told,
and the writes waiting for room are sent."
  (pcase message
    (`(romacs-agent-written-batch ,answers)
     (setq romacs-agent-writer--in-flight (max 0 (1- romacs-agent-writer--in-flight)))
     (pcase-dolist (`(,id ,answer) answers)
       (when-let* ((entry (gethash id romacs-agent-writer--pending)))
         (remhash id romacs-agent-writer--pending)
         (when (eq (car answer) 'error)
           (message "romacs-agent-writer: %s: %S" (nth 1 entry) (cadr answer)))
         (if (and (eq (car answer) 'error) (null (nth 3 entry)))
             ;; Nobody is told of it: it is kept, and made later.
             (romacs-agent-writer--failed entry)
           (when-let* ((done (nth 3 entry)))
             (with-demoted-errors "romacs-agent-writer: after a write: %S" (funcall done answer))))))
     (romacs-agent-writer--flush))))

(add-hook 'isolate-message-functions #'romacs-agent-writer--receive)

(defun romacs-agent-writer--failed (entry)
  "Keep ENTRY, a write that failed, and send it again after a while: 1, 2,
4... seconds, `romacs-agent-writer--failed-tries' times; then it waits
for the editor's end (`romacs-agent-writer-drain')."
  (let* ((tries (1+ (nth 4 entry))) (key (nth 5 entry))
         (kept (list (nth 0 entry) (nth 1 entry) (nth 2 entry) (nth 3 entry) tries key)))
    ;; A later write of its KEY, queued already, supersedes it.
    (unless (and key (romacs-agent-writer--later-p key (car entry)))
      (puthash (car kept) kept romacs-agent-writer--failed)
      (when (< tries romacs-agent-writer--failed-tries)
        (run-with-timer (expt 2 (1- tries)) nil #'romacs-agent-writer--again (car kept))))))

(defun romacs-agent-writer--supersede (key)
  "Drop the failed writes of KEY: a later one of it is queued."
  (let ((stale nil))
    (maphash (lambda (id entry) (when (equal (nth 5 entry) key) (push id stale))) romacs-agent-writer--failed)
    (dolist (id stale) (remhash id romacs-agent-writer--failed))))

(defun romacs-agent-writer--later-p (key id)
  "Whether a write of KEY later than ID is queued and not answered."
  (catch 'later
    (maphash (lambda (other entry) (when (and (> other id) (equal (nth 5 entry) key)) (throw 'later t)))
             romacs-agent-writer--pending)
    nil))

(defun romacs-agent-writer--again (id)
  "Send the failed write ID again, unless it is made or gone meanwhile."
  (when-let* ((entry (gethash id romacs-agent-writer--failed)))
    (remhash id romacs-agent-writer--failed)
    (puthash id entry romacs-agent-writer--pending)
    (romacs-agent-writer--queue entry)
    (romacs-agent-writer--flush)))

(defun romacs-agent-writer--serve ()
  "In the writer: run each batch of writes as it comes, in order, and
answer it, until asked to stop."
  (let ((romacs-agent-writer-queued nil) (message nil))
    (with-demoted-errors "romacs-agent-writer: starting: %S" (run-hooks 'romacs-agent-writer-start-functions))
    (while (not (eq (car-safe (setq message (isolate-receive))) 'romacs-agent-writer-stop))
      (pcase message
        (`(romacs-agent-write-batch ,writes)
         (isolate-send 'parent
                       (list 'romacs-agent-written-batch
                             (mapcar (pcase-lambda (`(,id ,function ,args))
                                       (list id (condition-case err (list 'ok (apply function args))
                                                  (error (list 'error err)))))
                                     writes))))))
    t))

(defun romacs-agent-writer-drain (&optional quittable)
  "Let the writer make every write queued, however long that takes,
saying so: the editor is ending, or handing over to its successor,
which reads the store.  Writes the answers' callbacks queue meanwhile
are made too.  A writer gone meanwhile is started again and sent what
it did not answer.  The writer then stops; a later write starts
another.  QUITTABLE: a restart is beginning, and \\[keyboard-quit]
cancels it; the writes stay queued, made in the background."
  (when (> (+ (romacs-agent-writer-pending-count) (romacs-agent-writer--held)) 0)
    (let ((count (+ (romacs-agent-writer-pending-count) (romacs-agent-writer--held))) (tries 0)
          (romacs-agent-writer--draining t))
      (message (if quittable "Saving %d agent records... (C-g cancels the restart)"
                 "Saving %d agent records...")
               count)
      (redisplay t)
      ;; What failed before is tried once more, now.
      (maphash (lambda (id _) (romacs-agent-writer--again id)) (copy-hash-table romacs-agent-writer--failed))
      ;; Until nothing is queued and no writer runs: writes the answers'
      ;; callbacks queue, even while the writer stops, are made too.
      (while (and (or (> (romacs-agent-writer--unanswered) 0) romacs-agent-writer--job
                      (run-hook-with-args-until-success 'romacs-agent-writer-held-live-functions))
                  (< tries 4))
        (cond
         ((> (romacs-agent-writer--unanswered) 0)
          (unless (and romacs-agent-writer--job (not (isolate-future-done-p romacs-agent-writer--job)))
            ;; Gone: a new writer, sent what is still unanswered.
            (cl-incf tries)
            (setq romacs-agent-writer--job nil)
            (romacs-agent-writer--unsend))
          (romacs-agent-writer--flush)
          (condition-case nil (isolate-await romacs-agent-writer--job 0.1)
            (isolate-timeout nil)
            ;; A writer that cannot be waited on counts as gone.
            (error (cl-incf tries))))
         (romacs-agent-writer--job
          ;; All answered: the writer stops, forgotten at once, so what is
          ;; queued while it stops goes to a new one; then look again.
          (let ((job romacs-agent-writer--job))
            (setq romacs-agent-writer--job nil)
            (unless (isolate-future-done-p job)
              (isolate-send (isolate-future-isolate job) (list 'romacs-agent-writer-stop))
              (ignore-errors (isolate-await job)))))
         ;; Records on their way through their own host: its answers.
         (t (accept-process-output nil 0.1))))
      ;; What failed, or no writer would answer: made here, the editor
      ;; ending, a few rounds while what is made queues more.
      (let ((rounds 0))
        (while (and (> (romacs-agent-writer-pending-count) 0) (< (cl-incf rounds) 5))
          (maphash (lambda (id entry) (puthash id entry romacs-agent-writer--pending)) romacs-agent-writer--failed)
          (clrhash romacs-agent-writer--failed)
          ;; Nothing made this round: the rest is not saved.
          (when (zerop (romacs-agent-writer--here)) (setq rounds 5))))
      (let ((left (+ (romacs-agent-writer-pending-count) (romacs-agent-writer--held))))
        (message (if (> left 0) "Saving %d agent records...failed: %d not saved" "Saving %d agent records...done")
                 count left)))))

(defun romacs-agent-writer--at-exit ()
  "The editor is ending: every queued write is made, uninterrupted, since
quitting now would lose them."
  (let ((inhibit-quit t)) (romacs-agent-writer-drain)))

(defun romacs-agent-writer--before-handover ()
  "A restart begins: every queued write is made before its image and its
successor, which reads them; \[keyboard-quit] cancels the restart, and
the session goes on with its writes still queued."
  (condition-case err (let ((inhibit-quit nil)) (romacs-agent-writer-drain t))
    (quit
     ;; Cancelled: what is still to be made is queued again, in order, and
     ;; sent as soon as a writer can be had.
     (romacs-agent-writer--unsend)
     (run-with-timer 0 nil #'romacs-agent-writer--flush)
     (signal (car err) (cdr err)))))

(defun romacs-agent-writer--obstacle ()
  "Why a requested restart waits (`warm-restart-obstacle-functions'): the
writes not yet made, which the writer makes meanwhile, off the editor."
  (let ((count (+ (romacs-agent-writer-pending-count) (romacs-agent-writer--held))))
    (and (> count 0) (list (format "%d agent records not yet written" count)))))

;; Late, after anything else that writes at the end.
(add-hook 'kill-emacs-hook #'romacs-agent-writer--at-exit 90)
(add-hook 'warm-restart-before-handover-functions #'romacs-agent-writer--before-handover)
(add-hook 'warm-restart-obstacle-functions #'romacs-agent-writer--obstacle)

(provide 'romacs-agent-writer)
;;; romacs-agent-writer.el ends here

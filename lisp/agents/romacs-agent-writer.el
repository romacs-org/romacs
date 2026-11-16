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

(defvar romacs-agent-writer--job nil "The writer's future while it runs.")
(defvar romacs-agent-writer--next 0 "The id of the next write.")
(defvar romacs-agent-writer--draining nil
  "Non-nil while the writer is drained: its end sends nothing again.")

(defvar romacs-agent-writer--pending (make-hash-table)
  "The writes not answered, by id: each (ID FUNCTION ARGS DONE).")

(defvar romacs-agent-writer--unsent nil
  "The writes waiting for room in the writer's queue, oldest first.")
(defvar romacs-agent-writer--unsent-tail nil "The last cons of `romacs-agent-writer--unsent'.")

(defun romacs-agent-writer-pending-count ()
  "How many writes are queued and not yet answered."
  (hash-table-count romacs-agent-writer--pending))

(defconst romacs-agent-writer--window 16
  "The batches sent and not answered at most: a worker's queue holds
`isolate-queue-limit' messages.  Writes past them wait here, unsent.")

(defconst romacs-agent-writer--batch 500 "The writes a batch carries at most.")

(defvar romacs-agent-writer--in-flight 0 "The batches sent and not answered.")

(defun romacs-agent-write (function args &optional done)
  "Run (apply FUNCTION ARGS), a write to the agents' store, off the editor.
FUNCTION is a symbol the writer defines.  DONE, when given, is called on
the editor with (ok VALUE) or (error ERR) once it ran.  In a process
that does not queue (`romacs-agent-writer-queued-p') it runs here, now,
and its answer is returned; queued, `queued' is: it never waits, and a
full channel to the writer holds it here until there is room."
  (if (not (romacs-agent-writer-queued-p))
      (let ((answer (condition-case err (list 'ok (apply function args))
                      (error (list 'error err)))))
        (when done (funcall done answer))
        answer)
    (let ((entry (list (cl-incf romacs-agent-writer--next) function args done)))
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

(defun romacs-agent-writer--flush ()
  "Send the unsent writes, in order, a batch at a time, while fewer than
`romacs-agent-writer--window' batches are unanswered."
  (while (and romacs-agent-writer--unsent (< romacs-agent-writer--in-flight romacs-agent-writer--window))
    (let ((isolate (condition-case err (romacs-agent-writer--isolate)
                     (error (romacs-agent-writer--here err) nil)))
          (batch nil) (n 0))
      (while (and isolate romacs-agent-writer--unsent (< n romacs-agent-writer--batch))
        (pcase-let ((`(,id ,function ,args ,_) (pop romacs-agent-writer--unsent)))
          (push (list id function args) batch)
          (cl-incf n)))
      (when isolate
        (unless romacs-agent-writer--unsent (setq romacs-agent-writer--unsent-tail nil))
        (isolate-send isolate (list 'romacs-agent-write-batch (nreverse batch)))
        (cl-incf romacs-agent-writer--in-flight)))))

(defun romacs-agent-writer--here (why)
  "No writer can be had (WHY): every unanswered write is made here, in
order, rather than lost.  A commit in the editor, only when it is that
or losing records (a world gone stale under a running editor)."
  (message "romacs-agent-writer: no writer (%s): %d writes made here"
           (error-message-string why) (romacs-agent-writer-pending-count))
  (let ((entries nil) (romacs-agent-writer-queued nil))
    (maphash (lambda (_id entry) (push entry entries)) romacs-agent-writer--pending)
    (setq romacs-agent-writer--unsent nil romacs-agent-writer--unsent-tail nil romacs-agent-writer--in-flight 0)
    (clrhash romacs-agent-writer--pending)
    (dolist (entry (sort entries (lambda (a b) (< (car a) (car b)))))
      (pcase-let* ((`(,_ ,function ,args ,done) entry)
                   (answer (condition-case err (list 'ok (apply function args)) (error (list 'error err)))))
        (when done (with-demoted-errors "romacs-agent-writer: after a write: %S" (funcall done answer)))))))

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
         (when-let* ((done (nth 3 entry)))
           (with-demoted-errors "romacs-agent-writer: after a write: %S" (funcall done answer)))))
     (romacs-agent-writer--flush))))

(add-hook 'isolate-message-functions #'romacs-agent-writer--receive)

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

(defun romacs-agent-writer-drain ()
  "Let the writer make every write queued, however long that takes, saying
so: the editor is ending, or handing over to its successor, which reads
the store.  Writes the answers' callbacks queue meanwhile are made too.
A writer gone meanwhile is started again and sent what it did not
answer.  The writer then stops; a later write starts another."
  (when (> (romacs-agent-writer-pending-count) 0)
    (let ((count (romacs-agent-writer-pending-count)) (tries 0) (romacs-agent-writer--draining t))
      (message "Saving %d agent records..." count)
      (redisplay t)
      ;; Until nothing is queued and no writer runs: writes the answers'
      ;; callbacks queue, even while the writer stops, are made too.
      (while (and (or (> (romacs-agent-writer-pending-count) 0) romacs-agent-writer--job) (< tries 4))
        (if (> (romacs-agent-writer-pending-count) 0)
            (progn
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
          ;; All answered: the writer stops, forgotten at once, so what is
          ;; queued while it stops goes to a new one; then look again.
          (let ((job romacs-agent-writer--job))
            (setq romacs-agent-writer--job nil)
            (unless (isolate-future-done-p job)
              (isolate-send (isolate-future-isolate job) (list 'romacs-agent-writer-stop))
              (ignore-errors (isolate-await job))))))
      ;; No writer would answer: made here.
      (when (> (romacs-agent-writer-pending-count) 0)
        (romacs-agent-writer--here '(error "the writer did not answer")))
      (message (if (> (romacs-agent-writer-pending-count) 0) "Saving %d agent records...failed: %d not saved"
                 "Saving %d agent records...done")
               count (romacs-agent-writer-pending-count)))))

;; Late, after anything else that writes at the end.
(add-hook 'kill-emacs-hook #'romacs-agent-writer-drain 90)
(add-hook 'warm-restart-before-handover-functions #'romacs-agent-writer-drain)

(provide 'romacs-agent-writer)
;;; romacs-agent-writer.el ends here

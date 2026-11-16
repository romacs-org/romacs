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
                 romacs-agent-store romacs-agent-drawn romacs-agent-writer cl-extra help-mode)
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

(defvar romacs-agent-writer--pending nil
  "The writes sent and not answered, oldest first: (ID FUNCTION ARGS DONE).")

(defun romacs-agent-write (function args &optional done)
  "Run (apply FUNCTION ARGS), a write to the agents' store, off the editor.
FUNCTION is a symbol the writer defines.  DONE, when given, is called on
the editor with (ok VALUE) or (error ERR) once it ran.  In a process
that does not queue (`romacs-agent-writer-queued-p') it runs here, now,
and its answer is returned; queued, `queued' is."
  (if (not (romacs-agent-writer-queued-p))
      (let ((answer (condition-case err (list 'ok (apply function args))
                      (error (list 'error err)))))
        (when done (funcall done answer))
        answer)
    (let ((entry (list (cl-incf romacs-agent-writer--next) function args done)))
      (setq romacs-agent-writer--pending (append romacs-agent-writer--pending (list entry)))
      (romacs-agent-writer--send entry)
      'queued)))

(defun romacs-agent-written-p (answer)
  "Whether ANSWER, `romacs-agent-write''s, says its write was made or is
queued to be: its value when it ran here."
  (pcase answer
    ('queued t)
    (`(ok ,value) value)))

(defun romacs-agent-writer--isolate ()
  "The writer's worker, started when it is not running."
  (unless (and romacs-agent-writer--job (not (isolate-future-done-p romacs-agent-writer--job)))
    (setq romacs-agent-writer--job
          (isolate-spawn 'romacs-agent-writer--serve
                         :fresh t :name "romacs-agent writer" :require romacs-agent-writer-features
                         :bindings (and romacs-agent-writer-bindings-function
                                        (funcall romacs-agent-writer-bindings-function))))
    (isolate-on-done romacs-agent-writer--job #'romacs-agent-writer--ended))
  (isolate-future-isolate romacs-agent-writer--job))

(defun romacs-agent-writer--send (entry)
  "Send ENTRY, (ID FUNCTION ARGS DONE), to the writer."
  (pcase-let ((`(,id ,function ,args ,_) entry))
    (isolate-send (romacs-agent-writer--isolate) (list 'romacs-agent-write id function args))))

(defun romacs-agent-writer--ended (future)
  "The writer's job ended (FUTURE): asked to, or it died.  What it did
not answer is sent again to a new one, in order: a write is an upsert,
or a record whose second copy a fold skips."
  (when (eq future romacs-agent-writer--job)
    (setq romacs-agent-writer--job nil)
    (when (and romacs-agent-writer--pending (not romacs-agent-writer--draining))
      (message "romacs-agent-writer: the writer ended, %d writes sent again" (length romacs-agent-writer--pending))
      (mapc #'romacs-agent-writer--send romacs-agent-writer--pending))))

(defun romacs-agent-writer--receive (_sender message)
  "The writer's answer MESSAGE: what waited on it is told."
  (pcase message
    (`(romacs-agent-written ,id ,answer)
     (when-let* ((entry (assq id romacs-agent-writer--pending)))
       (setq romacs-agent-writer--pending (delq entry romacs-agent-writer--pending))
       (when (eq (car answer) 'error)
         (message "romacs-agent-writer: %s: %S" (nth 1 entry) (cadr answer)))
       (when-let* ((done (nth 3 entry)))
         (with-demoted-errors "romacs-agent-writer: after a write: %S" (funcall done answer)))))))

(add-hook 'isolate-message-functions #'romacs-agent-writer--receive)

(defun romacs-agent-writer--serve ()
  "In the writer: run each write as it comes, answering each, until asked
to stop."
  (let ((romacs-agent-writer-queued nil) (message nil))
    (with-demoted-errors "romacs-agent-writer: starting: %S" (run-hooks 'romacs-agent-writer-start-functions))
    (while (not (eq (car-safe (setq message (isolate-receive))) 'romacs-agent-writer-stop))
      (pcase message
        (`(romacs-agent-write ,id ,function ,args)
         (isolate-send 'parent
                       (list 'romacs-agent-written id
                             (condition-case err (list 'ok (apply function args))
                               (error (list 'error err))))))))
    t))

(defun romacs-agent-writer-drain ()
  "Let the writer make every write it was sent, however long that takes,
saying so: the editor is ending, or handing over to its successor, which
reads the store.  A writer gone meanwhile is started again and sent what
it did not answer.  The writer stops; a later write starts another."
  (when romacs-agent-writer--pending
    (let ((count (length romacs-agent-writer--pending)) (tries 0) (romacs-agent-writer--draining t))
      (message "Saving %d agent records..." count)
      (redisplay t)
      (while (and romacs-agent-writer--pending (< (cl-incf tries) 4))
        (unless (and romacs-agent-writer--job (not (isolate-future-done-p romacs-agent-writer--job)))
          ;; Gone: a new writer, sent what is still unanswered.
          (setq romacs-agent-writer--job nil)
          (mapc #'romacs-agent-writer--send romacs-agent-writer--pending))
        (let ((job romacs-agent-writer--job))
          (isolate-send (isolate-future-isolate job) (list 'romacs-agent-writer-stop))
          ;; Its loop ends after every write sent before the stop: all made.
          (when (condition-case nil (progn (isolate-await job) t) (error nil))
            (setq romacs-agent-writer--pending nil))
          (when (eq job romacs-agent-writer--job) (setq romacs-agent-writer--job nil))))
      (message (if romacs-agent-writer--pending "Saving %d agent records...failed: %d not saved"
                 "Saving %d agent records...done")
               count (length romacs-agent-writer--pending)))))

;; Late, after anything else that writes at the end.
(add-hook 'kill-emacs-hook #'romacs-agent-writer-drain 90)
(add-hook 'warm-restart-before-handover-functions #'romacs-agent-writer-drain)

(provide 'romacs-agent-writer)
;;; romacs-agent-writer.el ends here

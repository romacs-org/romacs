;;; isolate-worker.el --- A worker isolate's loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The worker's side of isolates (lisp/vm/isolate.el, the spawner's): its
;; loop, which takes its parent's jobs and messages and answers each job
;; with a frozen result, and between jobs runs its processes' output and
;; its timers, an owner's as any worker's (lisp/vm/owner.el); a zygote's
;; forking and its preparation.  Not in GNU (ledger/divergences VM2-ISOLATE).

;;; Code:

(defun romacs--isolate-transportable (data)
  "DATA with every element that cannot be frozen replaced by
(isolate-unfreezable PRINTED)."
  (if (condition-case nil (progn (freeze data) t) (error nil))
      data
    (mapcar (lambda (element)
              (if (condition-case nil (progn (freeze element) t) (error nil))
                  element
                (list 'isolate-unfreezable (prin1-to-string element))))
            (if (listp data) data (list data)))))

(defun romacs--isolate-reply (generation outcome changed)
  "Post OUTCOME, (KIND PAYLOAD), for GENERATION; CHANGED says the features did.
A result that cannot be sent is reported as an error; a parent that is
gone gets nothing."
  (condition-case err
      (romacs--isolate-post 0 (car outcome) generation (list (nth 1 outcome) changed))
    (error
     (condition-case nil
         (romacs--isolate-post 0 'error generation
                               (list (list 'error (list (format "Cannot send result: %s" (error-message-string err)))
                                           nil '(error))
                                     changed))
       (error nil)))))

(defun romacs--isolate-backtrace ()
  "The frames of the running job outward from the error being signalled,
one a line, at most 40 of 200 characters each: `isolate-error-backtrace'."
  (let ((lines nil) (count 0) (print-length 8) (print-level 3))
    (mapbacktrace (lambda (_evald function args _flags)
                    (when (< count 40)
                      (setq count (1+ count))
                      (let ((line (prin1-to-string (cons function args))))
                        (push (if (> (length line) 200) (substring line 0 200) line) lines))))
                  #'romacs--isolate-backtrace)
    (mapconcat #'identity (nreverse lines) "\n")))

(defun romacs--isolate-run-job (generation job)
  "Run JOB, (FUNCTION ARGS BINDINGS REQUIRE DIRECTORY), and reply.  An
error's reply carries the backtrace where it was signalled."
  (let* ((function (nth 0 job))
         (require (nth 3 job))
         (directory (nth 4 job))
         (before (length features))
         (backtrace nil)
         (romacs--isolate-nested (1+ romacs--isolate-nested))
         (outcome
          (cond
           ((memq generation romacs--isolate-cancelled)
            (setq romacs--isolate-cancelled (delq generation romacs--isolate-cancelled))
            (list 'error (list 'quit nil nil '(quit))))
           ;; This worker's features are not what the job was queued for.
           (romacs--isolate-retired
            (list 'error (list 'isolate-retired nil nil '(isolate-retired isolate-error error))))
           (t
            (condition-case err
                ;; The job runs under its claim, a marker above this
                ;; handler's entry (`romacs--isolate-claim'): an interrupt
                ;; naming it quits here, once (the safepoint clears the claim
                ;; as it quits). The claim ends as that entry goes: at the
                ;; job's end, as an escape lands here, or by a caught panic.
                ;; Its memory budget is no claim's: a child's binds from its
                ;; fork (`romacs--isolate-run-forks') for its whole life.
                (progn
                  (romacs--isolate-claim generation)
                  (prog1
                      (handler-bind-1
                       (lambda ()
                         (let ((args (thaw (nth 1 job)))
                               (bindings (thaw (nth 2 job))))
                           (dolist (feature require) (require feature))
                           (dolist (binding bindings) (set (car binding) (cdr binding)))
                           (when directory (setq default-directory directory))
                           (list 'result
                                 (apply (if (and (consp function) (eq (car function) 'lambda)) (eval function t) function)
                                        args))))
                       '(error) (lambda (_error) (setq backtrace (romacs--isolate-backtrace))))
                    ;; The job's end, inside its claim and handler: a
                    ;; collection due, its memory (which replaces the
                    ;; result), what collections left pending.
                    (romacs--safepoint)))
              (quit (romacs--isolate-quit-outcome))
              (error (list 'error (list (car err) (romacs--isolate-transportable (cdr err)) backtrace
                                        (get (car err) 'error-conditions)))))))))
    ;; A quit an interrupt left deferred in this job's compile, which an
    ;; escape then passed over, ends with the job: the next job, in this
    ;; same request, must not take it.
    (romacs--take-deferred-escape nil)
    ;; The preparation loads the worker's baseline features; a later job
    ;; that loads more retires the worker: the spawner reuses it no more and
    ;; the jobs queued behind go back to run elsewhere.  An owner serves on:
    ;; its calls may load what they need.
    (let ((changed (and (not (eq function 'romacs--isolate-prepare)) (/= before (length features)))))
      (when (and changed (not romacs--isolate-owner)) (setq romacs--isolate-retired t))
      (when outcome (romacs--isolate-reply generation outcome changed)))
    ;; A child forked for one job ends after it.
    (when romacs--isolate-job-worker
      (setq romacs--isolate-serving nil))))

(defun romacs--isolate-quit-outcome ()
  "A job's outcome when it quit: an interrupt's quit, or nil, no reply,
when a termination quit it (its spawner killed it, or its zygote died):
the spawner settles the job from the death that follows."
  (unless (romacs--isolate-terminated-p)
    (list 'error (list 'quit nil nil '(quit)))))

(defun romacs--isolate-note-control-frames ()
  "Take every frame that arrived: interrupts, bye and messages now, jobs
onto the queue."
  (let (frame)
    (while (setq frame (romacs--isolate-take 0))
      (let ((kind (car frame)))
        (cond ((eq kind 'job)
               (setq romacs--isolate-deferred (nconc romacs--isolate-deferred (list frame))))
              ((eq kind 'fork)
               (setq romacs--isolate-forks (nconc romacs--isolate-forks (list (nth 1 frame)))))
              ((memq kind '(message text))
               (let ((message (if (eq kind 'text) frame (nth 2 frame))))
                 (setq romacs--isolate-inbox (nconc romacs--isolate-inbox (list message)))))
              ((eq kind 'interrupt)
               ;; For a job not yet run; one running took it at a safepoint.
               (push (nth 1 frame) romacs--isolate-cancelled))
              ((eq kind 'bye)
               (setq romacs--isolate-serving nil)))))))

(defun romacs--isolate-run-queued ()
  "Run the jobs queued now in order, reading what arrived before each: a
cancellation sent behind a job cancels it before it runs.  Jobs that
arrive meanwhile wait for the next round, so a stream of calls holds an
owner's process output back at most one round."
  (romacs--isolate-wait 0)
  (romacs--isolate-note-control-frames)
  (let ((left (length romacs--isolate-deferred)))
    (while (and (> left 0) romacs--isolate-deferred romacs--isolate-serving
                (not (romacs--isolate-terminated-p)))
      (setq left (1- left))
      ;; A signal nobody took (its job was in a leaf, or done) is stale.
      (romacs--isolate-interrupted-p)
      (romacs--isolate-wait 0)
      (romacs--isolate-note-control-frames)
      ;; A termination read with the frames starts no further job.
      (unless (romacs--isolate-terminated-p)
        (let ((frame (pop romacs--isolate-deferred)))
          (romacs--isolate-run-job (nth 1 frame) (nth 2 frame)))))))

(defun romacs--isolate-serve-frames ()
  "Take every frame from the parent; run the queued jobs unless inside one."
  (romacs--isolate-note-control-frames)
  (when (= romacs--isolate-nested 0)
    (romacs--isolate-run-queued)))

(defun romacs--isolate-serve ()
  "The worker's loop, an owner's too (ownership design O1 §2.4), until bye
or termination.  Each round is `romacs--isolate-round-plan''s: it takes
its calls once, at its head, then at most `owner-output-batch'
dispatches of process output and the due timers; then, unless calls are
queued, it waits on the parent, the processes and the next timer, and
only notes what arrived, for the next round's head.  So a stream of
output never holds a queued call back more than one round, nor a call a
batch of ready output."
  (setq romacs--isolate-serving t)
  ;; A zygote told to end forks first the children asked for before it
  ;; was: its spawner waits for them.
  (while (and (or romacs--isolate-serving (and romacs--isolate-zygote romacs--isolate-forks))
              (not (romacs--isolate-terminated-p)))
    (let ((timeout nil) (turned nil))
      (dolist (step (romacs--isolate-round-plan (list :zygote romacs--isolate-zygote)))
        (when (and (eq step 'turns) romacs--isolate-serving (not romacs--isolate-zygote))
          (setq turned t))
        (setq timeout (or (romacs--isolate-round-step step) timeout)))
      (dolist (step (romacs--isolate-wait-plan (romacs--isolate-loop-facts turned)))
        (romacs--isolate-round-step step timeout))))
  ;; A zygote told to end outlives its children (they die with it),
  ;; forking no more.
  (when romacs--isolate-zygote
    (while (and (> (romacs--isolate-forked) 0) (not (romacs--isolate-terminated-p)))
      (setq romacs--isolate-forks nil)
      (romacs--isolate-wait nil)
      (romacs--isolate-note-control-frames))))

(defun romacs--isolate-loop-facts (turned)
  "What `romacs--isolate-wait-plan' knows of this worker now, TURNED
whether its output and timers ran this round."
  (list :serving romacs--isolate-serving :queued (and romacs--isolate-deferred t)
        :forks (and romacs--isolate-forks t) :zygote romacs--isolate-zygote :turned turned))

(defun romacs--isolate-round-step (step &optional timeout)
  "Carry out STEP of a worker's round (`romacs--isolate-round-plan'):
`interrupts' (a signal between jobs: its frame, if any, names a queued
job), `forks', `calls' (the jobs queued now), `turns' (output, sentinels
and timers: the seconds the wait may last), `note' (frames a wait inside
a job, a filter or a timer read and queued in Rust, or the wait itself,
since no socket says so again), `wait' (up to TIMEOUT)."
  (cond ((eq step 'interrupts) (romacs--isolate-interrupted-p) nil)
        ((eq step 'forks) (romacs--isolate-run-forks) nil)
        ((eq step 'calls) (romacs--isolate-run-queued) nil)
        ;; A worker that became a zygote in this round's calls runs nothing more.
        ((eq step 'turns) (and romacs--isolate-serving (not romacs--isolate-zygote) (romacs--isolate-run-turns)))
        ((eq step 'note) (romacs--isolate-note-control-frames) nil)
        ((eq step 'wait) (romacs--isolate-wait timeout (and (process-list) t)) nil)))

(defun romacs--isolate-run-turns ()
  "`romacs--isolate-run-between' as a command loop runs a command: a quit
or an error a filter, sentinel or timer lets out ends that turn, the
error shown, never the loop.  Then 0: the loop looks again at once."
  (condition-case err
      (romacs--isolate-run-between)
    ((quit error)
     (unless (eq (car err) 'quit) (message "%s" (error-message-string err)))
     0)))

(defun romacs--isolate-run-between ()
  "What the loop runs between jobs: process output and sentinels, at most
`owner-output-batch' calls of them (`romacs--process-dispatches'), then
the due timers; no job starts inside them.  The seconds the loop may
wait: 0 while output may be left, else until the next timer, or nil."
  (let* ((romacs--isolate-nested (1+ romacs--isolate-nested))
         (more (romacs--isolate-run-output)))
    ;; A timer may start the next timer: the wait counts from the lists
    ;; as they are after the pass.
    (when (boundp 'timer-list) (romacs--timer-check))
    (if more 0 (and (boundp 'timer-list) (romacs--timer-delay)))))

(defun romacs--isolate-run-output ()
  "Run process output and sentinels, at most `owner-output-batch' calls of
them: non-nil if more may be left, output or status work a spent batch
left (a reaped process's sentinel has no descriptor to wake the wait).
With no process left, a status change pending is no work: it is
acknowledged, and nothing is left."
  (unless (process-list) (romacs--proc-ticks-acknowledge))
  (let ((romacs--process-dispatches (romacs--owner-output-batch))
        (count 0) (more t))
    (while (and more (< count (romacs--owner-output-batch)) (not (romacs--process-spent-p))
                (process-list) (not (romacs--isolate-terminated-p)))
      (setq more (romacs--process-idle) count (1+ count)))
    (or (romacs--process-spent-p) (romacs--proc-ticks-pending) (and more (> count 0)))))

(defun romacs--isolate-run-forks ()
  "In a zygote: fork each child asked for.  In a child, return as a worker
of the spawner's own, for one job, with nothing of the zygote's queue; a
zygote that cannot fork ends, and its spawner forgets it."
  ;; Children asked for while it prepares wait until it is a zygote.
  (while (and romacs--isolate-forks romacs--isolate-zygote)
    (let ((generation (pop romacs--isolate-forks)))
      ;; The child runs nothing until released, after its report.
      (unless (condition-case nil
                  (and (romacs--isolate-fork generation)
                       (progn (romacs--isolate-release generation) t))
                ;; A zygote that cannot fork or report a child ends now,
                ;; and with it the children it forked (an unreported one
                ;; unrun): its spawner settles their jobs.
                (error (setq romacs--isolate-serving nil romacs--isolate-forks nil
                             romacs--isolate-zygote nil)
                       t))
        (setq romacs--isolate-forks nil romacs--isolate-deferred nil romacs--isolate-inbox nil
              romacs--isolate-cancelled nil romacs--isolate-retired nil
              romacs--isolate-zygote nil romacs--isolate-job-worker t romacs--isolate-serving t)
        (romacs--isolate-job-budget romacs--isolate-job-headroom)))))

(defun romacs--isolate-become-zygote (headroom)
  "In a prepared worker: become the zygote of its features, a process that
only forks a child per job (worker-vms design §2.2); a child's job may
grow the heap by HEADROOM bytes.  Its preparation ran the warm functions,
filling the caches the children inherit.  Refused when its preparation
failed (`romacs--isolate-unprepared'), its children would lack the
features, or left a subprocess, a timer or a thread behind, which a
child would share or lose.  The files the preparation loaded (the warm
functions' loads too), their stamp, and this thread, the one that forks."
  (let ((refusal (or (and romacs--isolate-unprepared
                          (format "its preparation failed: %S" romacs--isolate-unprepared))
                     (and (fboundp 'process-list) (process-list) "a subprocess")
                     (romacs--isolate-new-timer)
                     (romacs--isolate-fork-refusal))))
    (when refusal (signal 'isolate-refused (list "Cannot become a zygote" refusal))))
  ;; Compacted, with room to grow: a child's allocations go to fresh pages.
  (romacs--isolate-compact-next)
  (garbage-collect)
  (romacs--isolate-reserve romacs--isolate-zygote-headroom)
  (romacs--isolate-name-process "romacs-zygote")
  (setq romacs--isolate-zygote t romacs--isolate-job-headroom headroom)
  (let ((files (romacs--isolate-prepared-files)))
    (list files (romacs--isolate-stamp files) (romacs--isolate-thread-id))))

(defun romacs--isolate-new-timer ()
  "A description of a timer the preparation started, or nil."
  (let ((found nil))
    (dolist (list (list (cons (romacs--isolate-timers 'timer-list) (nth 1 romacs--isolate-baseline))
                        (cons (romacs--isolate-timers 'timer-idle-list) (nth 2 romacs--isolate-baseline))))
      (dolist (timer (car list))
        (unless (or found (memq timer (cdr list)))
          (setq found (format "a timer %S" timer)))))
    found))

(defun romacs--isolate-timers (variable)
  "The timers VARIABLE lists, or nil where timers are not loaded."
  (and (boundp variable) (symbol-value variable)))

(defun romacs--isolate-prepared-files ()
  "The files the preparation loaded: those in `load-history' now and not
before it (the loader rebuilds the list at each load, so no tail of it
marks where the preparation began)."
  (let ((before (car romacs--isolate-baseline)) files)
    (dolist (entry load-history (nreverse files))
      (let ((file (car-safe entry)))
        (when (and (stringp file) (not (and before (gethash file before))))
          (push file files))))))

;;; isolate-worker.el ends here

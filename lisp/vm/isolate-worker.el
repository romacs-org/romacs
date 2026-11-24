;;; isolate-worker.el --- A worker isolate's loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The worker's side of isolates (lisp/vm/isolate.el, the spawner's): its
;; loop, which takes its parent's jobs and messages and answers each job
;; with a frozen result; a zygote's forking and its preparation.  Not in
;; GNU (ledger/divergences VM2-ISOLATE).

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

(defun romacs--isolate-run-job (generation job)
  "Run JOB, (FUNCTION ARGS BINDINGS REQUIRE DIRECTORY), and reply."
  (let* ((function (nth 0 job))
         (require (nth 3 job))
         (directory (nth 4 job))
         (before (length features))
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
                      (let ((args (thaw (nth 1 job)))
                            (bindings (thaw (nth 2 job))))
                        (dolist (feature require) (require feature))
                        (dolist (binding bindings) (set (car binding) (cdr binding)))
                        (when directory (setq default-directory directory))
                        (list 'result
                              (apply (if (and (consp function) (eq (car function) 'lambda)) (eval function t) function)
                                     args)))
                    ;; The job's end, inside its claim and handler: a
                    ;; collection due, its memory (which replaces the
                    ;; result), what collections left pending.
                    (romacs--safepoint)))
              (quit (list 'error (list 'quit nil nil '(quit))))
              (error (list 'error (list (car err) (romacs--isolate-transportable (cdr err)) nil
                                        (get (car err) 'error-conditions)))))))))
    ;; A quit an interrupt left deferred in this job's compile, which an
    ;; escape then passed over, ends with the job: the next job, in this
    ;; same request, must not take it.
    (romacs--take-deferred-escape nil)
    ;; The preparation loads the worker's baseline features; a later job
    ;; that loads more retires the worker: the spawner reuses it no more and
    ;; the jobs queued behind go back to run elsewhere.
    (let ((changed (and (not (eq function 'romacs--isolate-prepare)) (/= before (length features)))))
      (when changed (setq romacs--isolate-retired t))
      (romacs--isolate-reply generation outcome changed))
    ;; A child forked for one job ends after it.
    (when romacs--isolate-job-worker
      (setq romacs--isolate-serving nil))))

(defun romacs--isolate-note-control-frames ()
  "Take every frame that arrived: interrupts, bye and messages now, jobs
onto the queue."
  (let (frame)
    (while (setq frame (romacs--isolate-take 0))
      (let ((kind (car frame)))
        (cond ((eq kind 'job)
               (setq romacs--isolate-deferred (nconc romacs--isolate-deferred (list frame))))
              ;; A job that could not be read is answered in its turn.
              ((and (eq kind 'frame-error) (eq (car (nth 2 frame)) 'job))
               (setq romacs--isolate-deferred (nconc romacs--isolate-deferred (list frame))))
              ((eq kind 'fork)
               (setq romacs--isolate-forks (nconc romacs--isolate-forks (list (nth 1 frame)))))
              ;; A message that could not be read arrives as its frame.
              ((memq kind '(message text frame-error))
               (let ((message (if (eq kind 'message) (nth 2 frame) frame)))
                 (setq romacs--isolate-inbox (nconc romacs--isolate-inbox (list message)))))
              ((eq kind 'interrupt)
               ;; For a job not yet run; one running took it at a safepoint.
               (push (nth 1 frame) romacs--isolate-cancelled))
              ((eq kind 'bye)
               (setq romacs--isolate-serving nil)))))))

(defun romacs--isolate-run-queued ()
  "Run the queued jobs in order, reading what arrived before each: a
cancellation sent behind a job cancels it before it runs."
  (while (and romacs--isolate-deferred romacs--isolate-serving
              (not (romacs--isolate-terminated-p)))
    ;; A signal nobody took (its job was in a leaf, or done) is stale.
    (romacs--isolate-interrupted-p)
    (romacs--isolate-wait 0)
    (romacs--isolate-note-control-frames)
    ;; A termination read with the frames starts no further job.
    (unless (romacs--isolate-terminated-p)
      (let ((frame (pop romacs--isolate-deferred)))
        (if (eq (car frame) 'frame-error)
            (romacs--isolate-refuse-frame (nth 1 frame) (nth 2 frame))
          (romacs--isolate-run-job (nth 1 frame) (nth 2 frame)))))))

(defun romacs--isolate-refuse-frame (generation detail)
  "Answer the job at GENERATION, whose frame could not be read (DETAIL),
with `isolate-frame-error'; the job itself never runs."
  (romacs--isolate-reply generation (list 'error (romacs--isolate-frame-error-reply detail)) nil)
  (when romacs--isolate-job-worker
    (setq romacs--isolate-serving nil)))

(defun romacs--isolate-serve-frames ()
  "Take every frame from the parent; run the queued jobs unless inside one."
  (romacs--isolate-note-control-frames)
  (when (= romacs--isolate-nested 0)
    (romacs--isolate-run-queued)))

(defun romacs--isolate-serve ()
  "The worker's loop: serve the parent's jobs until bye or termination."
  (setq romacs--isolate-serving t)
  (while (and romacs--isolate-serving (not (romacs--isolate-terminated-p)))
    ;; A signal between jobs: its frame, if any, names a queued job.
    (romacs--isolate-interrupted-p)
    (romacs--isolate-run-forks)
    (romacs--isolate-run-queued)
    (when (and romacs--isolate-serving (null romacs--isolate-deferred)
               (or (null romacs--isolate-forks) (not romacs--isolate-zygote)))
      (romacs--isolate-wait nil)
      (romacs--isolate-serve-frames)))
  ;; A zygote told to end outlives its children's jobs (they die with it),
  ;; forking no more.
  (when romacs--isolate-zygote
    (while (and (> (romacs--isolate-forked) 0) (not (romacs--isolate-terminated-p)))
      (setq romacs--isolate-forks nil)
      (romacs--isolate-wait nil)
      (romacs--isolate-note-control-frames))))

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
              romacs--isolate-zygote nil romacs--isolate-job-worker t)
        (romacs--isolate-job-budget romacs--isolate-job-headroom)))))

(defun romacs--isolate-become-zygote (headroom)
  "In a prepared worker: become the zygote of its features, a process that
only forks a child per job (worker-vms design §2.2); a child's job may
grow the heap by HEADROOM bytes.  Its preparation ran the warm functions,
filling the caches the children inherit.  Refused while its preparation
left a subprocess, a timer or a thread behind, which a child would share
or lose.  The files the preparation loaded (the warm functions' loads
too), their stamp, and this thread, the one that forks."
  (let ((refusal (or (and (fboundp 'process-list) (process-list) "a subprocess")
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

;;; isolate.el --- Isolated VMs that exchange frozen values and text  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The Lisp half of isolates (vm-concurrency design §4): worker VMs in
;; child processes that run a function on frozen arguments and answer with
;; a frozen result, over the leaf primitives of prims/isolate.rs.  A worker
;; adopts its spawner's world (or boots as its spawner did), sees none of
;; the spawner's later state, and is seeded once with
;; `isolate-inherited-variables'.  Ordinary messages and job data cross
;; frozen (`freeze'); closures cannot cross, a symbol or a lambda form with
;; `:bindings' can (ledger/divergences VM2-ISOLATE-CLOSURES).
;; `isolate-send-text' sends a string as up to 4096 UTF-8 bytes.  Text
;; bypasses freezing and arrives as (text GENERATION STRING) through
;; `isolate-receive' or `isolate-message-functions'.
;;
;; Not in GNU (ledger/divergences VM2-ISOLATE).  `isolate-await' re-signals a
;; worker's error where GNU's `thread-join' would store it in
;; `thread-last-error'.
;;
;; Delivery: results, errors and messages arrive at wait points.  The
;; command loop's idle (`romacs--isolate-idle', lisp/vm/keyboard.el and
;; the GUI's timers) and `isolate-await' pump them; a worker's loop
;; (`romacs--isolate-serve') takes its jobs the same way.

;;; Code:

(defvar isolate-boot 'bare
  "How this VM booted, which its workers repeat: `world', `source' or `bare'.
The launcher sets it; a VM made by `Vm::new' is bare.")

(defvar isolate-world nil
  "The world this VM restored from, as (STORE . KEY), or nil.
A worker of a `world' VM adopts exactly this world.  One that cannot
refuses it: its jobs settle with `isolate-world-unavailable' and this is
set to nil, so later workers boot from source.")

(defvar isolate-compiled-store nil
  "The compiled-Lisp store this VM loads from, or nil.
A worker of a `world' VM loads its libraries from it too, never compiling
into it.  The launcher sets it.")

(defvar isolate-worker-program nil
  "The program a worker runs, or nil for this executable.")

(defvar isolate-worker-arguments nil
  "Arguments every worker is started with, after the boot arguments.")

(defvar isolate-pool-size nil
  "How many pooled jobs run at once before queueing.
nil means one fewer than the processors the workers may run on: those of a
zygote's forking thread, whose children inherit them, so whatever pins
the workers narrows the pool; without a zygote, those of this process.")

(defun romacs--isolate-zygote-affinity (zygote)
  "(COUNT . DIGEST) of the processors ZYGOTE's children would run on, at
most a second old; nil until it is ready."
  (let ((cached (aref zygote 14)))
    (if (and cached (< (- (float-time) (car cached)) 1.0))
        (cdr cached)
      (let ((affinity (and (aref zygote 11) (romacs--isolate-affinity (aref zygote 11)))))
        (aset zygote 14 (cons (float-time) affinity))
        affinity))))

(defun isolate--pool-size (&optional zygote)
  "The pool's bound: `isolate-pool-size', else from the processors of
ZYGOTE's children, of those of any ready zygote, or of this process."
  (or isolate-pool-size
      (let ((affinity (and zygote (romacs--isolate-zygote-affinity zygote))))
        (unless affinity
          (maphash (lambda (_ other)
                     (when (and (null affinity) (eq (aref other 3) 'ready))
                       (setq affinity (romacs--isolate-zygote-affinity other))))
                   romacs--isolate-zygotes))
        (max 1 (1- (if affinity (car affinity) (num-processors)))))))

(defvar isolate-zygotes 'world
  "Whether pooled jobs run on workers forked from a zygote.
A zygote is a worker prepared once with a feature set that then only
forks: each job gets a fresh child of it in milliseconds, which runs that
one job and exits.  t: always; nil: never (each job runs on a worker
spawned and prepared for it, which ends after it); `world': when this VM
restored from a world, as a session does.  Either way no pooled job sees
what another left.")

(defvar isolate-zygote-warm-functions nil
  "Alist of (FEATURE . FUNCTION): a worker prepared with features including
FEATURE calls FUNCTION once, after loading them.  A zygote does before it
forks, so its children inherit what FUNCTION caches for the session (a
program's version, say) instead of each child computing it again; every
other worker does too, so a job sees the same state on any.
What FUNCTION loads is part of the preparation: it does not count as a job
changing the features.  An error in FUNCTION is ignored.")

(defvar isolate-job-memory-headroom (* 64 1024 1024)
  "Bytes a job on a forked child may grow its heap by.
Allocation below this runs no collection, whose garbage dies with the
child.  A child runs under the value its zygote was made with, from its
fork for its whole life: a collection that finds live growth past it ends
the child at once, and the job's reply is an `isolate-memory-exhausted'
error whose data is (BUDGET), that budget in bytes.  The job's handlers,
finalizers, `post-gc-hook' and cleanups never see it.  The commands it
reported get SIGHUP to their groups at its death, as at any worker's; one
it could not report or cannot reach, or one that ignores SIGHUP, outlives
it, as after a crash.")

(defvar isolate-pool-memory-limit (* 1024 1024 1024)
  "Bytes the pool may hold, sampled as proportional memory, with
`isolate-job-memory-headroom' reserved for each running job.
A job that would pass it waits for room.")

(defvar isolate-zygote-limit 4
  "How many zygotes may live at once; the least recently used one goes first.")

(defvar isolate-process-limit nil
  "How many worker processes may run at once, whatever their role:
zygotes, their spare and job children, pooled and created workers, and
those retired or ended but not yet reaped.  A pooled job past it waits; a
worker asked for outright (`isolate-create', `:fresh') signals
`isolate-process-limit'.  nil means the pool's width, a zygote and a spare
for each of `isolate-zygote-limit', and 4 more.")

(defvar isolate-refuse-degraded nil
  "Non-nil: a job that would run other than this VM asks
\(`isolate-future-degraded') signals `isolate-degraded' instead.  Read as
the job is dispatched: a job that waited for room is judged then, its
future settling with the error.")

(defvar romacs--isolate-zygote-headroom (* 1024 1024)
  "Heap entries a zygote reserves room for, so its children grow no table.")

(defvar isolate-queue-limit 64
  "Jobs and messages a worker may have waiting before `isolate-queue-full'.")

(defvar isolate-kill-timeout 1.0
  "Seconds `isolate-kill' gives a worker to exit on SIGTERM before SIGKILL.")

(defvar isolate-interrupt-timeout 2.0
  "Seconds `isolate-interrupt' gives a job to stop before the worker is killed.")

(defvar isolate-inherited-variables
  '(load-path default-directory process-environment exec-path user-emacs-directory)
  "Variables a new worker is seeded with, thawed into its own copies.
A value that cannot be frozen is left out.")

(defvar isolate-message-functions nil
  "Abnormal hook run in the spawner with (SENDER MESSAGE).
SENDER is the worker's isolate.  MESSAGE is the value sent by
`isolate-send' (frozen data, or a seed's private copies), the frame
\(text GENERATION STRING) from `isolate-send-text', or the frame
\(frame-error GENERATION (message REASON)) for a message that could not
be read.  Workers take parent messages with `isolate-receive'.")

(defvar isolate-error-functions nil
  "Abnormal hook run with (ISOLATE EVENT DATA) when a worker dies or is refused.")

(defvar isolate-error-backtrace nil
  "The worker's backtrace while `isolate-await' re-signals its error.")

(defvar romacs--isolates nil "The live isolates this VM spawned.")
(defvar romacs--isolate-generation 0 "The last job generation this VM issued.")
(defvar romacs--isolate-inbox nil
  "In a worker: frozen values and text frames not yet taken, oldest first.")
(defvar romacs--isolate-deferred nil "In a worker: jobs that arrived inside a job, oldest first.")
(defvar romacs--isolate-serving nil "In a worker: whether its loop runs.")
(defvar romacs--isolate-nested 0 "In a worker: how deep inside a job the frame loop runs.")
(defvar romacs--isolate-cancelled nil "In a worker: generations interrupted before they ran.")
(defvar romacs--isolate-zygotes (make-hash-table :test 'equal)
  "The zygotes, by feature list: [isolate-zygote ISOLATE FEATURES STATE
SPARE INPUTS FILES FILES-STAMP DIRECTORIES DIRECTORIES-STAMP USED THREAD
MASK HEADROOM AFFINITY]; THREAD its forking thread, MASK its processors'
digest at its last fork, HEADROOM the budget of its children, AFFINITY
\(TIME . (COUNT . DIGEST)) as last asked.")
(defvar romacs--isolate-waiting nil
  "Futures of pooled jobs waiting for room in the pool, oldest first; at
most `isolate-queue-limit'.  Each has no isolate until it is dispatched.")
(defvar romacs--isolate-forks nil "In a zygote: the generations of the children asked for, oldest first.")
(defvar romacs--isolate-zygote nil "In a worker: non-nil once it became a zygote.")
(defvar romacs--isolate-job-worker nil "In a worker: non-nil in a child forked for one job.")
(defvar romacs--isolate-job-headroom nil "In a zygote: `isolate-job-memory-headroom', from its spawner.")
(defvar romacs--isolate-baseline nil
  "In a worker: (LOADED TIMERS IDLE-TIMERS) before its preparation, LOADED
a hash table of the files `load-history' named.")
(defvar romacs--isolate-world-refusal nil
  "(REASON WORLD) once a worker refused the world, which later workers
then boot without (`romacs--isolate-world-refused').")
(defvar romacs--isolate-retired nil
  "In a worker: a job changed the features, so queued jobs go back to the
spawner to run on a worker with the features they were queued for.")

(define-error 'isolate-error "Isolate error")
(define-error 'isolate-world-unavailable "No world for a worker isolate" 'isolate-error)
(define-error 'isolate-killed "Isolate killed" 'isolate-error)
(define-error 'isolate-died "Isolate died" 'isolate-error)
(define-error 'isolate-queue-full "Isolate queue full" 'isolate-error)
(define-error 'isolate-timeout "Isolate wait timed out" 'isolate-error)
(define-error 'isolate-refused "Isolate refused" 'isolate-error)
(define-error 'isolate-retired "Isolate retired" 'isolate-error)
(define-error 'isolate-process-limit "Isolate process limit reached" '(isolate-queue-full isolate-error))
;; Data: the job's degradations, as `isolate-future-degraded' lists them.
(define-error 'isolate-degraded "Isolate job would run degraded" 'isolate-error)
;; A zygote refused: its features' jobs run on pooled workers.  Data: (REASON).
(define-error 'isolate-zygote-refused "Isolate zygote refused" 'isolate-error)
;; A frame whose value could not be read: its generation's reply, from the
;; side that read it.  Data: (KIND REASON), KIND the frame's.
(define-error 'isolate-frame-error "Isolate frame unreadable" 'isolate-error)

(defun romacs--isolate-frame-error-reply (detail)
  "The error reply for a frame whose value could not be read: DETAIL is
\=(KIND REASON), as `romacs--isolate-take' gives it."
  (list 'isolate-frame-error detail nil (get 'isolate-frame-error 'error-conditions)))
;; The job failed, not its isolate: an `error', which its awaiter's
;; `error' handler catches.  Data: (BUDGET), in bytes.
(define-error 'isolate-memory-exhausted
  "Memory exhausted: the job grew past its budget, isolate-job-memory-headroom")

;;;; Records

;; An isolate: [isolate ID NAME STATE REQUIRE PENDING TAINTED DEADLINE
;; PROGRAM HELLO ROLE ZYGOTE WORLD ENDING].  STATE is `live', `killed', `retired' or `dead'; PENDING an alist of
;; (GENERATION . FUTURE) in order of submission, the first running; TAINTED
;; once a job changed the worker's features (not reused); DEADLINE
;; (GENERATION . TIME) after an interrupt; PROGRAM what the worker runs;
;; HELLO once the worker's hello arrived (it catches signals from then);
;; ROLE nil for a worker `isolate-create' made, `pool' for one the pool
;; made, `fresh' for one it made for a `:fresh' job (outside its bound;
;; one job each, as `pool'), `zygote', or `job' for a child forked for one
;; job; ZYGOTE the
;; zygote record a `job' child is of; WORLD the (STORE . KEY) it was told
;; to adopt; ENDING (TIME . SIGNAL) once it was retired, SIGNAL the last
;; sent it since (nil, 15, 9; `romacs--isolate-check-deadlines').
(defun romacs--isolate-make (id name require program &optional world)
  "An isolate's record; WORLD is the (STORE . KEY) its worker was told to adopt."
  (record 'isolate id name 'live require nil nil nil program nil nil nil world nil))
(defun isolate-p (object) "Non-nil for an isolate." (eq (type-of object) 'isolate))
(defun romacs--isolate-id (isolate) (aref isolate 1))
(defun isolate-name (isolate) "ISOLATE's name, or nil." (aref isolate 2))
(defun isolate-live-p (isolate) "Non-nil while ISOLATE's worker runs." (eq (aref isolate 3) 'live))
(defun isolate-list () "The live isolates." (copy-sequence romacs--isolates))
(defun isolate-parent ()
  "In a worker, `parent', the isolate that spawned it; nil in a spawner."
  (and (romacs--isolate-parent-p) 'parent))
(defun isolate-main-p () "Non-nil in the VM that spawns workers, not in one." (not (romacs--isolate-parent-p)))

;; A future: [isolate-future ISOLATE GENERATION STATE VALUE CALLBACKS JOB
;; DEGRADED].  STATE is `pending', `done', `error', `cancelled', `killed' or
;; `died'; JOB (FUNCTION ARGS BINDINGS REQUIRE DIRECTORY) is what was
;; queued, for a resubmission when its worker retires before running it;
;; DEGRADED how its worker runs other than asked, as dispatched.
(defun romacs--isolate-future (isolate generation job)
  (record 'isolate-future isolate generation 'pending nil nil job nil))
(defun isolate-future-p (object) "Non-nil for a future." (eq (type-of object) 'isolate-future))
(defun isolate-future-isolate (future) "The isolate FUTURE runs on." (aref future 1))
(defun isolate-future-degraded (future)
  "How FUTURE's job runs other than this VM asks, as its worker was when it
was dispatched: nil, or a list of (CONDITION . DATA), each condition an
error: `isolate-world-unavailable' (REASON WORLD), the worker booted
without the world a refusal made this VM forget; `isolate-zygote-refused'
\(REASON), it runs on a pooled worker since its features' zygote was
refused.  Its result is a normal one either way, the warm functions run;
the job ran slower than asked.  `isolate-error-functions' heard each
refusal as it happened."
  (aref future 7))
(defun isolate-future-done-p (future) "Non-nil once FUTURE is resolved." (not (eq (aref future 3) 'pending)))

(defun romacs--isolate-resolve (future state value)
  "Resolve FUTURE as STATE with VALUE and run its callbacks."
  (romacs--isolate-notify (romacs--isolate-mark-resolved future state value)))

(defun romacs--isolate-mark-resolved (future state value)
  "Resolve FUTURE as STATE with VALUE, running none of its callbacks:
\(FUTURE . CALLBACKS), them in order, for `romacs--isolate-notify'; nil
when it was resolved already."
  (when (eq (aref future 3) 'pending)
    (aset future 3 state)
    (aset future 4 value)
    (let ((callbacks (aref future 5)))
      (aset future 5 nil)
      (cons future (reverse callbacks)))))

(defun romacs--isolate-notify (resolved)
  "Run the callbacks RESOLVED, from `romacs--isolate-mark-resolved', names."
  (romacs--isolate-each (lambda (callback) (funcall callback (car resolved))) (cdr resolved)))

(defun isolate-on-done (future callback)
  "Call CALLBACK with FUTURE once it is resolved, at a wait point; now if it is."
  (if (isolate-future-done-p future)
      (funcall callback future)
    (aset future 5 (cons callback (aref future 5))))
  future)

;;;; Spawning

(defun romacs--isolate-boot-arguments ()
  "The worker's boot arguments for this VM's boot: the world it adopts,
else the chain from source (a VM that booted from source, or whose world
is gone), else a bare VM.  A worker with no world boots slower; one that
cannot adopt the world refuses it once (`romacs--isolate-world-refused')."
  (cond ((and (eq isolate-boot 'world) isolate-world)
         (append (list "--world" (car isolate-world) (cdr isolate-world))
                 (and isolate-compiled-store (list "--compiled-store" isolate-compiled-store))))
        ((memq isolate-boot '(world source)) (list "--source"))
        (t (list "--bare"))))

(defun romacs--isolate-seeds ()
  "The inherited variables as an alist of (SYMBOL . VALUE), the freezable ones."
  (let (seeds)
    (dolist (variable isolate-inherited-variables)
      (when (boundp variable)
        (let ((value (symbol-value variable)))
          (when (condition-case nil (progn (freeze value) t) (error nil))
            (push (cons variable value) seeds)))))
    (nreverse seeds)))

(defun romacs--isolate-prepare (seeds require &optional warm)
  "In a worker: install the SEEDS as this VM's own values, load REQUIRE and
call the functions WARM (`isolate-zygote-warm-functions'), ignoring their
errors."
  (dolist (seed seeds)
    (set (car seed) (thaw (cdr seed))))
  (setq romacs--isolate-baseline
        (list (let ((files (make-hash-table :test 'equal)))
                (dolist (entry load-history files)
                  (when (stringp (car-safe entry)) (puthash (car entry) t files))))
              (romacs--isolate-timers 'timer-list) (romacs--isolate-timers 'timer-idle-list)))
  (dolist (feature require)
    (require feature))
  (dolist (function warm)
    (condition-case nil (funcall function) (error nil)))
  t)

(defun isolate-create (&rest keys)
  "Start a long-lived worker isolate; KEYS are :name and :require.
The worker boots as this VM did, is seeded with `isolate-inherited-variables'
and has the features of :require loaded.  `isolate-process-limit' when
that many workers run."
  (romacs--isolate-start-worker (plist-get keys :name) (plist-get keys :require) 'create))

(defun romacs--isolate-start-worker (name require kind)
  "Start a worker named NAME prepared with REQUIRE's features and their
warm functions: its isolate; `isolate-process-limit' without room for a
process of KIND (`romacs--isolate-admit-p')."
  (unless (romacs--isolate-admit-p kind require)
    (signal 'isolate-process-limit (list (length romacs--isolates) (isolate--process-limit))))
  (let* ((arguments (append (list "--parent" (number-to-string (emacs-pid)))
                            (romacs--isolate-boot-arguments)
                            isolate-worker-arguments))
         (program (or isolate-worker-program (romacs--isolate-executable)))
         (id (romacs--isolate-start program arguments))
         (isolate (romacs--isolate-make id name require program
                                        ;; A copy: the arguments are what the
                                        ;; worker was told, whatever later
                                        ;; becomes of `isolate-world'.
                                        (and (eq isolate-boot 'world) isolate-world
                                             (cons (copy-sequence (car isolate-world))
                                                   (copy-sequence (cdr isolate-world)))))))
    (push isolate romacs--isolates)
    (condition-case err
        (romacs--isolate-submit isolate 'romacs--isolate-prepare (list (romacs--isolate-seeds) require (romacs--isolate-warm-functions require))
                               nil nil nil)
      ;; Gone already: its death is reported once delivered, not hidden
      ;; as a retirement.
      (isolate-died (signal (car err) (cdr err)))
      ;; Unprepared, it is no one's: it ends at once.
      (error (isolate-retire isolate) (signal (car err) (cdr err))))
    isolate))

(defun romacs--isolate-recycle ()
  "End the idle workers a job changed the features of: they are not reused.
Retired, not killed: each ends its loop by itself and nothing waits for
its exit here, where a killed worker's end cost a spawn hundreds of
milliseconds of the editor's under load.  A zygote and its children are
never recycled: a zygote runs no job but its preparation, and a child
ends after its one job."
  (dolist (isolate (copy-sequence romacs--isolates))
    (when (and (isolate-live-p isolate) (aref isolate 6) (null (aref isolate 5))
               (memq (aref isolate 10) '(nil pool fresh)))
      (isolate-retire isolate))))

(defun romacs--isolate-acquire (require fresh)
  "A worker for a pooled job requiring REQUIRE: a child of their zygote
when pooled jobs fork (`isolate-zygotes'), else a worker the pool made
with them that has taken no job yet, else a new one while
`isolate-pool-size' allows, making room by killing an idle such worker of
other features; else `queued', to wait for one to end.  A worker the
pool makes runs one job and ends (`romacs--isolate-submit'), so no
pooled job sees what another left.  FRESH asks for a new worker outside
the pool's bound."
  (romacs--isolate-recycle)
  (let ((live (romacs--isolate-pooled))
        (forked (and (not fresh) (romacs--isolate-zygote-worker require t)))
        (unspent nil) (idle nil))
    (unless (or fresh forked)
      (setq unspent (romacs--isolate-with-features (romacs--isolate-unspent) require)))
    (cond (forked forked)
          ;; A live zygote with no room: the job waits for it.
          ((and (not fresh) (romacs--isolate-zygote-live-p require)) 'queued)
          ;; Its zygote not made for want of room for its process: the job
          ;; waits for that room (made by ending an idle worker if need be).
          ((and (not fresh) (romacs--isolate-zygotes-p) (null (gethash require romacs--isolate-zygotes))
                (not (romacs--isolate-admit-p 'start require)))
           'queued)
          (fresh (romacs--isolate-pool-create require 'create))
          ;; Prepared or preparing: the job runs after the preparation.
          (unspent unspent)
          ((< (length live) (isolate--pool-size))
           (if (romacs--isolate-admit-p 'start require) (romacs--isolate-pool-create require 'start) 'queued))
          ((setq idle (let (found) (dolist (i (romacs--isolate-unspent)) (when (null (aref i 5)) (setq found i))) found))
           (isolate-kill idle)
           (romacs--isolate-pool-create require 'start))
          ;; Every pooled worker runs its job: one of them ends soon.
          (t 'queued))))

(defun isolate-pending (isolate)
  "How many jobs ISOLATE has queued or running."
  (length (aref isolate 5)))

(defun romacs--isolate-pooled ()
  "The live workers the pool made, what `isolate-pool-size' bounds: those
running their job, or prepared for one; not zygotes nor their children,
nor a caller's own (`isolate-create')."
  (delq nil (mapcar (lambda (i) (and (isolate-live-p i) (eq (aref i 10) 'pool) i)) romacs--isolates)))

(defun romacs--isolate-unspent ()
  "The live workers the pool made that have taken no job: the ones a
pooled job may go to."
  (delq nil (mapcar (lambda (i) (and (not (aref i 6)) i)) (romacs--isolate-pooled))))

(defun romacs--isolate-pool-create (require kind)
  "A worker the pool makes for REQUIRE's features, admitted as KIND
\(`romacs--isolate-admit-p'): one of the pool's (`pool'), or, for KIND
`create', a `:fresh' job's, outside its bound (`fresh')."
  (let ((isolate (romacs--isolate-start-worker nil require kind)))
    (aset isolate 10 (if (eq kind 'create) 'fresh 'pool))
    isolate))

(defun romacs--isolate-one-job-p (isolate)
  "Whether ISOLATE is a worker the pool made, which takes one job and ends."
  (memq (aref isolate 10) '(pool fresh)))

(defun isolate-idle-worker (require)
  "A worker prepared with REQUIRE's features and running nothing, or nil:
the spare child of REQUIRE's zygote, else an idle pooled worker."
  (or (condition-case nil (romacs--isolate-zygote-worker require nil) (isolate-process-limit nil))
      (let (found)
        (dolist (isolate (romacs--isolate-unspent))
          (when (and (aref isolate 9) (null (aref isolate 5)) (equal (aref isolate 4) require))
            (setq found isolate)))
        found)))

(defun isolate-warm (require)
  "Prepare a worker with REQUIRE's features ahead of its first job: its
zygote when pooled jobs run on forked workers (`isolate-zygotes'), else a
pooled worker, unless one is live (prepared or preparing) or the pool is at
`isolate-pool-size' or `isolate-process-limit'; the isolate started, or nil."
  (let ((zygote (and (romacs--isolate-zygotes-p) (gethash require romacs--isolate-zygotes))))
    (cond ((not (romacs--isolate-zygotes-p)) (romacs--isolate-warm-pooled require))
          ((null zygote)
           (condition-case nil (aref (romacs--isolate-zygote-make require) 1) (isolate-process-limit nil)))
          ;; Features no zygote can hold run on pooled workers.
          ((eq (aref zygote 3) 'refused) (romacs--isolate-warm-pooled require))
          (t (condition-case nil (romacs--isolate-zygote-worker require t) (isolate-process-limit nil)) nil))))

(defun romacs--isolate-warm-pooled (require)
  "Start a pooled worker with REQUIRE's features unless one that has taken
no job is live, or the pool is full, or no worker may start
\(`isolate-process-limit'); the isolate started, or nil."
  (romacs--isolate-recycle)
  (let ((live (romacs--isolate-pooled)))
    (and (not (romacs--isolate-with-features (romacs--isolate-unspent) require))
         (< (length live) (isolate--pool-size))
         (condition-case nil (romacs--isolate-pool-create require 'start) (isolate-process-limit nil)))))

(defun isolate-pooled-worker (require &optional if-busy)
  "A worker for background work with REQUIRE's features: an idle prepared
one (a zygote's spare child first), else a pooled one started now while the
pool has room, else one still preparing for its first job.  If its queue
is full or an ending worker can free room, return IF-BUSY.  Otherwise
return nil when no worker can take the job.
Unlike `isolate-spawn', never kill a worker of other features to make room."
  (when (> isolate-queue-limit 0)
    (let ((worker (or (isolate-idle-worker require)
                      (condition-case nil (romacs--isolate-zygote-worker require t)
                        (isolate-process-limit nil))
                      (and (not (romacs--isolate-zygote-live-p require))
                           ;; A worker that cannot start, or prepare, is none.
                           (condition-case nil (romacs--isolate-warm-pooled require) (error nil)))
                      (romacs--isolate-with-features (romacs--isolate-unspent) require))))
      (cond ((and worker (< (isolate-pending worker) isolate-queue-limit)) worker)
            ((or worker (romacs--isolate-ending-p)) if-busy)))))

(defun romacs--isolate-with-features (isolates require)
  "The first of ISOLATES whose features are REQUIRE, or nil."
  (let (found)
    (dolist (isolate isolates)
      (when (and (null found) (equal (aref isolate 4) require))
        (setq found isolate)))
    found))

(defun romacs--isolate-submit (isolate function args bindings require directory)
  "Queue (apply FUNCTION ARGS) on ISOLATE: its future."
  (unless (isolate-live-p isolate)
    (signal 'isolate-died (list isolate)))
  (when (>= (length (aref isolate 5)) isolate-queue-limit)
    (signal 'isolate-queue-full (list isolate)))
  (when (eq (aref isolate 10) 'job)
    ;; A forked child runs one job, in the spawner's directory unless the
    ;; job names one.
    (when (aref isolate 5) (signal 'isolate-retired (list isolate)))
    (unless directory (setq directory default-directory)))
  ;; A worker the pool made runs one job: what it leaves is no later job's.
  (when (and (romacs--isolate-one-job-p isolate) (aref isolate 6))
    (signal 'isolate-retired (list isolate)))
  (setq romacs--isolate-generation (1+ romacs--isolate-generation))
  ;; The job is frozen once, here: what the worker gets, and what a
  ;; resubmission resends, is this snapshot, whatever the caller does to
  ;; its objects afterwards.
  (let* ((job (freeze (list function args bindings require directory)))
         (future (romacs--isolate-future isolate romacs--isolate-generation job)))
    ;; What cannot be sent signals here: an argument that cannot freeze
    ;; (`Cannot freeze', above), a full queue (`isolate-queue-full'), a
    ;; worker gone (`isolate-died', whose death the next wait point reports).
    (condition-case err
        (romacs--isolate-post (romacs--isolate-id isolate) 'job romacs--isolate-generation job)
      (isolate-died (signal 'isolate-died (list isolate))))
    (aset isolate 5 (append (aref isolate 5) (list (cons romacs--isolate-generation future))))
    (romacs--isolate-taken isolate)
    future))

(defun romacs--isolate-taken (isolate)
  "ISOLATE was sent a job: a zygote's spare is taken, and its zygote forks
the next; a worker the pool made is spent, to end after it."
  (cond ((eq (aref isolate 10) 'job) (romacs--isolate-zygote-take isolate))
        ((romacs--isolate-one-job-p isolate) (aset isolate 6 t))))

(defun romacs--isolate-resubmit (future)
  "Queue FUTURE's job again, on a worker with the features it was queued
for: its worker retired before running it."
  (let* ((job (aref future 6))
         (require (thaw (nth 3 job)))
         (isolate (progn
                    ;; Judged before a worker is found or started for it.
                    (romacs--isolate-refuse-degraded
                     (romacs--isolate-degradation-for 'pool (and (eq isolate-boot 'world) isolate-world) require nil))
                    (romacs--isolate-acquire require nil))))
    (if (eq isolate 'queued)
        (progn (aset future 1 nil)
               (setq romacs--isolate-waiting (cons future romacs--isolate-waiting)))
      (aset future 7 (romacs--isolate-refuse-degraded (romacs--isolate-degradation isolate)))
      (setq romacs--isolate-generation (1+ romacs--isolate-generation))
      (romacs--isolate-post (romacs--isolate-id isolate) 'job romacs--isolate-generation job)
      (aset future 1 isolate)
      (aset future 2 romacs--isolate-generation)
      (aset isolate 5 (append (aref isolate 5) (list (cons romacs--isolate-generation future))))
      (romacs--isolate-taken isolate))))

(defun romacs--isolate-enqueue (function args bindings require directory)
  "Queue (apply FUNCTION ARGS) for room in the pool: its future, on no
isolate until it is dispatched."
  (when (>= (length romacs--isolate-waiting) isolate-queue-limit)
    (signal 'isolate-queue-full (list require)))
  (let ((future (romacs--isolate-future nil 0 (freeze (list function args bindings require
                                                            (or directory default-directory))))))
    (setq romacs--isolate-waiting (nconc romacs--isolate-waiting (list future)))
    future))

(defun romacs--isolate-drain-waiting ()
  "Dispatch the waiting jobs, oldest first, until one is sent back: each
is judged by its own zygote's room."
  (let ((going t))
    (while (and going romacs--isolate-waiting)
      (let ((future (pop romacs--isolate-waiting)))
        (when (eq (aref future 3) 'pending)
          (romacs--isolate-resubmit-or-fail future)
          ;; Sent back to wait: nothing more goes now.
          (when (eq (car romacs--isolate-waiting) future) (setq going nil)))))))

(defun isolate-spawn (function &rest args)
  "Run (apply FUNCTION ARGS) in a worker isolate; its future.
FUNCTION is a symbol the worker calls its own definition of, or a lambda
form whose free variables come from :bindings, an alist of (SYMBOL . VALUE).
A closure object is refused.  The keyword tail of ARGS: :name, :bindings,
:require FEATURES, :directory, :isolate ISOLATE (run there), :fresh t (a
new worker).  With no room in the pool the job waits:
its future has no isolate until it is dispatched."
  (let (keys plain)
    (while args
      (if (and (keywordp (car args))
               (memq (car args) '(:name :bindings :require :directory :isolate :fresh)))
          (progn (push (car args) keys) (push (car (cdr args)) keys) (setq args (cdr (cdr args))))
        (push (car args) plain)
        (setq args (cdr args))))
    (setq keys (nreverse keys) plain (nreverse plain))
    (unless (or (symbolp function) (and (consp function) (eq (car function) 'lambda)))
      (signal 'error (list "Cannot send closure" function)))
    (let* ((require (plist-get keys :require))
           (chosen (plist-get keys :isolate))
           (isolate (progn
                      ;; Judged before a worker is found or started for it:
                      ;; a job refused leaves none behind.
                      (unless chosen
                        (romacs--isolate-refuse-degraded
                         (romacs--isolate-degradation-for 'pool (and (eq isolate-boot 'world) isolate-world) require nil)))
                      (or chosen (romacs--isolate-acquire require (plist-get keys :fresh))))))
      (if (eq isolate 'queued)
          (romacs--isolate-enqueue function plain (plist-get keys :bindings) require (plist-get keys :directory))
        (let* ((degraded (romacs--isolate-refuse-degraded (romacs--isolate-degradation isolate chosen)))
               (future (condition-case err
                           (romacs--isolate-submit isolate function plain (plist-get keys :bindings) require
                                                   (plist-get keys :directory))
                         (error
                          ;; A worker started for this job alone is spent all
                          ;; the same, and ends once idle: none is left behind.
                          (when (and (plist-get keys :fresh) (not chosen))
                            (aset isolate 6 t)
                            (isolate-retire isolate))
                          (signal (car err) (cdr err))))))
          (aset future 7 degraded)
          future)))))

(defun romacs--isolate-degradation (isolate &optional chosen)
  "How a job on ISOLATE would run other than this VM asks, as
`isolate-future-degraded' lists it; CHOSEN when the caller named ISOLATE,
which no zygote would have run the job on instead."
  (let ((worker (if (eq (aref isolate 10) 'job) (aref (aref isolate 11) 1) isolate)))
    (romacs--isolate-degradation-for (aref isolate 10) (aref worker 12) (aref isolate 4) chosen)))

(defun romacs--isolate-degradation-for (role world features chosen)
  "The degradations of a job on a worker of ROLE booted with WORLD (nil:
without one) and prepared with FEATURES; CHOSEN as for
`romacs--isolate-degradation'."
  (let ((zygote (and (not chosen) (memq role '(nil pool fresh)) (romacs--isolate-zygotes-p)
                     (gethash features romacs--isolate-zygotes)))
        (found nil))
    (when (and zygote (eq (aref zygote 3) 'refused))
      (push (cons 'isolate-zygote-refused (list (aref zygote 6))) found))
    (when (and romacs--isolate-world-refusal (eq isolate-boot 'world) (null world))
      (push (cons 'isolate-world-unavailable romacs--isolate-world-refusal) found))
    found))

(defun romacs--isolate-refuse-degraded (degraded)
  "`isolate-degraded' with DEGRADED as data, when there are any and
`isolate-refuse-degraded' is set; else DEGRADED."
  (when (and degraded isolate-refuse-degraded)
    (signal 'isolate-degraded degraded))
  degraded)

;;;; Zygotes

;; A zygote (worker-vms design §2.2-2.4) is a worker prepared with a
;; feature set that then only forks.  Each pooled job for those features
;; runs in a child forked from it, which shares the zygote's prepared pages
;; until it writes them, runs that one job and exits: no job sees what an
;; earlier one left.  The zygote keeps one child forked ahead, its spare.
;; It is retired, with its spare, once anything its preparation read has
;; changed: the inherited variables, the files it loaded, the directories
;; of `load-path'.

(defun romacs--isolate-zygotes-p ()
  "Whether pooled jobs run on forked workers here (`isolate-zygotes')."
  (if (eq isolate-zygotes 'world)
      (and (eq isolate-boot 'world) isolate-world t)
    isolate-zygotes))

(defun romacs--isolate-input-variables ()
  "The variables a zygote's preparation read: the inherited ones, but
`default-directory' only while `load-path' holds a relative directory.  A
job brings its own directory."
  (let ((relative nil))
    ;; nil stands for the current directory, as the loader reads it.
    (dolist (directory load-path)
      (when (or (null directory) (and (stringp directory) (not (file-name-absolute-p directory))))
        (setq relative t)))
    (if relative isolate-inherited-variables
      (delq 'default-directory (copy-sequence isolate-inherited-variables)))))

(defun romacs--isolate-inputs ()
  "The preparation inputs, as an alist of (VARIABLE . its value), a list
copied (`setenv' changes an entry in place)."
  (mapcar (lambda (variable)
            (let ((value (and (boundp variable) (symbol-value variable))))
              (cons variable (if (consp value) (copy-sequence value) value))))
          (romacs--isolate-input-variables)))

(defun romacs--isolate-inputs-current-p (inputs)
  "Whether the preparation INPUTS are what the variables hold now."
  (let ((same (equal (mapcar #'car inputs) (romacs--isolate-input-variables))))
    (dolist (input inputs same)
      (unless (and same (equal (cdr input) (and (boundp (car input)) (symbol-value (car input)))))
        (setq same nil)))))

(defun romacs--isolate-load-directories ()
  "The directories of `load-path', expanded (nil as the current one)."
  (let (directories)
    (dolist (directory load-path (nreverse directories))
      (when (or (null directory) (stringp directory))
        (push (expand-file-name (or directory ".")) directories)))))

(defun romacs--isolate-zygote-make (require)
  "Start the zygote for REQUIRE's features: its record, preparing."
  (romacs--isolate-zygote-evict)
  (let* ((inputs (romacs--isolate-inputs))
         (directories (romacs--isolate-load-directories))
         (stamp (romacs--isolate-stamp directories))
         (isolate (romacs--isolate-start-worker nil require 'start))
         (zygote (record 'isolate-zygote isolate require 'preparing nil inputs nil nil
                         directories stamp (float-time) nil nil isolate-job-memory-headroom nil)))
    (aset isolate 10 'zygote)
    (puthash require zygote romacs--isolate-zygotes)
    (isolate-on-done (romacs--isolate-submit isolate 'romacs--isolate-become-zygote
                                             (list isolate-job-memory-headroom) nil nil nil)
                     (lambda (future) (romacs--isolate-zygote-ready zygote future)))
    zygote))

(defun romacs--isolate-warm-functions (require)
  "The `isolate-zygote-warm-functions' of REQUIRE's features, in order."
  (let (functions)
    (dolist (entry isolate-zygote-warm-functions (nreverse functions))
      (when (memq (car entry) require) (push (cdr entry) functions)))))

(defun romacs--isolate-zygote-ready (zygote future)
  "ZYGOTE's preparation ended as FUTURE says: ready, with a spare forked,
and the idle pooled workers of its features retired; or refused, for the
rest of the session, ended with its spare, its jobs running on pooled
workers and told so (`isolate-future-degraded'), the refusal heard by
`isolate-error-functions' as `isolate-zygote-refused' with (REASON)."
  (when (eq (gethash (aref zygote 2) romacs--isolate-zygotes) zygote)
    ;; However large: what it holds its children share, where workers of
    ;; their own would each hold a copy (romacs#396).
    (if (eq (aref future 3) 'done)
        (let ((value (aref future 4)))
          (aset zygote 6 (car value))
          (aset zygote 7 (nth 1 value))
          (aset zygote 11 (nth 2 value))
          ;; A child asked for while it prepared forks with these.
          (aset zygote 12 (cdr (romacs--isolate-zygote-affinity zygote)))
          (aset zygote 3 'ready)
          ;; The pool's idle workers of these features are not needed now.
          ;; A spare asked for while it prepared is forked now.
          (dolist (isolate (romacs--isolate-pooled))
            (when (and (eq (aref isolate 10) 'pool) (equal (aref isolate 4) (aref zygote 2)) (null (aref isolate 5)))
              (romacs--isolate-end isolate)))
          (unless (aref zygote 4)
            (condition-case nil (aset zygote 4 (romacs--isolate-zygote-fork zygote)) (error nil))))
      (let* ((value (aref future 4))
             (reason (if (eq (aref future 3) 'error) (list (car value) (nth 1 value))
                       (list (aref future 3)))))
        (aset zygote 3 'refused)
        (aset zygote 6 reason)
        (romacs--isolate-end (aref zygote 1))
        (run-hook-with-args 'isolate-error-functions (aref zygote 1) 'isolate-zygote-refused (list reason nil))))))

(defun romacs--isolate-zygote-current-p (zygote)
  "Whether nothing ZYGOTE's preparation read has changed since, the
budget its children are forked with included; while it prepares, the
files it loads are not known yet."
  (and (romacs--isolate-inputs-current-p (aref zygote 5))
       (eql (aref zygote 13) isolate-job-memory-headroom)
       (eql (aref zygote 9) (romacs--isolate-stamp (aref zygote 8)))
       (or (not (eq (aref zygote 3) 'ready))
           (eql (aref zygote 7) (romacs--isolate-stamp (aref zygote 6))))))

(defun romacs--isolate-zygote-retire (zygote)
  "Forget ZYGOTE and end it, its spare with it (`isolate-retire'), without
waiting."
  (remhash (aref zygote 2) romacs--isolate-zygotes)
  (aset zygote 4 nil)
  (romacs--isolate-end (aref zygote 1)))

(defun romacs--isolate-end (isolate)
  "End ISOLATE: retired when idle, else killed; nothing when it is gone,
its death delivered or not."
  (when (isolate-live-p isolate)
    (condition-case nil
        (or (isolate-retire isolate) (isolate-kill isolate))
      (isolate-error nil))))

(defun romacs--isolate-zygote-evict ()
  "Retire the least recently used zygote while `isolate-zygote-limit' are live."
  (while (>= (hash-table-count romacs--isolate-zygotes) (max 1 isolate-zygote-limit))
    (let (oldest)
      (maphash (lambda (_ zygote) (when (or (null oldest) (< (aref zygote 10) (aref oldest 10))) (setq oldest zygote)))
               romacs--isolate-zygotes)
      (romacs--isolate-zygote-retire oldest))))

(defun romacs--isolate-zygote-fork (zygote)
  "A new child of ZYGOTE, for one job; `isolate-process-limit' without
room for its process."
  (unless (romacs--isolate-admit-p 'fork (aref zygote 2))
    (signal 'isolate-process-limit (list (length romacs--isolates) (isolate--process-limit))))
  (let* ((parent (aref zygote 1))
         (id (romacs--isolate-fork-request (romacs--isolate-id parent)))
         (child (romacs--isolate-make id (isolate-name parent) (aref zygote 2) (aref parent 8))))
    (aset child 10 'job)
    (aset child 11 zygote)
    (when (eq (aref zygote 3) 'ready)
      (aset zygote 12 (cdr (romacs--isolate-zygote-affinity zygote))))
    (push child romacs--isolates)
    child))

(defun romacs--isolate-reserved ()
  "The bytes reserved for the running forked jobs: the budget each was
forked with."
  (let ((bytes 0))
    (dolist (isolate romacs--isolates bytes)
      (when (and (eq (aref isolate 10) 'job) (aref isolate 5))
        (setq bytes (+ bytes (aref (aref isolate 11) 13)))))))

(defun romacs--isolate-room-p (&optional zygote)
  "Whether one more forked job fits, on ZYGOTE if given: fewer than the
pool's width run, and its memory with the reservations of those and this
one is in bounds.  With nothing running one always fits: no job's end would
ever let it in."
  (let ((busy (romacs--isolate-busy-children)))
    (or (= busy 0)
        (and (< busy (isolate--pool-size zygote))
             (<= (+ (* 1024 (romacs--isolate-pool-memory)) (romacs--isolate-reserved) isolate-job-memory-headroom)
                 isolate-pool-memory-limit)))))

(defun romacs--isolate-zygote-live-p (require)
  "Whether REQUIRE's zygote prepares or is ready, so its jobs wait for it."
  (and (romacs--isolate-zygotes-p)
       (let ((zygote (gethash require romacs--isolate-zygotes)))
         (and zygote (memq (aref zygote 3) '(preparing ready)) t))))

(defun romacs--isolate-busy-children (&optional zygote)
  "How many forked children run a job, of ZYGOTE's (an isolate) if given."
  (let ((count 0))
    (dolist (isolate romacs--isolates count)
      (when (and (eq (aref isolate 10) 'job) (aref isolate 5)
                 (or (null zygote) (eq (aref (aref isolate 11) 1) zygote)))
        (setq count (1+ count))))))

(defun isolate--process-limit ()
  "The bound on worker processes (`isolate-process-limit')."
  (or isolate-process-limit
      (+ (isolate--pool-size) (* 2 (max 1 isolate-zygote-limit)) 4)))

(defun romacs--isolate-ending-p ()
  "Whether a worker process will end without being asked: one retired, or
one running its only job (a forked child, a spent pool worker)."
  (let ((found nil))
    (dolist (isolate romacs--isolates found)
      (when (or (eq (aref isolate 3) 'retired)
                (and (aref isolate 5)
                     (or (eq (aref isolate 10) 'job) (and (romacs--isolate-one-job-p isolate) (aref isolate 6)))))
        (setq found t)))))

(defun romacs--isolate-admit-p (kind &optional require)
  "Whether one more worker process may begin now: fewer than
`isolate-process-limit' run, every role counted until it is reaped, and
for KIND `start' (a worker for pooled jobs, which may wait) the pool's
memory with its jobs' reservations is within `isolate-pool-memory-limit'
while any process is on its way out to free it.  KIND `create' is a
worker asked for outright, which cannot wait; `fork' a zygote's child,
whose job `romacs--isolate-room-p' judges.  Without room and nothing on
its way out, an idle worker of features other than REQUIRE is ended to
make it (nil: the room comes at its end); with none to end,
`isolate-process-limit'."
  (let ((count (length romacs--isolates))
        (ending (romacs--isolate-ending-p)))
    (cond ((and (< count (isolate--process-limit))
                (or (not (eq kind 'start)) (not ending)
                    (<= (+ (* 1024 (romacs--isolate-pool-memory)) (romacs--isolate-reserved))
                        isolate-pool-memory-limit)))
           t)
          (ending nil)
          ((romacs--isolate-evict-idle require) nil)
          (t (signal 'isolate-process-limit (list count (isolate--process-limit)))))))

(defun romacs--isolate-evict-idle (require)
  "End one idle worker of features other than REQUIRE, without waiting: a
pooled one, else the least recently used zygote none of whose children
runs a job.  Non-nil if one was."
  (let ((victim nil))
    (dolist (isolate (romacs--isolate-pooled))
      (when (and (null victim) (eq (aref isolate 10) 'pool) (null (aref isolate 5))
                 (not (equal (aref isolate 4) require)))
        (setq victim isolate)))
    (if victim
        (progn (romacs--isolate-end victim) t)
      (maphash (lambda (features zygote)
                 (when (and (not (equal features require)) (eq (aref zygote 3) 'ready)
                            (= 0 (romacs--isolate-busy-children (aref zygote 1)))
                            (or (null victim) (< (aref zygote 10) (aref victim 10))))
                   (setq victim zygote)))
               romacs--isolate-zygotes)
      (when victim
        (romacs--isolate-zygote-retire victim)
        t))))

(defun romacs--isolate-zygote-worker (require make)
  "The spare child of REQUIRE's zygote, forked now if it has none, when the
zygote is ready or preparing, nothing it read has changed, and the pool
has room (`romacs--isolate-room-p'); else nil.  A zygote whose inputs changed
is retired.  MAKE starts a zygote for REQUIRE when there is none."
  (when (romacs--isolate-zygotes-p)
    (let ((zygote (gethash require romacs--isolate-zygotes)))
      ;; Dead or retired, its death not delivered yet: forgotten now, its
      ;; spare ended.
      (when (and zygote (memq (aref zygote 3) '(preparing ready)) (romacs--isolate-gone-p (aref zygote 1)))
        (romacs--isolate-zygote-retire zygote)
        (setq zygote nil))
      (when (and zygote (memq (aref zygote 3) '(preparing ready)) (not (romacs--isolate-zygote-current-p zygote)))
        (romacs--isolate-zygote-retire zygote)
        ;; Ending it may have resubmitted a queued child's job, which made
        ;; its replacement.
        (setq zygote (gethash require romacs--isolate-zygotes)))
      ;; At the process limit with nothing on its way out, the room is made,
      ;; or `isolate-process-limit' says there is none to make.
      (when (and make (null zygote) (romacs--isolate-admit-p 'start require))
        (setq zygote (condition-case nil (romacs--isolate-zygote-make require) (error nil))))
      (cond ((null zygote) nil)
            ;; A zygote still preparing forks the child once it is ready:
            ;; the job waits for the rest of its preparation, not the session.
            ((not (memq (aref zygote 3) '(preparing ready))) nil)
            ((not (romacs--isolate-room-p zygote)) nil)
            (t (aset zygote 10 (float-time))
               (when (and (aref zygote 4) (romacs--isolate-gone-p (aref zygote 4)))
                 (aset zygote 4 nil))
               ;; A spare keeps the processors it was forked with.
               (let ((affinity (romacs--isolate-zygote-affinity zygote)))
                 (when (and (aref zygote 4) affinity (not (eql (cdr affinity) (aref zygote 12))))
                   (romacs--isolate-end (aref zygote 4))
                   (aset zygote 4 nil)))
               (or (aref zygote 4)
                   ;; nil while room is coming; with none to make the job is
                   ;; told (`isolate-process-limit'), never queued for ever.
                   (and (romacs--isolate-admit-p 'fork require)
                        (condition-case nil (aset zygote 4 (romacs--isolate-zygote-fork zygote)) (error nil)))))))))

(defun romacs--isolate-gone-p (isolate)
  "Whether ISOLATE's process has ended, delivered or not: a wait point
delivers a death after it saw it, so an isolate may look live a while."
  (or (not (isolate-live-p isolate))
      (condition-case nil (romacs--isolate-reap (romacs--isolate-id isolate)) (error t))))

(defun romacs--isolate-zygote-take (isolate)
  "ISOLATE, a forked child, takes its job: when it was its zygote's spare,
the zygote forks the next."
  (let ((zygote (aref isolate 11)))
    (when (and zygote (eq (aref zygote 4) isolate))
      (aset zygote 4 nil)
      (when (eq (gethash (aref zygote 2) romacs--isolate-zygotes) zygote)
        (condition-case nil (aset zygote 4 (romacs--isolate-zygote-fork zygote)) (error nil))))))

(defun romacs--isolate-forget-role (isolate)
  "ISOLATE has ended: a zygote is forgotten (a refused one stays, so its
features are not tried again); a spare leaves its zygote's slot."
  (let ((zygote (aref isolate 11)))
    (cond ((eq (aref isolate 10) 'zygote)
           (let ((record (gethash (aref isolate 4) romacs--isolate-zygotes)))
             (when (and record (eq (aref record 1) isolate) (not (eq (aref record 3) 'refused)))
               (remhash (aref isolate 4) romacs--isolate-zygotes))))
          ((and zygote (eq (aref zygote 4) isolate))
           (aset zygote 4 nil)))))

(defun isolate-zygote (require)
  "The zygote for REQUIRE's features, started now if there is none:
(FEATURES STATE PID SPARE-PID), STATE being `preparing', `ready' or
`refused'."
  (romacs--isolate-zygote-describe
   (or (gethash require romacs--isolate-zygotes)
       (romacs--isolate-zygote-make require))))

(defun isolate-zygote-list ()
  "The zygotes, each as `isolate-zygote' describes it."
  (let (zygotes)
    (maphash (lambda (_ zygote) (push (romacs--isolate-zygote-describe zygote) zygotes))
             romacs--isolate-zygotes)
    zygotes))

(defun romacs--isolate-zygote-describe (zygote)
  (let ((spare (aref zygote 4)))
    (list (aref zygote 2) (aref zygote 3)
          (romacs--isolate-pid (romacs--isolate-id (aref zygote 1)))
          (and spare (romacs--isolate-pid (romacs--isolate-id spare))))))

;;;; Waiting and delivery

(defun romacs--isolate-pump (timeout)
  "Wait up to TIMEOUT for isolate traffic and deliver what has arrived,
including frames an earlier wait read and nobody delivered; non-nil if
anything came now."
  (let ((came (romacs--isolate-wait timeout)))
    ;; The deadlines are checked, and their wake set, whatever a callback
    ;; the dispatch ran signals.
    (unwind-protect (romacs--isolate-dispatch)
      (romacs--isolate-check-deadlines))
    came))

(defun romacs--isolate-idle ()
  "What a wait point does for the isolates: deliver what has arrived, and
dispatch the jobs waiting for room, which no worker may be left to wake."
  (when (or romacs--isolates romacs--isolate-waiting)
    (romacs--isolate-pump 0)))

(defun romacs--isolate-settle (isolate state value)
  "Resolve every pending future of ISOLATE as STATE with VALUE, all before
any of their callbacks runs, and run each callback whatever an earlier
one signals."
  (romacs--isolate-each #'romacs--isolate-notify (romacs--isolate-take-settled isolate state value)))

(defun romacs--isolate-take-settled (isolate state value)
  "Take ISOLATE's pending futures and resolve them as STATE with VALUE,
running no callback: what `romacs--isolate-notify' is to run, a list."
  (let ((pending (aref isolate 5)) (resolved nil))
    (aset isolate 5 nil)
    (dolist (entry pending (nreverse resolved))
      (let ((one (romacs--isolate-mark-resolved (cdr entry) state value)))
        (when one (push one resolved))))))

(defun romacs--isolate-each (function list)
  "Call FUNCTION on each element of LIST, in order, every one whatever an
earlier call signals or throws; that exit goes on once all have run.  It
loops, and nests only past an exit."
  (while list
    (let ((done nil) (element (pop list)))
      (unwind-protect (progn (funcall function element) (setq done t))
        (unless done
          (romacs--isolate-each function (prog1 list (setq list nil))))))))

(defun romacs--isolate-dead (isolate event data)
  "ISOLATE's worker is gone for EVENT with DATA: the children it reported
and had not retired get SIGHUP to their groups (what its own shutdown would
have sent), its futures settle, the event is reported, and it is forgotten."
  (let ((retired (eq (aref isolate 3) 'retired))
        (pending (aref isolate 5)))
    (unless (memq (aref isolate 3) '(killed retired))
      (aset isolate 3 'dead))
    (let ((children (romacs--isolate-signal-children (romacs--isolate-id isolate)))
          (orphans nil) (unforked nil) (own nil)
          ;; A child its zygote never reported was never released, so it
          ;; ran nothing (its zygote died first): its job runs elsewhere.
          (resubmit (and (eq (aref isolate 10) 'job) (eq event 'isolate-died)
                         (null (romacs--isolate-pid (romacs--isolate-id isolate))))))
      (romacs--isolate-forget-role isolate)
      ;; Out of the dispatch before any callback runs: a pump a callback
      ;; makes cannot handle this death again.
      (setq romacs--isolates (delq isolate romacs--isolates))
      (when (eq (aref isolate 10) 'zygote)
        (let ((ended (romacs--isolate-end-children isolate)))
          (setq orphans (car ended) unforked (cdr ended))))
      (if resubmit
          (aset isolate 5 nil)
        (setq own (romacs--isolate-take-settled isolate event (list isolate data))))
      ;; Notified, forgotten and reported whatever a callback signals.
      (unwind-protect
          (romacs--isolate-each
           #'funcall
           (list (lambda () (romacs--isolate-notify-orphans orphans unforked))
                 (lambda ()
                   (if resubmit
                       (romacs--isolate-each (lambda (entry) (romacs--isolate-resubmit-or-fail (cdr entry))) pending)
                     (romacs--isolate-each #'romacs--isolate-notify own)))))
        (romacs--isolate-close (romacs--isolate-id isolate))
        ;; A spare child that dies took no job: it is replaced, not reported.
        (unless (or retired (and (eq (aref isolate 10) 'job) (null pending)))
          (run-hook-with-args 'isolate-error-functions isolate event (list data children)))))))

(defun romacs--isolate-end-children (zygote)
  "ZYGOTE, an isolate, has died: its children with a job die with it
\(a job never started runs elsewhere), whatever they answer meanwhile.
One that runs gets SIGKILL and stays counted (`isolate-process-limit')
until its own exit arrives, which is then no news; its futures are
settled now, running no callback (no later frame of its answers one).
\(ORPHANS . UNFORKED): ORPHANS a list of (CHILD RESOLVED...), what
`romacs--isolate-notify' is to run for it, UNFORKED the children never
reported, each in the order their deaths are to be told."
  (let ((orphans nil) (unforked nil))
    (dolist (child (copy-sequence romacs--isolates))
      (when (and (eq (aref child 10) 'job) (eq (aref (aref child 11) 1) zygote) (aref child 5))
        (if (null (romacs--isolate-pid (romacs--isolate-id child)))
            (push child unforked)
          (push (cons child (romacs--isolate-take-settled child 'isolate-died (list child nil))) orphans)
          (romacs--isolate-signal (romacs--isolate-id child) 9)
          (romacs--isolate-mark-ending child))))
    (cons (nreverse orphans) (nreverse unforked))))

(defun romacs--isolate-notify-orphans (orphans unforked)
  "Tell the deaths `romacs--isolate-end-children' found, ORPHANS and
UNFORKED: each orphan's callbacks, then its death to
`isolate-error-functions'; each unforked child's death handled, unless a
callback's pump has handled it meanwhile.  Every one whatever an earlier
one signals."
  (unwind-protect
      (romacs--isolate-each
       (lambda (orphan)
         (unwind-protect (romacs--isolate-each #'romacs--isolate-notify (cdr orphan))
           (run-hook-with-args 'isolate-error-functions (car orphan) 'isolate-died (list nil nil))))
       orphans)
    (romacs--isolate-each (lambda (child) (when (memq child romacs--isolates)
                                            (romacs--isolate-dead child 'isolate-died nil)))
                          unforked)))

(defun romacs--isolate-world-refused (isolate reason)
  "ISOLATE's worker could not adopt the world it was told to, for REASON.
It is ended; its jobs settle with `isolate-world-unavailable', whose data
is REASON and that world; and the world is forgotten when `isolate-world'
still names it, so later workers boot from source
\(`romacs--isolate-boot-arguments'), none of them refused again."
  (let ((id (romacs--isolate-id isolate))
        (world (aref isolate 12)))
    (when (equal isolate-world world)
      (setq isolate-world nil romacs--isolate-world-refusal (list reason world)))
    (romacs--isolate-signal id 9)
    (while (null (romacs--isolate-reap id)) (romacs--isolate-wait 0.01))
    ;; Out of the dispatch before its futures' callbacks run: the death
    ;; the reap queued is not reported as well; and closed whatever a
    ;; callback signals, since no later dispatch will see it.
    (setq romacs--isolates (delq isolate romacs--isolates))
    (unwind-protect
        (romacs--isolate-dead isolate 'isolate-world-unavailable (list reason world))
      (romacs--isolate-close id))))

(defun romacs--isolate-memory-error (budget)
  "A job's reply when its child ended for growing past BUDGET bytes."
  (list 'isolate-memory-exhausted (list budget) nil
        (get 'isolate-memory-exhausted 'error-conditions)))

(defun romacs--isolate-resubmit-or-fail (future)
  "Queue FUTURE's job again elsewhere, or settle it with the error that stops it."
  (condition-case err
      (romacs--isolate-resubmit future)
    (error (romacs--isolate-resolve future 'error (list (car err) (cdr err) nil (get (car err) 'error-conditions))))))

(defun romacs--isolate-accept-hello (isolate text)
  "Handle ISOLATE's hello TEXT and settle it if refused."
  (aset isolate 9 t)
  (let ((mismatch (romacs--isolate-hello-check text (aref isolate 8)))
        (id (romacs--isolate-id isolate)))
    (cond
     (mismatch
      (romacs--isolate-signal id 9)
      (while (null (romacs--isolate-reap id)) (romacs--isolate-wait 0.01))
      (romacs--isolate-dead isolate 'isolate-refused (list mismatch text)))
     ((romacs--isolate-hello-refusal text)
      (romacs--isolate-world-refused isolate (romacs--isolate-hello-refusal text))))))

(defun romacs--isolate-dispatch ()
  "Deliver every frame that has arrived from every worker."
  (dolist (isolate (copy-sequence romacs--isolates))
    (let ((id (romacs--isolate-id isolate)) frame)
      (while (and (memq isolate romacs--isolates) (setq frame (romacs--isolate-take id)))
        (let ((kind (car frame)) (generation (nth 1 frame)) (value (nth 2 frame)))
          ;; A result that could not be read settles its job with the reason.
          (when (and (eq kind 'frame-error) (memq (car value) '(result error)))
            (setq kind 'error value (list (romacs--isolate-frame-error-reply value) nil)))
          (cond
           ((eq kind 'hello)
            (romacs--isolate-accept-hello isolate value))
           ((memq kind '(result error))
            ;; A forked child exits after its one job: its end is no news.
            (when (eq (aref isolate 10) 'job) (romacs--isolate-mark-ending isolate))
            (let ((entry (assq generation (aref isolate 5))))
              (when (nth 1 value) (aset isolate 6 t))
              (when entry
                (aset isolate 5 (delq entry (aref isolate 5)))
                (aset isolate 7 nil)
                (unwind-protect
                    (if (and (eq kind 'error) (eq (car (car value)) 'isolate-retired))
                        ;; Queued behind a job that changed the features: it
                        ;; runs on a worker with the features it was queued for.
                        (condition-case err
                            (romacs--isolate-resubmit (cdr entry))
                          (error (romacs--isolate-resolve (cdr entry) 'error (list (car err) (cdr err) nil (get (car err) 'error-conditions)))))
                      (romacs--isolate-resolve (cdr entry) (if (eq kind 'result) 'done 'error) (car value)))
                  ;; A spent worker of the pool's ends with its job's answer.
                  (when (and (romacs--isolate-one-job-p isolate) (aref isolate 6) (null (aref isolate 5)))
                    (isolate-retire isolate))))))
           ((eq kind 'message)
            (run-hook-with-args 'isolate-message-functions isolate value))
           ;; A text frame, or a message that could not be read: the frame.
           ((memq kind '(text frame-error))
            (run-hook-with-args 'isolate-message-functions isolate frame))
           ((eq kind 'memory)
            ;; A job child ending for growing past its budget, the one its
            ;; zygote record holds, its death next: its job's reply,
            ;; `isolate-memory-exhausted', and its end no news. Only a job's
            ;; child sends it.
            (when (eq (aref isolate 10) 'job)
              (romacs--isolate-mark-ending isolate)
              (romacs--isolate-settle isolate 'error (romacs--isolate-memory-error (aref (aref isolate 11) 13)))))
           ((eq kind 'died)
            (romacs--isolate-dead isolate 'isolate-died value)))))))
  (romacs--isolate-drain-waiting))

(defun romacs--isolate-check-deadlines ()
  "Kill a worker whose interrupted job has not stopped in time.  Signal a
retired one that has not ended `isolate-kill-timeout' after its retiring
\(a zygote's from when its last child's job ended): SIGTERM, then SIGKILL
as long after; its death arrives at a later wait point.  Never waits."
  (let ((now (float-time)))
    ;; The wake is set for the next deadline whatever a kill's callback signals.
    (unwind-protect
        (dolist (isolate (copy-sequence romacs--isolates))
          (let ((deadline (aref isolate 7))
                (ending (aref isolate 13)))
            (cond ((and deadline (< (cdr deadline) now) (assq (car deadline) (aref isolate 5)))
                   (isolate-kill isolate))
                  ((and ending (eq (aref isolate 3) 'retired) (< (+ (car ending) isolate-kill-timeout) now)
                        (not (eql (cdr ending) 9)))
                   (if (and (eq (aref isolate 10) 'zygote) (> (romacs--isolate-busy-children isolate) 0))
                       (setcar ending now)
                     (let ((number (if (cdr ending) 9 15)))
                       (condition-case nil (romacs--isolate-signal (romacs--isolate-id isolate) number) (error nil))
                       (aset isolate 13 (cons now number))))))))
      (let ((next nil))
        (dolist (isolate romacs--isolates)
          (let* ((deadline (aref isolate 7)) (ending (aref isolate 13))
                 (at (cond ((and deadline (assq (car deadline) (aref isolate 5))) (cdr deadline))
                           ((and ending (eq (aref isolate 3) 'retired) (not (eql (cdr ending) 9)))
                            (+ (car ending) isolate-kill-timeout)))))
            (when (and at (or (null next) (< at next))) (setq next at))))
        (when next (romacs--isolate-wake-at next))))))

(defun romacs--isolate-future-value (future)
  "FUTURE's value, or its worker's error re-signalled."
  (let ((state (aref future 3)) (value (aref future 4)))
    (cond ((eq state 'done) value)
          ((eq state 'error)
           (let ((condition (nth 0 value)) (data (nth 1 value)) (conditions (nth 3 value)))
             ;; A condition the worker defined: its whole ancestry, so every
             ;; ancestor's handler matches as it would there.
             (unless (get condition 'error-conditions)
               (put condition 'error-conditions (if (consp conditions) conditions (list condition 'error)))
               (put condition 'error-message "Isolate worker error"))
             (let ((isolate-error-backtrace (nth 2 value)))
               (signal condition (if (frozen-p data) (thaw data) data)))))
          ((eq state 'pending) (signal 'isolate-timeout (list future)))
          ((eq state 'cancelled) (signal 'quit nil))
          (t (signal state (if (listp value) value (list value)))))))

(defun isolate-await (future &optional timeout)
  "Wait for FUTURE: its frozen value, or its worker's error re-signalled.
With TIMEOUT seconds, `isolate-timeout' when it has not resolved by then.
Isolate traffic and timers are pumped meanwhile, never the keyboard."
  (let ((end (and timeout (+ (float-time) timeout))))
    (while (and (eq (aref future 3) 'pending)
                (or (null end) (< (float-time) end)))
      ;; At most half a second a wait, so the deadlines are checked.
      (romacs--isolate-pump (if end (min 0.5 (max 0 (- end (float-time)))) 0.5))
      (when (fboundp 'romacs--timer-check) (romacs--timer-check))))
  (romacs--isolate-future-value future))

(defun romacs--isolate-interrupt-generation (isolate generation &optional queued)
  "Interrupt ISOLATE's job GENERATION: the frame naming it, then SIGUSR1.
The worker takes the signal only while running that generation; the frame
cancels the job before it runs otherwise. QUEUED sets no kill deadline.
A worker whose hello has not arrived does not catch signals yet and runs
no job: it gets the frame alone."
  (romacs--isolate-post (romacs--isolate-id isolate) 'interrupt generation nil)
  (when (aref isolate 9)
    (romacs--isolate-signal (romacs--isolate-id isolate) 10))
  (unless queued
    (aset isolate 7 (cons generation (+ (float-time) isolate-interrupt-timeout)))
    (romacs--isolate-wake-at (cdr (aref isolate 7)))))

(defun isolate-future-cancel (future)
  "Cancel FUTURE: interrupt its worker if it is running this job, else drop it.
What has arrived is taken first, so a job that finished is left alone and
the job running after it is not hit in its place."
  (romacs--isolate-pump 0)
  ;; The pump may have queued the job again elsewhere (a retired worker):
  ;; its worker and generation are read after it.
  (let ((isolate (aref future 1)))
    (when (and (eq (aref future 3) 'pending) (null isolate))
      ;; Waiting for room: it never ran.
      (setq romacs--isolate-waiting (delq future romacs--isolate-waiting))
      (romacs--isolate-resolve future 'cancelled nil))
    (when (eq (aref future 3) 'pending)
      (let ((pending (aref isolate 5)))
        (if (and pending (eq (cdr (car pending)) future))
            (romacs--isolate-interrupt-generation isolate (aref future 2))
          ;; Queued: the worker skips it when its turn comes, or quits it if
          ;; it has just started.
          (aset isolate 5 (delq (rassq future pending) pending))
          (condition-case nil
              (romacs--isolate-interrupt-generation isolate (aref future 2) t)
            (error nil))
          (romacs--isolate-resolve future 'cancelled nil))))))

;;;; Messages

(defun isolate-send (isolate message)
  "Send MESSAGE, frozen, to ISOLATE (`parent' in a worker)."
  (romacs--isolate-post (if (eq isolate 'parent) 0 (romacs--isolate-id isolate)) 'message 0 message)
  message)

(defun isolate-send-text (isolate text)
  "Send TEXT as an inline UTF-8 frame to ISOLATE (`parent' in a worker).
Return TEXT after sending it.  Its bytes must be valid UTF-8 and fit in
4096 bytes.  It arrives as (text 0 STRING), with a fresh string."
  (romacs--isolate-post (if (eq isolate 'parent) 0 (romacs--isolate-id isolate)) 'text 0 text)
  text)

(defun isolate-receive (&optional timeout)
  "In a worker: the next message from the parent, waiting up to TIMEOUT.
Return the frozen value from `isolate-send', (text GENERATION STRING)
for a text frame, or (frame-error GENERATION (message REASON)) for a
message that could not be read.  Return nil if no message arrives.
A TIMEOUT of 0 polls: it takes what has arrived without waiting, so a
running job can look for new requests between its own steps."
  (let ((end (and timeout (+ (float-time) timeout))))
    (unless romacs--isolate-inbox
      (romacs--isolate-note-control-frames))
    (while (and (null romacs--isolate-inbox)
                (or (null end) (< (float-time) end)))
      (romacs--isolate-wait (if end (max 0 (- end (float-time))) nil))
      (romacs--isolate-serve-frames))
    (pop romacs--isolate-inbox)))

;;;; Interrupting and killing

(defun isolate-interrupt (isolate)
  "Make ISOLATE's running job quit at its next safepoint, as C-g would.
The interrupt names the job's generation, so a job queued behind it is
cancelled before it runs and a job that has finished is left alone."
  (when (isolate-live-p isolate)
    ;; What has finished is not running: take what arrived first.
    (romacs--isolate-pump 0)
    (let ((running (car (aref isolate 5))))
      (when running
        (romacs--isolate-interrupt-generation isolate (car running))
        t))))

(defun isolate-kill (isolate)
  "End ISOLATE now: SIGTERM, then SIGKILL after `isolate-kill-timeout'.
The children its worker reported and had not reaped get SIGHUP to their
groups.  Pending futures resolve with `isolate-killed'.  The outcomes of
the children's signals, an alist of (PID . OUTCOME)."
  (when (and (isolate-live-p isolate) (not (romacs--isolate-kill-unforked isolate)))
    (let ((id (romacs--isolate-id isolate)))
      (romacs--isolate-signal id 15)
      (let ((end (+ (float-time) isolate-kill-timeout)))
        (while (and (null (romacs--isolate-reap id)) (< (float-time) end))
          (romacs--isolate-wait 0.02)))
      (unless (romacs--isolate-reap id)
        (romacs--isolate-signal id 9)
        (while (null (romacs--isolate-reap id))
          (romacs--isolate-wait 0.02)))
      (aset isolate 3 'killed)
      (let ((children (romacs--isolate-signal-children id)))
        (romacs--isolate-dead isolate 'isolate-killed children)
        children))))

(defun romacs--isolate-kill-unforked (isolate)
  "When ISOLATE is a child its zygote has not reported yet, wait up to
`isolate-kill-timeout' for the report.  A child still unreported then
(never released, so it ran nothing) is killed by settling its jobs as
killed and ending its zygote, which ends it too: t.  Else nil, and
`isolate-kill' signals it as any worker."
  (let ((id (romacs--isolate-id isolate)))
    (when (and (eq (aref isolate 10) 'job) (null (romacs--isolate-pid id)))
      (let ((end (+ (float-time) isolate-kill-timeout)))
        (while (and (null (romacs--isolate-pid id)) (null (romacs--isolate-reap id)) (< (float-time) end))
          (romacs--isolate-wait 0.02)))
      (when (null (romacs--isolate-pid id))
        ;; Its jobs are killed before its zygote's end could send them elsewhere.
        (aset isolate 3 'killed)
        (romacs--isolate-settle isolate 'isolate-killed (list isolate nil))
        (isolate-kill (aref (aref isolate 11) 1))
        (when (memq isolate romacs--isolates)
          (romacs--isolate-dead isolate 'isolate-killed nil))
        t))))

(defun isolate-retire (isolate)
  "End idle ISOLATE without waiting for it: it takes no more jobs, gets
`bye' and ends its loop as a worker does on its spawner's bye; its end is
then forgotten at a wait point, not reported as a death.  A zygote ends
once its children have: those running a job finish it, the idle ones (its
spare) are retired with it.  One that has not ended in
`isolate-kill-timeout' is signalled (`romacs--isolate-check-deadlines').
Nil, doing nothing, unless ISOLATE is live and runs nothing."
  (when (and (isolate-live-p isolate) (null (aref isolate 5)))
    (romacs--isolate-mark-ending isolate)
    ;; A worker gone, or not reading, gets no bye: its death, or its
    ;; deadline (`romacs--isolate-check-deadlines'), ends it in any case.
    (condition-case nil (romacs--isolate-post (romacs--isolate-id isolate) 'bye 0 nil)
      (isolate-error nil))
    (when (eq (aref isolate 10) 'zygote)
      (dolist (child (copy-sequence romacs--isolates))
        (when (and (eq (aref child 10) 'job) (eq (aref (aref child 11) 1) isolate))
          (isolate-retire child))))
    t))

(defun romacs--isolate-mark-ending (isolate)
  "ISOLATE is retired: it ends by itself, from now."
  (aset isolate 3 'retired)
  (aset isolate 13 (cons (float-time) nil))
  (romacs--isolate-wake-at (+ (float-time) isolate-kill-timeout)))

(defvar romacs--isolate-wake nil
  "(TIME . TIMER) of the timer that next pumps for a deadline, or nil.")

(defun romacs--isolate-wake-at (time)
  "Have a timer pump at TIME, unless one does sooner: a deadline must not
wait for traffic or input.  A VM without timers is pumped by its waits
\(`isolate-await' every half second)."
  (when (and (fboundp 'run-at-time)
             (or (null romacs--isolate-wake) (< time (car romacs--isolate-wake))
                 ;; One whose timer is gone without firing (cancelled).
                 (not (memq (cdr romacs--isolate-wake) timer-list))))
    (when romacs--isolate-wake (cancel-timer (cdr romacs--isolate-wake)))
    (setq romacs--isolate-wake
          (cons time (run-at-time (max 0 (- time (float-time))) nil #'romacs--isolate-wake-pump)))))

(defun romacs--isolate-wake-pump ()
  "A deadline's timer: pump, which checks the deadlines and sets the next."
  (setq romacs--isolate-wake nil)
  (romacs--isolate-pump 0))

(defun isolate-kill-all ()
  "Kill every live isolate, as a session's end does."
  (let ((waiting romacs--isolate-waiting))
    (setq romacs--isolate-waiting nil)
    (dolist (future waiting)
      (romacs--isolate-resolve future 'isolate-killed (list nil nil))))
  (dolist (isolate (copy-sequence romacs--isolates))
    (isolate-kill isolate)))

(provide 'isolate)
;;; isolate.el ends here

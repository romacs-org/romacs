;;; isolate.el --- Isolated VMs that exchange frozen values  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The Lisp half of isolates (vm-concurrency design §4): worker VMs in
;; child processes that run a function on frozen arguments and answer with
;; a frozen result, over the leaf primitives of prims/isolate.rs.  A worker
;; adopts its spawner's world (or boots as its spawner did), sees none of
;; the spawner's later state, and is seeded once with
;; `isolate-inherited-variables'.  Every value that crosses is frozen
;; (`freeze'); closures cannot cross, a symbol or a lambda form with
;; `:bindings' can (DIVERGENCES.md VM2-ISOLATE-CLOSURES).
;;
;; Not in GNU (DIVERGENCES.md VM2-ISOLATE).  `isolate-await' re-signals a
;; worker's error where GNU's `thread-join' would store it in
;; `thread-last-error'.
;;
;; Delivery: results, errors and messages arrive at wait points.  The
;; command loop's idle (`romacs--isolate-idle', rlisp/vm/keyboard.el and
;; the GUI's timers) and `isolate-await' pump them; a worker's loop
;; (`romacs--isolate-serve') takes its jobs the same way.

;;; Code:

(defvar isolate-boot 'bare
  "How this VM booted, which its workers repeat: `world', `source' or `bare'.
The launcher sets it; a VM made by `Vm::new' is bare.")

(defvar isolate-world nil
  "The world this VM restored from, as (STORE . KEY), or nil.
A worker of a `world' VM adopts exactly this world.")

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
one job and exits.  t: always; nil: never (jobs run on spawned, reused
workers); `world': when this VM restored from a world, as a session does.")

(defvar isolate-zygote-warm-functions nil
  "Alist of (FEATURE . FUNCTION): a zygote whose features include FEATURE
calls FUNCTION once before it forks, so its children inherit what FUNCTION
caches for the session (a program's version, say) instead of each child
computing it again.  An error in FUNCTION is ignored.")

(defvar isolate-job-memory-headroom (* 64 1024 1024)
  "Bytes a job on a forked child may grow its heap by.
Allocation below this runs no collection, whose garbage dies with the
child; live growth past it signals memory exhaustion in the job.")

(defvar isolate-pool-memory-limit (* 1024 1024 1024)
  "Bytes the pool may hold, sampled as proportional memory, with
`isolate-job-memory-headroom' reserved for each running job.
A job that would pass it waits for room.")

(defvar isolate-zygote-memory-limit (* 256 1024 1024)
  "Bytes of proportional memory a zygote may hold once prepared; past it,
its features run on pooled workers.")

(defvar isolate-zygote-limit 4
  "How many zygotes may live at once; the least recently used one goes first.")

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
  "Abnormal hook run with (SENDER MESSAGE) for each message received.
In the spawner SENDER is the worker's isolate; in a worker it is `parent'.")

(defvar isolate-error-functions nil
  "Abnormal hook run with (ISOLATE EVENT DATA) when a worker dies or is refused.")

(defvar isolate-error-backtrace nil
  "The worker's backtrace while `isolate-await' re-signals its error.")

(defvar romacs--isolates nil "The live isolates this VM spawned.")
(defvar romacs--isolate-generation 0 "The last job generation this VM issued.")
(defvar romacs--isolate-inbox nil "In a worker: messages received and not yet taken, oldest first.")
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

;;;; Records

;; An isolate: [isolate ID NAME STATE REQUIRE PENDING TAINTED DEADLINE
;; PROGRAM HELLO ROLE ZYGOTE].  STATE is `live', `killed', `retired' or `dead'; PENDING an alist of
;; (GENERATION . FUTURE) in order of submission, the first running; TAINTED
;; once a job changed the worker's features (not reused); DEADLINE
;; (GENERATION . TIME) after an interrupt; PROGRAM what the worker runs;
;; HELLO once the worker's hello arrived (it catches signals from then);
;; ROLE nil for a worker `isolate-create' made, `pool' for one the pool
;; made, `zygote', or `job' for a child forked for one job; ZYGOTE the
;; zygote record a `job' child is of.
(defun romacs--isolate-make (id name require program)
  (record 'isolate id name 'live require nil nil nil program nil nil nil))
(defun isolate-p (object) "Non-nil for an isolate." (eq (type-of object) 'isolate))
(defun romacs--isolate-id (isolate) (aref isolate 1))
(defun isolate-name (isolate) "ISOLATE's name, or nil." (aref isolate 2))
(defun isolate-live-p (isolate) "Non-nil while ISOLATE's worker runs." (eq (aref isolate 3) 'live))
(defun isolate-list () "The live isolates." (copy-sequence romacs--isolates))
(defun isolate-parent ()
  "In a worker, `parent', the isolate that spawned it; nil in a spawner."
  (and (romacs--isolate-parent-p) 'parent))
(defun isolate-main-p () "Non-nil in the VM that spawns workers, not in one." (not (romacs--isolate-parent-p)))

;; A future: [isolate-future ISOLATE GENERATION STATE VALUE CALLBACKS JOB].
;; STATE is `pending', `done', `error', `cancelled', `killed' or `died';
;; JOB (FUNCTION ARGS BINDINGS REQUIRE DIRECTORY) is what was queued, for a
;; resubmission when its worker retires before running it.
(defun romacs--isolate-future (isolate generation job)
  (record 'isolate-future isolate generation 'pending nil nil job))
(defun isolate-future-p (object) "Non-nil for a future." (eq (type-of object) 'isolate-future))
(defun isolate-future-isolate (future) "The isolate FUTURE runs on." (aref future 1))
(defun isolate-future-done-p (future) "Non-nil once FUTURE is resolved." (not (eq (aref future 3) 'pending)))

(defun romacs--isolate-resolve (future state value)
  "Resolve FUTURE as STATE with VALUE and run its callbacks."
  (when (eq (aref future 3) 'pending)
    (aset future 3 state)
    (aset future 4 value)
    (let ((callbacks (aref future 5)))
      (aset future 5 nil)
      (dolist (callback (reverse callbacks))
        (funcall callback future)))))

(defun isolate-on-done (future callback)
  "Call CALLBACK with FUTURE once it is resolved, at a wait point; now if it is."
  (if (isolate-future-done-p future)
      (funcall callback future)
    (aset future 5 (cons callback (aref future 5))))
  future)

;;;; Spawning

(defun romacs--isolate-boot-arguments (allow-source-boot)
  "The worker's boot arguments for this VM's boot, or a refusal."
  (cond ((eq isolate-boot 'world)
         (unless isolate-world
           (signal 'isolate-world-unavailable (list "This VM has no world a worker could adopt")))
         (append (list "--world" (car isolate-world) (cdr isolate-world))
                 (and isolate-compiled-store (list "--compiled-store" isolate-compiled-store))))
        ((eq isolate-boot 'source)
         (unless allow-source-boot
           (signal 'isolate-world-unavailable (list "This VM booted from source; pass :allow-source-boot t")))
         (list "--source"))
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

(defun romacs--isolate-prepare (seeds require)
  "In a worker: install the SEEDS as this VM's own values and load REQUIRE."
  (dolist (seed seeds)
    (set (car seed) (thaw (cdr seed))))
  (setq romacs--isolate-baseline
        (list (let ((files (make-hash-table :test 'equal)))
                (dolist (entry load-history files)
                  (when (stringp (car-safe entry)) (puthash (car entry) t files))))
              (romacs--isolate-timers 'timer-list) (romacs--isolate-timers 'timer-idle-list)))
  (dolist (feature require)
    (require feature))
  t)

(defun isolate-create (&rest keys)
  "Start a long-lived worker isolate; KEYS are :name, :require, :allow-source-boot.
The worker boots as this VM did, is seeded with `isolate-inherited-variables'
and has the features of :require loaded."
  (let* ((name (plist-get keys :name))
         (require (plist-get keys :require))
         (arguments (append (list "--parent" (number-to-string (emacs-pid)))
                            (romacs--isolate-boot-arguments (plist-get keys :allow-source-boot))
                            isolate-worker-arguments))
         (program (or isolate-worker-program (romacs--isolate-executable)))
         (id (romacs--isolate-start program arguments))
         (isolate (romacs--isolate-make id name require program)))
    (push isolate romacs--isolates)
    (romacs--isolate-submit isolate 'romacs--isolate-prepare (list (romacs--isolate-seeds) require) nil nil nil)
    isolate))

(defun romacs--isolate-recycle ()
  "Kill the idle workers a job changed the features of: they are not reused."
  (dolist (isolate (copy-sequence romacs--isolates))
    (when (and (isolate-live-p isolate) (aref isolate 6) (null (aref isolate 5)))
      (isolate-kill isolate))))

(defun romacs--isolate-acquire (require fresh allow-source-boot)
  "A pooled worker for jobs requiring REQUIRE: an idle one with the same
features, else the least loaded one with them, else a new one while
`isolate-pool-size' allows, making room by killing an idle worker of other
features; `isolate-queue-full' when every pooled worker is busy with other
features.  FRESH asks for a new worker outside the pool's bound."
  (romacs--isolate-recycle)
  ;; A retiring worker (tainted, still answering the jobs queued behind
  ;; the change) is on its way out: it does not count toward the bound.
  (let ((live (romacs--isolate-pooled))
        (forked (and (not fresh) (romacs--isolate-zygote-worker require t)))
        best idle)
    (unless (or fresh forked)
      (dolist (isolate live)
        (when (and (equal (aref isolate 4) require)
                   (or (null best) (< (length (aref isolate 5)) (length (aref best 5)))))
          (setq best isolate))))
    (cond (forked forked)
          ;; A live zygote with no room: the job waits for it.
          ((and (not fresh) (romacs--isolate-zygote-live-p require)) 'queued)
          (fresh (romacs--isolate-pool-create require allow-source-boot))
          ((and best (null (aref best 5))) best)
          ((< (length live) (isolate--pool-size))
           (romacs--isolate-pool-create require allow-source-boot))
          (best best)
          ((setq idle (let (found) (dolist (i live) (when (null (aref i 5)) (setq found i))) found))
           (isolate-kill idle)
           (romacs--isolate-pool-create require allow-source-boot))
          (t (signal 'isolate-queue-full (list require))))))

(defun isolate-pending (isolate)
  "How many jobs ISOLATE has queued or running."
  (length (aref isolate 5)))

(defun romacs--isolate-pooled ()
  "The live pooled workers: neither zygotes nor their children, nor retiring."
  (delq nil (mapcar (lambda (i) (and (isolate-live-p i) (not (aref i 6)) (memq (aref i 10) '(nil pool)) i)) romacs--isolates)))

(defun romacs--isolate-pool-create (require allow-source-boot)
  "A worker the pool makes for REQUIRE's features."
  (let ((isolate (isolate-create :require require :allow-source-boot allow-source-boot)))
    (aset isolate 10 'pool)
    isolate))

(defun isolate-idle-worker (require)
  "A worker prepared with REQUIRE's features and running nothing, or nil:
the spare child of REQUIRE's zygote, else an idle pooled worker."
  (or (romacs--isolate-zygote-worker require nil)
      (let (found)
        (dolist (isolate (romacs--isolate-pooled))
          (when (and (aref isolate 9) (null (aref isolate 5)) (equal (aref isolate 4) require))
            (setq found isolate)))
        found)))

(defun isolate-warm (require &optional allow-source-boot)
  "Prepare a worker with REQUIRE's features ahead of its first job: its
zygote when pooled jobs run on forked workers (`isolate-zygotes'), else a
pooled worker, unless one is live (prepared or preparing) or the pool is at
`isolate-pool-size'; the isolate started, or nil."
  (let ((zygote (and (romacs--isolate-zygotes-p) (gethash require romacs--isolate-zygotes))))
    (cond ((not (romacs--isolate-zygotes-p)) (romacs--isolate-warm-pooled require allow-source-boot))
          ((null zygote) (aref (romacs--isolate-zygote-make require) 1))
          ;; Features no zygote can hold run on pooled workers.
          ((eq (aref zygote 3) 'refused) (romacs--isolate-warm-pooled require allow-source-boot))
          (t (romacs--isolate-zygote-worker require t) nil))))

(defun romacs--isolate-warm-pooled (require allow-source-boot)
  "Start a pooled worker with REQUIRE's features unless one is live or the
pool is full; the isolate started, or nil."
  (romacs--isolate-recycle)
  (let ((live (romacs--isolate-pooled)))
    (and (not (romacs--isolate-with-features live require))
         (< (length live) (isolate--pool-size))
         (romacs--isolate-pool-create require allow-source-boot))))

(defun isolate-pooled-worker (require &optional allow-source-boot)
  "A worker for background work with REQUIRE's features: an idle prepared
one (a zygote's spare child first), else a pooled one started now while the
pool has room, else the least loaded live pooled one with those features;
nil when there is none.  Unlike `isolate-spawn', it never kills a worker of
other features to make room."
  (or (isolate-idle-worker require)
      (romacs--isolate-zygote-worker require t)
      (and (not (romacs--isolate-zygote-live-p require))
           (condition-case nil (romacs--isolate-warm-pooled require allow-source-boot) (error nil)))
      (let (best)
        (dolist (isolate (romacs--isolate-pooled) best)
          (when (and (equal (aref isolate 4) require)
                     (or (null best) (< (length (aref isolate 5)) (length (aref best 5)))))
            (setq best isolate))))))

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
    ;; Sent: a spare is taken, and its zygote forks the next.
    (when (eq (aref isolate 10) 'job) (romacs--isolate-zygote-take isolate))
    future))

(defun romacs--isolate-resubmit (future)
  "Queue FUTURE's job again, on a worker with the features it was queued
for: its worker retired before running it."
  (let* ((job (aref future 6))
         (require (thaw (nth 3 job)))
         (isolate (romacs--isolate-acquire require nil t)))
    (if (eq isolate 'queued)
        (progn (aset future 1 nil)
               (setq romacs--isolate-waiting (cons future romacs--isolate-waiting)))
      (setq romacs--isolate-generation (1+ romacs--isolate-generation))
      (romacs--isolate-post (romacs--isolate-id isolate) 'job romacs--isolate-generation job)
      (aset future 1 isolate)
      (aset future 2 romacs--isolate-generation)
      (aset isolate 5 (append (aref isolate 5) (list (cons romacs--isolate-generation future))))
      (when (eq (aref isolate 10) 'job) (romacs--isolate-zygote-take isolate)))))

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
new worker), :allow-source-boot t.  With no room in the pool the job waits:
its future has no isolate until it is dispatched."
  (let (keys plain)
    (while args
      (if (and (keywordp (car args))
               (memq (car args) '(:name :bindings :require :directory :isolate :allow-source-boot :fresh)))
          (progn (push (car args) keys) (push (car (cdr args)) keys) (setq args (cdr (cdr args))))
        (push (car args) plain)
        (setq args (cdr args))))
    (setq keys (nreverse keys) plain (nreverse plain))
    (unless (or (symbolp function) (and (consp function) (eq (car function) 'lambda)))
      (signal 'error (list "Cannot send closure" function)))
    (let* ((require (plist-get keys :require))
           (isolate (or (plist-get keys :isolate)
                        (romacs--isolate-acquire require (plist-get keys :fresh) (plist-get keys :allow-source-boot)))))
      (if (eq isolate 'queued)
          (romacs--isolate-enqueue function plain (plist-get keys :bindings) require (plist-get keys :directory))
        (romacs--isolate-submit isolate function plain (plist-get keys :bindings) require (plist-get keys :directory))))))

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
         (isolate (isolate-create :require require :allow-source-boot t))
         (zygote (record 'isolate-zygote isolate require 'preparing nil inputs nil nil
                         directories stamp (float-time) nil nil isolate-job-memory-headroom nil)))
    (aset isolate 10 'zygote)
    (puthash require zygote romacs--isolate-zygotes)
    (isolate-on-done (romacs--isolate-submit isolate 'romacs--isolate-become-zygote
                                             (list (romacs--isolate-warm-functions require) isolate-job-memory-headroom)
                                             nil nil nil)
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
rest of the session, its jobs running on pooled workers."
  (when (eq (gethash (aref zygote 2) romacs--isolate-zygotes) zygote)
    (if (and (eq (aref future 3) 'done)
             (<= (* 1024 (romacs--isolate-pss (romacs--isolate-id (aref zygote 1)))) isolate-zygote-memory-limit))
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
      (aset zygote 3 'refused)
      (aset zygote 6 (aref future 4))
      (romacs--isolate-end (aref zygote 1)))))

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
  "Forget ZYGOTE and end it and its spare, without waiting."
  (remhash (aref zygote 2) romacs--isolate-zygotes)
  (let ((spare (aref zygote 4)))
    (aset zygote 4 nil)
    (when spare (romacs--isolate-end spare)))
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
  "A new child of ZYGOTE, for one job."
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

(defun romacs--isolate-busy-children ()
  "How many forked children run a job."
  (let ((count 0))
    (dolist (isolate romacs--isolates count)
      (when (and (eq (aref isolate 10) 'job) (aref isolate 5))
        (setq count (1+ count))))))

(defun romacs--isolate-zygote-worker (require make)
  "The spare child of REQUIRE's zygote, forked now if it has none, when the
zygote is ready or preparing, nothing it read has changed, and the pool
has room (`romacs--isolate-room-p'); else nil.  A zygote whose inputs changed
is retired.  MAKE starts a zygote for REQUIRE when there is none."
  (when (romacs--isolate-zygotes-p)
    (let ((zygote (gethash require romacs--isolate-zygotes)))
      ;; Dead, its death not delivered yet: forgotten now.
      (when (and zygote (memq (aref zygote 3) '(preparing ready)) (romacs--isolate-gone-p (aref zygote 1)))
        (remhash require romacs--isolate-zygotes)
        (setq zygote nil))
      (when (and zygote (memq (aref zygote 3) '(preparing ready)) (not (romacs--isolate-zygote-current-p zygote)))
        (romacs--isolate-zygote-retire zygote)
        ;; Ending it may have resubmitted a queued child's job, which made
        ;; its replacement.
        (setq zygote (gethash require romacs--isolate-zygotes)))
      (when (and make (null zygote))
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
                   (condition-case nil (aset zygote 4 (romacs--isolate-zygote-fork zygote)) (error nil))))))))

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
    (romacs--isolate-dispatch)
    (romacs--isolate-check-deadlines)
    came))

(defun romacs--isolate-idle ()
  "What a wait point does for the isolates: deliver what has arrived."
  (when romacs--isolates
    (romacs--isolate-pump 0)))

(defun romacs--isolate-settle (isolate state value)
  "Resolve every pending future of ISOLATE as STATE with VALUE."
  (let ((pending (aref isolate 5)))
    (aset isolate 5 nil)
    (dolist (entry pending)
      (romacs--isolate-resolve (cdr entry) state value))))

(defun romacs--isolate-dead (isolate event data)
  "ISOLATE's worker is gone for EVENT with DATA: the children it reported
and had not retired get SIGHUP to their groups (what its own shutdown would
have sent), its futures settle, the event is reported, and it is forgotten."
  (let ((retired (eq (aref isolate 3) 'retired))
        (pending (aref isolate 5)))
    (unless (memq (aref isolate 3) '(killed retired))
      (aset isolate 3 'dead))
    (let ((children (romacs--isolate-signal-children (romacs--isolate-id isolate))))
      (romacs--isolate-forget-role isolate)
      (when (eq (aref isolate 10) 'zygote)
        ;; Its children die with it: those with a job settle now (a job
        ;; never started runs elsewhere), whatever they answer meanwhile.
        (dolist (child (copy-sequence romacs--isolates))
          (when (and (eq (aref child 10) 'job) (eq (aref (aref child 11) 1) isolate) (aref child 5))
            (romacs--isolate-signal (romacs--isolate-id child) 9)
            (romacs--isolate-dead child 'isolate-died nil))))
      (if (and (eq (aref isolate 10) 'job) (eq event 'isolate-died)
               (null (romacs--isolate-pid (romacs--isolate-id isolate))))
          ;; A child its zygote never reported was never released, so it
          ;; ran nothing (its zygote died first): its job runs elsewhere.
          (progn (aset isolate 5 nil)
                 (dolist (entry pending) (romacs--isolate-resubmit-or-fail (cdr entry))))
        (romacs--isolate-settle isolate event (list isolate data)))
      (romacs--isolate-close (romacs--isolate-id isolate))
      (setq romacs--isolates (delq isolate romacs--isolates))
      ;; A spare child that dies took no job: it is replaced, not reported.
      (unless (or retired (and (eq (aref isolate 10) 'job) (null pending)))
        (run-hook-with-args 'isolate-error-functions isolate event (list data children))))))

(defun romacs--isolate-resubmit-or-fail (future)
  "Queue FUTURE's job again elsewhere, or settle it with the error that stops it."
  (condition-case err
      (romacs--isolate-resubmit future)
    (error (romacs--isolate-resolve future 'error (list (car err) (cdr err) nil (get (car err) 'error-conditions))))))

(defun romacs--isolate-dispatch ()
  "Deliver every frame that has arrived from every worker."
  (dolist (isolate (copy-sequence romacs--isolates))
    (let ((id (romacs--isolate-id isolate)) frame)
      (while (and (memq isolate romacs--isolates) (setq frame (romacs--isolate-take id)))
        (let ((kind (car frame)) (generation (nth 1 frame)) (value (nth 2 frame)))
          (cond
           ((eq kind 'hello)
            (aset isolate 9 t)
            (unless (romacs--isolate-hello-ok-p value (aref isolate 8))
              (romacs--isolate-signal id 9)
              (while (null (romacs--isolate-reap id)) (romacs--isolate-wait 0.01))
              (romacs--isolate-dead isolate 'isolate-refused (list "worker is not this executable" value))))
           ((memq kind '(result error))
            ;; A forked child exits after its one job: its end is no news.
            (when (eq (aref isolate 10) 'job) (aset isolate 3 'retired))
            (let ((entry (assq generation (aref isolate 5))))
              (when (nth 1 value) (aset isolate 6 t))
              (when entry
                (aset isolate 5 (delq entry (aref isolate 5)))
                (aset isolate 7 nil)
                (if (and (eq kind 'error) (eq (car (car value)) 'isolate-retired))
                    ;; Queued behind a job that changed the features: it
                    ;; runs on a worker with the features it was queued for.
                    (condition-case err
                        (romacs--isolate-resubmit (cdr entry))
                      (error (romacs--isolate-resolve (cdr entry) 'error (list (car err) (cdr err) nil (get (car err) 'error-conditions)))))
                  (romacs--isolate-resolve (cdr entry) (if (eq kind 'result) 'done 'error) (car value))))))
           ((eq kind 'message)
            (run-hook-with-args 'isolate-message-functions isolate value))
           ((eq kind 'died)
            (romacs--isolate-dead isolate 'isolate-died value)))))))
  (romacs--isolate-drain-waiting))

(defun romacs--isolate-check-deadlines ()
  "Kill a worker whose interrupted job has not stopped in time."
  (dolist (isolate (copy-sequence romacs--isolates))
    (let ((deadline (aref isolate 7)))
      (when (and deadline (< (cdr deadline) (float-time))
                 (assq (car deadline) (aref isolate 5)))
        (isolate-kill isolate)))))

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
      (romacs--isolate-pump (if end (max 0 (- end (float-time))) 0.5))
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
    (aset isolate 7 (cons generation (+ (float-time) isolate-interrupt-timeout)))))

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

(defun isolate-receive (&optional timeout)
  "In a worker: the next message from the parent, waiting up to TIMEOUT.
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

;;;; Buffer snapshots

;; A snapshot (design §3.6, §4.4) is a frozen record of a buffer's
;; accessible text and what a worker needs to treat it as that buffer:
;; [buffer-snapshot NAME TEXT BEGIN TICK CHARS-TICK POINT FILE DIRECTORY
;; MODE LOCALS GENERATION].  The text is copied (raw bytes included: it
;; is the buffer's own string); a worker gets a buffer of its own over it
;; with `with-snapshot-buffer'.  Positions in a snapshot are 1-based from
;; its first character; the owner's position of that character is
;; `snapshot-begin'.

(defvar romacs--snapshot-generations 0 "The last snapshot generation this VM issued.")

(defvar-local buffer-snapshot-generation nil
  "The generation of a buffer's snapshots: results from a snapshot apply
only while it is the same.  Not permanent, so a major mode's
`kill-all-local-variables' ends it; `buffer-snapshot-invalidate' too.")

(defun buffer-snapshot-invalidate (&optional buffer)
  "Make results computed from BUFFER's earlier snapshots stale."
  (with-current-buffer (or buffer (current-buffer))
    (setq buffer-snapshot-generation nil)))

(defun romacs--snapshot-generation ()
  "The current buffer's snapshot generation, a new one if it has none."
  (or buffer-snapshot-generation
      (setq buffer-snapshot-generation (setq romacs--snapshot-generations (1+ romacs--snapshot-generations)))))

(defun buffer-snapshot (&optional buffer &rest keys)
  "A frozen snapshot of BUFFER's accessible text.  KEYS: :properties t
also copies the text properties; :locals, a list of variables, records
BUFFER's values of those that are buffer-local and freezable."
  (with-current-buffer (or buffer (current-buffer))
    (let (locals)
      (dolist (variable (plist-get keys :locals))
        (when (local-variable-p variable)
          (let ((value (symbol-value variable)))
            (when (condition-case nil (progn (freeze value) t) (error nil))
              (push (cons variable value) locals)))))
      (freeze (record 'buffer-snapshot (buffer-name)
                      (if (plist-get keys :properties)
                          (buffer-substring (point-min) (point-max))
                        (buffer-substring-no-properties (point-min) (point-max)))
                      (point-min) (buffer-modified-tick) (buffer-chars-modified-tick)
                      (- (point) (point-min) -1) buffer-file-name default-directory
                      major-mode (nreverse locals) (romacs--snapshot-generation))))))

(defun buffer-snapshot-p (object) "Non-nil for a buffer snapshot." (eq (type-of object) 'buffer-snapshot))
(defun snapshot-buffer-name (snapshot) "The name of SNAPSHOT's buffer." (aref snapshot 1))
(defun snapshot-string (snapshot) "SNAPSHOT's text, frozen." (aref snapshot 2))
(defun snapshot-begin (snapshot) "The owner's position of SNAPSHOT's first character." (aref snapshot 3))
(defun snapshot-tick (snapshot) "The buffer's `buffer-modified-tick' at SNAPSHOT." (aref snapshot 4))
(defun snapshot-chars-tick (snapshot) "The buffer's `buffer-chars-modified-tick' at SNAPSHOT." (aref snapshot 5))
(defun snapshot-point (snapshot) "Point at SNAPSHOT, in its own positions." (aref snapshot 6))
(defun snapshot-file-name (snapshot) "The buffer's `buffer-file-name' at SNAPSHOT." (aref snapshot 7))
(defun snapshot-mode (snapshot) "The buffer's `major-mode' at SNAPSHOT." (aref snapshot 9))
(defun snapshot-locals (snapshot) "The buffer-local values SNAPSHOT recorded, an alist." (aref snapshot 10))
(defun snapshot-generation (snapshot) "The buffer's `buffer-snapshot-generation' at SNAPSHOT." (aref snapshot 11))
(defun snapshot-local (snapshot variable)
  "SNAPSHOT's value of VARIABLE, or nil when it recorded none."
  (cdr (assq variable (snapshot-locals snapshot))))

(defun buffer-snapshot-current-p (snapshot &optional buffer)
  "Non-nil while results computed from SNAPSHOT still apply to BUFFER: its
text (`buffer-chars-modified-tick'), major mode, generation and the
recorded locals are as SNAPSHOT has them.  `buffer-modified-tick' is not
compared: it counts text property changes too, which the owner's own
redisplay fontification makes without moving any position."
  (let ((buffer (or buffer (get-buffer (snapshot-buffer-name snapshot)))))
    (and (buffer-live-p buffer)
         (with-current-buffer buffer
           (and (= (buffer-chars-modified-tick) (snapshot-chars-tick snapshot))
                (eq major-mode (snapshot-mode snapshot))
                (eql buffer-snapshot-generation (snapshot-generation snapshot))
                (let ((same t))
                  (dolist (local (snapshot-locals snapshot) same)
                    (unless (and (local-variable-p (car local))
                                 (equal (symbol-value (car local)) (cdr local)))
                      (setq same nil)))))))))

(defun romacs--with-snapshot-buffer (snapshot no-hooks body)
  "Call BODY in a buffer of this VM's own over SNAPSHOT's text; see
`with-snapshot-buffer'."
  (let ((buffer (generate-new-buffer (format "*snapshot %s*" (snapshot-buffer-name snapshot)) t)))
    (unwind-protect
        (with-current-buffer buffer
          (insert (snapshot-string snapshot))
          (set-buffer-modified-p nil)
          (goto-char (snapshot-point snapshot))
          (setq default-directory (aref snapshot 8))
          (if no-hooks
              (let ((delay-mode-hooks t)) (funcall (snapshot-mode snapshot)))
            (funcall (snapshot-mode snapshot)))
          ;; After the mode, as `normal-mode' applies file-local variables.
          (dolist (local (snapshot-locals snapshot))
            (set (make-local-variable (car local)) (thaw (cdr local))))
          (setq buffer-read-only t)
          (let ((tick (buffer-chars-modified-tick)))
            (prog1 (funcall body)
              (unless (= tick (buffer-chars-modified-tick))
                (signal 'error (list (format "Snapshot text changed in %s" (buffer-name))))))))
      (kill-buffer buffer))))

(defmacro with-snapshot-buffer (snapshot &rest body)
  "Run BODY in a temporary buffer of this VM's own whose text is SNAPSHOT's.
The buffer is in SNAPSHOT's major mode (its hooks run unless BODY starts
with `:no-hooks t'), has SNAPSHOT's recorded locals, point and directory,
and is read-only.  Its text properties (under `inhibit-read-only', as Font
Lock and `with-silent-modifications' bind it), overlays, markers, locals
and the mode's caches are this VM's to change; its text is not: a change
even under `inhibit-read-only' signals once BODY returns, since results
are positions in SNAPSHOT.  It visits no file."
  (declare (indent 1) (debug t))
  (let ((no-hooks (and (eq (car body) :no-hooks) (prog1 (nth 1 body) (setq body (nthcdr 2 body))))))
    `(romacs--with-snapshot-buffer ,snapshot ,no-hooks (lambda () ,@body))))

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
then forgotten at a wait point, not reported as a death.  Nil, doing
nothing, unless ISOLATE is live and runs nothing."
  (when (and (isolate-live-p isolate) (null (aref isolate 5)))
    (aset isolate 3 'retired)
    (romacs--isolate-post (romacs--isolate-id isolate) 'bye 0 nil)
    t))

(defun isolate-kill-all ()
  "Kill every live isolate, as a session's end does."
  (let ((waiting romacs--isolate-waiting))
    (setq romacs--isolate-waiting nil)
    (dolist (future waiting)
      (romacs--isolate-resolve future 'isolate-killed (list nil nil))))
  (dolist (isolate (copy-sequence romacs--isolates))
    (isolate-kill isolate)))

;;;; The worker's loop

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
                (prog1
                    (progn
                      ;; The job runs under its generation, inside this
                      ;; handler: an interrupt naming it quits here, once
                      ;; (the safepoint clears the mark as it quits), and
                      ;; its memory budget binds from here on, thawing its
                      ;; arguments included.
                      (romacs--isolate-running generation)
                      (let ((args (thaw (nth 1 job)))
                            (bindings (thaw (nth 2 job))))
                        (dolist (feature require) (require feature))
                        (dolist (binding bindings) (set (car binding) (cdr binding)))
                        (when directory (setq default-directory directory))
                        (list 'result
                              (apply (if (and (consp function) (eq (car function) 'lambda)) (eval function t) function)
                                     args))))
                  ;; The job's end, inside its generation and handler: a
                  ;; collection due, its memory (which replaces the
                  ;; result), what collections left pending.
                  (romacs--safepoint)
                  (romacs--isolate-running nil))
              (quit (romacs--isolate-running nil)
                    (list 'error (list 'quit nil nil '(quit))))
              (error (romacs--isolate-running nil)
                     (list 'error (list (car err) (romacs--isolate-transportable (cdr err)) nil
                                        (get (car err) 'error-conditions)))))))))
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
              ((eq kind 'fork)
               (setq romacs--isolate-forks (nconc romacs--isolate-forks (list (nth 1 frame)))))
              ((eq kind 'message)
               (setq romacs--isolate-inbox (nconc romacs--isolate-inbox (list (nth 2 frame)))))
              ((eq kind 'interrupt)
               ;; For a job not yet run; one running took it at a safepoint.
               (push (nth 1 frame) romacs--isolate-cancelled))
              ((eq kind 'bye)
               (setq romacs--isolate-serving nil)))))))

(defun romacs--isolate-run-queued ()
  "Run the queued jobs in order, reading what arrived before each: a
cancellation sent behind a job cancels it before it runs."
  (while (and romacs--isolate-deferred romacs--isolate-serving)
    ;; A signal nobody took (its job was in a leaf, or done) is stale.
    (romacs--isolate-interrupted-p)
    (romacs--isolate-wait 0)
    (romacs--isolate-note-control-frames)
    (let ((frame (pop romacs--isolate-deferred)))
      (romacs--isolate-run-job (nth 1 frame) (nth 2 frame)))))

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

(defun romacs--isolate-become-zygote (warm headroom)
  "In a prepared worker: become the zygote of its features, a process that
only forks a child per job (worker-vms design §2.2).  The functions WARM
run first, filling the caches the children inherit; a child's job may grow
the heap by HEADROOM bytes.  Refused while its
preparation left a subprocess, a timer or a thread behind, which a child
would share or lose.  The files the preparation loaded, their stamp, and
this thread, the one that forks."
  (dolist (function warm)
    (condition-case nil (funcall function) (error nil)))
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

(provide 'isolate)
;;; isolate.el ends here

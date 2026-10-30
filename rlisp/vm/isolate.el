;;; isolate.el --- Isolated VMs that exchange frozen values  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

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

(defvar isolate-worker-program nil
  "The program a worker runs, or nil for this executable.")

(defvar isolate-worker-arguments nil
  "Arguments every worker is started with, after the boot arguments.")

(defvar isolate-pool-size (max 1 (1- (if (fboundp 'num-processors) (num-processors) 2)))
  "How many pooled workers `isolate-spawn' starts before queueing on them.")

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
;; PROGRAM HELLO].  STATE is `live', `killed' or `dead'; PENDING an alist of
;; (GENERATION . FUTURE) in order of submission, the first running; TAINTED
;; once a job changed the worker's features (not reused); DEADLINE
;; (GENERATION . TIME) after an interrupt; PROGRAM what the worker runs;
;; HELLO once the worker's hello arrived (it catches signals from then).
(defun romacs--isolate-make (id name require program)
  (record 'isolate id name 'live require nil nil nil program nil))
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
         (list "--world" (car isolate-world) (cdr isolate-world)))
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
  (let ((live (delq nil (mapcar (lambda (i) (and (isolate-live-p i) (not (aref i 6)) i)) romacs--isolates)))
        best idle)
    (unless fresh
      (dolist (isolate live)
        (when (and (equal (aref isolate 4) require)
                   (or (null best) (< (length (aref isolate 5)) (length (aref best 5)))))
          (setq best isolate))))
    (cond (fresh (isolate-create :require require :allow-source-boot allow-source-boot))
          ((and best (null (aref best 5))) best)
          ((< (length live) isolate-pool-size)
           (isolate-create :require require :allow-source-boot allow-source-boot))
          (best best)
          ((setq idle (let (found) (dolist (i live) (when (null (aref i 5)) (setq found i))) found))
           (isolate-kill idle)
           (isolate-create :require require :allow-source-boot allow-source-boot))
          (t (signal 'isolate-queue-full (list require))))))

(defun romacs--isolate-submit (isolate function args bindings require directory)
  "Queue (apply FUNCTION ARGS) on ISOLATE: its future."
  (unless (isolate-live-p isolate)
    (signal 'isolate-died (list isolate)))
  (when (>= (length (aref isolate 5)) isolate-queue-limit)
    (signal 'isolate-queue-full (list isolate)))
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
    future))

(defun romacs--isolate-resubmit (future)
  "Queue FUTURE's job again, on a worker with the features it was queued
for: its worker retired before running it."
  (let* ((job (aref future 6))
         (require (nth 3 job))
         (isolate (romacs--isolate-acquire require nil t)))
    (setq romacs--isolate-generation (1+ romacs--isolate-generation))
    (romacs--isolate-post (romacs--isolate-id isolate) 'job romacs--isolate-generation job)
    (aset future 1 isolate)
    (aset future 2 romacs--isolate-generation)
    (aset isolate 5 (append (aref isolate 5) (list (cons romacs--isolate-generation future))))))

(defun isolate-spawn (function &rest args)
  "Run (apply FUNCTION ARGS) in a worker isolate; its future.
FUNCTION is a symbol the worker calls its own definition of, or a lambda
form whose free variables come from :bindings, an alist of (SYMBOL . VALUE).
A closure object is refused.  The keyword tail of ARGS: :name, :bindings,
:require FEATURES, :directory, :isolate ISOLATE (run there), :fresh t (a
new worker), :allow-source-boot t."
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
      (romacs--isolate-submit isolate function plain (plist-get keys :bindings) require (plist-get keys :directory)))))

;;;; Waiting and delivery

(defun romacs--isolate-pump (timeout)
  "Wait up to TIMEOUT for isolate traffic and deliver it; non-nil if any came."
  (let ((came (romacs--isolate-wait timeout)))
    (when came (romacs--isolate-dispatch))
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
  (unless (eq (aref isolate 3) 'killed)
    (aset isolate 3 'dead))
  (let ((children (romacs--isolate-signal-children (romacs--isolate-id isolate))))
    (romacs--isolate-settle isolate event (list isolate data))
    (romacs--isolate-close (romacs--isolate-id isolate))
    (setq romacs--isolates (delq isolate romacs--isolates))
    (run-hook-with-args 'isolate-error-functions isolate event (list data children))))

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
            (romacs--isolate-dead isolate 'isolate-died value))))))))

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
  "In a worker: the next message from the parent, waiting up to TIMEOUT."
  (let ((end (and timeout (+ (float-time) timeout))))
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
  (when (isolate-live-p isolate)
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

(defun isolate-kill-all ()
  "Kill every live isolate, as a session's end does."
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
         (args (thaw (nth 1 job)))
         (bindings (thaw (nth 2 job)))
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
            ;; The job runs under its generation: an interrupt naming it
            ;; quits here, once (the safepoint clears the mark as it quits).
            (romacs--isolate-running generation)
            (condition-case err
                (prog1
                    (progn
                      (dolist (feature require) (require feature))
                      (dolist (binding bindings) (set (car binding) (cdr binding)))
                      (when directory (setq default-directory directory))
                      (list 'result
                            (apply (if (and (consp function) (eq (car function) 'lambda)) (eval function t) function)
                                   args)))
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
      (romacs--isolate-reply generation outcome changed))))

(defun romacs--isolate-note-control-frames ()
  "Take every frame that arrived: interrupts, bye and messages now, jobs
onto the queue."
  (let (frame)
    (while (setq frame (romacs--isolate-take 0))
      (let ((kind (car frame)))
        (cond ((eq kind 'job)
               (setq romacs--isolate-deferred (nconc romacs--isolate-deferred (list frame))))
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
    (romacs--isolate-run-queued)
    (when (and romacs--isolate-serving (null romacs--isolate-deferred))
      (romacs--isolate-wait nil)
      (romacs--isolate-serve-frames))))

(provide 'isolate)
;;; isolate.el ends here

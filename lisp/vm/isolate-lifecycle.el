;;; isolate-lifecycle.el --- An isolate's lifecycle as one transition  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Every change to a worker isolate's life, an owner's and a zygote's
;; included (lisp/vm/isolate.el, lisp/vm/owner.el), is one pure
;; transition, `romacs--isolate-transition': from what is known of the
;; isolate now and what happened, its next phase and the effects to carry
;; out.  One edge, `romacs--isolate-run', carries them out: every
;; signal, ending mark, frame, settlement and follow-up of a whole batch
;; first, and only then the futures' callbacks and
;; `isolate-error-functions', each whatever an earlier one signals.  So no
;; callback ever sees an isolate halfway through a change, and none can
;; cut one short.  Two pure siblings plan what runs inside a worker: its
;; loop's rounds (`romacs--isolate-round-plan') and a status pass's
;; recovery (`romacs--process-status-row').  The table they implement is
;; the ownership lane's O1a-2-lifecycle.md.  Not in GNU.

;;; Code:

;;;; The transition: pure

(defun romacs--isolate-transition (state event)
  "ISOLATE's next phase and effects, (PHASE . EFFECTS), for EVENT, a list
\(NAME ARGS...), given STATE, a plist from `romacs--isolate-facts'; nil
when EVENT changes nothing.  PHASE nil keeps the phase.  EFFECTS are in
the order the edge (`romacs--isolate-run') carries them out."
  (let ((name (car event)))
    (cond ((eq name 'kill) (romacs--isolate-on-kill state))
          ((eq name 'end) (if (plist-get state :pending)
                              (romacs--isolate-on-kill state)
                            (romacs--isolate-on-retire state)))
          ((eq name 'retire) (romacs--isolate-on-retire state))
          ((eq name 'deadline) (romacs--isolate-on-deadline state))
          ((eq name 'answered) (romacs--isolate-on-answered state (cdr event)))
          ((eq name 'spent) (romacs--isolate-on-spent state))
          ;; Only a job child has a memory budget to end for.
          ((eq name 'memory)
           (and (eq (plist-get state :role) 'job) `(retired (end nil) (settle error ,(nth 1 event)))))
          ((eq name 'orphaned) (romacs--isolate-on-orphaned (nth 1 event)))
          ((eq name 'refused) `(nil (settle error ,(nth 1 event))))
          ((eq name 'refuse) (romacs--isolate-on-refuse state (nth 1 event) (nth 2 event)))
          ((memq name '(hello-refused world-refused)) (romacs--isolate-on-hello-refused state event))
          ;; A death is delivered once: a dead one hears no other.
          ((eq name 'death) (and (not (eq (plist-get state :phase) 'dead))
                                 (romacs--isolate-on-death state (cdr event))))
          ((eq name 'child-gone) (romacs--isolate-on-child-gone state))
          ((eq name 'shutdown) (romacs--isolate-on-shutdown state)))))

(defun romacs--isolate-kills-later-p (state)
  "Whether STATE's isolate is killed without waiting for it: an owner,
or a job child its zygote has not reported (it has nothing to signal yet)."
  (or (eq (plist-get state :role) 'owner)
      (and (eq (plist-get state :role) 'job) (not (plist-get state :forked)))))

(defun romacs--isolate-killed-condition (state)
  "The condition STATE's isolate's calls settle with when it is killed."
  (if (eq (plist-get state :role) 'owner) 'owner-killed 'isolate-killed))

(defun romacs--isolate-on-kill (state &optional cause)
  "`kill': a live isolate that is killed later gets SIGTERM, SIGKILL at
its deadline, and its calls settled now; any other is killed and waited
for (`kill-wait'), its death carrying CAUSE (`shutdown', or nil)."
  (cond ((not (eq (plist-get state :phase) 'live)) nil)
        ((romacs--isolate-kills-later-p state)
         (let ((condition (romacs--isolate-killed-condition state)))
           `(retired (signal 15) (end 15) (settle ,condition nil) (hook ,condition nil nil))))
        (t `(killed (kill-wait ,cause)))))

(defun romacs--isolate-on-retire (state)
  "`retire': a live isolate running nothing ends by itself after `bye';
a zygote retires its idle children with it."
  (when (and (eq (plist-get state :phase) 'live) (not (plist-get state :pending)))
    (append '(retired (end nil) (post bye))
            (and (eq (plist-get state :role) 'zygote) '((retire-children))))))

(defun romacs--isolate-on-deadline (state)
  "`deadline': a retired isolate's end is overdue: SIGTERM, then SIGKILL.
A zygote lingering for its children has no deadline: it ends after them."
  (let ((signal (plist-get state :signal)))
    (when (and (eq (plist-get state :phase) 'retired) (not (eql signal 9))
               (not (plist-get state :lingering)))
      (let ((next (if signal 9 15)))
        `(nil (signal ,next) (end ,next))))))

(defun romacs--isolate-on-answered (state reply)
  "`answered' with REPLY, (KIND GENERATION VALUE), a worker's result or
error for a call: a job child marked ending (its zygote told), a worker
whose features changed tainted, the call resolved, or queued again
elsewhere when a worker that is no owner retired before running it, and
a one-job worker checked for its end, all before any callback runs."
  (let* ((kind (nth 0 reply)) (value (nth 2 reply))
         (role (plist-get state :role))
         (live-job (and (eq role 'job) (eq (plist-get state :phase) 'live)))
         (requeue (and (eq kind 'error) (eq (car-safe (car value)) 'isolate-retired) (not (eq role 'owner)))))
    (cons (and live-job 'retired)
          (append (and live-job '((end nil) (zygote-gone)))
                  (and (nth 1 value) '((taint)))
                  `((reply ,(nth 1 reply) ,(if requeue 'requeue kind) ,(car value)))
                  (and (memq role '(pool fresh)) '((spent-check)))))))

(defun romacs--isolate-on-spent (state)
  "`spent': a worker the pool made that took its one job and has no call
left retires (`romacs--isolate-on-retire')."
  (and (memq (plist-get state :role) '(pool fresh)) (plist-get state :spent)
       (romacs--isolate-on-retire state)))

(defun romacs--isolate-on-orphaned (cause)
  "`orphaned': a busy forked job child whose zygote died gets SIGKILL and
its calls settle as the zygote's death (`isolate-died'), or, under
CAUSE `shutdown', as killed with the rest."
  (let ((condition (if (eq cause 'shutdown) 'isolate-killed 'isolate-died)))
    `(retired (signal 9) (end nil) (settle ,condition nil) (hook ,condition nil nil))))

(defun romacs--isolate-on-refuse (state reason current)
  "`refuse': a zygote whose preparation failed for REASON ends; when it is
CURRENT, its features' zygote still, `isolate-error-functions' hears it."
  (let ((end (romacs--isolate-transition state '(end))))
    (if current
        (append (or end (list nil)) `((hook isolate-zygote-refused ,reason nil)))
      end)))

(defun romacs--isolate-on-hello-refused (state event)
  "`hello-refused' (DATA, its hello's mismatch and text) or
`world-refused' (REASON WORLD, the world it could not adopt): a live
worker gets SIGKILL and is not waited for, its calls settle with
`isolate-refused' or `isolate-world-unavailable', the refusal is
reported, and its death arrives later as any death does.  It is no
zygote or spare from now (its role forgotten).  A world refused is
forgotten first, so later workers boot from source and a zygote's queued
jobs run there; a zygote of another build fails its queued jobs with the
refusal, since any other would be refused too."
  (when (eq (plist-get state :phase) 'live)
    (let* ((world (eq (car event) 'world-refused))
           (condition (if world 'isolate-world-unavailable 'isolate-refused))
           (data (if world (cdr event) (nth 1 event))))
      ;; A worker that never served is no zygote or spare from now: its
      ;; role goes at once, its record when its death comes.
      (append '(retired)
              (and world `((forget-world ,@(cdr event))))
              '((forget-role) (signal 9) (end 9))
              (and (not world) (eq (plist-get state :role) 'zygote)
                   `((orphan-children (refused ,@data))))
              (if world
                  `((settle isolate-world-unavailable ,data))
                `((settle isolate-refused ,data)))
              `((hook ,condition ,data nil))))))

(defun romacs--isolate-on-death (state death)
  "`death' with DEATH, (CONDITION DATA CAUSE): its reported subprocess
groups get SIGHUP, it is forgotten, a zygote's busy children die with it
\(with CAUSE), its calls settle (a job child never forked runs
elsewhere), and the death is reported unless it was retired or a spare
that took no job."
  (let ((role (plist-get state :role)) (phase (plist-get state :phase))
        (condition (nth 0 death)) (data (nth 1 death)))
    (cons (if (memq phase '(killed retired)) nil 'dead)
          (append '((hup) (forget) (close))
                  (and (eq role 'zygote) `((orphan-children ,(nth 2 death))))
                  (and (plist-get state :child) '((zygote-gone)))
                  (if (and (eq role 'job) (eq condition 'isolate-died) (not (plist-get state :forked)))
                      '((resubmit))
                    `((settle ,condition ,data)))
                  (unless (or (eq phase 'retired) (and (eq role 'job) (not (plist-get state :pending))))
                    `((hook ,condition ,data t)))))))

(defun romacs--isolate-on-child-gone (state)
  "`child-gone': one of a retired zygote's children died or ended its
job: once none is left it lingers for, its end is due
`isolate-kill-timeout' from the last (it has no deadline while it lingers)."
  (when (and (eq (plist-get state :phase) 'retired) (null (plist-get state :signal)))
    '(nil (end nil))))

(defun romacs--isolate-on-shutdown (state)
  "`shutdown': every retired isolate, and every live owner, zygote or job
child its zygote has not reported, gets SIGKILL now and is not waited
for, its subprocess groups SIGHUP, its role forgotten, and a live one's
calls settled; any other live worker is killed as `kill' does, its death
carrying the cause.  A shutdown ends job children before their zygotes
\(`romacs--isolate-shutdown-order'), so none runs elsewhere."
  (let ((phase (plist-get state :phase)))
    (cond ((eq phase 'retired) '(nil (forget-role) (signal 9) (end 9) (hup)))
          ((not (eq phase 'live)) nil)
          ((or (romacs--isolate-kills-later-p state) (eq (plist-get state :role) 'zygote))
           (let ((condition (romacs--isolate-killed-condition state)))
             `(retired (forget-role) (signal 9) (end 9) (hup) (settle ,condition nil) (hook ,condition nil nil))))
          (t (romacs--isolate-on-kill state 'shutdown)))))

;;;; Admission: pure

(defun romacs--isolate-admission (kind count need limit ending room)
  "Whether NEED more worker processes of KIND may begin, COUNT running of
LIMIT, ENDING whether one is on its way out, ROOM whether the pool's
memory fits: `admit', `wait' (the room comes at an end), `evict' (end an
idle worker for it) or `refuse'.  An owner, which cannot wait, ends no
worker for room: it is admitted or refused, its zygote counted in NEED."
  (cond ((and (<= (+ count need) limit) (or (not (eq kind 'start)) (not ending) room)) 'admit)
        ((eq kind 'owner) 'refuse)
        (ending 'wait)
        (t 'evict)))

;;;; A worker's round and a status pass: pure

(defun romacs--isolate-round-plan (facts)
  "The steps of a worker's round, in order, given FACTS, a plist: :zygote
\(it only forks).  A round takes its calls once, at its head, then runs
one batch of process output and the due timers before it looks for
calls again: a call that arrives meanwhile, a wait's included, waits for
the next round's head (lifecycle §7)."
  (if (plist-get facts :zygote)
      '(interrupts forks calls note)
    '(interrupts forks calls turns note)))

(defun romacs--isolate-wait-plan (facts)
  "What a worker does once its round is done, given FACTS, a plist:
:serving, :queued (calls noted), :forks (children asked of a zygote),
:zygote and :turned (its output and timers ran this round).  Wait, then
only note what arrived (the next round's head runs it), unless calls or
forks are already queued, or a worker ran no output this round (a child
forked in it, its round planned as its zygote's): then nothing."
  (and (plist-get facts :serving) (not (plist-get facts :queued))
       (not (and (plist-get facts :zygote) (plist-get facts :forks)))
       (or (plist-get facts :zygote) (plist-get facts :turned))
       '(wait note)))

(defun romacs--process-status-row (stage event bounded rest registered)
  "The effects on the process a status pass is at, at STAGE (`draining'
its output, in its `sentinel', or `between' processes), for EVENT:
`spent' (no filter call left: it stays changed, re-ticked, the pass
held) or `exit' (a callback left the pass), REST whether processes are
left it has not visited, REGISTERED whether the process is still in the
process list.  In a BOUNDED pass (a worker's loop, which serves on after
a quit) an exit keeps every status not yet notified pending: a process
still draining and registered is re-ticked, and with processes left the
global tick is raised for them; a process whose sentinel began, or one
a callback deleted, leaves nothing pending.  Unbounded (the master), an
exit does what GNU's does: nothing, losing them (lifecycle §8)."
  (cond ((eq event 'spent) '((re-tick)))
        ((not bounded) nil)
        ((and (eq stage 'draining) registered) '((re-tick)))
        (rest '((raise)))))

;;;; What is known of an isolate

(defun romacs--isolate-lingering-p (zygote)
  "Whether ZYGOTE (an isolate) lingers: it has owners not yet reaped, or
children running a job.  Neither dies before it does, so it waits for
them; an idle child (a spare) is no reason to."
  (> (+ (romacs--isolate-owners zygote) (romacs--isolate-busy-children zygote)) 0))

(defun romacs--isolate-facts (isolate)
  "What `romacs--isolate-transition' knows of ISOLATE now, a plist: :role,
:phase, :signal (the last sent since it began ending), :forked (its pid
is known), :pending (it has calls), :spent (a job changed its features,
or it took its one job), :lingering (a zygote with owners or busy
children, `romacs--isolate-lingering-p') and :child (a job child or an
owner)."
  (let ((role (aref isolate 10)))
    (list :role role :phase (aref isolate 3) :signal (cdr (aref isolate 13))
          :forked (and (romacs--isolate-pid (romacs--isolate-id isolate)) t)
          :pending (and (aref isolate 5) t)
          :spent (and (aref isolate 6) t)
          :lingering (and (eq role 'zygote) (romacs--isolate-lingering-p isolate))
          :child (and (memq role '(job owner)) (aref isolate 11) t))))

;;;; The edge

(defun romacs--isolate-defer (thunk)
  "Run THUNK once the running batch's effects are done (`romacs--isolate-batch')."
  (setcar romacs--isolate-batch (append (car romacs--isolate-batch) (list thunk))))

(defun romacs--isolate-step (isolate event)
  "Carry out EVENT on ISOLATE (`romacs--isolate-run'): the children a
synchronous kill signalled, else non-nil when EVENT changed anything."
  (romacs--isolate-run (list (cons isolate event))))

(defun romacs--isolate-run (queue &optional resolved)
  "Carry out the transitions of QUEUE, a list of (ISOLATE . EVENT), in
order, each with the events it leads to: every effect of all of them,
then the futures' callbacks (RESOLVED's first, from
`romacs--isolate-mark-resolved') and the hooks, every one whatever an
earlier one signals.  Inside a batch's effects, it joins that batch.  The
first transition's value (`romacs--isolate-step')."
  (if romacs--isolate-batch
      (romacs--isolate-run-items queue resolved)
    (let ((batch (list nil)) (value nil))
      (unwind-protect
          (let ((romacs--isolate-batch batch))
            (setq value (romacs--isolate-run-items queue resolved)))
        (romacs--isolate-each #'funcall (car batch)))
      value)))

(defun romacs--isolate-run-items (queue resolved)
  "Carry out QUEUE's transitions into the running batch, RESOLVED's
callbacks first: the first transition's value."
  (when resolved
    (romacs--isolate-defer (lambda () (romacs--isolate-each #'romacs--isolate-notify resolved))))
  (let ((value nil) (first t))
    (dolist (item queue value)
      (let ((done (romacs--isolate-collect (car item) (cdr item))))
        (when first (setq value (car done) first nil))
        (dolist (thunk (cdr done)) (romacs--isolate-defer thunk))))))

(defun romacs--isolate-collect (isolate event)
  "Carry out EVENT's transition on ISOLATE and those it leads to, running
no callback: (VALUE . LATER), LATER the thunks that notify and report
them, those of the events it led to (a dead zygote's children, say)
before its own."
  (let ((out (romacs--isolate-apply isolate event)) (later nil))
    (dolist (follow (aref out 1))
      (setq later (append later (cdr (romacs--isolate-collect (car follow) (cdr follow))))))
    (cons (aref out 0) (append later (aref out 2)))))

(defun romacs--isolate-apply (isolate event)
  "Carry out EVENT's transition on ISOLATE, running no callback: [VALUE
FOLLOW-UPS LATER], FOLLOW-UPS the (ISOLATE . EVENT) it leads to, LATER
the thunks that notify and report it."
  (let* ((plan (romacs--isolate-transition (romacs--isolate-facts isolate) event))
         ;; [VALUE FOLLOW-UPS LATER CHILDREN]
         (out (vector (and plan t) nil nil nil)))
    (when (car plan) (aset isolate 3 (car plan)))
    (dolist (effect (cdr plan))
      (romacs--isolate-effect isolate event effect out))
    (aset out 1 (nreverse (aref out 1)))
    (aset out 2 (nreverse (aref out 2)))
    out))

(defun romacs--isolate-effect (isolate event effect out)
  "Carry out EFFECT of EVENT on ISOLATE, into OUT (`romacs--isolate-apply')."
  (let ((id (romacs--isolate-id isolate)) (kind (car effect)))
    (cond ((eq kind 'signal)
           (condition-case nil (romacs--isolate-signal id (nth 1 effect)) (error nil)))
          ((eq kind 'end) (romacs--isolate-end-mark isolate (nth 1 effect)))
          ((eq kind 'post)
           ;; A worker gone, or not reading, gets no bye: its death, or its
           ;; deadline, ends it in any case.
           (condition-case nil (romacs--isolate-post id 'bye 0 nil) (isolate-error nil)))
          ((eq kind 'kill-wait) (romacs--isolate-kill-wait isolate (nth 1 effect) out))
          ((eq kind 'hup) (aset out 3 (romacs--isolate-signal-children id)))
          ((eq kind 'forget)
           (romacs--isolate-forget-role isolate)
           ;; Out of the dispatch before any callback runs: a pump a
           ;; callback makes cannot handle this death again.
           (setq romacs--isolates (delq isolate romacs--isolates)))
          ((eq kind 'close) (romacs--isolate-close id))
          ((eq kind 'forget-role) (romacs--isolate-forget-role isolate))
          ((eq kind 'taint) (aset isolate 6 t))
          ((eq kind 'forget-world) (romacs--isolate-forget-world (nth 1 effect) (nth 2 effect)))
          ((memq kind '(retire-children orphan-children zygote-gone spent-check))
           (romacs--isolate-follow isolate effect out))
          (t (romacs--isolate-effect-later isolate event effect out)))))

(defun romacs--isolate-effect-later (isolate event effect out)
  "Carry out EFFECT of EVENT on ISOLATE that callbacks or hooks hear:
settle its futures now, and queue in OUT what is to run once the whole
batch is done."
  (let ((kind (car effect)))
    (cond ((eq kind 'settle)
           (let* ((state (nth 1 effect)) (data (nth 2 effect))
                  (resolved (romacs--isolate-take-settled
                             isolate state (if (eq state 'error) data (list isolate data)))))
             (romacs--isolate-later out (lambda () (romacs--isolate-each #'romacs--isolate-notify resolved)))))
          ((eq kind 'reply) (romacs--isolate-reply-effect isolate (cdr effect) out))
          ((eq kind 'resubmit)
           (let ((pending (aref isolate 5)))
             (aset isolate 5 nil)
             ;; Queued elsewhere now; one that cannot be settles, notified
             ;; with the batch (`romacs--isolate-resolve').
             (romacs--isolate-each (lambda (entry) (romacs--isolate-resubmit-or-fail (cdr entry))) pending)))
          ((eq kind 'hook)
           (let ((condition (nth 1 effect))
                 (data (list (nth 2 effect) (and (nth 3 effect) (aref out 3)))))
             (romacs--isolate-later
              out (lambda () (run-hook-with-args 'isolate-error-functions isolate condition data)))))
          (t (error "No isolate effect %S for %S" effect event)))))

(defun romacs--isolate-reply-effect (isolate reply out)
  "Effect `reply' of ISOLATE: REPLY, (GENERATION KIND PAYLOAD), resolves
its call GENERATION as KIND says (`result', `error', or `requeue': run it
elsewhere), notified in OUT; a call no longer pending is no news."
  (let ((entry (assq (nth 0 reply) (aref isolate 5))) (kind (nth 1 reply)))
    (when entry
      (aset isolate 5 (delq entry (aref isolate 5)))
      (aset isolate 7 nil)
      (if (eq kind 'requeue)
          (romacs--isolate-resubmit-or-fail (cdr entry))
        (let ((one (romacs--isolate-mark-resolved (cdr entry) (if (eq kind 'result) 'done 'error) (nth 2 reply))))
          (romacs--isolate-later out (lambda () (romacs--isolate-notify one))))))))

(defun romacs--isolate-later (out thunk)
  "Queue THUNK in OUT to run once the batch's effects are done."
  (aset out 2 (cons thunk (aref out 2))))

(defun romacs--isolate-follow (isolate effect out)
  "Queue in OUT the events EFFECT of ISOLATE leads to: its idle job
children retired with it, a dead zygote's busy children dead with it
\(with its death's cause), its zygote told a child is gone, a one-job
worker checked for its end."
  (let ((push (lambda (other event) (aset out 1 (cons (cons other event) (aref out 1)))))
        (kind (car effect)) (cause (nth 1 effect)))
    (cond ((eq kind 'spent-check) (funcall push isolate '(spent)))
          ((eq kind 'zygote-gone)
           (let ((zygote (aref (aref isolate 11) 1)))
             (when (memq zygote romacs--isolates) (funcall push zygote '(child-gone)))))
          (t
           (dolist (child (copy-sequence romacs--isolates))
             (when (and (eq (aref child 10) 'job) (eq (aref (aref child 11) 1) isolate))
               (cond ((eq kind 'retire-children) (funcall push child '(retire)))
                     ((not (aref child 5)))
                     ((romacs--isolate-pid (romacs--isolate-id child)) (funcall push child `(orphaned ,cause)))
                     ;; A child never reported was never released, so it
                     ;; ran nothing: its job runs elsewhere, unless the
                     ;; session ends.
                     ((eq cause 'shutdown) (funcall push child '(death isolate-killed nil shutdown)))
                     ;; Its zygote is of another build: so would any be.
                     ((eq (car-safe cause) 'refused) (funcall push child `(death isolate-refused ,(cdr cause))))
                     (t (funcall push child '(death isolate-died nil))))))))))

(defun romacs--isolate-forget-world (reason world)
  "Forget WORLD, which a worker could not adopt for REASON, when
`isolate-world' still names it: later workers boot from source
\(`romacs--isolate-boot-arguments'), none of them refused again."
  (when (equal isolate-world world)
    (setq isolate-world nil romacs--isolate-world-refusal (list reason world))))

(defun romacs--isolate-end-mark (isolate signal)
  "ISOLATE ends by itself from now, SIGNAL (nil, 15, 9) the last sent it:
the deadlines look again `isolate-kill-timeout' later, unless SIGNAL is
9, the last there is."
  (aset isolate 13 (cons (float-time) signal))
  (unless (eql signal 9)
    (romacs--isolate-wake-at (+ (float-time) isolate-kill-timeout))))

(defun romacs--isolate-kill-wait (isolate cause out)
  "Kill ISOLATE, a worker that is no owner, and wait for its end: SIGTERM,
then SIGKILL after `isolate-kill-timeout'.  Its death, with CAUSE,
follows in OUT, with the children it reported signalled; they are OUT's
value too."
  (let ((id (romacs--isolate-id isolate)))
    (romacs--isolate-signal id 15)
    (let ((end (+ (float-time) isolate-kill-timeout)))
      (while (and (null (romacs--isolate-reap id)) (< (float-time) end))
        (romacs--isolate-wait 0.02)))
    (unless (romacs--isolate-reap id)
      (romacs--isolate-signal id 9)
      (while (null (romacs--isolate-reap id))
        (romacs--isolate-wait 0.02)))
    (let ((children (romacs--isolate-signal-children id)))
      (aset out 0 children)
      (aset out 1 (cons (list isolate 'death 'isolate-killed children cause) (aref out 1))))))

(provide 'isolate-lifecycle)
;;; isolate-lifecycle.el ends here

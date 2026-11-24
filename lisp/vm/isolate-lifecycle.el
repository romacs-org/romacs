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
;; cut one short.  The table it implements is the ownership lane's
;; O1a-2-lifecycle.md.  Not in GNU.

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
          ((eq name 'answered)
           ;; Its zygote may have lingered for it: no longer busy, the job
           ;; child is no reason to.
           (and (eq (plist-get state :phase) 'live) (eq (plist-get state :role) 'job)
                '(retired (end nil) (zygote-gone))))
          ;; Only a job child has a memory budget to end for.
          ((eq name 'memory)
           (and (eq (plist-get state :role) 'job) `(retired (end nil) (settle error ,(nth 1 event)))))
          ((eq name 'orphaned)
           '(retired (signal 9) (end nil) (settle isolate-died nil) (hook isolate-died nil nil)))
          ((eq name 'refused) `(nil (settle error ,(nth 1 event))))
          ((eq name 'refuse) (romacs--isolate-on-refuse state (nth 1 event) (nth 2 event)))
          ;; A death is delivered once: a dead one hears no other.
          ((eq name 'death) (and (not (eq (plist-get state :phase) 'dead))
                                 (romacs--isolate-on-death state (nth 1 event) (nth 2 event))))
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

(defun romacs--isolate-on-kill (state)
  "`kill': a live isolate that is killed later gets SIGTERM, SIGKILL at
its deadline, and its calls settled now; any other is killed and waited
for (`kill-wait')."
  (cond ((not (eq (plist-get state :phase) 'live)) nil)
        ((romacs--isolate-kills-later-p state)
         (let ((condition (romacs--isolate-killed-condition state)))
           `(retired (signal 15) (end 15) (settle ,condition nil) (hook ,condition nil nil))))
        (t '(killed (kill-wait)))))

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

(defun romacs--isolate-on-refuse (state reason current)
  "`refuse': a zygote whose preparation failed for REASON ends; when it is
CURRENT, its features' zygote still, `isolate-error-functions' hears it."
  (let ((end (romacs--isolate-transition state '(end))))
    (if current
        (append (or end (list nil)) `((hook isolate-zygote-refused ,reason nil)))
      end)))

(defun romacs--isolate-on-death (state condition data)
  "`death' for CONDITION with DATA: its reported subprocess groups get
SIGHUP, it is forgotten, a zygote's busy children die with it, its calls
settle (a job child never forked runs elsewhere), and the death is
reported unless it was retired or a spare that took no job."
  (let ((role (plist-get state :role)) (phase (plist-get state :phase)))
    (cons (if (memq phase '(killed retired)) nil 'dead)
          (append '((hup) (forget) (close))
                  (and (eq role 'zygote) '((orphan-children)))
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
  "`shutdown': an isolate killed later gets SIGKILL now, retired or not,
its subprocess groups SIGHUP, and a live one's calls settled; any other
live one is killed as `kill' does."
  (let ((phase (plist-get state :phase)))
    (cond ((not (romacs--isolate-kills-later-p state)) (romacs--isolate-on-kill state))
          ((not (memq phase '(live retired))) nil)
          ((eq phase 'retired) '(nil (signal 9) (end 9) (hup)))
          (t (let ((condition (romacs--isolate-killed-condition state)))
               `(retired (signal 9) (end 9) (hup) (settle ,condition nil) (hook ,condition nil nil)))))))

;;;; What is known of an isolate

(defun romacs--isolate-lingering-p (zygote)
  "Whether ZYGOTE (an isolate) lingers: it has owners not yet reaped, or
children running a job.  Neither dies before it does, so it waits for
them; an idle child (a spare) is no reason to."
  (> (+ (romacs--isolate-owners zygote) (romacs--isolate-busy-children zygote)) 0))

(defun romacs--isolate-facts (isolate)
  "What `romacs--isolate-transition' knows of ISOLATE now, a plist: :role,
:phase, :signal (the last sent since it began ending), :forked (its pid
is known), :pending (it has calls), :lingering (a zygote with owners or
busy children, `romacs--isolate-lingering-p') and :child (a job child or
an owner)."
  (let ((role (aref isolate 10)))
    (list :role role :phase (aref isolate 3) :signal (cdr (aref isolate 13))
          :forked (and (romacs--isolate-pid (romacs--isolate-id isolate)) t)
          :pending (and (aref isolate 5) t)
          :lingering (and (eq role 'zygote) (romacs--isolate-lingering-p isolate))
          :child (and (memq role '(job owner)) (aref isolate 11) t))))

;;;; The edge

(defun romacs--isolate-step (isolate event)
  "Carry out EVENT on ISOLATE (`romacs--isolate-run'): the children a
synchronous kill signalled, else non-nil when EVENT changed anything."
  (romacs--isolate-run (list (cons isolate event))))

(defun romacs--isolate-run (queue &optional resolved)
  "Carry out the transitions of QUEUE, a list of (ISOLATE . EVENT), in
order, each with the events it leads to: every effect of all of them,
then the futures' callbacks (RESOLVED's first, from
`romacs--isolate-mark-resolved') and the hooks, every one whatever an
earlier one signals.  The first transition's value (`romacs--isolate-step')."
  (let ((later (and resolved (list (lambda () (romacs--isolate-each #'romacs--isolate-notify resolved)))))
        (value nil) (first t))
    (unwind-protect
        (dolist (item queue)
          (let ((done (romacs--isolate-collect (car item) (cdr item))))
            (when first (setq value (car done) first nil))
            (setq later (append later (cdr done)))))
      (romacs--isolate-each #'funcall later))
    value))

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
          ((eq kind 'kill-wait) (romacs--isolate-kill-wait isolate out))
          ((eq kind 'hup) (aset out 3 (romacs--isolate-signal-children id)))
          ((eq kind 'forget)
           (romacs--isolate-forget-role isolate)
           ;; Out of the dispatch before any callback runs: a pump a
           ;; callback makes cannot handle this death again.
           (setq romacs--isolates (delq isolate romacs--isolates)))
          ((eq kind 'close) (romacs--isolate-close id))
          ((memq kind '(retire-children orphan-children zygote-gone))
           (romacs--isolate-follow isolate kind out))
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
          ((eq kind 'resubmit)
           (let ((pending (aref isolate 5)))
             (aset isolate 5 nil)
             (romacs--isolate-later
              out (lambda () (romacs--isolate-each (lambda (entry) (romacs--isolate-resubmit-or-fail (cdr entry)))
                                                   pending)))))
          ((eq kind 'hook)
           (let ((condition (nth 1 effect))
                 (data (list (nth 2 effect) (and (nth 3 effect) (aref out 3)))))
             (romacs--isolate-later
              out (lambda () (run-hook-with-args 'isolate-error-functions isolate condition data)))))
          (t (error "No isolate effect %S for %S" effect event)))))

(defun romacs--isolate-later (out thunk)
  "Queue THUNK in OUT to run once the batch's effects are done."
  (aset out 2 (cons thunk (aref out 2))))

(defun romacs--isolate-follow (isolate kind out)
  "Queue in OUT the events effect KIND of ISOLATE leads to: its idle job
children retired with it, a dead zygote's busy children dead with it,
its zygote told a child is gone."
  (let ((push (lambda (other event) (aset out 1 (cons (cons other event) (aref out 1))))))
    (cond ((eq kind 'zygote-gone)
           (let ((zygote (aref (aref isolate 11) 1)))
             (when (memq zygote romacs--isolates) (funcall push zygote '(child-gone)))))
          (t
           (dolist (child (copy-sequence romacs--isolates))
             (when (and (eq (aref child 10) 'job) (eq (aref (aref child 11) 1) isolate))
               (cond ((eq kind 'retire-children) (funcall push child '(retire)))
                     ;; A child never reported was never released, so it
                     ;; ran nothing: its job runs elsewhere.
                     ((not (aref child 5)))
                     ((romacs--isolate-pid (romacs--isolate-id child)) (funcall push child '(orphaned)))
                     (t (funcall push child '(death isolate-died nil))))))))))

(defun romacs--isolate-end-mark (isolate signal)
  "ISOLATE ends by itself from now, SIGNAL (nil, 15, 9) the last sent it:
the deadlines look again `isolate-kill-timeout' later, unless SIGNAL is
9, the last there is."
  (aset isolate 13 (cons (float-time) signal))
  (unless (eql signal 9)
    (romacs--isolate-wake-at (+ (float-time) isolate-kill-timeout))))

(defun romacs--isolate-kill-wait (isolate out)
  "Kill ISOLATE, a worker that is no owner, and wait for its end: SIGTERM,
then SIGKILL after `isolate-kill-timeout'.  Its death follows in OUT, with
the children it reported signalled; they are OUT's value too."
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
      (aset out 1 (cons (list isolate 'death 'isolate-killed children) (aref out 1))))))

(provide 'isolate-lifecycle)
;;; isolate-lifecycle.el ends here

;;; owner.el --- Owners: isolates that serve for the session  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An owner (ownership design O1 §2) is a child of a zygote
;; (lisp/vm/isolate.el) forked to serve for the session rather than one
;; job: its role is `owner', it opts out of each one-job rule, it outlives
;; its zygote's retirement, and its calls run in order, one at a time, in
;; its loop (`romacs--isolate-serve'), between bounded batches of its
;; processes' output.  Processes a call starts are bound to it: their
;; filters and sentinels run there.
;;
;; Liveness is a ping (crates/romacs-vm/src/owner/mod.rs): while a call is
;; outstanding, or a ping unanswered, the master pings it every half
;; `owner-wedge-delay', and every wait the owner makes answers in Rust.
;; An owner that answers no ping within the delay is wedged, whatever its
;; call is doing.  Killing one never waits.  Not in GNU.

;;; Code:

(defvar owner-wedge-delay 1.0
  "Seconds an owner may leave a ping unanswered before it is wedged.
The master pings an owner every half this while a call to it is
outstanding or a ping unanswered; an owner answers from every wait it
makes, so only a Lisp loop that never waits, or a native hang, leaves
one unanswered.")

(defvar owner-output-batch 16
  "Process output dispatches an owner runs between two looks for calls.
A call queued behind a stream of output waits at most this many.")

(defun romacs--owner-wedge-delay ()
  "`owner-wedge-delay' when it is a number of seconds from 0.01 to under
a million, else its default, 1.0: an option set wrong never stops a pump."
  (if (and (numberp owner-wedge-delay) (<= 0.01 owner-wedge-delay) (< owner-wedge-delay 1e6))
      owner-wedge-delay
    1.0))

(defun romacs--owner-output-batch ()
  "`owner-output-batch' when it is a positive integer, else its default, 16."
  (if (and (integerp owner-output-batch) (> owner-output-batch 0)) owner-output-batch 16))

;; A call settled by its owner's kill: its outcome is unknown, it may
;; have run (ownership design O1 §2.2).  Data: (ISOLATE nil).
(define-error 'owner-killed "Owner killed" 'isolate-killed)

;; An owner's start found features it was asked for absent.  Data:
;; (FEATURES), the absent ones.
(define-error 'owner-unprepared "Owner lacks the features it was started with" 'isolate-error)

;; An owner: [owner ISOLATE START], START the future of its `owner--start'.
;; The record is the owner's identity; ISOLATE and START are its current
;; incarnation (process, zygote, calls, ping), whose life
;; `romacs--isolate-transition' runs.  An incarnation's death is final;
;; the owner's need not be: a later recovery moves the record onto a new
;; incarnation.
(defun owner-p (object) "Non-nil for an owner." (eq (type-of object) 'owner))
(defun owner--isolate (owner) "OWNER's isolate, in the role `owner'." (aref owner 1))
(defun owner--start-future (owner) "The future of OWNER's `owner--start'." (aref owner 2))

(defun owner--spawn (&optional require)
  "Start an owner prepared with REQUIRE's features: a child of their
zygote, which is made if there is none.  It is `starting' until its first
call, `owner--start', answers; calls sent meanwhile run after it.  If that
call fails the owner is `refused', and killed: a zygote refused meanwhile
\(its preparation failed, say) settles its calls with
`isolate-zygote-refused' and the reason, a feature absent in it with
`owner-unprepared'.  `isolate-process-limit' without room for its
process; `isolate-zygote-refused' when REQUIRE's zygote was refused."
  (let ((zygote (romacs--isolate-zygote-for require nil)))
    (when (and zygote (eq (aref zygote 3) 'refused))
      (signal 'isolate-zygote-refused (list (aref zygote 6))))
    ;; The whole owner, the zygote it needs included, is admitted before
    ;; anything changes; owners count against `isolate-process-limit'
    ;; among themselves and the pool's processes, and end no worker for
    ;; room (`romacs--isolate-admission').
    (unless (and (romacs--isolate-admit-p 'owner require (if zygote 1 2))
                 (setq zygote (or zygote (romacs--isolate-zygote-for require t))))
      (signal 'isolate-process-limit (list (length romacs--isolates) (isolate--process-limit))))
    (let* ((isolate (romacs--isolate-zygote-child zygote))
           (start (progn (aset isolate 10 'owner)
                         (romacs--isolate-submit isolate 'owner--start (list require) nil nil nil)))
           (owner (record 'owner isolate start)))
      (isolate-on-done start (lambda (future)
                               (unless (eq (aref future 3) 'done) (owner-kill owner))))
      (romacs--owner-watch-soon isolate)
      owner)))

(defun owner-call (owner function &rest args)
  "Run (apply FUNCTION ARGS) on OWNER: its future (`isolate-await').
FUNCTION is a symbol OWNER calls its own definition of, or a lambda form.
Calls run in the order they are sent, from whichever caller.  An error is
re-signalled by `isolate-await' with its condition and data, and the
owner's backtrace in `isolate-error-backtrace'.  `isolate-died' once
OWNER is gone."
  (unless (or (symbolp function) (eq (car-safe function) 'lambda))
    (signal 'error (list "Cannot send closure" function)))
  (let* ((isolate (owner--isolate owner))
         (future (romacs--isolate-submit isolate function args nil nil nil)))
    (romacs--owner-watch-soon isolate)
    future))

(defun owner-kill (owner)
  "End OWNER without waiting for it: SIGTERM now, SIGKILL
`isolate-kill-timeout' later.  Its pending calls settle with
`owner-killed' at once; their outcome is unknown.  Nil."
  (isolate-kill (owner--isolate owner))
  nil)

(defun owner--start-error (owner)
  "The error OWNER's start failed with, (CONDITION . DATA), or nil."
  (let ((start (owner--start-future owner)))
    (and (eq (aref start 3) 'error)
         (cons (car (aref start 4)) (nth 1 (aref start 4))))))

(defun owner-state (owner)
  "OWNER's state: `starting' until its start answers, `refused' once it
failed (`owner-describe' says why), `live', `wedged' while a ping goes
unanswered past `owner-wedge-delay', `closing' once killed until its end
is seen, `dead'."
  (let ((isolate (owner--isolate owner)))
    (cond ((owner--start-error owner) 'refused)
          ((not (memq isolate romacs--isolates)) 'dead)
          ((eq (aref isolate 3) 'retired) 'closing)
          ((not (isolate-live-p isolate)) 'dead)
          ((not (isolate-future-done-p (owner--start-future owner))) 'starting)
          ((car (romacs--owner-ping (romacs--isolate-id isolate) (romacs--owner-wedge-delay) nil)) 'wedged)
          (t 'live))))

(defun owner-describe (owner)
  "What is known of OWNER, a plist: :state (`owner-state'), :pid, :pings
\(SENT . ANSWERED), :zygote, its zygote's pid, and :error, what its start
failed with, (CONDITION . DATA), or nil."
  (let* ((isolate (owner--isolate owner))
         (id (romacs--isolate-id isolate))
         (pings (and (memq isolate romacs--isolates)
                     (romacs--owner-ping id (romacs--owner-wedge-delay) nil)))
         (zygote (aref (aref isolate 11) 1)))
    (list :state (owner-state owner)
          :pid (and (memq isolate romacs--isolates) (romacs--isolate-pid id))
          :pings (and pings (cons (nth 2 pings) (nth 3 pings)))
          :zygote (and (memq zygote romacs--isolates) (romacs--isolate-pid (romacs--isolate-id zygote)))
          :error (owner--start-error owner))))

(defun romacs--owner-watch (isolate)
  "Judge owner ISOLATE's liveness now, pinging it if one is due (a call
to it is outstanding, or a ping unanswered): the seconds until it needs
another look, or nil."
  (when (isolate-live-p isolate)
    (nth 1 (romacs--owner-ping (romacs--isolate-id isolate) (romacs--owner-wedge-delay) (and (aref isolate 5) t)))))

(defun romacs--owner-watch-soon (isolate)
  "A call to owner ISOLATE went: ping it now, and look again when due."
  (let ((look (romacs--owner-watch isolate)))
    (when look (romacs--isolate-wake-at (+ (float-time) look)))))

;;;; The owner's side

(defun owner--start (require)
  "In a child its spawner forked to be an owner: serve for the session.
Its job budget goes, no one-job rule applies, and its waits answer pings.
`owner-unprepared' when a feature of REQUIRE is absent: it serves never
without the world it was asked for."
  (let ((absent nil))
    (dolist (feature require) (unless (featurep feature) (push feature absent)))
    (when absent (signal 'owner-unprepared (list (nreverse absent)))))
  (setq romacs--isolate-job-worker nil romacs--isolate-owner t)
  (romacs--owner-become)
  (romacs--isolate-name-process "romacs-owner")
  t)

(provide 'owner)
;;; owner.el ends here

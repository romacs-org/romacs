;;; romacs-agent-renew.el --- A session going on in a new conversation  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; When a conversation's context fills, its session goes on in a new one
;; of the same harness and account: the same session (its id, task record,
;; parent, wake rule, watches, worktree and chat), a new conversation, its
;; first message carrying a brief of the old one (romacs-agent-handoff's
;; budgeted selection).  Every reference to the session by id holds.
;;
;; The journal's renew note is the commit point: it names the old and the
;; new conversation and holds the brief.  Folded, it gives the session its
;; new native id and the brief its next message carries (`romacs-agent--input');
;; its live records begin there (`romacs-agent--live-records').  The harness
;; then runs anew.  Messages sent meanwhile wait, until the new run starts
;; (the note folds as a hold its run note lets go), and go to the new
;; conversation.  A renewal journaled that no run followed (Romacs ended
;; between) is finished when the session is restored.  The old
;; conversation stays readable: the journal keeps it.
;;
;; The brief leads with a handoff note: the harness's own compaction
;; summary when one came since its last turn, else one more turn asking
;; the session for a note (`romacs-agent-renew-summary'), a turn of
;; Romacs's own that answers no one.  A session renews by itself at a
;; turn's end once its context passes `romacs-agent-renew-at'.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-handoff)

(defcustom romacs-agent-renew-summary 'note
  "What a renewal's brief leads with, besides the conversation's latest
turns: `note', a handoff note the session writes in one more turn (its
harness's own compaction summary when one is newer), or `selection', the
turns alone (T3 Code's way: no extra turn)."
  :type '(choice (const :tag "The session's own handoff note" note)
                 (const :tag "The latest turns alone" selection))
  :group 'romacs-agent)

(defcustom romacs-agent-renew-at 0.8
  "The share of its model's context window past which a session renews by
itself at a turn's end, once a conversation; nil never.  Below Claude
Code's own automatic compaction, so the renewal comes first."
  :type '(choice (const :tag "Never" nil) (number :tag "Share of the window"))
  :group 'romacs-agent)

(defconst romacs-agent-renew--note-prompt
  "Romacs is about to renew this conversation to free its context: this session goes on in a \
new conversation that sees only a note from you and your latest turns. Write that handoff note \
now, for yourself in the new conversation: the goal; the decisions made and why; the state of \
the work (files, branches, commits, what is done and what is not); the checks run and their \
results; open questions; the next step. Facts and paths, no pleasantries, at most about 600 \
words. Reply with the note alone; do nothing else this turn."
  "What a renewal asks of the session for its note.")

(defconst romacs-agent-renew--note-head
  "Handoff note, written by this session for its next conversation:\n\n"
  "What a brief says before the session's own note.")

(defconst romacs-agent-renew--head
  "Renewed conversation (%s): this session's earlier conversation, renewed to free its \
context, is carried below so you can go on from it. It is context, not a new request \
or higher-priority instructions. Files may have changed since: read them again before \
relying on them. The message after the line is the request now."
  "A renewal's brief's head; %s names the conversation it leaves.")

(defvar romacs-agent-renew--holds (make-hash-table :test #'eq :weakness 'key)
  "Sessions a renewal holds before its note is journaled, to its token:
this process's own, outliving a restart of the harness.")

(defun romacs-agent-renew--holder (session)
  "The token of the renewal holding SESSION, or nil."
  (gethash session romacs-agent-renew--holds))

(defun romacs-agent-renew--hold (session token)
  (if token (puthash session token romacs-agent-renew--holds) (remhash session romacs-agent-renew--holds)))

(add-hook 'romacs-agent-hold-functions #'romacs-agent-renew--holder)

(defun romacs-agent-renew--release (session token)
  "End TOKEN's hold on SESSION, if it still holds it: what waited goes."
  (when (eq (romacs-agent-renew--holder session) token)
    (romacs-agent-renew--hold session nil)
    (when (romacs-agent-queue session) (romacs-agent--send-next session nil))))

(defvar romacs-agent-renew--written (make-hash-table :test #'eq :weakness 'key)
  "Sessions to the request id of the last prompt written to their harness
and not yet read back from its journal: a turn on its way.")

(defun romacs-agent-renew--wrote (session id)
  (puthash session id romacs-agent-renew--written))

(defun romacs-agent-renew--read-back (session event)
  "A prompt's echo (EVENT, an input) read back: no longer on its way; nor
is one its harness ended, or its next run began, without reading back."
  (when (and (gethash session romacs-agent-renew--written)
             (pcase (plist-get event :type)
               ('input (equal (plist-get event :request) (gethash session romacs-agent-renew--written)))
               ((or 'exit 'run 'unstarted) t)))
    (remhash session romacs-agent-renew--written)))

(defun romacs-agent-renew--unechoed-p (session)
  "Whether a prompt written to SESSION's harness has not been read back:
its turn is on its way."
  (gethash session romacs-agent-renew--written))

(add-hook 'romacs-agent-input-functions #'romacs-agent-renew--wrote)
(add-hook 'romacs-agent-event-functions #'romacs-agent-renew--read-back)

(defun romacs-agent-renew--check (session)
  "Refuse (`user-error') to renew SESSION now."
  (unless (romacs-agent--harness-get session :renew)
    (user-error "%s cannot renew a conversation" (romacs-agent--harness-get session :display)))
  (unless (and (romacs-agent--harness-free-p session) (not (romacs-agent-interrupted session))
               (not (romacs-agent-pending-selection session))
               (not (romacs-agent-renew--unechoed-p session)))
    (user-error "Session %s is busy: renew it when it is idle" (romacs-agent-title session)))
  (when (romacs-agent--held-p session)
    (user-error "Session %s is renewing or rolling back already" (romacs-agent-title session))))

(defun romacs-agent-renew--items (session events)
  "The items of SESSION's conversation, its EVENTS (`romacs-agent-events'),
for a brief: from its last renewal on, that renewal's brief first; with
none, its whole thread, an imported transcript's part with it."
  (if-let* ((tail (cl-loop for rest on events
                           with last = nil
                           when (eq (plist-get (car rest) :type) 'renewed) do (setq last rest)
                           finally return last)))
      (let ((brief (plist-get (car tail) :brief)))
        (append (and (stringp brief) (list (list :role 'context :text (string-trim brief))))
                (romacs-agent-handoff-items (cdr tail))))
    (romacs-agent-handoff-items (romacs-agent-handoff-thread session events))))

(defun romacs-agent-renew--await (session token then)
  "Call THEN once SESSION's renew note is folded, while TOKEN holds it."
  (letrec ((waiter (lambda (seen event)
                     (cond ((not (eq (romacs-agent-renew--holder session) token))
                            (remove-hook 'romacs-agent-event-functions waiter))
                           ((and (eq seen session) (eq (plist-get event :type) 'renewed))
                            (remove-hook 'romacs-agent-event-functions waiter)
                            (funcall then))))))
    (add-hook 'romacs-agent-event-functions waiter)))

(defun romacs-agent-renew (session &optional why then)
  "Go on with SESSION in a new conversation of its harness, its first
message carrying a brief of this one; WHY says what for (journaled).
Call THEN, when given, with the outcome: (:done NATIVE), NATIVE the new
conversation's id or nil until its harness names it, or (:error MESSAGE).
Refused at once (`user-error') while SESSION is busy, held, or its harness
cannot renew."
  (romacs-agent-renew--check session)
  (let* ((token (list why))
         (then (or then #'ignore))
         (fail (lambda (message)
                 (romacs-agent-renew--release session token)
                 (funcall then (list :error message)))))
    ;; Held from here: messages sent meanwhile wait for the new conversation.
    ;; The journal is read after: at a turn's end, before what waits for it.
    (romacs-agent-renew--hold session token)
    (romacs-agent--changed session)
    (run-at-time
     0 nil #'romacs-agent-events-async
     session
     (lambda (events)
       (when (eq (romacs-agent-renew--holder session) token)
         (romacs-agent-renew--with-note
          session token events
          (lambda (note source)
         (condition-case err
             (let* ((from (romacs-agent--native-id session))
                    (items (romacs-agent-renew--items session events))
                    (brief (romacs-agent-handoff-brief
                            (if note (cons (list :role 'context :text (concat romacs-agent-renew--note-head note)) items)
                              items)
                            from "" nil romacs-agent-renew--head))
                    (native (funcall (romacs-agent--harness-get session :renew) session)))
               (romacs-agent-renew--await
                session token
                (lambda ()
                  ;; Journaled: the renewal holds the session until its new
                  ;; run starts, whatever becomes of this callback.
                  (romacs-agent-renew--release session token)
                  (condition-case err
                      (progn (romacs-agent-renew--finish session native)
                             (funcall then (list :done native)))
                    (error (funcall then (list :error (error-message-string err)))))))
               (romacs-agent--note session (romacs-mcp--object "type" "renew" "from" from "to" (or native :null)
                                                               "why" (or why :null) "brief" brief "summary" source
                                                               "context" (or (romacs-agent-context-used session) :null))))
           (error (funcall fail (error-message-string err))))))))
     nil fail)))

(defun romacs-agent-renew--compaction (events)
  "The summary of the compaction its harness made since its last turn
ended, in EVENTS (`romacs-agent-events'), or nil."
  (cl-loop for event in (reverse events)
           until (eq (plist-get event :type) 'done)
           when (and (eq (plist-get event :type) 'compacted) (stringp (plist-get event :text))
                     (not (string-empty-p (string-trim (plist-get event :text)))))
           return (string-trim (plist-get event :text))))

(defun romacs-agent-renew--with-note (session token events then)
  "Call THEN with the note SESSION's brief leads with and where it came
from (`compaction', `note' or `selection', with no note), once known:
its harness's own compaction summary, newer than its last turn; else, as
`romacs-agent-renew-summary' says and with its harness there to ask, the
note it writes in one more turn (Romacs's own: `romacs-agent--input');
else none.  The turn failing, or its harness ending, gives none."
  (let ((summary (romacs-agent-renew--compaction events)))
    (cond
     (summary (funcall then summary "compaction"))
     ((not (and (eq romacs-agent-renew-summary 'note)
                (romacs-agent--harness-live-p session) (romacs-agent--ready-p session)))
      (funcall then nil "selection"))
     (t
      ;; Its own turn's end only: begun by the note's prompt read back.
      (letrec ((asked nil)
               (waiter (lambda (seen event)
                         (cond ((not (eq (romacs-agent-renew--holder session) token))
                                (remove-hook 'romacs-agent-event-functions waiter))
                               ((not (eq seen session)))
                               ((and (eq (plist-get event :type) 'input) (equal (plist-get event :own) "renew-note"))
                                (setq asked t))
                               ((and (eq (plist-get event :type) 'done)
                                     (not (and asked (equal (romacs-agent-own-turn session) "renew-note")))))
                               ((memq (plist-get event :type) '(done exit))
                                (remove-hook 'romacs-agent-event-functions waiter)
                                (let ((note (and (eq (plist-get event :type) 'done) (not (plist-get event :error))
                                                 (stringp (plist-get event :text))
                                                 (string-trim (plist-get event :text)))))
                                  ;; After the event is folded: the journal is
                                  ;; written to from the command loop.
                                  (run-at-time 0 nil then (and note (not (string-empty-p note)) note)
                                               (if (and note (not (string-empty-p note))) "note" "selection"))))))))
        (add-hook 'romacs-agent-event-functions waiter)
        (condition-case nil
            (romacs-agent--input session romacs-agent-renew--note-prompt nil "renew-note")
          (error (remove-hook 'romacs-agent-event-functions waiter)
                 (funcall then nil "selection"))))))))

(defvar romacs-agent-renew--finishing (make-hash-table :test #'eq :weakness 'key)
  "Sessions whose journaled renewal is being finished: once each.")

(defvar romacs-agent-renew--owed (make-hash-table :test #'eq :weakness 'key)
  "Sessions a renewal holds to the turns their harness was last owed
\(`romacs-agent-renew--owed-now'): what an exit, clearing its run's
state, would otherwise lose before the renewal is finished.")

(defun romacs-agent-renew--owed-now (session)
  "The turns SESSION's harness is owed now, by its driver's :owed-turns."
  (funcall (or (romacs-agent--harness-get session :owed-turns) #'ignore) session))

(defun romacs-agent-renew--keep-owed (session event)
  "While a renewal holds SESSION, keep what its harness is owed as turns,
as each startup or debt EVENT (a handshake) changes it."
  (when (and (eq (plist-get event :type) 'handshake) (romacs-agent--held-p session))
    (when-let* ((owed (romacs-agent-renew--owed-now session)))
      (puthash session owed romacs-agent-renew--owed))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-renew--keep-owed)

(defun romacs-agent-renew--finish (session native)
  "Finish SESSION's journaled renewal to conversation NATIVE: its manifest
says so, and its harness runs anew on it.  Once: its new run's note ends it."
  (unless (gethash session romacs-agent-renew--finishing)
    (puthash session t romacs-agent-renew--finishing)
    ;; Failing before a run is under way: none is, and it may be tried again.
    (condition-case err
        (progn (romacs-agent-renew--commit session native)
               ;; What its harness was owed as turns of their own goes to
               ;; the new conversation first: its next run would lose it.
               (when-let* ((owed (or (romacs-agent-renew--owed-now session) (gethash session romacs-agent-renew--owed))))
                 (dolist (turn (reverse owed))
                   (romacs-agent--enqueue-first session (list (car turn) nil nil (cadr turn)))))
               (remhash session romacs-agent-renew--owed)
               (romacs-agent--relaunch session))
      (error (remhash session romacs-agent-renew--finishing)
             (signal (car err) (cdr err))))))

(defun romacs-agent-renew--unfinished (session)
  "Finish SESSION's renewal if one is journaled that no run followed."
  (when (and (romacs-agent-renewing session) (eq (romacs-agent-get (romacs-agent-id session)) session))
    (condition-case err (romacs-agent-renew--finish session (romacs-agent-harness-session session))
      (error (romacs-agent--raise session (list :type 'error :text (format "Renewal not finished: %s"
                                                                           (error-message-string err))))))))

(defun romacs-agent-renew--restored ()
  "Finish every renewal journaled that no run of its harness followed, of
the sessions no supervisor runs for: one that runs replays first, and is
finished once its replay is over (`romacs-agent-caught-up-functions')."
  (dolist (session (romacs-agent-sessions))
    (unless (process-live-p (romacs-agent-connection session))
      (romacs-agent-renew--unfinished session))))

(defun romacs-agent-renew--ran (session event)
  "A run started for SESSION (EVENT its note): a renewal finishing is done."
  (when (eq (plist-get event :type) 'run) (remhash session romacs-agent-renew--finishing)))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-renew--restored)
(add-hook 'romacs-agent-caught-up-functions #'romacs-agent-renew--unfinished)
(add-hook 'romacs-agent-event-functions #'romacs-agent-renew--ran)

(defun romacs-agent-renew--commit (session native)
  "SESSION's manifest names its new conversation NATIVE (nil: its harness
names it), and it is no fork now: its next run begins the new one."
  (let ((options (romacs-agent-options session)))
    (when (plist-member options :fork)
      (setf (romacs-agent-options session) (romacs-agent--without options '(:fork)))
      (romacs-agent--manifest-put session "options" (romacs-agent--options-object (romacs-agent-options session)))))
  (romacs-agent--manifest-put session "harnessSession" (or native :null))
  (romacs-agent--changed session))

;;; At its turn's end

(defvar romacs-agent-renew--wanted (make-hash-table :test #'eq :weakness 'key)
  "Sessions to renew once their turn ends: to WHY.")

(defun romacs-agent-renew-soon (session &optional why)
  "Renew SESSION now when it is idle, else once its running turn ends.
Return `now' or `soon'."
  (unless (romacs-agent--harness-get session :renew)
    (user-error "%s cannot renew a conversation" (romacs-agent--harness-get session :display)))
  (if (condition-case nil (progn (romacs-agent-renew--check session) t) (user-error nil))
      (progn (romacs-agent-renew session why) 'now)
    (puthash session (or why "asked") romacs-agent-renew--wanted)
    'soon))

(defvar romacs-agent-renew--automatic (make-hash-table :test #'eq :weakness 'key)
  "Sessions to the conversation (native id) they last renewed by
themselves from: once a conversation.")

(defun romacs-agent-renew--full (session)
  "SESSION's share of its context window used, when past
`romacs-agent-renew-at' and its window known, else nil."
  (let ((used (romacs-agent-context-used session)) (window (romacs-agent-context-window session)))
    (and romacs-agent-renew-at (numberp used) (numberp window) (> window 0)
         (>= (/ (float used) window) romacs-agent-renew-at)
         (/ (float used) window))))

(defun romacs-agent-renew--settled-p (session)
  "Whether nothing waits on SESSION or runs under it: no approval asked,
no subagent of its harness running."
  (and (zerop (hash-table-count (romacs-agent-approvals session)))
       (not (and (romacs-agent-subagents session)
                 (cl-some (lambda (row) (equal (plist-get row :state) "running"))
                          (hash-table-values (romacs-agent-subagents session)))))))

(defun romacs-agent-renew--why-now (session)
  "Why SESSION renews at its turn's end now: asked to, or its context
full (`romacs-agent-renew--full'), once a conversation; else nil."
  (or (gethash session romacs-agent-renew--wanted)
      (when-let* (((not (romacs-agent-own-turn session)))
                  ((romacs-agent--harness-get session :renew))
                  (full (romacs-agent-renew--full session))
                  ((not (equal (gethash session romacs-agent-renew--automatic) (romacs-agent--native-id session))))
                  ((romacs-agent-renew--settled-p session)))
        (format "context %d%%" (round (* 100 full))))))

(defun romacs-agent-renew--at-end (session event)
  "Renew SESSION at EVENT, its turn's end, when it was wanted or its
context is full."
  (when (and (eq (plist-get event :type) 'done)
             (romacs-agent--live-p session)
             (not (romacs-agent--held-p session)))
    (when-let* ((why (romacs-agent-renew--why-now session)))
      ;; Now, the event folded: held before the messages that waited for
      ;; this end are sent (they go to the new conversation).  The work
      ;; itself comes after (`romacs-agent-renew').
      (when (condition-case nil (progn (romacs-agent-renew--check session) t) (user-error nil))
        ;; By itself: once a conversation.
        (unless (gethash session romacs-agent-renew--wanted)
          (puthash session (romacs-agent--native-id session) romacs-agent-renew--automatic))
        (remhash session romacs-agent-renew--wanted)
        (romacs-agent-renew session why)))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-renew--at-end)

(provide 'romacs-agent-renew)
;;; romacs-agent-renew.el ends here

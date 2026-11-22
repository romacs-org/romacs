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

(defvar romacs-agent-renew--owed (make-hash-table :test #'eq :weakness 'key)
  "Sessions a renewal holds to the turns their harness was last owed
\(`romacs-agent-renew--owed-now'): what an exit, clearing its run's
state, would otherwise lose before the renewal is finished.")

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
    ;; Its debt is its own again, unless the renewal is journaled.
    (unless (romacs-agent-renewing session) (remhash session romacs-agent-renew--owed))
    (romacs-agent--dispatch session)))

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
    ;; What its harness is owed now goes with it, whatever ends its run.
    (romacs-agent-renew--note-owed session)
    (romacs-agent--changed session)
    (run-at-time
     0 nil #'romacs-agent-renew--read
     session
     (lambda (summary items)
       (when (eq (romacs-agent-renew--holder session) token)
         (romacs-agent-renew--with-note
          session token summary
          (lambda (note source)
         (condition-case err
             (let* ((from (romacs-agent--native-id session))
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
     fail)))

(defun romacs-agent-renew--read (session then fail)
  "Call THEN with what SESSION's renewal takes of its conversation: its
harness's compaction summary since its last turn, or nil, and its brief's
items (`romacs-agent-renew--items'); FAIL with why, should reading fail.
Its records are read and replayed in a worker VM, to where they end now:
however long its journal, the editor reads none of them."
  (let ((future (condition-case err
                    (isolate-spawn 'romacs-agent-renew--thread (romacs-agent-dir session) (romacs-agent--journal-end session)
                                   (romacs-agent-seq session)
                                   :name "romacs-agent renew"
                                   ;; A worker of its own: one the user waits for
                                   ;; never queues behind background moves.
                                   :fresh t
                                   :require '(romacs-agent romacs-agent-claude romacs-agent-codex romacs-agent-events
                                                           romacs-agent-turns romacs-agent-renew)
                                   :bindings (romacs-agent--fold-bindings))
                  (error (funcall fail (error-message-string err)) nil))))
    (when future
      (isolate-on-done
       future
       (lambda (done)
         ;; Its own worker, done with: let go, answered or not.
         (when-let* ((worker (ignore-errors (isolate-future-isolate done)))) (ignore-errors (isolate-retire worker)))
         (pcase (condition-case err (cons 'read (isolate-await done))
                  (error (cons 'failed (error-message-string err))))
           (`(read ,summary ,items) (funcall then summary items))
           (`(failed . ,why) (funcall fail why))
           (other (funcall fail (format "the worker answered %S" other)))))))))

(declare-function romacs-agent-turns-page "romacs-agent-turns" (session &optional before limit))
(declare-function romacs-agent-turns-first-after "romacs-agent-turns" (session seq))

(defun romacs-agent-renew--thread (dir end &optional last)
  "In a worker VM: what a renewal takes of the conversation of the session
recorded in DIR, its records to place END, the last of them seq LAST:
\(SUMMARY ITEMS), as `romacs-agent-renew--read' gives them.  Its turns
since its last renewal come from their rows (romacs-agent-turns), as many
as a brief keeps, and only what came after the newest of them from its
records; with no rows, from its records since that renewal."
  (require 'romacs-agent-turns)
  (let* ((romacs-agent-store-patience romacs-agent-store-patient)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--of-manifest dir (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
         (note (romacs-agent-renew--last-note session))
         (from (if note (gethash "seq" note) 0))
         (rows (romacs-agent-renew--rows session from (or last most-positive-fixnum)))
         ;; What came after the newest row read, from its records; with no
         ;; place for it there, its records since the renewal instead.
         (after (and rows (romacs-agent--journal-after session (plist-get (car (last rows)) :last-seq))))
         (rows (and (integerp after) rows))
         (events (append
                  ;; The renewal its rows come after, as its record would raise it.
                  (and rows note (list (list :type 'renewed :brief (gethash "brief" (gethash "note" note)))))
                  (mapcan #'romacs-agent-renew--row-events rows)
                  (romacs-agent-events session end (cond (rows after) (note (romacs-agent-renew--place-of session from)))))))
    (list (romacs-agent-renew--compaction events) (romacs-agent-renew--items session events))))

(defun romacs-agent-renew--last-note (session)
  "The record of SESSION's last renewal note, or nil.  Only a record that
holds the note's own type is parsed: text that speaks of renewals, its
quotes escaped, never matches."
  (romacs-agent--journal-last session '("\"type\":\"renew\"") (lambda (record) (romacs-agent--note-p record "renew"))))

(defun romacs-agent-renew--rows (session from last)
  "The rows of SESSION's turns after seq FROM and to seq LAST, oldest
first, as many of the latest as a brief keeps (`romacs-agent-handoff-budget')
and, when earlier ones are left out, the first of them and a gap's mark
\(the symbol `gap') between: nil when it has none."
  (let ((rows nil) (size 0) (before nil) (more t))
    (while (and more (<= size romacs-agent-handoff-budget))
      (let ((page (romacs-agent-turns-page session before romacs-agent-renew--rows-page)))
        (dolist (row (plist-get page :rows))
          (when (and (> (plist-get row :first-seq) from) (<= (plist-get row :last-seq) last)
                     (<= size romacs-agent-handoff-budget))
            (push row rows)
            (cl-incf size (romacs-agent-renew--row-size row))))
        (setq before (plist-get (car (last (plist-get page :rows))) :first-seq)
              more (and (plist-get page :more) before (> before from)))))
    ;; Left out: the conversation's first turn still leads, as a brief keeps it.
    (when-let* (((and rows (> size romacs-agent-handoff-budget)))
                (row (romacs-agent-turns-first-after session from))
                ((< (plist-get row :first-seq) (plist-get (car rows) :first-seq))))
      (setq rows (cons row (cons 'gap rows))))
    rows))

(defconst romacs-agent-renew--rows-page 50 "The turns' rows read at a time.")

(defun romacs-agent-renew--row-size (row)
  "About the characters ROW's turn gives a brief."
  (+ (length (or (plist-get row :prompt) "")) (length (or (plist-get row :reply) ""))
     (* 80 (cl-count "tool" (plist-get row :items) :key (lambda (item) (gethash "kind" item)) :test #'equal))))

(defun romacs-agent-renew--input (text messages)
  "An input event of TEXT, with MESSAGES (a row's: who said what in it, as
tables) as its event carries them."
  (let ((messages (mapcar (lambda (message)
                            (let ((from (gethash "from" message)))
                              (list :kind (gethash "kind" message)
                                    :from (list :name (and (hash-table-p from) (gethash "name" from))))))
                          (append messages nil))))
    (append (list :type 'input :text text) (and messages (list :messages messages)))))

(defun romacs-agent-renew--row-events (row)
  "ROW, a turn's (`romacs-agent-turns-page'), or `gap', as the events its
records raised, as far as a brief reads them (`romacs-agent-handoff-items')."
  (if (eq row 'gap) (list (list :type 'gap :text "earlier turns of this conversation left out"))
    (let ((events nil))
      (push (romacs-agent-renew--input (plist-get row :prompt) (plist-get row :messages)) events)
      (seq-doseq (item (plist-get row :items))
        (pcase (gethash "kind" item)
          ("text" (unless (gethash "parent" item)
                    (push (list :type 'delta :text (gethash "text" item) :message (gethash "message" item)) events)
                    (push (list :type 'message-end :message (gethash "message" item)) events)))
          ("tool" (push (list :type 'tool :state 'started :id (gethash "id" item) :name (gethash "name" item)
                              :input (gethash "input" item) :parent (gethash "parent" item))
                        events))
          ("input" (push (append (romacs-agent-renew--input (gethash "text" item) (gethash "messages" item))
                                 (list :steer (gethash "steer" item)))
                         events))
          ("compacted" (push (list :type 'compacted :text (gethash "text" item)) events))))
      (push (list :type 'done :text (plist-get row :reply)) events)
      (nreverse events))))

(defun romacs-agent-renew--place-of (session seq)
  "The place at which SESSION's record SEQ begins, or nil."
  (if (romacs-agent-events-p session) seq (romacs-agent--journal-file-after session seq nil t)))

(defun romacs-agent-renew--compaction (events)
  "The summary of the compaction its harness made since its last turn
ended, in EVENTS (`romacs-agent-events'), or nil."
  (cl-loop for event in (reverse events)
           until (eq (plist-get event :type) 'done)
           when (and (eq (plist-get event :type) 'compacted) (stringp (plist-get event :text))
                     (not (string-empty-p (string-trim (plist-get event :text)))))
           return (string-trim (plist-get event :text))))

(defun romacs-agent-renew--with-note (session token summary then)
  "Call THEN with the note SESSION's brief leads with and where it came
from (`compaction', `note' or `selection', with no note), once known:
SUMMARY, its harness's own compaction summary newer than its last turn
\(`romacs-agent-renew--compaction'); else, as
`romacs-agent-renew-summary' says and with its harness there to ask, the
note it writes in one more turn (Romacs's own: `romacs-agent--input');
else none.  The turn failing, or its harness ending, gives none."
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
      ;; Written now, through its hold, or not at all.
      (let ((item (romacs-agent-item romacs-agent-renew--note-prompt :own "renew-note" :token token)))
        (condition-case nil
            ;; Never left queued, however this ends (a quit too): at the
            ;; queue's head it would hold everything behind it.
            (unwind-protect
                (progn (romacs-agent--admit session item t)
                       (when (memq item (romacs-agent-queue session)) (error "Not written")))
              (romacs-agent--dequeue session item))
          ;; A quit too: the renewal goes on without its note, and lets go
          ;; of its hold as it does.
          ((quit error) (remove-hook 'romacs-agent-event-functions waiter)
           (funcall then nil "selection"))))))))

(defvar romacs-agent-renew--finishing (make-hash-table :test #'eq :weakness 'key)
  "Sessions whose journaled renewal is being finished: once each.")

(defun romacs-agent-renew--owed-now (session)
  "The turns SESSION's harness is owed now, by its driver's :owed-turns."
  (funcall (or (romacs-agent--harness-get session :owed-turns) #'ignore) session))

(defun romacs-agent-renew--note-owed (session)
  "Keep what SESSION's harness is owed as turns now, for its renewal."
  (if-let* ((owed (romacs-agent-renew--owed-now session)))
      (puthash session owed romacs-agent-renew--owed)
    (remhash session romacs-agent-renew--owed)))

(defun romacs-agent-renew--keep-owed (session event)
  "While a renewal (not a rollback) holds SESSION, keep what its harness
is owed as turns as each startup or debt EVENT (a handshake) changes it;
an exit, clearing its run's state, leaves the last kept."
  (when (and (eq (plist-get event :type) 'handshake)
             (or (romacs-agent-renew--holder session) (romacs-agent-renewing session)))
    (romacs-agent-renew--note-owed session)))

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
                   (romacs-agent--enqueue-first session (romacs-agent-item (car turn) :images (cadr turn) :kind 'next))))
               (remhash session romacs-agent-renew--owed)
               (romacs-agent--relaunch session))
      (error (remhash session romacs-agent-renew--finishing)
             (signal (car err) (cdr err))))))

(defun romacs-agent-renew--unfinished (session)
  "Finish SESSION's renewal if one is journaled that no run followed."
  (when (and (romacs-agent-renewing session) (eq (romacs-agent-loaded (romacs-agent-id session)) session))
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
  "A run started for SESSION (EVENT its note): a renewal finishing is done.
Its launch failed instead (`unstarted'): the renewal, journaled, waits
for the next resume, which finishes it, and SESSION's view says so."
  (pcase (plist-get event :type)
    ('run (remhash session romacs-agent-renew--finishing))
    ('unstarted
     (remhash session romacs-agent-renew--finishing)
     (when (romacs-agent-renewing session)
       (romacs-agent--raise session (list :type 'error
                                          :text (substitute-command-keys
                                                 "Renewal not finished: resume the session (\\<romacs-agent-menu-map>\\[romacs-agent-chat-resume] in its actions menu) to finish it")))))))

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

(defun romacs-agent-renew-threshold (session)
  "The share of its model's context window at which SESSION renews by
itself at a turn's end (`romacs-agent-renew-at'), for a view to show; nil
when it will not: renewing off, its harness unable to renew, or renewed
by itself from this conversation already."
  (and romacs-agent-renew-at (romacs-agent--harness-get session :renew)
       (not (equal (gethash session romacs-agent-renew--automatic) (romacs-agent--native-id session)))
       romacs-agent-renew-at))

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

;;; romacs-agent-summary.el --- A session's state as plain data  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What a session's views read of it, as plain data: its store row's
;; fields and what goes on in it now, from the session's own facts, never
;; a chat's.  A session's owner publishes it, and the board, menus,
;; switcher, MCP tools and agents service read it rather than the live
;; `romacs-agent' record, which holds processes, buffers and timers.  A
;; tool that names a session by id asks its owner for it first, loaded
;; (`romacs-agent-tools--resolve'): a row alone has no turns or reply.  A
;; tagged list of strings without text properties, so `prin1' and `read',
;; or any message between VMs, carry it whole.  Each slot is read
;; through the live computation that already makes it: no rule is
;; restated here.
;;
;; A view of another machine's sessions reads them from its source
;; (`romacs-agent-log--source'), whose methods here say what it knows.
;;
;; A summary is made once and kept (`romacs-agent--summaries') while what
;; it is made from holds, as an owner publishes it on change: a loaded
;; session's until a fact of it changes (`romacs-agent--summary-stale'),
;; one not loaded while its store row is the same, a remote one while its
;; service's row is.  Views read the kept one.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)

(declare-function romacs-agent-resume-time "romacs-agent-tools" (session))
(declare-function romacs-agent-tools--wake-rule "romacs-agent-tools" (child))
(declare-function romacs-agent-tools--wanted-p "romacs-agent-tools" (session seq &optional summary))
(declare-function romacs-agent-store-unloaded-rows "romacs-agent-store" (&optional parent))
(declare-function romacs-agent-store-row "romacs-agent-store" (id))
(declare-function romacs-agent-store-row-parent "romacs-agent-store" (row))
(declare-function romacs-agent-store--row-of "romacs-agent-store" (row))
(declare-function romacs-agent-ui--shown-key "romacs-agent-ui" (key parent done failed))
(declare-function romacs-agent-ui--state-key "romacs-agent-ui" (session))
(declare-function romacs-agent-ui--default-effort "romacs-agent-ui" (session selection))
(declare-function romacs-agent-watch-unit "romacs-agent-watch" (session))

(cl-defstruct (romacs-agent-summary (:type list) :named
                                    (:constructor romacs-agent-summary--make)
                                    (:copier nil))
  "What a session's views read of it, as plain data (`romacs-agent-summary')."
  ;; Its store row's (`romacs-agent-store--row-of').
  id dir harness provider model title cwd parent effort plan resume-at state
  last-time cost input-seq done-seq seen-seq settled-seq failed
  permission-mode context-used context-window
  ;; What goes on now: what it does (`romacs-agent-doing'); when its
  ;; running turn began; its workers as the header's line and as rows, a
  ;; delegated session's by its :id; the model or effort switch under
  ;; way; its live requests, as MCP's pendingApproval counts them; the
  ;; messages queued; whether its supervisor acknowledged the attachment.
  activity turn-start children-summary children-rows pending-selection
  waiting-count queue-length attached
  ;; The plan it offers to proceed with, by its event's seq, in plan
  ;; mode only (`romacs-agent-plan-offered'); what waits on its user,
  ;; `approval', `question' or `plan', and how many; its state's key
  ;; (`romacs-agent-ui--states'); the unit it watches; the tool its
  ;; running turn called last; the model it is to run (else the one it
  ;; runs), and, while it names no effort, the default effort of the one
  ;; it runs.
  plan-offer user-wait-kind user-wait-count state-key watch-unit active-tool
  selected-model default-effort
  ;; As a task record says it (`romacs-agent-tools--task'): whether the
  ;; plan it offers waits on its user (`romacs-agent-plan-offer-waits');
  ;; its turns, its last reply (the record's own string, not copied:
  ;; `romacs-agent-summary--shared'), whether it was cancelled; the role,
  ;; worktree and branch it was delegated with, its harness's name for its
  ;; conversation (`romacs-agent--native-id'), the model asked for, when
  ;; its end wakes its parent (`romacs-agent-tools--wake-rule'); its last
  ;; end at its instance's usage limit, the move to another account aside.
  ;; Where it could go on from there is read when asked, never kept
  ;; (`romacs-agent-summary-alternatives').  Whether its last end still
  ;; waits to be delivered to its parent (`romacs-agent-tools--wanted-p'),
  ;; and the session a handoff carries it on in (its :handed-to).
  plan-offer-waits turns reply cancelled role worktree branch conversation
  requested-model wake-rule limited undelivered handed-to)

(defun romacs-agent-summary--plain (value)
  "VALUE with every string in it, at any depth, without text properties:
no live object rides in a property (a closure, a buffer)."
  (cond ((stringp value) (substring-no-properties value))
        ((consp value)
         (let* ((copy (list nil)) (end copy))
           (while (consp value)
             (setq end (setcdr end (list (romacs-agent-summary--plain (pop value))))))
           (setcdr end (romacs-agent-summary--plain value))
           (cdr copy)))
        ((vectorp value) (vconcat (mapcar #'romacs-agent-summary--plain value)))
        (t value)))

(defun romacs-agent-summary--shared (text)
  "TEXT as a summary carries it: TEXT itself when no text property rides
on it, as a long reply is not copied each time its summary is made; else
a copy without them (`romacs-agent-summary--plain')."
  (if (or (not (stringp text)) (string-empty-p text)
          (and (null (text-properties-at 0 text)) (null (next-property-change 0 text))))
      text
    (substring-no-properties text)))

(defun romacs-agent-summary-alternatives (summary)
  "The enabled instances the session SUMMARY says could go on under, with
room (`romacs-agent-alternatives'): read when asked, as room comes back
when a limit resets, with no write to say so."
  (romacs-agent--alternatives (romacs-agent-summary-harness summary)
                              (romacs-agent-summary-provider summary)))

(defun romacs-agent-summary--worker (row)
  "Worker ROW (`romacs-agent-ui--summarize-children') as data: a
delegated session's by its id."
  (if-let* ((child (plist-get row :session)))
      (plist-put (romacs-agent--without row '(:session)) :id (romacs-agent-id child))
    row))

(cl-defgeneric romacs-agent-summary--entries (source id)
  "SOURCE's session ID, nil when none is known by ID, else with ID nil
every session SOURCE shows, as a list of (ID STAMP KNOWN . MAKE): STAMP
what its summary is made from, the same while it holds; KNOWN whether
its parent is known, which its key reads (`romacs-agent-summary--key')
and no fact of its own says; MAKE a function giving its record, nil when
STAMP is that record.")

(defun romacs-agent-summary--loaded-entry (session known)
  "Loaded SESSION as an entry (`romacs-agent-summary--entries'), stamped by
its record, which its events change (`romacs-agent--summary-stale'), and
made from it: no MAKE."
  (list (romacs-agent-id session) session known))

(defun romacs-agent-summary--row-entry (row known)
  "Store ROW as an entry (`romacs-agent-summary--entries'), stamped by
itself, its record made from it alone (`romacs-agent-store--row-of')."
  (cl-list* (car row) row known (lambda () (romacs-agent-store--row-of row))))

(defun romacs-agent-summary--parent-known (parent)
  "Whether session PARENT is known, loaded or by its row, as
`romacs-agent-get' finds it; nil for no PARENT."
  (and parent (or (romacs-agent-loaded parent) (romacs-agent-store-row parent)) t))

(defun romacs-agent-summary--own (id)
  "This editor's own entries (`romacs-agent-summary--entries'), the loaded
ones first, as the registry holds them, then those not loaded, by their
store rows, left unloaded."
  (if id
      (if-let* ((session (romacs-agent-loaded id)))
          (list (romacs-agent-summary--loaded-entry
                 session (romacs-agent-summary--parent-known (plist-get (romacs-agent-options session) :parent))))
        (and-let* ((row (romacs-agent-store-row id)))
          (list (romacs-agent-summary--row-entry
                 row (romacs-agent-summary--parent-known (romacs-agent-store-row-parent row))))))
    (let* ((sessions (hash-table-values romacs-agent--sessions)) (rows (romacs-agent-store-unloaded-rows))
           (ids (make-hash-table :test #'equal :size (+ (length sessions) (length rows)))))
      (dolist (session sessions) (puthash (romacs-agent-id session) t ids))
      (dolist (row rows) (puthash (car row) t ids))
      (cl-flet ((known (parent) (and parent (gethash parent ids) t)))
        (append (mapcar (lambda (session)
                          (romacs-agent-summary--loaded-entry
                           session (known (plist-get (romacs-agent-options session) :parent))))
                        sessions)
                (mapcar (lambda (row) (romacs-agent-summary--row-entry row (known (romacs-agent-store-row-parent row))))
                        rows))))))

(cl-defmethod romacs-agent-summary--entries (_source id)
  "This editor's own (`romacs-agent-summary--own')."
  (romacs-agent-summary--own id))

(cl-defgeneric romacs-agent-summary--key (source session known)
  "SESSION's state key (`romacs-agent-ui--states') as SOURCE says it, as
every view of it shows it, KNOWN whether its parent is known.")

(cl-defmethod romacs-agent-summary--key (_source session known)
  "As this editor says it, from SESSION's data (`romacs-agent-ui--shown-key')."
  (romacs-agent-ui--shown-key (romacs-agent-ui--state-key session) known
                              (romacs-agent-done-seq session) (romacs-agent-failed session)))

(defun romacs-agent-summary--user-waits (session doing key offer)
  "What waits on SESSION's user, as (KIND . COUNT): its requests the user
answers (`romacs-agent--user-answers-p'), a `question' when its state
KEY says so, else `approval', while it is DOING `waiting'; and the plan
it OFFERs to proceed with while that waits on the user
\(`romacs-agent-plan-offer-waits'), `plan' when nothing else waits."
  (let ((asks (cl-loop for id being the hash-keys of (romacs-agent-approvals session)
                       count (romacs-agent--user-answers-p session id))))
    (cons (cond ((eq doing 'waiting) (if (eq key 'question) 'question 'approval))
                (offer 'plan))
          (+ asks (if offer 1 0)))))

(defvar romacs-agent-summary--defaults nil
  "While every session's summary is made at once, the default efforts read
\(`romacs-agent-ui--default-effort'), by what each is read from: the
sessions on one model share its catalogue's read.")

(defun romacs-agent-summary--default-effort (session selection)
  "SESSION's default effort for SELECTION (`romacs-agent-ui--default-effort'),
read once a summary pass (`romacs-agent-summary--defaults')."
  (if (null romacs-agent-summary--defaults)
      (romacs-agent-ui--default-effort session selection)
    (let* ((key (list (romacs-agent-harness session) selection (romacs-agent-model session)
                      (plist-get (romacs-agent-driver-state session) :reported-effort)))
           (known (gethash key romacs-agent-summary--defaults 'unread)))
      (if (eq known 'unread)
          (puthash key (romacs-agent-ui--default-effort session selection) romacs-agent-summary--defaults)
        known))))

(defun romacs-agent-summary--of (source session known)
  "SOURCE's SESSION as a summary (`romacs-agent-summary'), KNOWN whether
its parent is known."
  (let* ((id (romacs-agent-id session))
         (options (romacs-agent-options session))
         (doing (romacs-agent-doing session))
         (key (romacs-agent-summary--key source session known))
         (offer (romacs-agent-plan-offer-waits session))
         (waits (romacs-agent-summary--user-waits session doing key offer))
         (selection (romacs-agent--selection session))
         (summary
          (romacs-agent-summary--plain
           (romacs-agent-summary--make
            :id id :dir (romacs-agent-dir session)
            :harness (romacs-agent-harness session) :provider (romacs-agent-provider session)
            :model (romacs-agent-model session) :title (romacs-agent-title session)
            :cwd (romacs-agent-cwd session) :parent (plist-get options :parent)
            :effort (plist-get options :effort) :plan (plist-get options :plan)
            :resume-at (romacs-agent-resume-time session) :state (romacs-agent-state session)
            :last-time (romacs-agent-last-time session) :cost (romacs-agent-cost session)
            :input-seq (romacs-agent-input-seq session) :done-seq (romacs-agent-done-seq session)
            :seen-seq (romacs-agent-seen-seq session) :settled-seq (romacs-agent-settled-seq session)
            :failed (romacs-agent-failed session) :permission-mode (romacs-agent-permission-mode session)
            :context-used (romacs-agent-context-used session)
            :context-window (romacs-agent-context-window session)
            :activity doing :turn-start (romacs-agent-turn-start session)
            :children-summary (romacs-agent-children-summary session)
            :children-rows (mapcar #'romacs-agent-summary--worker (romacs-agent-children-rows session))
            :pending-selection (romacs-agent--without (romacs-agent-pending-selection session) '(:timer))
            :waiting-count (hash-table-count (romacs-agent-approvals session))
            :queue-length (romacs-agent-queue-length session)
            :attached (romacs-agent--attached-p id)
            :plan-offer (romacs-agent-plan-offered session)
            :user-wait-kind (car waits) :user-wait-count (cdr waits)
            :state-key key :watch-unit (romacs-agent-watch-unit session)
            :active-tool (romacs-agent-activity session)
            :selected-model (or (plist-get selection :model) (romacs-agent-model session))
            :default-effort (unless (plist-get options :effort) (romacs-agent-summary--default-effort session selection))))))
    (romacs-agent-summary--task summary session (and offer t))))

(defun romacs-agent-summary--task (summary session offer)
  "SUMMARY of SESSION with what a task record says of it
\(`romacs-agent-tools--task'), OFFER whether the plan it offers waits on
its user: as data, its reply shared, not copied
\(`romacs-agent-summary--shared')."
  (let ((options (romacs-agent-options session)))
    (setf (romacs-agent-summary-plan-offer-waits summary) offer
          (romacs-agent-summary-turns summary) (romacs-agent-turns session)
          (romacs-agent-summary-cancelled summary) (romacs-agent-cancelled session)
          (romacs-agent-summary-role summary) (romacs-agent-summary--plain (plist-get options :role))
          (romacs-agent-summary-worktree summary) (romacs-agent-summary--plain (plist-get options :worktree))
          (romacs-agent-summary-branch summary) (romacs-agent-summary--plain (plist-get options :branch))
          (romacs-agent-summary-conversation summary) (romacs-agent-summary--plain (romacs-agent--native-id session))
          (romacs-agent-summary-requested-model summary) (romacs-agent-summary--plain (plist-get options :model))
          (romacs-agent-summary-wake-rule summary) (romacs-agent-summary--plain (romacs-agent-tools--wake-rule session))
          (romacs-agent-summary-limited summary)
          (romacs-agent-summary--plain (romacs-agent--without (romacs-agent-limited session) '(:moving :moving-since)))
          (romacs-agent-summary-reply summary) (romacs-agent-summary--shared (romacs-agent-reply session))
          (romacs-agent-summary-handed-to summary) (romacs-agent-summary--plain (plist-get options :handed-to)))
    ;; Its cancellation read first, above.
    (setf (romacs-agent-summary-undelivered summary)
          (and (plist-get options :parent) (> (or (romacs-agent-done-seq session) 0) 0)
               (romacs-agent-tools--wanted-p session (romacs-agent-done-seq session) summary)
               t))
    summary))

(defvar romacs-agent-summary--made-under nil
  "What every summary kept (`romacs-agent--summaries') was made under: the
approval policy, the models and the providers.")

(defun romacs-agent-summary--still ()
  "Drop every summary kept when what all of them were made under changed."
  (pcase-let ((`(,policy ,models . ,providers) romacs-agent-summary--made-under))
    (unless (and romacs-agent-summary--made-under (eq policy romacs-agent-approval-policy)
                 (eq models romacs-agent-models) (eq providers romacs-agent-providers))
      (clrhash romacs-agent--summaries)
      (setq romacs-agent-summary--made-under
            (cons romacs-agent-approval-policy (cons romacs-agent-models romacs-agent-providers))))))

(defun romacs-agent-summary--all-stale (&rest _)
  "Every summary kept is stale: the models reported changed."
  (clrhash romacs-agent--summaries))

(add-hook 'romacs-agent-models-changed-functions #'romacs-agent-summary--all-stale)

(defun romacs-agent-summary--kept (source entry)
  "ENTRY's summary (`romacs-agent-summary--entries'), SOURCE's: the one
kept while its stamp, and whether its parent is known, hold; else made
and kept, as (STAMP KNOWN . SUMMARY)."
  (pcase-let* ((`(,id ,stamp ,known . ,make) entry)
               (kept (gethash id romacs-agent--summaries)))
    (if (and kept (or (eq (car kept) stamp) (and (consp stamp) (equal (car kept) stamp)))
             (eq (cadr kept) known))
        (cddr kept)
      (cddr (puthash id (cl-list* stamp known (romacs-agent-summary--of source (if make (funcall make) stamp) known))
                     romacs-agent--summaries)))))

(defun romacs-agent-summary-kept (id)
  "Session ID's summary as kept, not checked against what it was made
from, or nil when none is: for a view asked again between its draws (the
board's tick).  A loaded session's is dropped as it changes; one known by
a row alone changes only by that row, which a view draws again for."
  (cddr (gethash id romacs-agent--summaries)))

(defun romacs-agent-summary-held (session)
  "SESSION's summary (`romacs-agent-summary'), by its id; for a record
held of a session neither loaded nor in the store any more (one gone),
made from that record as it was."
  (or (romacs-agent-summary (romacs-agent-id session))
      (romacs-agent-summary--of nil session (and (plist-get (romacs-agent-options session) :parent) t))))

(defun romacs-agent-summary (id &optional source)
  "Session ID's summary (the `romacs-agent-summary' type), or nil when no
session is known by ID; with ID nil, a list of every session's.  The
sessions are SOURCE's (`romacs-agent-log--source'), by default this
editor's own: one not loaded is read from its store row alone, as
`romacs-agent-store-row-session' makes it, and stays unloaded.  Each is
made once, and kept while what it was made from holds."
  (romacs-agent-summary--still)
  ;; This editor's own read without a generic's dispatch: views and tools
  ;; read a summary often.
  (let ((entries (if source (romacs-agent-summary--entries source id) (romacs-agent-summary--own id))))
    (if id
        (and entries (romacs-agent-summary--kept source (car entries)))
      (let ((romacs-agent-summary--defaults (make-hash-table :test #'equal)))
        (mapcar (lambda (entry) (romacs-agent-summary--kept source entry)) entries)))))

(provide 'romacs-agent-summary)
;;; romacs-agent-summary.el ends here

;;; romacs-agent-summary.el --- A session's state as plain data  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What a session's views read of it, as plain data: its store row's
;; fields and what goes on in it now, from the session's own facts, never
;; a chat's.  A session's owner publishes it, and the board, menus,
;; switcher and MCP tools read it rather than the live `romacs-agent'
;; record, which holds processes, buffers and timers.  A tagged list of
;; strings without text properties, so `prin1' and `read', or any message
;; between VMs, carry it whole.  Each slot is read through the live
;; computation that already makes it: no rule is restated here.
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
(declare-function romacs-agent-store-unloaded-rows "romacs-agent-store" (&optional parent))
(declare-function romacs-agent-store-row "romacs-agent-store" (id))
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
  selected-model default-effort)

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

(defun romacs-agent-summary--worker (row)
  "Worker ROW (`romacs-agent-ui--summarize-children') as data: a
delegated session's by its id."
  (if-let* ((child (plist-get row :session)))
      (plist-put (romacs-agent--without row '(:session)) :id (romacs-agent-id child))
    row))

(cl-defgeneric romacs-agent-summary--entries (source id)
  "SOURCE's session ID, nil when none is known by ID, else with ID nil
every session SOURCE shows, as a list of (ID STAMP . MAKE): STAMP what
its summary is made from, the same while it holds, MAKE a function giving
its record.")

(cl-defmethod romacs-agent-summary--entries (_source id)
  "This editor's own: a loaded one stamped by its record, which its events
change (`romacs-agent--summary-stale'); one not loaded by its store row,
its record made from the row alone (`romacs-agent-store-unloaded') and
left unloaded."
  (cl-flet ((loaded (session) (cons (romacs-agent-id session) (cons session (lambda () session))))
            (stored (row) (cons (car row) (cons row (lambda () (romacs-agent-store--row-of row))))))
    (if id
        (if-let* ((session (romacs-agent-loaded id))) (list (loaded session))
          (and-let* ((row (romacs-agent-store-row id))) (list (stored row))))
      (append (mapcar #'loaded (hash-table-values romacs-agent--sessions))
              (mapcar #'stored (romacs-agent-store-unloaded-rows))))))

(cl-defgeneric romacs-agent-summary--key (source session)
  "SESSION's state key (`romacs-agent-ui--states') as SOURCE says it, as
every view of it shows it.")

(cl-defmethod romacs-agent-summary--key (_source session)
  "As this editor says it, from SESSION's data (`romacs-agent-ui--shown-key')."
  (romacs-agent-ui--shown-key (romacs-agent-ui--state-key session)
                              (and-let* ((parent (plist-get (romacs-agent-options session) :parent)))
                                (romacs-agent-get parent t))
                              (romacs-agent-done-seq session) (romacs-agent-failed session)))

(defun romacs-agent-summary--user-waits (session doing key)
  "What waits on SESSION's user, as (KIND . COUNT): its requests the user
answers (`romacs-agent--user-answers-p'), a `question' when its state
KEY says so, else `approval', while it is DOING `waiting'; and a plan it
offers to proceed with while that waits on the user
\(`romacs-agent-plan-offer-waits'), `plan' when nothing else waits."
  (let ((offer (romacs-agent-plan-offer-waits session))
        (asks (cl-loop for id being the hash-keys of (romacs-agent-approvals session)
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

(defun romacs-agent-summary--of (source session)
  "SOURCE's SESSION as a summary (`romacs-agent-summary')."
  (let* ((id (romacs-agent-id session))
         (options (romacs-agent-options session))
         (doing (romacs-agent-doing session))
         (key (romacs-agent-summary--key source session))
         (waits (romacs-agent-summary--user-waits session doing key))
         (selection (romacs-agent--selection session)))
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
kept while its stamp holds, else made and kept."
  (pcase-let* ((`(,id ,stamp . ,make) entry)
               (kept (gethash id romacs-agent--summaries)))
    (if (and kept (or (eq (car kept) stamp) (and (consp stamp) (equal (car kept) stamp))))
        (cdr kept)
      (cdr (puthash id (cons stamp (romacs-agent-summary--of source (funcall make))) romacs-agent--summaries)))))

(defun romacs-agent-summary-kept (id)
  "Session ID's summary as kept, not checked against what it was made
from, or nil when none is: for a view asked again between its draws (the
board's tick).  A loaded session's is dropped as it changes; one known by
a row alone changes only by that row, which a view draws again for."
  (cdr (gethash id romacs-agent--summaries)))

(defun romacs-agent-summary (id &optional source)
  "Session ID's summary (the `romacs-agent-summary' type), or nil when no
session is known by ID; with ID nil, a list of every session's.  The
sessions are SOURCE's (`romacs-agent-log--source'), by default this
editor's own: one not loaded is read from its store row alone, as
`romacs-agent-store-row-session' makes it, and stays unloaded.  Each is
made once, and kept while what it was made from holds."
  (romacs-agent-summary--still)
  (let* ((romacs-agent-summary--defaults (unless id (make-hash-table :test #'equal)))
         (summaries (mapcar (lambda (entry) (romacs-agent-summary--kept source entry))
                            (romacs-agent-summary--entries source id))))
    (if id (car summaries) summaries)))

(provide 'romacs-agent-summary)
;;; romacs-agent-summary.el ends here

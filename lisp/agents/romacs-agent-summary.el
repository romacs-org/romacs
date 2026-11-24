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

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)

(declare-function romacs-agent-resume-time "romacs-agent-tools" (session))

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
  waiting-count queue-length attached)

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

(defun romacs-agent-summary (id)
  "Session ID's summary (the `romacs-agent-summary' type), or nil when no
session is known by ID.  One not loaded is read from its store row
alone, as `romacs-agent-store-row-session' makes it, and stays unloaded."
  (when-let* ((session (romacs-agent-get id t)))
    (let ((options (romacs-agent-options session)))
      (romacs-agent-summary--plain
       (romacs-agent-summary--make
        :id (romacs-agent-id session) :dir (romacs-agent-dir session)
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
        :activity (romacs-agent-doing session) :turn-start (romacs-agent-turn-start session)
        :children-summary (romacs-agent-children-summary session)
        :children-rows (mapcar #'romacs-agent-summary--worker (romacs-agent-children-rows session))
        :pending-selection (romacs-agent--without (romacs-agent-pending-selection session) '(:timer))
        :waiting-count (hash-table-count (romacs-agent-approvals session))
        :queue-length (romacs-agent-queue-length session)
        :attached (romacs-agent--attached-p id))))))

(provide 'romacs-agent-summary)
;;; romacs-agent-summary.el ends here

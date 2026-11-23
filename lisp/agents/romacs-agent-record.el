;;; romacs-agent-record.el --- Decode the agent journal's data -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The supervisor's journal format and Romacs's notes.  This library
;; translates records to values independently of the running session
;; and its transport.

;;; Code:

(defun romacs-agent--value (value)
  "VALUE, with JSON null and false as nil."
  (unless (memq value '(:null :false)) value))

(defun romacs-agent--message-of (object)
  "The message (`romacs-agent-make-message') a journal record's OBJECT keeps."
  (let ((from (gethash "from" object)) (wake (gethash "wake" object)))
    (append (list :id (gethash "id" object) :kind (gethash "kind" object)
                  :from (and (hash-table-p from)
                             (list :session (romacs-agent--value (gethash "session" from))
                                   :name (romacs-agent--value (gethash "name" from))
                                   :address (romacs-agent--value (gethash "address" from))
                                   :claude-session (romacs-agent--value (gethash "claudeSession" from))
                                   :by-user (eq (gethash "byUser" from) t)))
                  :in-reply-to (romacs-agent--value (gethash "inReplyTo" object))
                  :text (romacs-agent--value (gethash "text" object))
                  :time (romacs-agent--value (gethash "t" object)))
            (and (hash-table-p wake)
                 (list :wake (append (list :state (gethash "state" wake) :chars (gethash "chars" wake))
                                     (and (stringp (gethash "to" wake)) (list :to (gethash "to" wake)))))))))

(defun romacs-agent--messages-of (vector)
  "The messages a journal record's VECTOR (its romacsMessages) keeps."
  (and (vectorp vector) (mapcar #'romacs-agent--message-of (append vector nil))))

(defun romacs-agent--selection-of (object)
  "OBJECT, a journal's or manifest's selection, as a plist."
  (and (hash-table-p object)
       (list :model (romacs-agent--value (gethash "model" object))
             :effort (romacs-agent--value (gethash "effort" object))
             :provider (romacs-agent--value (gethash "provider" object)))))

(defun romacs-agent--attachments-of (items)
  "ITEMS, an attachments note's (romacs-agent-attachments.el), as an
event carries them: plists of :path :kind (image, file or capture) :alt
:size :sha."
  (mapcar (lambda (item)
            (list :path (gethash "path" item) :kind (intern (gethash "kind" item))
                  :alt (romacs-agent--value (gethash "alt" item)) :size (romacs-agent--value (gethash "size" item))
                  :sha (romacs-agent--value (gethash "sha" item))))
          (append items nil)))

(defun romacs-agent--record-request (record)
  "The request id of RECORD, a user's message written, or nil."
  (let ((in (gethash "in" record)))
    (and (hash-table-p in) (gethash "romacsRequest" in))))

(defun romacs-agent--note-p (record type)
  "Whether RECORD is a journal note of TYPE."
  (let ((note (gethash "note" record)))
    (and (hash-table-p note) (equal (gethash "type" note) type))))

(defun romacs-agent--rollback-note-p (record)
  (romacs-agent--note-p record "rollback"))

(defun romacs-agent--record-cursor (record)
  "Driver state fields that RECORD changes about the conversation cursor."
  (cond ((romacs-agent--rollback-note-p record)
         (let ((at (gethash "at" (gethash "note" record) 'unknown)))
           (list :rolled-back t :resume-at (unless (eq at :null) at))))
        ((romacs-agent--note-p record "renew")
         (let ((prompt (gethash "continue" (gethash "note" record))))
           (list :rolled-back t :resume-at nil :renew-continuation (and (stringp prompt) prompt))))
        ((romacs-agent--record-request record)
         (if (eq (gethash "romacsBrief" (gethash "in" record)) t)
             '(:rolled-back nil :renew-continuation nil)
           '(:rolled-back nil)))))

(defun romacs-agent--record-prompt (input kept)
  "Input event for a per-turn INPUT, recovering KEPT text for its prompt."
  (let* ((prompt (gethash "prompt" input))
         (incoming (and kept (equal (plist-get kept :prompt) (secure-hash 'sha1 prompt)) kept)))
    (append (list :type 'input :text (if incoming (plist-get incoming :text) prompt)
                  :images (append (gethash "images" input) nil))
            (and incoming (list :messages (plist-get incoming :messages))))))

(defun romacs-agent--record-input-context (input steer)
  "The decoded context for INPUT's request or messages, when present.
STEER is non-nil when the journal already records a turn under way."
  (let ((request (gethash "romacsRequest" input))
        (messages (romacs-agent--messages-of (gethash "romacsMessages" input)))
        (own (romacs-agent--value (gethash "romacsOwn" input))))
    (when (or request messages)
      (append (and request (list :request request))
              (and messages (list :messages messages))
              (and own (list :own own))
              (and steer (list :steer t))))))

(defun romacs-agent--annotate-input-events (events context)
  "Add journal CONTEXT to each input event in EVENTS."
  (if context
      (mapcar (lambda (event)
                (if (eq (plist-get event :type) 'input) (append event context) event))
              events)
    events))

(defun romacs-agent--record-answer (note)
  "Attribution event for the approval answer in NOTE."
  (let ((by (or (romacs-agent--value (gethash "by" note)) "unknown"))
        (answers (gethash "answers" note)))
    ;; Protocol 1.0 clients may read only :by.
    (list :type 'attributed :request-id (gethash "request" note) :by by
          :actor (list :kind by :login (romacs-agent--value (gethash "login" note))
                       :client (romacs-agent--value (gethash "client" note)))
          :decision (gethash "decision" note) :scope (gethash "scope" note)
          :reason (romacs-agent--value (gethash "reason" note))
          :answers (and (hash-table-p answers) (> (hash-table-count answers) 0) answers))))

(defun romacs-agent--record-resume-event (note)
  "Decode the automatic resume status in NOTE."
  (pcase (gethash "state" note)
    ("scheduled" (list :type 'record-status :kind 'resume-scheduled :at (gethash "at" note)))
    ("fired" (list :type 'record-status :kind 'resume-fired :on (gethash "on" note)))
    (_ (list :type 'record-status :kind 'resume-cancelled :why (gethash "why" note)))))

(defun romacs-agent--record-note-event (note)
  "Translate NOTE into its event when it names one."
  (pcase (gethash "type" note)
    ("run" (list :type 'run))
    ("answer" (romacs-agent--record-answer note))
    ("selection" (list :type 'selection
                       :from (romacs-agent--selection-of (gethash "from" note))
                       :to (romacs-agent--selection-of (gethash "to" note))))
    ("replied" (list :type 'replied :message (gethash "message" note)
                     :reply (gethash "reply" note) :by-user (eq (gethash "byUser" note) t)))
    ("messages" (list :type 'incoming :text (gethash "text" note) :prompt (gethash "prompt" note)
                      :messages (romacs-agent--messages-of (gethash "messages" note))))
    ("renew" (list :type 'renewed :why (romacs-agent--value (gethash "why" note))
                   :from (romacs-agent--value (gethash "from" note))
                   :to (romacs-agent--value (gethash "to" note)) :brief (gethash "brief" note)))
    ("attachments" (list :type 'attachments :id (gethash "id" note)
                         :attachments (romacs-agent--attachments-of (gethash "items" note))
                         :compare (append (romacs-agent--value (gethash "compare" note)) nil)))
    ("handoff" (list :type 'record-status :kind 'handoff :from (gethash "from" note)
                     :to (gethash "to" note) :why (gethash "why" note) :resets (gethash "resets" note)))
    ("resume" (romacs-agent--record-resume-event note))
    ("fork" (list :type 'forked :from (gethash "from" note) :title (gethash "title" note)
                  :seq-from (gethash "seq" note) :turns (gethash "turns" note) :after (gethash "after" note)))
    ("rollback" (list :type 'rolled-back :to (gethash "to" note) :turns (gethash "turns" note)
                      :prompt (gethash "prompt" note) :files (eq (gethash "files" note) t)))))

(defun romacs-agent--record-note-events (note)
  "Translate the journal NOTE to events in order."
  (if (equal (gethash "type" note) "plan")
      (let ((on (eq (gethash "on" note) t)))
        (list (list :type 'plan-switch :on on)
              (list :type 'status :text (if on "plan mode" "plan mode off"))))
    (when-let* ((event (romacs-agent--record-note-event note)))
      (list event))))

(provide 'romacs-agent-record)
;;; romacs-agent-record.el ends here

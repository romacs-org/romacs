;;; romacs-agent-wire.el --- A session's events as the agent protocol sends them  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The agent session protocol (doc/agent-protocol.org) sends a session's
;; events as JSON.  A driver raises an event as a plist
;; (`romacs-agent--raise'); this is what of it the protocol sends:
;; `romacs-agent-wire-events' names each event type sent and its
;; fields, and is the schema clients read events by.  Every field named
;; is present, null when the event has none; a field not named, and an
;; event of a type not named, is the editor's own and is not sent.
;; A client reads an event back as it was raised with
;; `romacs-agent-wire-decode'.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defconst romacs-agent-wire-events
  '((input text (images list) steer own (messages list) request)
    (incoming text prompt (messages list))
    (delta message text parent)
    (thinking text parent)
    (message-end message parent)
    (tool id name (state symbol) input output parent)
    (tool-output id text)
    (task id task (state symbol) background activity summary title)
    (status text parent)
    (error text parent)
    (done text error (limit symbol) interrupted duration cost reason)
    (exit status)
    (compacted text)
    (usage context-used)
    (plan text)
    (selection (from list) (to list))
    (renewed why from to brief)
    (forked from title seq-from turns after)
    (rolled-back to turns prompt files)
    (replied message reply by-user)
    (unknown raw method)
    (approval (request request) parent policy)
    (answered (request-id handle) behavior answers)
    (attributed (request-id handle) by decision scope reason answers))
  "Each event type the protocol sends, then its fields.  A field's JSON
name is its keyword's, with `_' for `-'.  A field is a symbol, or (FIELD
KIND) for one a client reads back other than as JSON parses it
\(`romacs-agent-wire-decode'): `symbol', a name; `list', Lisp data,
its objects plists and its arrays lists; `request', the request an
approval asks, made of the event (`romacs-agent-wire-request'); `handle',
the request's handle, its JSON name \"handle\" (`romacs-agent-wire-handle').
An approval, an answer and who gave it are raised with the :run they were
asked in.")

(defun romacs-agent-wire--field (field)
  "FIELD of `romacs-agent-wire-events' as (NAME . KIND), KIND nil for JSON."
  (if (consp field) (cons (car field) (cadr field)) (list field)))

(defun romacs-agent-wire--plist-p (value)
  "Whether VALUE is a plist: keywords, each with its value."
  (and (proper-list-p value) value (cl-evenp (length value))
       (cl-loop for key in value by #'cddr always (keywordp key))))

(defun romacs-agent-wire-value (value)
  "VALUE as JSON: nil null, t true, a symbol its name, a plist an
object, another list an array; numbers, strings, and a harness's JSON
as parsed (vectors, hash tables) as they are."
  (cond ((null value) :null)
        ((memq value '(t :null :false)) value)
        ((keywordp value) (substring (symbol-name value) 1))
        ((symbolp value) (symbol-name value))
        ((or (numberp value) (stringp value) (vectorp value) (hash-table-p value)) value)
        ((romacs-agent-wire--plist-p value)
         (let ((object (make-hash-table :test #'equal)))
           (cl-loop for (key item) on value by #'cddr
                    do (puthash (romacs-agent-wire--name key) (romacs-agent-wire-value item) object))
           object))
        ((proper-list-p value) (vconcat (mapcar #'romacs-agent-wire-value value)))
        (t (signal 'wrong-type-argument (list 'romacs-agent-wire-value value)))))

(defun romacs-agent-wire--name (key)
  "The JSON name of the plist KEY, a keyword or a symbol."
  (string-replace "-" "_" (string-remove-prefix ":" (symbol-name key))))

(defun romacs-agent-wire-event (event)
  "EVENT, as raised, as the protocol sends it: an object of its type, its
record's seq and time, then its fields (`romacs-agent-wire-events');
nil when its type is not sent."
  (when-let* ((fields (cdr (assq (plist-get event :type) romacs-agent-wire-events))))
    (let ((object (make-hash-table :test #'equal)))
      (puthash "type" (symbol-name (plist-get event :type)) object)
      (puthash "seq" (romacs-agent-wire-value (plist-get event :seq)) object)
      (puthash "time" (romacs-agent-wire-value (plist-get event :time)) object)
      (dolist (field fields)
        (pcase-let ((`(,name . ,kind) (romacs-agent-wire--field field)))
          (pcase kind
            ('request (puthash "request" (romacs-agent-wire-request event) object))
            ('handle (puthash "handle" (romacs-agent-wire-handle (plist-get event :run) (plist-get event :request-id)) object))
            (_ (puthash (romacs-agent-wire--name name)
                        (romacs-agent-wire-value (plist-get event (intern (concat ":" (symbol-name name)))))
                        object)))))
      object)))

(defun romacs-agent-wire-handle (run id)
  "The handle of request ID, its harness's, asked in the run whose run
note is at seq RUN: its run and a short id made of ID alone, so history
and live give a request the same."
  (let ((handle (make-hash-table :test #'equal)))
    (puthash "run" (or run 0) handle)
    (puthash "id" (substring (secure-hash 'sha1 (format "%s" id)) 0 12) handle)
    handle))

(defun romacs-agent-wire-request (event)
  "The request approval EVENT asks (Protocol v1, Pending requests)."
  (let* ((questions (plist-get event :questions)) (plan (plist-get event :plan)) (input (plist-get event :input))
         (request (make-hash-table :test #'equal)))
    (cl-flet ((put (key value) (puthash key value request)))
      (put "handle" (romacs-agent-wire-handle (plist-get event :run) (plist-get event :request-id)))
      (put "kind" (cond (questions "question") (plan "plan") (t "tool")))
      (put "since" (romacs-agent-wire-value (plist-get event :time)))
      (put "origin" :null)
      (put "tool" (romacs-agent-wire-value (list :name (plist-get event :tool) :input input
                                                 :reason (plist-get event :description))))
      (put "questions"
           (if (null questions) :null
             (vconcat (cl-loop for (text . props) in questions for q from 1
                               collect (romacs-agent-wire-value
                                        (list :id (format "q%d" q) :text text :header (plist-get props :header)
                                              :select (if (plist-get props :multi) "many" "one") :free-text t
                                              :options (cl-loop for label in (plist-get props :options) for o from 1
                                                                collect (list :id (format "o%d" o) :label label
                                                                              :description (cdr (assoc label (plist-get props :notes)))))))))))
      (put "plan" (if (and plan (hash-table-p input) (stringp (gethash "plan" input))) (gethash "plan" input) :null))
      (put "choices" (if questions []
                       (romacs-agent-wire-value '((:decision "allow" :scope "once") (:decision "allow" :scope "session")
                                                  (:decision "deny" :scope "once"))))))
    request))

(defun romacs-agent-wire--asked (request)
  "What of an approval event REQUEST (`romacs-agent-wire-request') holds."
  (let* ((get (lambda (object key) (and (hash-table-p object) (romacs-agent-wire--decoded (gethash key object :null) nil))))
         (handle (funcall get request "handle")) (tool (funcall get request "tool"))
         (kind (funcall get request "kind")) (questions (funcall get request "questions")))
    (list :request-id (funcall get handle "id") :run (funcall get handle "run")
          :tool (funcall get tool "name") :input (funcall get tool "input") :description (funcall get tool "reason")
          :questions (and (vectorp questions)
                          (cl-loop for q across questions
                                   for options = (append (funcall get q "options") nil)
                                   collect (list (funcall get q "text")
                                                 :header (funcall get q "header")
                                                 :options (mapcar (lambda (o) (funcall get o "label")) options)
                                                 :notes (cl-loop for o in options
                                                                 when (funcall get o "description")
                                                                 collect (cons (funcall get o "label") (funcall get o "description")))
                                                 :multi (equal (funcall get q "select") "many"))))
          :plan (equal kind "plan")
          :user (and (member kind '("question" "plan")) t))))

(defun romacs-agent-wire-unit (events)
  "The EVENTS of one record unit, as raised, as `agents/event' sends them:
a vector of objects, those of a type not sent left out."
  (vconcat (delq nil (mapcar #'romacs-agent-wire-event events))))

(defun romacs-agent-wire--lisp (value)
  "JSON VALUE as Lisp data: objects plists, arrays lists, null and false nil."
  (cond ((memq value '(:null :false)) nil)
        ((hash-table-p value)
         (let ((plist nil))
           (maphash (lambda (key item)
                      (push (intern (concat ":" (string-replace "_" "-" key))) plist)
                      (push (romacs-agent-wire--lisp item) plist))
                    value)
           (nreverse plist)))
        ((vectorp value) (mapcar #'romacs-agent-wire--lisp value))
        (t value)))

(defun romacs-agent-wire--decoded (value kind)
  "VALUE, a field of KIND (`romacs-agent-wire-events') as JSON parses it
\(null and false `:null' and `:false'), as the event raised held it."
  (cond ((memq value '(:null :false)) nil)
        ((eq kind 'symbol) (if (stringp value) (intern value) value))
        ((eq kind 'list) (romacs-agent-wire--lisp value))
        (t value)))

(defun romacs-agent-wire-decode (object)
  "The event, as raised, that OBJECT, an event the protocol sent, is: a
plist of its type, :seq, :time and its fields; nil when its type is none
`romacs-agent-wire-events' names.  A field it does not name is passed
over, and one missing is nil."
  (when-let* ((type (and (stringp (gethash "type" object)) (intern-soft (gethash "type" object))))
              (fields (cdr (assq type romacs-agent-wire-events))))
    (append (list :type type
                  :seq (romacs-agent-wire--decoded (gethash "seq" object) nil)
                  :time (romacs-agent-wire--decoded (gethash "time" object) nil))
            (mapcan (lambda (field)
                      (pcase-let ((`(,name . ,kind) (romacs-agent-wire--field field)))
                        (pcase kind
                          ('request (romacs-agent-wire--asked (gethash "request" object)))
                          ('handle (let ((handle (gethash "handle" object)))
                                     (list :request-id (and (hash-table-p handle) (gethash "id" handle))
                                           :run (and (hash-table-p handle) (gethash "run" handle)))))
                          (_ (list (intern (concat ":" (symbol-name name)))
                                   (romacs-agent-wire--decoded (gethash (romacs-agent-wire--name name) object :null) kind))))))
                    fields))))

(defun romacs-agent-wire-decode-unit (events)
  "EVENTS, the vector of one record unit `agents/event' sent, as the
events raised for it: those of a type not known left out, the last
marked :last (`romacs-agent--record')."
  (let ((decoded (delq nil (mapcar #'romacs-agent-wire-decode events))))
    (mapcar (lambda (event) (append event (list :last (eq event (car (last decoded)))))) decoded)))

(provide 'romacs-agent-wire)
;;; romacs-agent-wire.el ends here

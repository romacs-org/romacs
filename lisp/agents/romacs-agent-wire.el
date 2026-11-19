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

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defconst romacs-agent-wire-events
  '((input text images steer own messages request)
    (delta message text parent)
    (thinking text parent)
    (message-end message parent)
    (tool id name state input output parent)
    (status text parent)
    (error text parent)
    (done text error limit duration cost reason)
    (exit status)
    (compacted text)
    (usage context-used)
    (plan text))
  "Each event type the protocol sends, then its fields.  A field's JSON
name is its keyword's, with `_' for `-'.")

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
        (puthash (romacs-agent-wire--name field)
                 (romacs-agent-wire-value (plist-get event (intern (concat ":" (symbol-name field)))))
                 object))
      object)))

(defun romacs-agent-wire-unit (events)
  "The EVENTS of one record unit, as raised, as `agents/event' sends them:
a vector of objects, those of a type not sent left out."
  (vconcat (delq nil (mapcar #'romacs-agent-wire-event events))))

(provide 'romacs-agent-wire)
;;; romacs-agent-wire.el ends here

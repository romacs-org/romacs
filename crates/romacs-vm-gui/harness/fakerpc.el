;;; fakerpc.el --- A test-only harness driver: persistent JSON-RPC  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The driver of fake_rpc.py (agent_live.py `rpc'), loaded by the test
;; alone: a harness Romacs learns of by its driver and nothing else.  It
;; starts up by `initialize' then `session/new' (`session/load' going on
;; with a conversation), each step waiting for the last one's answer, as
;; ACP's agents and Codex's app-server do.  `fakerpc-program' names the
;; harness's command line.

;;; Code:

(require 'romacs-agent)

(defvar fakerpc-program '("python3" "fake_rpc.py")
  "fake_rpc.py's command line.")

(defvar fakerpc-arguments nil)

(defun fakerpc--argv (session _resume)
  ;; Whether it goes on with a conversation is its startup's to say.
  (append (romacs-agent--environment-words session) fakerpc-program))

(defun fakerpc--request (session method params &optional id)
  (romacs-mcp--object "jsonrpc" "2.0" "id" (or id (romacs-agent-request-id session)) "method" method "params" params))

(defun fakerpc--next (session)
  "The next startup request, from this run's state."
  (let ((state (romacs-agent-driver-state session)))
    (cond ((not (plist-get state :init-id)) (fakerpc--request session "initialize" (romacs-mcp--object)))
          ((not (plist-get state :initialized)) nil)
          ((not (plist-get state :session-id))
           (if (romacs-agent-harness-session session)
               (fakerpc--request session "session/load" (romacs-mcp--object "sessionId" (romacs-agent-harness-session session)))
             (fakerpc--request session "session/new" (romacs-mcp--object "cwd" (romacs-agent-cwd session)))))
          ((not (plist-get state :session-ready)) nil)
          (t :ready))))

(defun fakerpc--input (session text id)
  (fakerpc--request session "session/prompt"
                    (romacs-mcp--object "sessionId" (romacs-agent-harness-session session) "prompt" text)
                    id))

(defun fakerpc--input-events (_session object)
  "The events of a request written to the harness."
  (pcase (gethash "method" object)
    ("initialize" (list (list :type 'handshake :state (list :init-id (gethash "id" object)))))
    ((or "session/new" "session/load") (list (list :type 'handshake :state (list :session-id (gethash "id" object)))))
    ("session/prompt" (list (list :type 'input :text (gethash "prompt" (gethash "params" object)))))
    ('nil (list (list :type 'answered :request-id (gethash "id" object)
                      :behavior (gethash "outcome" (gethash "result" object)))))))

(defun fakerpc--events (session object)
  "The events of a line the harness wrote."
  (let ((state (romacs-agent-driver-state session))
        (id (gethash "id" object)) (method (gethash "method" object)))
    (cond
     ((and method id)
      ;; A request of its own: a permission to ask.
      (let ((params (gethash "params" object)))
        (puthash id params (romacs-agent-approvals session))
        (list (list :type 'approval :request-id id :tool (gethash "tool" params) :input (gethash "input" params)))))
     ((equal method "session/update")
      (list (list :type 'delta :text (gethash "text" (gethash "params" object)) :message "reply")))
     (method nil)
     ((gethash "error" object)
      (list (list :type 'error :text (gethash "message" (gethash "error" object)))))
     ((eql id (plist-get state :init-id)) (list (list :type 'handshake :state (list :initialized t))))
     ((eql id (plist-get state :session-id))
      (list (list :type 'handshake :state (list :session-ready t))
            (list :type 'session :harness-session (gethash "sessionId" (gethash "result" object)))))
     (t (list (list :type 'message-end :message "reply")
              (list :type 'done :text (gethash "text" (gethash "result" object))))))))

(defun fakerpc--respond (_session request-id allow _message)
  (romacs-mcp--object "jsonrpc" "2.0" "id" request-id
                      "result" (romacs-mcp--object "outcome" (if allow "allow" "deny"))))

;;; Its history: STATE/sessions/ID.jsonl

(defun fakerpc--files (dir _limit)
  (let ((sessions (expand-file-name "sessions" dir)))
    (and (file-directory-p sessions)
         (mapcar (lambda (entry)
                   (list (car entry) (file-attribute-size (cdr entry))
                         (float-time (file-attribute-modification-time (cdr entry)))))
                 (directory-files-and-attributes sessions t "\\.jsonl\\'" t)))))

(defun fakerpc--describe (objects path)
  (when-let* ((first (cl-find-if (lambda (o) (equal (gethash "type" o) "prompt")) objects)))
    (list :harness 'fakerpc :id (file-name-base path) :cwd (gethash "cwd" first)
          :title (gethash "text" first))))

(defun fakerpc--history-events (objects)
  (mapcan (lambda (o)
            (pcase (gethash "type" o)
              ("prompt" (list (list :type 'input :text (gethash "text" o))))
              ("reply" (list (list :type 'delta :text (gethash "text" o) :message "h")
                             (list :type 'message-end :message "h")
                             (list :type 'done :text (gethash "text" o))))))
          objects))

(romacs-agent-define-harness 'fakerpc
  :display "Fake RPC" :icon "◆"
  :config-variable "FAKERPC_HOME" :config-default "~/.fakerpc"
  :program-option 'fakerpc-program :arguments-option 'fakerpc-arguments
  :lifetime 'persistent
  :argv #'fakerpc--argv
  :next #'fakerpc--next
  :input #'fakerpc--input
  :interrupt (lambda (session)
               (romacs-mcp--object "jsonrpc" "2.0" "method" "session/cancel"
                                   "params" (romacs-mcp--object "sessionId" (romacs-agent-harness-session session))))
  :respond #'fakerpc--respond
  :events #'fakerpc--events
  :input-events #'fakerpc--input-events
  :native-id-p (lambda (id) (and (stringp id) (string-prefix-p "rpc-" id)))
  :history '(:files fakerpc--files :keep always :describe fakerpc--describe :events fakerpc--history-events))

(provide 'fakerpc)
;;; fakerpc.el ends here

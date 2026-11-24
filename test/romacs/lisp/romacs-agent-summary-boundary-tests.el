;;; romacs-agent-summary-boundary-tests.el --- A session summary is the session's own plain data -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A summary says what MCP says of a session with no chat open, and no
;; live object reaches it through a string's text properties.

;;; Code:

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(require 'romacs-agent-log)
(require 'romacs-agent-summary)

(defmacro romacs-agent-summary-boundary--with-session (session &rest body)
  "Run BODY with SESSION bound to a registered idle session, no chat open."
  (declare (indent 1))
  `(romacs-agent-test-with-store
     (let* ((,session (romacs-agent--make
                       :id "21111111-2222-4333-8444-555555555555" :harness 'claude :provider "work"
                       :dir (getenv "XDG_STATE_HOME") :cwd temporary-file-directory
                       :title "Session title" :model "claude-x" :state 'idle :last-time 501.25))
            (romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent--children (make-hash-table :test #'equal))
            (romacs-agent-log--chats (make-hash-table :test #'equal))
            (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
            (romacs-agent-ui--children-timer nil))
       (puthash (romacs-agent-id ,session) ,session romacs-agent--sessions)
       (unwind-protect (progn ,@body)
         (when romacs-agent-ui--children-timer
           (cancel-timer romacs-agent-ui--children-timer))))))

(defun romacs-agent-summary-boundary--approval (session id)
  "Raise a live tool-call request ID of SESSION, in its turn."
  (romacs-agent--raise session '(:type input :text "Begin" :seq 38 :time 505.0))
  (puthash id t (romacs-agent-approvals session))
  (romacs-agent--raise session (list :type 'approval :request-id id :tool "Edit" :time 506.0)))

(defun romacs-agent-summary-boundary--as-mcp (session)
  "SESSION's summary beside what MCP says of it: (SUMMARY STATE PENDING)."
  (let ((task (romacs-agent-tools--task (romacs-agent-summary (romacs-agent-id session)))))
    (list (romacs-agent-summary (romacs-agent-id session))
          (gethash "state" task) (eq (gethash "pendingApproval" task) t))))

(ert-deftest romacs-agent-summary-boundary-no-chat-approval ()
  (romacs-agent-summary-boundary--with-session session
    (let ((romacs-agent-approval-policy 'ask))
      (romacs-agent-summary-boundary--approval session "ask")
      (should-not (gethash (romacs-agent-id session) romacs-agent-log--chats))
      (pcase-let ((`(,summary ,state ,pending) (romacs-agent-summary-boundary--as-mcp session)))
        (should (equal state "waiting-approval"))
        (should pending)
        (should (= (romacs-agent-summary-waiting-count summary) 1))
        (should (eq (romacs-agent-summary-activity summary) 'waiting))))))

(ert-deftest romacs-agent-summary-boundary-no-chat-auto-answered ()
  (romacs-agent-summary-boundary--with-session session
    (let ((romacs-agent-approval-policy 'allow))
      (romacs-agent-summary-boundary--approval session "auto")
      (should-not (romacs-agent-asks session))
      (pcase-let ((`(,summary ,state ,pending) (romacs-agent-summary-boundary--as-mcp session)))
        (should (equal state "running"))
        (should pending)
        (should (= (romacs-agent-summary-waiting-count summary) 1))
        (should (eq (romacs-agent-summary-activity summary) 'working))))))

(defun romacs-agent-summary-boundary--live-p (object seen)
  "Whether OBJECT holds a live object anywhere, text properties too,
visiting each node once via SEEN."
  (or (processp object) (bufferp object) (markerp object) (timerp object)
      (windowp object) (hash-table-p object)
      (and (not (symbolp object)) (functionp object))
      (unless (gethash object seen)
        (puthash object t seen)
        (cond ((consp object)
               (or (romacs-agent-summary-boundary--live-p (car object) seen)
                   (romacs-agent-summary-boundary--live-p (cdr object) seen)))
              ((vectorp object)
               (cl-some (lambda (item) (romacs-agent-summary-boundary--live-p item seen)) object))
              ((stringp object)
               (cl-loop for position = 0 then (next-property-change position object)
                        while position
                        thereis (romacs-agent-summary-boundary--live-p
                                 (text-properties-at position object) seen)))))))

(defun romacs-agent-summary-boundary--check-plain (summary)
  "SUMMARY holds nothing live, and `prin1' then `read' gives it back."
  (should-not (romacs-agent-summary-boundary--live-p summary (make-hash-table :test #'eq)))
  (let ((print-level nil) (print-length nil))
    (should (equal (car (read-from-string (prin1-to-string summary))) summary))))

(ert-deftest romacs-agent-summary-boundary-glyph-closure ()
  (romacs-agent-summary-boundary--with-session session
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (copy-marker (point-min) t))
      (puthash (romacs-agent-id session) (current-buffer) romacs-agent-log--chats)
      (let* ((chat (current-buffer))
             (romacs-agent-approval-policy 'ask)
             (romacs-agent-glyph-alist
              `((asks . ,(propertize "!" 'help-echo (lambda (&rest _) (buffer-name chat))))))
             (event (list :type 'approval :request-id "ask" :tool "Edit")))
        (puthash "ask" t (romacs-agent-approvals session))
        (romacs-agent--raise session event)
        (romacs-agent-log-draw session event)
        (should (eq (car (romacs-agent-log--doing session)) 'waiting))
        (romacs-agent-summary-boundary--check-plain (romacs-agent-summary (romacs-agent-id session)))))))

(ert-deftest romacs-agent-summary-boundary-child-title-buffer ()
  (romacs-agent-summary-boundary--with-session session
    (with-temp-buffer
      (let ((child (romacs-agent--make :id "live-child" :harness 'codex :provider "codex"
                                       :title (propertize "Live child" 'owner (current-buffer))
                                       :state 'running :last-time 505.0
                                       :options (list :parent (romacs-agent-id session))))
            (romacs-agent-glyphs nil))
        (puthash (romacs-agent-id child) child romacs-agent--sessions)
        (romacs-agent--index child)
        (romacs-agent-ui--summarize-children session)
        (should (romacs-agent-children-rows session))
        (romacs-agent-summary-boundary--check-plain (romacs-agent-summary (romacs-agent-id session)))))))

(provide 'romacs-agent-summary-boundary-tests)
;;; romacs-agent-summary-boundary-tests.el ends here

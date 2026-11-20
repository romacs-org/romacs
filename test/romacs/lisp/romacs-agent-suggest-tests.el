;;; romacs-agent-suggest-tests.el --- The prompt a harness predicts, in the composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(eval-and-compile
  (let ((romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (require 'romacs-agent)
    (require 'romacs-agent-claude)
    (require 'romacs-agent-ui)
    (require 'completion-preview)))

(defmacro romacs-agent-suggest-tests--registry (&rest body)
  "BODY with the driver registry its own: what it defines goes with it."
  `(let ((romacs-agent--drivers (copy-alist romacs-agent--drivers))
         (romacs-agent--driver-table (make-hash-table :test #'eq)))
     (dolist (cell romacs-agent--drivers) (puthash (car cell) cell romacs-agent--driver-table))
     ,@body))

(defun romacs-agent-suggest-tests--line (object)
  (json-parse-string (json-serialize object) :object-type 'hash-table))

(defun romacs-agent-suggest-tests--feed (session seq key object)
  "Journal record SEQ of SESSION: OBJECT under KEY (\"in\" or \"out\")."
  (let ((record (make-hash-table :test #'equal)))
    (puthash "seq" seq record)
    (puthash key (romacs-agent-suggest-tests--line object) record)
    (romacs-agent--record session record)))

(defun romacs-agent-suggest-tests--suggestion (text)
  (romacs-mcp--object "type" "prompt_suggestion" "suggestion" text "uuid" "u" "session_id" "s"))

(defun romacs-agent-suggest-tests--result ()
  (romacs-mcp--object "type" "result" "subtype" "success" "is_error" :false "result" "done"))

(ert-deftest romacs-agent-suggest-claude-line-is-a-suggestion-event ()
  (let ((session (romacs-agent--make :id "s" :harness 'claude)))
    (should (equal (romacs-agent--claude-events
                    session (romacs-agent-suggest-tests--line (romacs-agent-suggest-tests--suggestion " run it to confirm\n")))
                   '((:type suggestion :text "run it to confirm"))))
    ;; Nothing predicted is no event.
    (dolist (empty '("" "  \n"))
      (should-not (romacs-agent--claude-events
                   session (romacs-agent-suggest-tests--line (romacs-agent-suggest-tests--suggestion empty)))))))

(ert-deftest romacs-agent-suggest-driver-asks-while-on ()
  (romacs-agent-suggest-tests--registry
   (romacs-agent-define-harness 'plaindrv :lifetime 'persistent)
   (let ((claude (romacs-agent--make :id "c" :harness 'claude))
         (plain (romacs-agent--make :id "p" :harness 'plaindrv)))
     (let ((romacs-agent-suggestions t))
       (should (equal (romacs-agent--suggestion-words claude) '("--prompt-suggestions")))
       ;; A driver that cannot ask asks nothing.
       (should-not (romacs-agent--suggestion-words plain)))
     (let ((romacs-agent-suggestions nil))
       (should-not (romacs-agent--suggestion-words claude))))))

(ert-deftest romacs-agent-suggest-belongs-to-the-gap-between-turns ()
  (let ((romacs-agent-event-functions nil)
        (session (romacs-agent--make :id "s" :harness 'claude :state 'running)))
    ;; Predicted mid-turn (the turn before's, late): not offered.
    (romacs-agent-suggest-tests--feed session 1 "out" (romacs-agent-suggest-tests--suggestion "stale"))
    (should-not (romacs-agent-suggestion session))
    (romacs-agent-suggest-tests--feed session 2 "out" (romacs-agent-suggest-tests--result))
    (should (eq (romacs-agent-state session) 'idle))
    (romacs-agent-suggest-tests--feed session 3 "out" (romacs-agent-suggest-tests--suggestion "run it to confirm"))
    (should (equal (romacs-agent-suggestion session) "run it to confirm"))
    ;; The next prompt starts a turn: the prediction is moot.
    (romacs-agent-suggest-tests--feed session 4 "in" (romacs-agent--claude-user session "something else" 1))
    (should (eq (romacs-agent-state session) 'running))
    (should-not (romacs-agent-suggestion session))
    ;; A run's end takes it with it.
    (romacs-agent-suggest-tests--feed session 5 "out" (romacs-agent-suggest-tests--result))
    (romacs-agent-suggest-tests--feed session 6 "out" (romacs-agent-suggest-tests--suggestion "again"))
    (should (equal (romacs-agent-suggestion session) "again"))
    (let ((record (make-hash-table :test #'equal)))
      (puthash "seq" 7 record)
      (puthash "exit" 0 record)
      (romacs-agent--record session record))
    (should-not (romacs-agent-suggestion session))))

(ert-deftest romacs-agent-suggest-replayed-while-reattaching-between-turns ()
  ;; A turn ended and was seen; the attachment was lost (the sentinel's
  ;; detached), and attaching again replays the prediction journaled after.
  (let ((romacs-agent-event-functions nil)
        (session (romacs-agent--make :id "s" :harness 'claude :state 'running)))
    (romacs-agent-suggest-tests--feed session 1 "in" (romacs-agent--claude-user session "go" 1))
    (romacs-agent-suggest-tests--feed session 2 "out" (romacs-agent-suggest-tests--result))
    (setf (romacs-agent-state session) 'detached)
    (romacs-agent-suggest-tests--feed session 3 "out" (romacs-agent-suggest-tests--suggestion "run it to confirm"))
    ;; Kept, though not offered while detached.
    (should (equal (plist-get (romacs-agent-driver-state session) :suggestion) "run it to confirm"))
    (should-not (romacs-agent-suggestion session))
    ;; The attachment's acknowledgement finds it idle: offered.
    (setf (romacs-agent-state session) 'idle)
    (should (equal (romacs-agent-suggestion session) "run it to confirm"))
    ;; Detached with a prompt journaled since the last turn's end: mid-turn,
    ;; a late prediction is the turn before's.
    (romacs-agent-suggest-tests--feed session 4 "in" (romacs-agent--claude-user session "more" 2))
    (setf (romacs-agent-state session) 'detached)
    (romacs-agent-suggest-tests--feed session 5 "out" (romacs-agent-suggest-tests--suggestion "stale"))
    (should-not (plist-get (romacs-agent-driver-state session) :suggestion))))

(defmacro romacs-agent-suggest-tests--chat (suggestion &rest body)
  "BODY in an empty composer of an idle session whose harness predicted
SUGGESTION (nil, none)."
  (declare (indent 1))
  `(with-temp-buffer
     (romacs-agent-chat-mode)
     (setq romacs-agent-ui--session (romacs-agent--make :id "s" :harness 'claude :state 'idle
                                                         :driver-state (list :suggestion ,suggestion)))
     (let ((inhibit-read-only t)) (insert "> earlier\nreply\n"))
     (setq romacs-agent-ui--end (point-marker))
     (setq romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
     (romacs-agent-ui--composer-update)
     ,@body))

(defun romacs-agent-suggest-tests--shown ()
  "What the composer's overlay draws after it, as text."
  (let ((after (overlay-get romacs-agent-ui--composer 'after-string)))
    (and after (substring-no-properties after))))

;;; It is a completion: TAB, M-TAB, C-M-i take it; typed, a preview shows the rest

(defun romacs-agent-suggest-tests--preview-after (text)
  "The completion preview after TEXT is typed in the empty composer, or nil."
  (completion-preview-active-mode -1)
  (delete-region romacs-agent-ui--end (point-max))
  (insert text)
  (let ((this-command (if (string-empty-p text) 'delete-backward-char 'self-insert-command)))
    (completion-preview--post-command))
  (and completion-preview-active-mode
       (substring-no-properties (overlay-get completion-preview--overlay 'after-string))))

(provide 'romacs-agent-suggest-tests)
;;; romacs-agent-suggest-tests.el ends here

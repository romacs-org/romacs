;;; romacs-agent-idle-tests.el --- An idle thread compacted while warm -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-idle))

(defmacro romacs-agent-idle-tests--with (fields &rest body)
  "BODY with `session', a live idle Claude thread of FIELDS whose last
request, at time 1000, wrote a cache that lives an hour; `sent', what it
is sent."
  (declare (indent 1))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-idle-compaction t)
          (session (romacs-agent--make ,@fields :id "i" :harness 'claude :state 'idle :caught-up t
                                       :context-used 300000 :done-seq 7
                                       :driver-state (list :ready t :initialized t)))
          (sent nil))
     (puthash "i" session romacs-agent--sessions)
     (romacs-agent--raise session '(:type cache :ttl 3600 :time 1000.0))
     (cl-letf (((symbol-function 'process-live-p) (lambda (_) t))
               ((symbol-function 'romacs-agent-send) (lambda (_s text &rest _) (push text sent))))
       ,@body)))

(ert-deftest romacs-agent-idle-compacts-in-the-last-two-minutes ()
  (romacs-agent-idle-tests--with ()
    (should-not (romacs-agent-idle-due-p session (+ 1000 3600 -121)))
    (should (romacs-agent-idle-due-p session (+ 1000 3600 -60)))
    ;; Expired: a compaction would pay the cold read it avoids.
    (should-not (romacs-agent-idle-due-p session (+ 1000 3600)))
    ;; Events after the turn (a model switched, a status) move nothing.
    (romacs-agent--raise session '(:type status :text "x" :time 3000.0))
    (should (romacs-agent-idle-due-p session (+ 1000 3600 -60)))
    (romacs-agent-idle--compact session)
    (should (equal sent '("/compact")))
    ;; The compaction's own turn: never compacted again.
    (romacs-agent--raise session '(:type input :text " /COMPACT " :seq 8))
    (romacs-agent--raise session '(:type done :seq 9))
    (romacs-agent--raise session '(:type cache :ttl 3600 :time 1100.0))
    (should-not (romacs-agent-idle-due-p session (+ 1100 3600 -60)))
    ;; With instructions, and steered meanwhile, still a compaction.
    (romacs-agent--raise session '(:type input :text "/compact keep the plan" :seq 20))
    (romacs-agent--raise session '(:type input :text "and the tests" :seq 21))
    (should (plist-get (romacs-agent-driver-state session) :compact-turn))
    (romacs-agent--raise session '(:type done :seq 22))
    (should-not (romacs-agent-idle-due-p session (+ 1100 3600 -60)))
    ;; A real turn after it starts a new spell.
    (romacs-agent--raise session '(:type input :text "go on" :seq 10))
    (romacs-agent--raise session '(:type done :seq 11))
    (should (romacs-agent-idle-due-p session (+ 1100 3600 -60)))))

(ert-deftest romacs-agent-idle-an-unknown-expiry-is-not-guessed ()
  (romacs-agent-idle-tests--with ()
    ;; A request served wholly from the cache reports no life.
    (romacs-agent--raise session '(:type cache :ttl nil :time 1200.0))
    (should-not (romacs-agent-idle-due-p session (+ 1200 3600 -60)))
    ;; Five minutes on an API key: due then.
    (romacs-agent--raise session '(:type cache :ttl 300 :time 1300.0))
    (should (romacs-agent-idle-due-p session (+ 1300 300 -60)))))

(ert-deftest romacs-agent-idle-what-is-not-compacted ()
  (let ((at (+ 1000 3600 -60)))
    (romacs-agent-idle-tests--with (:context-used 150000)
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with (:options '(:parent "p"))
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with (:harness 'codex)
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with (:state 'running)
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with ()
      (puthash "a" t (romacs-agent-approvals session))
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with ()
      (setf (romacs-agent-subagents session) (make-hash-table :test #'equal))
      (puthash "t" '(:state "running") (romacs-agent-subagents session))
      (should-not (romacs-agent-idle-due-p session at)))
    (romacs-agent-idle-tests--with ()
      (let ((romacs-agent-idle-compaction nil)) (should-not (romacs-agent-idle-due-p session at))))))

(ert-deftest romacs-agent-idle-claude-reports-its-cache-life ()
  (let ((usage (lambda (h1 m5) (romacs-mcp--object "cache_creation" (romacs-mcp--object "ephemeral_1h_input_tokens" h1
                                                                                         "ephemeral_5m_input_tokens" m5)))))
    (should (= (romacs-agent--claude-cache-ttl (funcall usage 933 0)) 3600))
    (should (= (romacs-agent--claude-cache-ttl (funcall usage 0 40)) 300))
    (should-not (romacs-agent--claude-cache-ttl (funcall usage 0 0)))
    (should-not (romacs-agent--claude-cache-ttl (romacs-mcp--object)))
    (should-not (romacs-agent--claude-cache-ttl (romacs-mcp--object "cache_creation" (romacs-mcp--object "ephemeral_1h_input_tokens" :null))))
    ;; A top-level request's start says it; a subagent's does not.
    (let ((session (romacs-agent--make :id "c" :harness 'claude :tools (make-hash-table :test #'equal)
                                       :streamed (make-hash-table :test #'equal)))
          (start (lambda (parent)
                   (romacs-mcp--object "type" "stream_event" "parent_tool_use_id" (or parent :null)
                                       "event" (romacs-mcp--object "type" "message_start"
                                                                   "message" (romacs-mcp--object "id" "m" "usage" (funcall usage 5 0)))))))
      (should (equal (romacs-agent--claude-events session (funcall start nil)) '((:type cache :ttl 3600))))
      (should-not (romacs-agent--claude-events session (funcall start "t1"))))))

(provide 'romacs-agent-idle-tests)
;;; romacs-agent-idle-tests.el ends here

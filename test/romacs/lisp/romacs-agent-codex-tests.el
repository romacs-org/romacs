;;; romacs-agent-codex-tests.el --- Codex's app-server driver -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defvar romacs-agent-codex-tests--written nil
  "What the session wrote, newest first: (OBJECT . JOURNAL).")

(defun romacs-agent-codex-tests--json (object)
  "OBJECT as the journal gives it back: parsed from its JSON."
  (json-parse-string (romacs-agent--json object) :object-type 'hash-table :null-object :null :false-object :false))

(defmacro romacs-agent-codex-tests--with (fields &rest body)
  "BODY with `session', a live Codex session of FIELDS, its writes in
`romacs-agent-codex-tests--written' and journaled back by `echo'; `out'
journals what its harness writes."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-codex-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-codex-tests--written nil)
          (seq 0)
          (session (romacs-agent--make :id "c" :harness 'codex :provider "codex" :cwd "/work" :dir dir
                                       :title "t" :state 'starting :caught-up t ,@fields)))
     (puthash "c" session romacs-agent--sessions)
     (unwind-protect
         (cl-letf (((symbol-function 'romacs-agent--line)
                    (lambda (_session object &optional journal) (push (cons object journal) romacs-agent-codex-tests--written)))
                   ((symbol-function 'process-live-p) (lambda (_) t)))
           (cl-flet* ((feed (key object)
                        (let ((record (make-hash-table :test #'equal)))
                          (puthash "seq" (cl-incf seq) record)
                          (puthash key (romacs-agent-codex-tests--json object) record)
                          (romacs-agent--record session record)))
                      (last-written () (caar romacs-agent-codex-tests--written))
                      (echo () (let ((entry (car romacs-agent-codex-tests--written)))
                                 (feed "in" (or (cdr entry) (car entry)))))
                      (out (&rest fields) (feed "out" (apply #'romacs-mcp--object fields)))
                      (notify (method &rest params)
                        (out "method" method "params" (apply #'romacs-mcp--object params))))
             (ignore #'last-written #'echo #'out #'notify)
             ,@body))
       (delete-directory dir t))))

(defun romacs-agent-codex-tests--handshake (session last-written echo out)
  "Take SESSION through its startup, as the harness answers it."
  (romacs-agent--step session)
  (let ((initialize (funcall last-written)))
    (should (equal (gethash "method" initialize) "initialize"))
    (funcall echo)
    ;; Nothing more until it is answered.
    (should (eq (funcall last-written) initialize))
    (funcall out "id" (gethash "id" initialize) "result" (romacs-mcp--object "userAgent" "codex")))
  (should (equal (gethash "method" (funcall last-written)) "initialized"))
  (should-not (gethash "id" (funcall last-written)))
  (funcall echo)
  (funcall last-written))

(ert-deftest romacs-agent-codex-starts-a-thread ()
  (romacs-agent-codex-tests--with (:options '(:model "m"))
    (let ((start (romacs-agent-codex-tests--handshake session #'last-written #'echo #'out)))
      (should (equal (gethash "method" start) "thread/start"))
      (should (equal (gethash "developerInstructions" (gethash "params" start)) (romacs-agent--instructions session)))
      (should (equal (gethash "cwd" (gethash "params" start)) "/work"))
      (should (equal (gethash "model" (gethash "params" start)) "m"))
      (echo)
      (should-not (romacs-agent--ready-p session))
      (out "id" (gethash "id" start)
           "result" (romacs-mcp--object "thread" (romacs-mcp--object "id" "01aa-thread") "model" "m"))
      (should (romacs-agent--ready-p session))
      (should (equal (romacs-agent-harness-session session) "01aa-thread"))
      (should (eq (romacs-agent-state session) 'idle))
      ;; One of each step.
      (should (= (length romacs-agent-codex-tests--written) 3)))))

(ert-deftest romacs-agent-codex-resumes-its-thread ()
  ;; A thread `codex exec' made goes on the same way.
  (romacs-agent-codex-tests--with (:harness-session "01bb-exec")
    (let ((resume (romacs-agent-codex-tests--handshake session #'last-written #'echo #'out)))
      (should (equal (gethash "method" resume) "thread/resume"))
      (should (equal (gethash "threadId" (gethash "params" resume)) "01bb-exec"))
      (should (equal (gethash "developerInstructions" (gethash "params" resume)) (romacs-agent--instructions session)))
      (should-not (gethash "model" (gethash "params" resume)))
      (echo)
      (out "id" (gethash "id" resume) "error" (romacs-mcp--object "code" -32600 "message" "no rollout"))
      (should-not (romacs-agent--ready-p session))
      (should (= (length romacs-agent-codex-tests--written) 3)))))

(ert-deftest romacs-agent-codex-resume-ends-a-cut-turn ()
  ;; Its last run ended mid-turn: Codex does not take that turn up, so its
  ;; resume ends it, and what waited goes.
  (romacs-agent-codex-tests--with (:harness-session "01dd" :interrupted t :input-seq 3)
    (romacs-agent--enqueue session (list "waited" nil nil))
    (let ((resume (romacs-agent-codex-tests--handshake session #'last-written #'echo #'out)))
      (echo)
      (out "id" (gethash "id" resume) "result" (romacs-mcp--object "thread" (romacs-mcp--object "id" "01dd"))))
    (should-not (romacs-agent-interrupted session))
    (should (romacs-agent-failed session))
    (should (eq (romacs-agent-state session) 'idle))
    ;; The queue goes at the turn's end, a moment later.
    (romacs-agent--settle session)
    (should (equal (gethash "text" (aref (gethash "input" (gethash "params" (last-written))) 0)) "waited"))))

(defun romacs-agent-codex-tests--ready (session)
  "SESSION, as its startup leaves it, in the run it is in."
  (setf (romacs-agent-harness-session session) "th"
        (romacs-agent-driver-state session) (append (list :sent "thread" :thread "th" :ready t)
                                                    (romacs-agent-driver-state session))
        (romacs-agent-state session) 'idle))

(ert-deftest romacs-agent-codex-turn-and-steer ()
  (romacs-agent-codex-tests--with (:options '(:model "m" :effort "high"))
    (romacs-agent-codex-tests--ready session)
    (let* ((image (make-temp-file "romacs-codex-" nil ".png" "png")))
      (unwind-protect
          (romacs-agent-send session "look" nil nil (list image))
        (delete-file image))
      (pcase-let* ((`(,sent . ,record) (car romacs-agent-codex-tests--written))
                   (params (gethash "params" sent)))
        (should (equal (gethash "method" sent) "turn/start"))
        (should (equal (gethash "summary" params) "detailed"))
        (should (equal (gethash "model" params) "m"))
        (should (equal (gethash "effort" params) "high"))
        (should (equal (mapcar (lambda (item) (gethash "type" item)) (gethash "input" params))
                       '("text" "text" "localImage")))
        (should (equal (gethash "text" (aref (gethash "input" params) 1))
                       (romacs-agent--image-caption image)))
        ;; The journal keeps the message with its files, and the id written.
        (should (equal (gethash "id" record) (gethash "id" sent)))
        (should (equal (append (gethash "images" record) nil) (list image)))
        (echo)
        (should (eq (romacs-agent-state session) 'running))))
    (notify "turn/started" "threadId" "th" "turn" (romacs-mcp--object "id" "t1"))
    ;; A message now goes into the running turn.
    (romacs-agent-send session "and this")
    (let ((steer (last-written)))
      (should (equal (gethash "method" steer) "turn/steer"))
      (should (equal (gethash "expectedTurnId" (gethash "params" steer)) "t1"))
      (echo)
      (out "id" (gethash "id" steer) "result" (romacs-mcp--object "turnId" "t1"))
      (should-not (plist-get (romacs-agent-driver-state session) :steers)))
    ;; An interrupt names the turn.
    (romacs-agent-interrupt session)
    (should (equal (gethash "method" (last-written)) "turn/interrupt"))
    (should (equal (gethash "turnId" (gethash "params" (last-written))) "t1"))
    (echo)
    (notify "turn/completed" "threadId" "th"
            "turn" (romacs-mcp--object "id" "t1" "status" "interrupted" "error" :null))
    (should (eq (romacs-agent-state session) 'idle))
    (should (romacs-agent-failed session))
    ;; No turn: nothing to interrupt.
    (should-error (romacs-agent-interrupt session) :type 'user-error)))

(ert-deftest romacs-agent-codex-late-start-acknowledgement ()
  ;; The turn ended before its turn/start was answered: it stays over, so
  ;; the next message starts a turn and there is nothing to interrupt.
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (romacs-agent-send session "quick")
    (let ((start (last-written)))
      (echo)
      (notify "turn/started" "threadId" "th" "turn" (romacs-mcp--object "id" "t1"))
      (notify "turn/completed" "threadId" "th" "turn" (romacs-mcp--object "id" "t1" "status" "completed"))
      (out "id" (gethash "id" start) "result" (romacs-mcp--object "turn" (romacs-mcp--object "id" "t1"))))
    (should-not (plist-get (romacs-agent-driver-state session) :turn))
    (should (eq (romacs-agent-state session) 'idle))
    (should-error (romacs-agent-interrupt session) :type 'user-error)
    (romacs-agent-send session "next")
    (should (equal (gethash "method" (last-written)) "turn/start"))))

(ert-deftest romacs-agent-codex-steer-that-missed-its-turn ()
  ;; The turn ended as the steer went: it is sent as a turn of its own.
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (romacs-agent-send session "first")
    (echo)
    (notify "turn/started" "threadId" "th" "turn" (romacs-mcp--object "id" "t1"))
    (romacs-agent-send session "late")
    (let ((steer (last-written)))
      (should (equal (gethash "method" steer) "turn/steer"))
      (echo)
      (notify "turn/completed" "threadId" "th" "turn" (romacs-mcp--object "id" "t1" "status" "completed"))
      (out "id" (gethash "id" steer) "error" (romacs-mcp--object "code" -32600 "message" "no active turn to steer"))
      (let ((retry (last-written)))
        (should (equal (gethash "method" retry) "turn/start"))
        (should (equal (gethash "text" (aref (gethash "input" (gethash "params" retry)) 0)) "late"))
        (should (equal (gethash "clientUserMessageId" (gethash "params" retry)) (gethash "id" steer)))
        (echo)
        (should-not (plist-get (romacs-agent-driver-state session) :owed))
        (should (eq (romacs-agent-state session) 'running))
        ;; Paid once.
        (out "method" "thread/status/changed" "params" (romacs-mcp--object))
        (should (eq (last-written) retry))))))

(defun romacs-agent-codex-tests--asked (events)
  "The request id of the last approval among EVENTS."
  (plist-get (cl-find 'approval events :key (lambda (e) (plist-get e :type))) :request-id))

(ert-deftest romacs-agent-codex-approvals-are-their-run-s ()
  ;; Codex numbers its requests from 0 every run: an earlier run's row is
  ;; gone with its harness, and names nothing of the next.
  (romacs-agent-codex-tests--with ()
    (let ((events nil) (first nil) (second nil))
      (let ((romacs-agent-event-functions (list (lambda (_session event) (push event events)))))
        (feed "note" (romacs-mcp--object "type" "run"))
        (romacs-agent-codex-tests--ready session)
        (out "id" 0 "method" "item/commandExecution/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "i" "command" "rm -r a"))
        (setq first (romacs-agent-codex-tests--asked events))
        (feed "exit" 1)
        (should (= (hash-table-count (romacs-agent-approvals session)) 0))
        (should-error (romacs-agent-respond session first t) :type 'user-error)
        (feed "note" (romacs-mcp--object "type" "run"))
        (romacs-agent-codex-tests--ready session)
        (setq events nil)
        (out "id" 0 "method" "item/commandExecution/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t2" "itemId" "j" "command" "rm -r b"))
        (setq second (romacs-agent-codex-tests--asked events))
        (should-not (equal first second))
        (should-error (romacs-agent-respond session first t) :type 'user-error)
        ;; Nothing answered: the run's own startup lines alone were written.
        (should-not (cl-find-if (lambda (w) (gethash "result" (car w))) romacs-agent-codex-tests--written))
        (romacs-agent-respond session second t)
        (should (eql (gethash "id" (last-written)) 0))
        (echo)
        (should (equal (plist-get (car events) :request-id) second))
        (should (= (hash-table-count (romacs-agent-approvals session)) 0))))))

(ert-deftest romacs-agent-codex-approvals ()
  ;; Answered by the user: the `ask' policy.
  (let ((romacs-agent-approval-policy 'ask))
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (let ((events nil))
      (let ((romacs-agent-event-functions (list (lambda (_session event) (push event events)))))
        (out "id" 0 "method" "item/commandExecution/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "i" "command" "rm -r x"
                                          "cwd" "/work" "reason" "cleanup"))
        (let ((approval (cl-find 'approval events :key (lambda (e) (plist-get e :type)))))
          (should (equal (plist-get approval :tool) "Bash"))
          (should (equal (gethash "command" (plist-get approval :input)) "rm -r x"))
          (should (equal (plist-get approval :description) "cleanup")))
        (should (eq (romacs-agent-state session) 'waiting-approval))
        ;; Any answer that allows accepts (a plist of questions' answers too).
        (romacs-agent-respond session (romacs-agent-codex-tests--asked events) '(:answers nil))
        (should (equal (gethash "decision" (gethash "result" (last-written))) "accept"))
        (should (eql (gethash "id" (last-written)) 0))
        (echo)
        (should (equal (plist-get (car events) :behavior) "allow"))
        (should (eq (romacs-agent-state session) 'running))
        ;; A file change, declined.
        (out "id" 1 "method" "item/fileChange/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "f"))
        (romacs-agent-respond session (romacs-agent-codex-tests--asked events) nil)
        (should (equal (gethash "decision" (gethash "result" (last-written))) "decline"))
        (echo)
        (should (equal (plist-get (car events) :behavior) "deny"))
        ;; Allowed for the rest of the session.
        (out "id" 4 "method" "item/commandExecution/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "k" "command" "make"))
        (romacs-agent-respond session (romacs-agent-codex-tests--asked events) '(:always t))
        (should (equal (gethash "decision" (gethash "result" (last-written))) "acceptForSession"))
        (echo)
        (should (equal (plist-get (car events) :behavior) "allow"))
        ;; Permissions: what was asked, or nothing.
        (out "id" 2 "method" "item/permissions/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "p" "cwd" "/work"
                                          "reason" :null
                                          "permissions" (romacs-mcp--object "network" (romacs-mcp--object "enabled" t))))
        (romacs-agent-respond session (romacs-agent-codex-tests--asked events) t)
        (let ((result (gethash "result" (last-written))))
          (should (equal (gethash "scope" result) "turn"))
          (should (eq (gethash "enabled" (gethash "network" (gethash "permissions" result))) t)))
        (echo)
        (should (equal (plist-get (car events) :behavior) "allow"))
        ;; One its turn's end settled.
        (out "id" 3 "method" "item/commandExecution/requestApproval"
             "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "j" "command" "ls"))
        (notify "serverRequest/resolved" "threadId" "th" "requestId" 3)
        (should (eq (plist-get (car events) :type) 'answered))
        (should (= (hash-table-count (romacs-agent-approvals session)) 0)))))))

(defun romacs-agent-codex-tests--question (id text &rest fields)
  (apply #'romacs-mcp--object "id" id "question" text "isOther" t "isSecret" :false fields))

(ert-deftest romacs-agent-codex-questions ()
  ;; Codex's questions are an approval's, by their text; the wire's by id.
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (let ((events nil) (romacs-agent-event-functions nil)
          (asked (romacs-mcp--object
                  "threadId" "th" "turnId" "t1" "itemId" "call" "isBlocking" t "autoResolutionMs" :null
                  "questions" (vector (romacs-agent-codex-tests--question
                                       "color" "Which color?" "header" "Color"
                                       "options" (vector (romacs-mcp--object "label" "Red" "description" "warm")
                                                         (romacs-mcp--object "label" "Blue" "description" "")))
                                      (romacs-agent-codex-tests--question
                                       "name" "What name?" "header" "Name" "options" :null)))))
      (add-hook 'romacs-agent-event-functions (lambda (_session event) (push event events)))
      (out "id" 5 "method" "item/tool/requestUserInput" "params" asked)
      (let ((approval (cl-find 'approval events :key (lambda (e) (plist-get e :type)))))
        (should (equal (plist-get approval :questions)
                       '(("Which color?" :header "Color" :options ("Red" "Blue") :notes (("Red" . "warm")) :multi nil)
                         ("What name?" :header "Name" :options nil :notes nil :multi nil))))
        (should (eq (romacs-agent-state session) 'waiting-approval))
        ;; An option by its label, other words as a note.
        (romacs-agent-respond session (plist-get approval :request-id)
                              '(:answers (("Which color?" . "Blue") ("What name?" . "Ada"))))
        (let ((answers (gethash "answers" (gethash "result" (last-written)))))
          (should (eql (gethash "id" (last-written)) 5))
          (should (equal (gethash "answers" (gethash "color" answers)) ["Blue"]))
          (should (equal (gethash "answers" (gethash "name" answers)) ["user_note: Ada"])))
        (echo)
        (should (equal (plist-get (car events) :behavior) "allow")))
      ;; Dismissed: no answers.
      (out "id" 6 "method" "item/tool/requestUserInput" "params" asked)
      (romacs-agent-respond session (romacs-agent-codex-tests--asked events) nil)
      (should (= (hash-table-count (gethash "answers" (gethash "result" (last-written)))) 0))
      (echo)
      (should (equal (plist-get (car events) :behavior) "deny"))
      ;; A secret is not asked where its answer is journaled.
      (setq events nil)
      (out "id" 7 "method" "item/tool/requestUserInput"
           "params" (romacs-mcp--object "threadId" "th" "turnId" "t1" "itemId" "c2"
                                        "questions" (vector (romacs-agent-codex-tests--question
                                                             "pw" "Password?" "isSecret" t "options" :null))))
      (should-not (cl-find 'approval events :key (lambda (e) (plist-get e :type))))
      (should (string-match-p "secret" (plist-get (cl-find 'status events :key (lambda (e) (plist-get e :type))) :text)))
      (should (eql (gethash "id" (last-written)) 7))
      (should (gethash "error" (last-written))))))

(ert-deftest romacs-agent-codex-refuses-other-requests ()
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (out "id" 7 "method" "mcpServer/elicitation/request"
         "params" (romacs-mcp--object "threadId" "th" "serverName" "x" "mode" "url" "url" "https://example.org"))
    (let ((refusal (last-written)))
      (should (eql (gethash "id" refusal) 7))
      (should (= (gethash "code" (gethash "error" refusal)) -32601))
      (echo)
      (should-not (plist-get (romacs-agent-driver-state session) :owed))
      (notify "thread/status/changed")
      (should (eq (last-written) refusal)))))

(ert-deftest romacs-agent-codex-replay-writes-nothing ()
  ;; A copy drawing the journal again owes nothing.
  (romacs-agent-codex-tests--with ()
    (romacs-agent-codex-tests--ready session)
    (remhash "c" romacs-agent--sessions)
    (out "id" 7 "method" "mcpServer/elicitation/request" "params" (romacs-mcp--object))
    (should-not romacs-agent-codex-tests--written)
    (should (plist-get (romacs-agent-driver-state session) :owed))))

(defun romacs-agent-codex-tests--events (session line)
  (romacs-agent--codex-events session (romacs-agent-codex-tests--json line)))

(ert-deftest romacs-agent-codex-notifications ()
  (let ((session (romacs-agent--make :id "c" :harness 'codex :driver-state (list :turn "t1"))))
    (cl-flet ((events (method &rest params)
                (romacs-agent-codex-tests--events
                 session (romacs-mcp--object "method" method "params" (apply #'romacs-mcp--object params))))
              (types (events) (mapcar (lambda (e) (plist-get e :type)) events)))
      (should (equal (events "item/agentMessage/delta" "itemId" "m1" "delta" "o")
                     '((:type delta :text "o" :message "m1"))))
      ;; Streamed: its end closes it, the text not repeated.
      (let ((end (events "item/completed" "item" (romacs-mcp--object "type" "agentMessage" "id" "m1" "text" "ok"))))
        (should (equal end '((:type delta :text "\n" :message "m1") (:type message-end :message "m1"))))
        (should (equal (romacs-agent-reply session) "ok")))
      (should (equal (events "item/reasoning/summaryTextDelta" "itemId" "r1" "delta" "hm" "summaryIndex" 0)
                     '((:type thinking :text "hm"))))
      (should-not (events "item/completed" "item" (romacs-mcp--object "type" "reasoning" "id" "r1"
                                                                      "summary" ["hm"] "content" [])))
      (should (equal (events "item/completed" "item" (romacs-mcp--object "type" "reasoning" "id" "r2"
                                                                         "summary" ["a" "b"] "content" []))
                     '((:type thinking :text "a\n\nb"))))
      (let ((command (romacs-mcp--object "type" "commandExecution" "id" "x" "command" "ls" "status" "inProgress"
                                         "aggregatedOutput" :null "exitCode" :null)))
        (let ((started (events "item/started" "item" command)))
          (should (equal (types started) '(tool)))
          (should (equal (plist-get (car started) :name) "Bash")))
        (puthash "status" "failed" command)
        (puthash "aggregatedOutput" "nope" command)
        (puthash "exitCode" 2 command)
        (let ((done (car (events "item/completed" "item" command))))
          (should (eq (plist-get done :state) 'failed))
          (should (equal (plist-get done :output) "nope"))))
      (let ((mcp (car (events "item/started" "item"
                              (romacs-mcp--object "type" "mcpToolCall" "id" "y" "server" "romacs" "tool" "romacs_eval"
                                                  "arguments" (romacs-mcp--object "form" "1") "status" "inProgress")))))
        (should (equal (plist-get mcp :name) "mcp__romacs__romacs_eval")))
      (should (string-match-p "Plan: ✓ read  → edit"
                              (plist-get (car (events "turn/plan/updated" "explanation" :null
                                                      "plan" (vector (romacs-mcp--object "step" "read" "status" "completed")
                                                                     (romacs-mcp--object "step" "edit" "status" "inProgress"))))
                                         :text)))
      (let ((usage (events "thread/tokenUsage/updated"
                           "tokenUsage" (romacs-mcp--object "last" (romacs-mcp--object "totalTokens" 1200)
                                                            "total" (romacs-mcp--object "totalTokens" 5000)
                                                            "modelContextWindow" 258000))))
        (should (equal (car usage) '(:type usage :context-used 1200)))
        (should (equal (cadr usage) '(:type handshake :state (:window 258000)))))
      (setf (romacs-agent-driver-state session) (list :turn "t1" :window 258000))
      (let ((end (events "turn/completed" "turn" (romacs-mcp--object "id" "t1" "status" "failed" "durationMs" 1500
                                                                     "error" (romacs-mcp--object "message" "boom")))))
        (should (equal (car end) '(:type handshake :state (:turn nil :ended ("t1")))))
        (should (equal (plist-get (cadr end) :reason) "boom"))
        (should (plist-get (cadr end) :error))
        (should (= (plist-get (cadr end) :duration) 1.5))
        (should (= (plist-get (cadr end) :context-window) 258000)))
      (should (equal (types (events "error" "willRetry" t "error" (romacs-mcp--object "message" "r"))) '(status)))
      (should (equal (types (events "error" "willRetry" :false "error" (romacs-mcp--object "message" "r"))) '(error)))
      (should-not (events "mcpServer/startupStatus/updated" "name" "x")))))

(ert-deftest romacs-agent-codex-reads-exec-journals ()
  ;; A session an earlier Romacs ran by `codex exec' replays as it did.
  (let ((session (romacs-agent--make :id "c" :harness 'codex :options '(:model "m"))))
    (cl-flet ((events (&rest fields) (romacs-agent-codex-tests--events session (apply #'romacs-mcp--object fields))))
      (should (equal (events "type" "thread.started" "thread_id" "01cc")
                     '((:type session :harness-session "01cc" :model "m"))))
      (should-not (events "type" "turn.started"))
      (let ((tool (events "type" "item.completed"
                          "item" (romacs-mcp--object "id" "item_1" "type" "command_execution" "command" "ls"
                                                     "aggregated_output" "a" "exit_code" 0 "status" "completed"))))
        (should (equal (mapcar (lambda (e) (plist-get e :state)) tool) '(started completed)))
        (should (equal (plist-get (cadr tool) :id) "0:item_1"))
        (should (equal (plist-get (cadr tool) :output) "a")))
      (should (equal (events "type" "item.completed"
                             "item" (romacs-mcp--object "id" "item_0" "type" "agent_message" "text" "hi"))
                     '((:type delta :text "hi\n" :message "0:item_0") (:type message-end :message "0:item_0"))))
      (should (equal (events "type" "turn.completed" "usage" (romacs-mcp--object))
                     '((:type done :text "hi")))))))

(ert-deftest romacs-agent-codex-select ()
  (romacs-agent-codex-tests--with (:options '(:model "a" :effort "low"))
    ;; Each turn names its model and effort: nothing to write.
    (should-not (romacs-agent--codex-select session "b" "high"))
    (should (eq (romacs-agent--codex-select session nil "low") 'restart))
    (should (eq (romacs-agent--codex-select session "a" nil) 'restart))
    (romacs-agent--manifest session nil)
    (romacs-agent-codex-tests--ready session)
    (cl-letf (((symbol-function 'romacs-agent--harness-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent--note) #'ignore))
      (should (romacs-agent-select session "b" "high")))
    (should-not (romacs-agent-pending-selection session))
    (should (equal (romacs-agent--selection session) '(:model "b" :effort "high" :provider "codex")))
    (romacs-agent-send session "go")
    (let ((params (gethash "params" (last-written))))
      (should (equal (gethash "model" params) "b"))
      (should (equal (gethash "effort" params) "high")))))

(provide 'romacs-agent-codex-tests)
;;; romacs-agent-codex-tests.el ends here

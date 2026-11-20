;;; romacs-agent-wire-tests.el --- A session's events as the agent protocol sends them -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The protocol's test vectors (doc/agent-protocol.org): a harness's
;; records, each with the events `agents/event' sends for it.  The
;; records are a turn of each harness as it records it, trimmed to what
;; the drivers read.  A driver change that changes what is sent changes
;; these vectors, and so the protocol: on purpose, in review.

;;; Code:

(require 'ert)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-claude)
(require 'romacs-agent-codex)
(require 'romacs-agent-log)
(require 'romacs-agent-wire)

(defconst romacs-agent-wire-tests--claude
  '(
    ("{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1795099335.0}"
     . "[]")
    ("{\"in\":{\"message\":{\"content\":\"Run the Bash command: echo remote-ok\",\"role\":\"user\"},\"romacsRequest\":4,\"type\":\"user\"},\"seq\":2,\"t\":1795099335.1}"
     . "[{\"type\":\"input\",\"seq\":2,\"time\":1795099335.1,\"text\":\"Run the Bash command: echo remote-ok\",\"images\":null,\"steer\":null,\"own\":null,\"messages\":null,\"request\":4}]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_A\",\"event\":{\"type\":\"message_start\",\"message\":{\"id\":\"msg_A\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":2,\"cache_creation_input_tokens\":69,\"cache_read_input_tokens\":31065,\"cache_creation\":{\"ephemeral_1h_input_tokens\":69,\"ephemeral_5m_input_tokens\":0}}}}},\"seq\":3,\"t\":1795099336.35}"
     . "[]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_A\",\"event\":{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{}}}},\"seq\":4,\"t\":1795099336.36}"
     . "[]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_A\",\"event\":{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\"}}},\"seq\":5,\"t\":1795099336.37}"
     . "[]")
    ("{\"out\":{\"type\":\"assistant\",\"parent_tool_use_id\":null,\"message\":{\"id\":\"msg_A\",\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{\"command\":\"echo remote-ok\",\"description\":\"Echo remote-ok\"}}],\"usage\":{\"input_tokens\":2,\"cache_creation_input_tokens\":69,\"cache_read_input_tokens\":31065}}},\"seq\":6,\"t\":1795099336.7}"
     . "[{\"type\":\"usage\",\"seq\":6,\"time\":1795099336.7,\"context_used\":31136},{\"type\":\"tool\",\"seq\":6,\"time\":1795099336.7,\"id\":\"toolu_1\",\"name\":\"Bash\",\"state\":\"started\",\"input\":{\"command\":\"echo remote-ok\",\"description\":\"Echo remote-ok\"},\"output\":null,\"parent\":null}]")
    ("{\"out\":{\"type\":\"user\",\"parent_tool_use_id\":null,\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"remote-ok\",\"is_error\":false}]}},\"seq\":7,\"t\":1795099336.8}"
     . "[{\"type\":\"tool\",\"seq\":7,\"time\":1795099336.8,\"id\":\"toolu_1\",\"name\":null,\"state\":\"completed\",\"input\":null,\"output\":\"remote-ok\",\"parent\":null}]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_B\",\"event\":{\"type\":\"message_start\",\"message\":{\"id\":\"msg_B\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":2,\"cache_creation_input_tokens\":115,\"cache_read_input_tokens\":31134,\"cache_creation\":{\"ephemeral_1h_input_tokens\":115,\"ephemeral_5m_input_tokens\":0}}}}},\"seq\":8,\"t\":1795099337.9}"
     . "[]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_B\",\"event\":{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"The\"}}},\"seq\":9,\"t\":1795099337.99}"
     . "[{\"type\":\"delta\",\"seq\":9,\"time\":1795099337.99,\"message\":\"msg_B\",\"text\":\"The\",\"parent\":null}]")
    ("{\"out\":{\"type\":\"stream_event\",\"parent_tool_use_id\":null,\"api_message_id\":\"msg_B\",\"event\":{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" command printed `remote-ok`.\"}}},\"seq\":10,\"t\":1795099338.0}"
     . "[{\"type\":\"delta\",\"seq\":10,\"time\":1795099338.0,\"message\":\"msg_B\",\"text\":\" command printed `remote-ok`.\",\"parent\":null}]")
    ("{\"out\":{\"type\":\"assistant\",\"parent_tool_use_id\":null,\"message\":{\"id\":\"msg_B\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"The command printed `remote-ok`.\"}],\"usage\":{\"input_tokens\":2,\"cache_creation_input_tokens\":115,\"cache_read_input_tokens\":31134}}},\"seq\":11,\"t\":1795099338.04}"
     . "[{\"type\":\"usage\",\"seq\":11,\"time\":1795099338.04,\"context_used\":31251},{\"type\":\"message-end\",\"seq\":11,\"time\":1795099338.04,\"message\":\"msg_B\",\"parent\":null}]")
    ("{\"out\":{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"The command printed `remote-ok`.\",\"duration_ms\":2908,\"total_cost_usd\":0.00715835,\"modelUsage\":{\"claude-haiku-5-5\":{\"contextWindow\":1000000}}},\"seq\":12,\"t\":1795099338.1}"
     . "[{\"type\":\"done\",\"seq\":12,\"time\":1795099338.1,\"text\":\"The command printed `remote-ok`.\",\"error\":null,\"limit\":null,\"interrupted\":null,\"duration\":2.908,\"cost\":0.00715835,\"reason\":null}]")
    ("{\"exit\":0,\"seq\":13,\"t\":1795099340.0}"
     . "[{\"type\":\"exit\",\"seq\":13,\"time\":1795099340.0,\"status\":0}]"))
  "A Claude Code turn with a tool call, then a streamed reply, and its
run's exit: each record with what is sent for it.")

(defconst romacs-agent-wire-tests--codex
  '(
    ("{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1795100000.0}"
     . "[]")
    ("{\"in\":{\"id\":3,\"method\":\"turn/start\",\"params\":{\"threadId\":\"th\",\"input\":[{\"type\":\"text\",\"text\":\"List the files\"}]},\"romacsRequest\":1},\"seq\":2,\"t\":1795100000.1}"
     . "[{\"type\":\"input\",\"seq\":2,\"time\":1795100000.1,\"text\":\"List the files\",\"images\":null,\"steer\":null,\"own\":null,\"messages\":null,\"request\":1}]")
    ("{\"out\":{\"method\":\"turn/started\",\"params\":{\"threadId\":\"th\",\"turn\":{\"id\":\"t1\"}}},\"seq\":3,\"t\":1795100000.2}"
     . "[]")
    ("{\"out\":{\"method\":\"item/started\",\"params\":{\"threadId\":\"th\",\"turnId\":\"t1\",\"item\":{\"type\":\"commandExecution\",\"id\":\"x\",\"command\":\"ls\",\"status\":\"inProgress\",\"aggregatedOutput\":null,\"exitCode\":null}}},\"seq\":4,\"t\":1795100000.3}"
     . "[{\"type\":\"tool\",\"seq\":4,\"time\":1795100000.3,\"id\":\"x\",\"name\":\"Bash\",\"state\":\"started\",\"input\":{\"command\":\"ls\"},\"output\":null,\"parent\":null}]")
    ("{\"out\":{\"method\":\"item/completed\",\"params\":{\"threadId\":\"th\",\"turnId\":\"t1\",\"item\":{\"type\":\"commandExecution\",\"id\":\"x\",\"command\":\"ls\",\"status\":\"completed\",\"aggregatedOutput\":\"a.txt\\n\",\"exitCode\":0}}},\"seq\":5,\"t\":1795100000.4}"
     . "[{\"type\":\"tool\",\"seq\":5,\"time\":1795100000.4,\"id\":\"x\",\"name\":null,\"state\":\"completed\",\"input\":{\"command\":\"ls\"},\"output\":\"a.txt\\n\",\"parent\":null}]")
    ("{\"out\":{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"th\",\"turnId\":\"t1\",\"itemId\":\"m1\",\"delta\":\"One\"}},\"seq\":6,\"t\":1795100000.5}"
     . "[{\"type\":\"delta\",\"seq\":6,\"time\":1795100000.5,\"message\":\"m1\",\"text\":\"One\",\"parent\":null}]")
    ("{\"out\":{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"th\",\"turnId\":\"t1\",\"itemId\":\"m1\",\"delta\":\" file.\"}},\"seq\":7,\"t\":1795100000.6}"
     . "[{\"type\":\"delta\",\"seq\":7,\"time\":1795100000.6,\"message\":\"m1\",\"text\":\" file.\",\"parent\":null}]")
    ("{\"out\":{\"method\":\"item/completed\",\"params\":{\"threadId\":\"th\",\"turnId\":\"t1\",\"item\":{\"type\":\"agentMessage\",\"id\":\"m1\",\"text\":\"One file.\"}}},\"seq\":8,\"t\":1795100000.7}"
     . "[{\"type\":\"delta\",\"seq\":8,\"time\":1795100000.7,\"message\":\"m1\",\"text\":\"\\n\",\"parent\":null},{\"type\":\"message-end\",\"seq\":8,\"time\":1795100000.7,\"message\":\"m1\",\"parent\":null}]")
    ("{\"out\":{\"method\":\"thread/tokenUsage/updated\",\"params\":{\"threadId\":\"th\",\"tokenUsage\":{\"last\":{\"totalTokens\":1200},\"total\":{\"totalTokens\":5000},\"modelContextWindow\":258000}}},\"seq\":9,\"t\":1795100000.8}"
     . "[{\"type\":\"usage\",\"seq\":9,\"time\":1795100000.8,\"context_used\":1200}]")
    ("{\"out\":{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"th\",\"turn\":{\"id\":\"t1\",\"status\":\"completed\",\"durationMs\":1500}}},\"seq\":10,\"t\":1795100000.9}"
     . "[{\"type\":\"done\",\"seq\":10,\"time\":1795100000.9,\"text\":\"One file.\",\"error\":null,\"limit\":null,\"interrupted\":null,\"duration\":1.5,\"cost\":null,\"reason\":null}]"))
  "A Codex turn with a command, then a streamed reply: each record with
what is sent for it.")

(defun romacs-agent-wire-tests--check (harness vector)
  "Replay VECTOR's records on a new HARNESS session, each record's events
sent as expected."
  (let* ((session (romacs-agent--make :id "w" :harness harness :state 'running :title "t"))
         (scratch (romacs-agent--scratch session))
         (raised nil)
         (romacs-agent-event-functions (list (lambda (_ event) (push event raised)))))
    (pcase-dolist (`(,record . ,sent) vector)
      (setq raised nil)
      (romacs-agent--record scratch (json-parse-string record :false-object :false :null-object :null))
      (should (equal (cons record (json-serialize (romacs-agent-wire-unit (nreverse raised))))
                     (cons record sent)))
      (romacs-agent-wire-tests--round-trip raised))))

(defun romacs-agent-wire-tests--same (a b)
  "Whether A and B are the same data, tables by their JSON."
  (cond ((and (hash-table-p a) (hash-table-p b)) (equal (json-serialize a) (json-serialize b)))
        ((and (consp a) (consp b))
         (and (romacs-agent-wire-tests--same (car a) (car b)) (romacs-agent-wire-tests--same (cdr a) (cdr b))))
        (t (equal a b))))

(defun romacs-agent-wire-tests--sent (events)
  "EVENTS, raised for one record, as a client reads them: sent as JSON
text, parsed as a client parses it (`romacs-agent-rpc-parse'), decoded."
  (romacs-agent-wire-decode-unit
   (json-parse-string (json-serialize (romacs-agent-wire-unit events)) :null-object :null :false-object :false)))

(defun romacs-agent-wire-tests--draw (units)
  "The chat text UNITS, each a record's events, draw."
  (let ((session (romacs-agent--make :id "D" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (dolist (unit units) (dolist (event unit) (romacs-agent-log-draw session event)))
      (romacs-agent-log--close-stream)
      (buffer-substring-no-properties (point-min) romacs-agent-log--end))))

(defun romacs-agent-wire-tests--round-trip (events)
  "EVENTS, raised for one record, read back by a client as they were
raised: each sent field as it held it, the last of them marked :last."
  (let ((kept (seq-filter (lambda (event) (assq (plist-get event :type) romacs-agent-wire-events)) events))
        (read (romacs-agent-wire-tests--sent events)))
    (should (= (length read) (length kept)))
    (cl-loop for event in kept for back in read
             do (should (eq (plist-get back :type) (plist-get event :type)))
             do (should (equal (plist-get back :last) (eq event (car (last kept)))))
             do (dolist (field (cdr (assq (plist-get event :type) romacs-agent-wire-events)))
                  (let ((key (intern (concat ":" (symbol-name (car (romacs-agent-wire--field field)))))))
                    (should (romacs-agent-wire-tests--same (list key (plist-get back key))
                                                           (list key (plist-get event key)))))))))

(ert-deftest romacs-agent-wire-claude-vector ()
  (romacs-agent-wire-tests--check 'claude romacs-agent-wire-tests--claude))

(ert-deftest romacs-agent-wire-codex-vector ()
  (romacs-agent-wire-tests--check 'codex romacs-agent-wire-tests--codex))

(ert-deftest romacs-agent-wire-every-field-is-sent ()
  ;; Every field of a type sent is present, null when the event has none;
  ;; an event of a type not sent, and a field not named, is not sent.
  (dolist (entry romacs-agent-wire-events)
    (let ((object (romacs-agent-wire-event (list :type (car entry) :seq 1 :time 2.0 :unnamed t))))
      (should (equal (let ((keys nil)) (maphash (lambda (key _) (push key keys)) object) (nreverse keys))
                     (append '("type" "seq" "time")
                             (mapcar (lambda (field) (romacs-agent-wire--name (car (romacs-agent-wire--field field))))
                                     (cdr entry)))))
      (should (equal (gethash "type" object) (symbol-name (car entry))))
      (dolist (field (cdr entry))
        (should (eq (gethash (romacs-agent-wire--name (car (romacs-agent-wire--field field))) object) :null)))))
  (should-not (romacs-agent-wire-event '(:type handshake :seq 1 :state (:owed nil))))
  (should (equal (romacs-agent-wire-unit '((:type cache :seq 3 :ttl 300))) [])))

(ert-deftest romacs-agent-wire-values ()
  (should (equal (json-serialize
                  (romacs-agent-wire-value
                   (list :images '("a.png") :messages '((:from "s1" :text "hi")) :steer t :state 'started
                         :none nil :input (json-parse-string "{\"a\":[1,null,false]}"))))
                 "{\"images\":[\"a.png\"],\"messages\":[{\"from\":\"s1\",\"text\":\"hi\"}],\"steer\":true,\"state\":\"started\",\"none\":null,\"input\":{\"a\":[1,null,false]}}"))
  ;; A plist is keywords each with its value; another list is an array.
  (should (equal (json-serialize (romacs-agent-wire-value '(:a))) "[\"a\"]"))
  (should (equal (json-serialize (romacs-agent-wire-value '(:a 1 "b" 2))) "[\"a\",1,\"b\",2]"))
  ;; What JSON has no form for is refused, never sent broken.
  (should-error (romacs-agent-wire-value '(1 . 2)) :type 'wrong-type-argument))

;; One of each type sent, its fields as its drivers raise them.
(defconst romacs-agent-wire-tests--each
  (list (list :type 'input :text "look" :images '("host:/a.png") :steer t :own "own"
              :messages (list (romacs-agent-make-message "m-1" "child" '(:session "s1" :name "kid") nil "done"))
              :request 4)
        (list :type 'incoming :text "two" :prompt "ab12" :messages (list (romacs-agent-make-message "m-2" "sibling" '(:name "sib") "m-1" "hi")))
        (list :type 'tool :id "t1" :name "Bash" :state 'started :input (json-parse-string "{\"command\":\"ls\",\"x\":[1,null,false]}"
                                                                                           :null-object :null :false-object :false))
        (list :type 'tool-output :id "t1" :text "a\n")
        (list :type 'tool :id "t1" :state 'failed :output "no")
        (list :type 'task :id "k" :task "k1" :state 'progress :background t :activity "reading" :summary "s" :title "T")
        (list :type 'task :id "k" :state 'backgrounded)
        (list :type 'done :text "ok" :error t :limit 'model :interrupted t :duration 1.5 :cost 0.25 :reason "cut")
        (list :type 'done :text "ok" :limit t)
        (list :type 'selection :from '(:model "m" :effort nil :provider "claude") :to '(:model nil :effort "high" :provider "codex"))
        (list :type 'renewed :why "full" :from "a" :to "b" :brief "the brief")
        (list :type 'forked :from "s0" :title "First" :seq-from 40 :turns 3 :after 2)
        (list :type 'rolled-back :to 30 :turns 2 :prompt "again" :files t)
        (list :type 'replied :message "m-9" :reply "yes" :by-user t)
        (list :type 'unknown :raw (json-parse-string "{\"type\":\"odd\"}") :method nil)
        (list :type 'unknown :raw "a line")
        (list :type 'exit :status 0))
  "An event of each type sent, with values of each kind its fields hold.")

(ert-deftest romacs-agent-wire-reads-back-each-type ()
  ;; What a client decodes is what was raised, and draws the same chat.
  (let ((units (cl-loop for event in romacs-agent-wire-tests--each for seq from 1
                        collect (list (append event (list :seq seq :time (+ 1795100000.0 seq) :last t))))))
    (dolist (unit units) (romacs-agent-wire-tests--round-trip unit))
    (should (equal (romacs-agent-wire-tests--draw (mapcar #'romacs-agent-wire-tests--sent units))
                   (romacs-agent-wire-tests--draw units))))
  ;; A type or field a client does not know is passed over (must-ignore).
  (should-not (romacs-agent-wire-decode (json-parse-string "{\"type\":\"later\",\"seq\":1}")))
  (should (equal (romacs-agent-wire-decode (json-parse-string "{\"type\":\"exit\",\"seq\":1,\"time\":2.0,\"status\":3,\"new\":1}"))
                 '(:type exit :seq 1 :time 2.0 :status 3))))

(ert-deftest romacs-agent-wire-vectors-draw-the-same ()
  ;; Each harness's turn, read back through the wire, draws the chat it
  ;; draws as raised.
  (dolist (case (list (cons 'claude romacs-agent-wire-tests--claude) (cons 'codex romacs-agent-wire-tests--codex)))
    (let* ((session (romacs-agent--make :id "w" :harness (car case) :state 'running :title "t"))
           (scratch (romacs-agent--scratch session))
           (raised nil)
           (romacs-agent-event-functions (list (lambda (_ event) (push event raised))))
           (units (mapcar (lambda (entry)
                            (setq raised nil)
                            (romacs-agent--record scratch (json-parse-string (car entry) :false-object :false :null-object :null))
                            (nreverse raised))
                          (cdr case))))
      (should (equal (romacs-agent-wire-tests--draw (mapcar #'romacs-agent-wire-tests--sent units))
                     (romacs-agent-wire-tests--draw units))))))

;;; romacs-agent-wire-tests.el ends here

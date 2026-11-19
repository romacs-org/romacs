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
     . "[{\"type\":\"done\",\"seq\":12,\"time\":1795099338.1,\"text\":\"The command printed `remote-ok`.\",\"error\":null,\"limit\":null,\"duration\":2.908,\"cost\":0.00715835,\"reason\":null}]")
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
     . "[{\"type\":\"done\",\"seq\":10,\"time\":1795100000.9,\"text\":\"One file.\",\"error\":null,\"limit\":null,\"duration\":1.5,\"cost\":null,\"reason\":null}]"))
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
                     (cons record sent))))))

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
                     (append '("type" "seq" "time") (mapcar #'romacs-agent-wire--name (cdr entry)))))
      (should (equal (gethash "type" object) (symbol-name (car entry))))
      (dolist (field (cdr entry))
        (should (eq (gethash (romacs-agent-wire--name field) object) :null)))))
  (should-not (romacs-agent-wire-event '(:type handshake :seq 1 :state (:owed nil))))
  (should (equal (romacs-agent-wire-unit '((:type cache :seq 3 :ttl 300))) [])))

(ert-deftest romacs-agent-wire-values ()
  (should (equal (json-serialize
                  (romacs-agent-wire-value
                   (list :images '("a.png") :messages '((:from "s1" :text "hi")) :steer t :state 'started
                         :none nil :input (json-parse-string "{\"a\":[1,null,false]}"))))
                 "{\"images\":[\"a.png\"],\"messages\":[{\"from\":\"s1\",\"text\":\"hi\"}],\"steer\":true,\"state\":\"started\",\"none\":null,\"input\":{\"a\":[1,null,false]}}"))
  ;; What JSON has no form for is refused, never sent broken.
  (should-error (romacs-agent-wire-value '(1 . 2)) :type 'wrong-type-argument))

;;; romacs-agent-wire-tests.el ends here

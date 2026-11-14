;;; agent-restore.el --- scaling probe  -*- lexical-binding: t -*-

;; A server restoring a session whose journal holds N turns, its state kept
;; at its last turn's end (romacs-agent.el, "A session's state, kept"): it
;; reads that state and none of the journal's records, so a turn costs what
;; it left in the state, never its records.  Held by its slope,
;; instructions per turn of journal, not by an exponent.  Measured
;; 2026-11-14: about 24,000 a turn, nearly all the reader's on the tool
;; and message ids the state keeps (18 bytes a turn, about 600
;; instructions a byte); the whole journal read again, as before states
;; were kept, costs about 800,000 a turn.

(defun scaling--agent-restore-journal (n)
  "A recorded Claude Code session of N turns, each a prompt, a streamed
reply with a tool call, the tool's output and the turn's end."
  (scaling--agent)
  (let* ((id "5ca1ab1e-0000-4000-8000-000000000000")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (seq 0) (output (make-string 1024 ?x)))
    (make-directory dir t)
    (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                              (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" dir
                                                  "title" "restore" "titleAuto" :false "argv" []
                                                  "options" (romacs-mcp--object) "seenSeq" 0))
    (cl-flet ((line (key value) (concat (romacs-agent--json (romacs-mcp--object key value "seq" (cl-incf seq)
                                                                                "t" (+ 1e9 seq 0.5)))
                                        "\n"))
              (o (&rest fields) (apply #'romacs-mcp--object fields)))
      (with-temp-file (expand-file-name "journal.jsonl" dir)
        (insert (line "note" (o "type" "run")))
        (dotimes (i n)
          (let ((message (format "m%d" i)) (tool (format "t%d" i)))
            (insert (line "in" (o "type" "user" "romacsRequest" (1+ i) "message" (o "role" "user" "content" "go on")))
                    (line "out" (o "type" "stream_event" "parent_tool_use_id" :null
                                   "event" (o "type" "message_start" "message" (o "id" message))))
                    (line "out" (o "type" "stream_event" "parent_tool_use_id" :null
                                   "event" (o "type" "content_block_delta"
                                              "delta" (o "type" "text_delta" "text" "some reply text"))))
                    (line "out" (o "type" "assistant" "parent_tool_use_id" :null
                                   "message" (o "id" message
                                                "content" (vector (o "type" "text" "text" "some reply text")
                                                                  (o "type" "tool_use" "id" tool "name" "Bash"
                                                                     "input" (o "command" "make check"))))))
                    (line "out" (o "type" "user" "parent_tool_use_id" :null
                                   "message" (o "content" (vector (o "type" "tool_result" "tool_use_id" tool
                                                                     "content" output)))))
                    (line "out" (o "type" "result" "result" "some reply text" "total_cost_usd" 0.01)))))))
    dir))

(scaling-probe agent-restore :n 200 :slope 32000
  :setup (let ((dir (scaling--agent-restore-journal n)))
           (clrhash romacs-agent--sessions)
           ;; The first restore reads the journal, and the state is kept
           ;; once the editor is idle.
           (romacs-agent--load dir)
           (romacs-agent--keep-pending)
           dir)
  :body (progn (clrhash romacs-agent--sessions)
               (romacs-agent--load it)))

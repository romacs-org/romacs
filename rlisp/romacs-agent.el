;;; romacs-agent.el --- Agent sessions that outlive the editor -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent session is a harness (Claude Code's stream-json CLI) run by its
;; own supervisor, `romacs-agent-host', which owns the harness's stdio and
;; journals every line under $XDG_STATE_HOME/romacs/agents/ID/.  The
;; supervisor is nobody's child (a transient user unit), so the harness
;; and its turn survive a closed window, a server stop and a new build: the
;; next server attaches to the supervisor's socket and replays the journal.
;; The harness reaches Romacs's tools through `romacs-mcp --reconnect', which
;; keeps the harness's MCP session across server restarts.
;;
;; The harness runs with the user's own configuration and permission mode:
;; Romacs passes none.  When the harness asks the user to approve a tool
;; (its `can_use_tool' request), the session raises an `approval' event and
;; writes back exactly what the user answered.
;;
;; Events are plists with a :type (session status delta thinking tool
;; approval done error input exit unknown), the session and the journal
;; :seq; `romacs-agent-event-functions' receive them (romacs-agent-ui.el
;; renders them).

;;; Code:
(require 'cl-lib)
(require 'json)
(require 'romacs-mcp)
(declare-function romacs-agent-show "romacs-agent-ui")

(defgroup romacs-agent nil
  "Agent sessions in Romacs."
  :group 'external)

(defcustom romacs-agent-claude-program "claude"
  "The Claude Code executable."
  :type 'string :group 'romacs-agent)

(defcustom romacs-agent-claude-arguments nil
  "Further arguments for Claude Code, passed through untouched.
Romacs passes no permission mode of its own: the user's configuration
decides, and these arguments may say otherwise."
  :type '(repeat string) :group 'romacs-agent)

(defcustom romacs-agent-reply-timeout 120
  "Seconds a session's relay waits for a tool call's answer while the
editor is busy before it withdraws the call (`not executed' when it had
not started, `outcome unknown' when it had)."
  :type 'number :group 'romacs-agent)

(defcustom romacs-agent-unit-properties '("MemoryMax=8G")
  "systemd properties of each session's unit."
  :type '(repeat string) :group 'romacs-agent)

(defvar romacs-agent-event-functions nil
  "Functions called with a session and each event it raises.")

(cl-defstruct (romacs-agent (:constructor romacs-agent--make))
  "One agent session."
  id harness cwd dir host-socket connection (seq 0) (state 'starting)
  title model harness-session cost unit buffer
  (approvals (make-hash-table :test #'equal))
  (streamed (make-hash-table :test #'equal))
  (tools (make-hash-table :test #'equal))
  message input (requests 0))

(defvar romacs-agent--sessions (make-hash-table :test #'equal)
  "Session id to session.")

;;; Places

(defun romacs-agent--state-root ()
  (expand-file-name "romacs/agents/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))

(defun romacs-agent--runtime-root ()
  (expand-file-name "romacs/agents/" (or (getenv "XDG_RUNTIME_DIR") (format "/tmp/romacs%d" (user-uid)))))

(defun romacs-agent--short (id) (substring id 0 8))

(defun romacs-agent--program (name)
  "Romacs's own program NAME, beside the running binary, else on PATH."
  (let ((beside (expand-file-name name invocation-directory)))
    (if (file-executable-p beside) beside (or (executable-find name) name))))

(defun romacs-agent--uuid ()
  "A random version 4 UUID, as Claude Code's --session-id takes it."
  (format "%04x%04x-%04x-4%03x-%04x-%04x%04x%04x"
          (random 65536) (random 65536) (random 65536) (random 4096)
          (logior #x8000 (random 16384)) (random 65536) (random 65536) (random 65536)))

(defun romacs-agent--write-json (file object)
  "Write OBJECT as JSON to FILE, atomically, readable only by the user."
  (let ((temporary (concat file ".tmp")))
    (with-temp-file temporary
      (insert (json-serialize object :false-object :false :null-object :null)))
    (set-file-modes temporary #o600)
    (rename-file temporary file t)))

(defun romacs-agent--read-json (file)
  (when (file-readable-p file)
    (with-temp-buffer
      (insert-file-contents file)
      (ignore-errors (json-parse-buffer :object-type 'hash-table :null-object nil :false-object nil)))))

(defun romacs-agent--endpoint ()
  "The native endpoint's socket, started for this session when needed."
  (or romacs-mcp--socket
      (let ((directory (expand-file-name (format "romacs/session-%d/" (emacs-pid))
                                         (or (getenv "XDG_RUNTIME_DIR") temporary-file-directory))))
        (make-directory directory t)
        (set-file-modes directory #o700)
        (romacs-mcp-start (expand-file-name "server.sock" directory))
        romacs-mcp--socket)))

;;; Claude Code

(defun romacs-agent--claude-argv (session resume)
  "Claude Code's command line for SESSION, resuming its conversation when
RESUME is non-nil."
  (let ((dir (romacs-agent-dir session)))
    (romacs-agent--write-json
     (expand-file-name "mcp.json" dir)
     (romacs-mcp--object
      "mcpServers" (romacs-mcp--object
                    "romacs" (romacs-mcp--object
                              "command" (romacs-agent--program "romacs-mcp")
                              "args" (vector "--socket" (romacs-agent--endpoint) "--reconnect"
                                             "--session" (romacs-agent-id session)
                                             "--reply-timeout" (number-to-string romacs-agent-reply-timeout))))))
    (append (list romacs-agent-claude-program "-p" "--output-format" "stream-json"
                  "--input-format" "stream-json" "--verbose" "--include-partial-messages"
                  ;; The harness's own approvals come to this session's UI.
                  "--permission-prompt-tool" "stdio"
                  "--mcp-config" (expand-file-name "mcp.json" dir))
            (if resume (list "--resume" (romacs-agent-id session))
              (list "--session-id" (romacs-agent-id session)))
            romacs-agent-claude-arguments)))

(defun romacs-agent--line (session object)
  "Write OBJECT as one line to SESSION's harness."
  (let ((connection (romacs-agent-connection session)))
    (unless (process-live-p connection) (user-error "Session %s is not attached" (romacs-agent-title session)))
    (process-send-string
     connection
     (concat (json-serialize (romacs-mcp--object "write" (json-serialize object :false-object :false :null-object :null)))
             "\n"))))

(defun romacs-agent--claude-user (text)
  (romacs-mcp--object "type" "user" "message" (romacs-mcp--object "role" "user" "content" text)))

(defun romacs-agent--text-of (content)
  "The text of a message CONTENT: a string or a vector of blocks."
  (if (stringp content) content
    (mapconcat (lambda (block) (or (and (hash-table-p block) (gethash "text" block)) "")) content "")))

(defun romacs-agent--value (value)
  "VALUE, with JSON null and false as nil."
  (unless (memq value '(:null :false)) value))

(defun romacs-agent--claude-events (session line)
  "The events of LINE, one of Claude Code's stream-json objects."
  (let ((type (gethash "type" line))
        (parent (romacs-agent--value (gethash "parent_tool_use_id" line))))
    (pcase type
      ("system"
       (pcase (gethash "subtype" line)
         ("init" (list (list :type 'session :model (gethash "model" line)
                             :harness-session (gethash "session_id" line)
                             :mcp (gethash "mcp_servers" line))))
         ("status" (list (list :type 'status :text (gethash "status" line))))
         (_ nil)))
      ("stream_event"
       (let* ((event (gethash "event" line))
              (kind (and event (gethash "type" event))))
         (pcase kind
           ("message_start"
            (setf (romacs-agent-message session) (gethash "id" (gethash "message" event)))
            nil)
           ("content_block_delta"
            (let* ((delta (gethash "delta" event)) (dtype (gethash "type" delta)))
              (puthash (romacs-agent-message session) t (romacs-agent-streamed session))
              (pcase dtype
                ("text_delta" (list (list :type 'delta :text (gethash "text" delta) :parent parent)))
                ("thinking_delta" (list (list :type 'thinking :text (gethash "thinking" delta) :parent parent))))))
           (_ nil))))
      ("assistant"
       (let* ((message (gethash "message" line))
              (streamed (gethash (gethash "id" message) (romacs-agent-streamed session))))
         (cl-loop for block across (gethash "content" message)
                  for btype = (gethash "type" block)
                  when (and (equal btype "text") (not streamed))
                  collect (list :type 'delta :text (gethash "text" block) :parent parent)
                  when (equal btype "tool_use")
                  collect (progn
                            (puthash (gethash "id" block) (or parent t) (romacs-agent-tools session))
                            (list :type 'tool :state 'started :id (gethash "id" block)
                                  :name (gethash "name" block) :input (gethash "input" block)
                                  :parent parent)))))
      ("user"
       (let ((content (gethash "content" (gethash "message" line))))
         (if (stringp content)
             (list (list :type 'status :text content :parent parent))
           (cl-loop for block across content
                    for btype = (gethash "type" block)
                    when (equal btype "tool_result")
                    collect (list :type 'tool :state (if (eq (gethash "is_error" block) t) 'failed 'completed)
                                  :id (gethash "tool_use_id" block)
                                  :output (romacs-agent--text-of (gethash "content" block))
                                  :parent parent)
                    when (equal btype "text")
                    collect (list :type 'status :text (gethash "text" block) :parent parent)))))
      ("result"
       (list (list :type 'done :cost (gethash "total_cost_usd" line)
                   :error (eq (gethash "is_error" line) t) :text (gethash "result" line))))
      ("control_request"
       (let ((request (gethash "request" line)))
         (when (equal (gethash "subtype" request) "can_use_tool")
           (puthash (gethash "request_id" line) request (romacs-agent-approvals session))
           (list (list :type 'approval :request-id (gethash "request_id" line)
                       :tool (gethash "tool_name" request) :input (gethash "input" request)
                       :description (gethash "description" request))))))
      ("control_response"
       (let ((id (gethash "request_id" (gethash "response" line))))
         (when (and id (gethash id (romacs-agent-approvals session)))
           (remhash id (romacs-agent-approvals session)))
         nil))
      ("rate_limit_event" nil)
      (_ (list (list :type 'unknown :raw line))))))

(defun romacs-agent--input-events (session line)
  "The events of LINE, written to SESSION's harness: a user's prompt, or
an approval answered."
  (pcase (gethash "type" line)
    ("user" (list (list :type 'input :text (romacs-agent--text-of (gethash "content" (gethash "message" line))))))
    ("control_response"
     (let ((id (gethash "request_id" (gethash "response" line))))
       (remhash id (romacs-agent-approvals session))
       (list (list :type 'answered :request-id id
                   :behavior (gethash "behavior" (gethash "response" (gethash "response" line)))))))
    (_ nil)))

;;; Journal records

(defun romacs-agent--raise (session event)
  "Apply EVENT to SESSION's state and give it to the event functions."
  (pcase (plist-get event :type)
    ('session (setf (romacs-agent-model session) (plist-get event :model)
                    (romacs-agent-harness-session session) (plist-get event :harness-session)
                    (romacs-agent-state session) 'running))
    ('input (setf (romacs-agent-state session) 'running))
    ('approval (setf (romacs-agent-state session) 'waiting-approval))
    ('answered (setf (romacs-agent-state session) 'running))
    ('done (setf (romacs-agent-state session) 'idle)
           (setf (romacs-agent-cost session) (plist-get event :cost)))
    ('exit (setf (romacs-agent-state session) 'ended)))
  (run-hook-with-args 'romacs-agent-event-functions session event))

(defun romacs-agent--record (session record)
  "Handle one journal RECORD of SESSION."
  (let ((seq (gethash "seq" record)))
    (when (and (integerp seq) (> seq (romacs-agent-seq session)))
      (setf (romacs-agent-seq session) seq)
      (let ((out (gethash "out" record :absent))
            (in (gethash "in" record :absent))
            (exit (gethash "exit" record :absent)))
        (let ((events (cond ((hash-table-p out) (romacs-agent--claude-events session out))
                            ((stringp out) (list (list :type 'unknown :raw out)))
                            ((hash-table-p in) (romacs-agent--input-events session in))
                            ((not (eq exit :absent)) (list (list :type 'exit :status exit))))))
          ;; :last marks a record's last event: a view that has shown it has
          ;; shown the whole record.
          (while events
            (romacs-agent--raise session (append (car events) (list :seq seq :last (null (cdr events)))))
            (setq events (cdr events))))))))

(defun romacs-agent--read-journal (session function)
  "Call FUNCTION with each record of SESSION's journal, in order."
  (let ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
    (when (file-readable-p journal)
      (with-temp-buffer
        (insert-file-contents journal)
        (goto-char (point-min))
        (while (not (eobp))
          (let ((record (ignore-errors
                          (json-parse-string (buffer-substring (point) (line-end-position))
                                             :object-type 'hash-table :null-object :null :false-object :false))))
            (when (hash-table-p record) (funcall function record)))
          (forward-line 1))))))

(defun romacs-agent--filter (process chunk)
  (let* ((session (process-get process 'romacs-agent))
         (text (concat (process-get process 'pending) chunk))
         (start 0))
    (while (string-match "\n" text start)
      (let ((line (substring text start (match-beginning 0))))
        (setq start (match-end 0))
        (let ((record (ignore-errors
                        ;; JSON false and null are kept, so a tool's input
                        ;; written back (an approval) is the one the harness sent.
                        (json-parse-string (decode-coding-string line 'utf-8)
                                           :object-type 'hash-table :null-object :null :false-object :false))))
          (cond ((not (hash-table-p record)))
                ((gethash "refused" record)
                 (romacs-agent--raise session (list :type 'error :text "another server is attached")))
                (t (condition-case err (romacs-agent--record session record)
                     (error (message "romacs-agent: %s" (error-message-string err)))))))))
    (process-put process 'pending (substring text start))))

(defun romacs-agent--sentinel (process _event)
  (unless (process-live-p process)
    (let ((session (process-get process 'romacs-agent)))
      (when (eq (romacs-agent-connection session) process)
        (setf (romacs-agent-connection session) nil)
        (unless (eq (romacs-agent-state session) 'ended)
          (setf (romacs-agent-state session) 'detached))
        (romacs-agent--raise session (list :type 'status :text "detached"))))))

(defun romacs-agent--attach (session &optional tries steal)
  "Attach to SESSION's supervisor and replay what this server has not seen.
Retried for a while: a new supervisor takes a moment to listen."
  (condition-case nil
      (let ((process (make-network-process
                      :name (format "romacs-agent-%s" (romacs-agent--short (romacs-agent-id session)))
                      :family 'local :service (romacs-agent-host-socket session)
                      :coding 'no-conversion :noquery t
                      :filter #'romacs-agent--filter :sentinel #'romacs-agent--sentinel)))
        (process-put process 'romacs-agent session)
        (setf (romacs-agent-connection session) process)
        (process-send-string
         process (concat (json-serialize
                          (romacs-mcp--object "attach" (romacs-mcp--object "from" (1+ (romacs-agent-seq session))
                                                                           "steal" (if steal t :false))))
                         "\n"))
        process)
    (file-error
     (if (> (or tries 0) 0)
         (run-at-time 0.1 nil #'romacs-agent--attach session (1- tries) steal)
       (setf (romacs-agent-state session) 'detached)
       (romacs-agent--raise session (list :type 'status :text "supervisor not reachable"))
       nil))))

;;; Lifecycle

(defun romacs-agent--manifest (session argv)
  (romacs-agent--write-json
   (expand-file-name "manifest.json" (romacs-agent-dir session))
   (romacs-mcp--object "id" (romacs-agent-id session) "harness" (symbol-name (romacs-agent-harness session))
                       "cwd" (romacs-agent-cwd session) "title" (romacs-agent-title session)
                       "created" (float-time) "argv" (vconcat argv)
                       "unit" (or (romacs-agent-unit session) :null)
                       "hostSocket" (romacs-agent-host-socket session))))

(defun romacs-agent--environment-arguments ()
  "systemd-run arguments giving a unit this session's environment, by
name: systemd-run takes each value from its own environment, so none
shows on a command line other users can read."
  (cl-loop for entry in process-environment
           when (string-match "\\`\\([A-Za-z_][A-Za-z0-9_]*\\)=" entry)
           collect (concat "--setenv=" (match-string 1 entry))))

(defun romacs-agent--launch (session argv)
  "Run SESSION's supervisor with harness ARGV, as a user unit when a user
manager answers, else detached."
  (let* ((host (romacs-agent--program "romacs-agent-host"))
         (command (append (list host "--dir" (romacs-agent-dir session)
                                "--socket" (romacs-agent-host-socket session) "--")
                          argv))
         (unit (format "romacs-agent-%s" (romacs-agent--short (romacs-agent-id session))))
         (default-directory (file-name-as-directory (romacs-agent-cwd session))))
    (make-directory (file-name-directory (romacs-agent-host-socket session)) t)
    (set-file-modes (file-name-directory (romacs-agent-host-socket session)) #o700)
    (if (zerop (apply #'call-process "systemd-run" nil nil nil
                      (append (list "--user" "--quiet" "--collect" (concat "--unit=" unit)
                                    "--slice=romacs-agents.slice"
                                    (concat "--working-directory=" (expand-file-name default-directory)))
                              (mapcar (lambda (p) (concat "--property=" p)) romacs-agent-unit-properties)
                              (romacs-agent--environment-arguments)
                              (cons "--" command))))
        (setf (romacs-agent-unit session) unit)
      (apply #'call-process "setsid" nil 0 nil "--fork" command)
      (setf (romacs-agent-unit session) nil))
    (romacs-agent--manifest session argv)))

;;;###autoload
(defun romacs-agent-start (cwd &optional prompt title)
  "Start a Claude Code session in CWD, sending PROMPT when given.
Return the session."
  (interactive (list (read-directory-name "Agent directory: " default-directory)
                     (read-string "Prompt (empty for none): ")))
  (let* ((id (romacs-agent--uuid))
         (session (romacs-agent--make
                   :id id :harness 'claude :cwd (expand-file-name cwd)
                   :title (or title (format "%s %s" (file-name-nondirectory (directory-file-name cwd))
                                            (romacs-agent--short id)))
                   :dir (expand-file-name id (romacs-agent--state-root))
                   :host-socket (expand-file-name (concat (romacs-agent--short id) "/host.sock")
                                                  (romacs-agent--runtime-root)))))
    (make-directory (romacs-agent-dir session) t)
    (set-file-modes (romacs-agent-dir session) #o700)
    (puthash id session romacs-agent--sessions)
    (romacs-agent--launch session (romacs-agent--claude-argv session nil))
    (romacs-agent--attach session 50)
    (when (and prompt (not (string-empty-p prompt)))
      (romacs-agent--when-attached session (lambda () (romacs-agent-send session prompt))))
    (when (called-interactively-p 'any) (romacs-agent-show session))
    session))

(defun romacs-agent--when-attached (session function &optional tries)
  "Call FUNCTION once SESSION is attached."
  (if (process-live-p (romacs-agent-connection session))
      (funcall function)
    (when (> (or tries 100) 0)
      (run-at-time 0.1 nil #'romacs-agent--when-attached session function (1- (or tries 100))))))

(defun romacs-agent-send (session text)
  "Send TEXT to SESSION's harness as the user's next message."
  (romacs-agent--line session (romacs-agent--claude-user text)))

(defun romacs-agent-interrupt (session)
  "Interrupt SESSION's turn; queued messages stay."
  (romacs-agent--line session (romacs-mcp--object
                               "type" "control_request"
                               "request_id" (format "romacs-interrupt-%d" (cl-incf (romacs-agent-requests session)))
                               "request" (romacs-mcp--object "subtype" "interrupt"))))

(defun romacs-agent-respond (session request-id allow &optional message)
  "Answer SESSION's approval REQUEST-ID: ALLOW non-nil allows the tool as
asked, else it is denied with MESSAGE."
  (let ((request (gethash request-id (romacs-agent-approvals session))))
    (unless request (user-error "No such approval request"))
    (romacs-agent--line
     session
     (romacs-mcp--object
      "type" "control_response"
      "response" (romacs-mcp--object
                  "subtype" "success" "request_id" request-id
                  "response" (if allow
                                 (romacs-mcp--object "behavior" "allow"
                                                     "updatedInput" (or (gethash "input" request) (romacs-mcp--object)))
                               (romacs-mcp--object "behavior" "deny"
                                                   "message" (or message "Denied by the user"))))))))

(defun romacs-agent--host-pid (session)
  (let ((host (romacs-agent--read-json (expand-file-name "host.json" (romacs-agent-dir session)))))
    (and host (gethash "pid" host))))

(defun romacs-agent-harness-pid (session)
  "The pid of SESSION's harness, from its supervisor's record."
  (let ((host (romacs-agent--read-json (expand-file-name "host.json" (romacs-agent-dir session)))))
    (and host (gethash "harness" host))))

(defun romacs-agent-stop (session)
  "End SESSION: its unit stops, ending the harness and everything it started."
  (if (romacs-agent-unit session)
      (call-process "systemctl" nil 0 nil "--user" "stop" (romacs-agent-unit session))
    (when-let* ((pid (romacs-agent--host-pid session))) (signal-process pid 'TERM))
    (when-let* ((pid (romacs-agent-harness-pid session))) (signal-process pid 'TERM))))

(defun romacs-agent-resume (session)
  "Run SESSION's harness again on its conversation, after it ended."
  (when (process-live-p (romacs-agent-connection session))
    (user-error "Session %s is still attached" (romacs-agent-title session)))
  ;; A turn the harness's end cut short goes on (Claude Code's own switch).
  (let ((process-environment (cons "CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1" process-environment)))
    (romacs-agent--launch session (romacs-agent--claude-argv session t)))
  (setf (romacs-agent-state session) 'starting)
  (romacs-agent--attach session 50))

;;; A new server

(defun romacs-agent--load (dir)
  "The session recorded in DIR, attached when its supervisor runs."
  (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
    (when (and manifest (not (gethash (gethash "id" manifest) romacs-agent--sessions)))
      (let ((session (romacs-agent--make
                      :id (gethash "id" manifest) :harness (intern (gethash "harness" manifest))
                      :cwd (gethash "cwd" manifest) :title (gethash "title" manifest)
                      :dir dir :unit (gethash "unit" manifest)
                      :host-socket (gethash "hostSocket" manifest) :state 'detached)))
        (puthash (romacs-agent-id session) session romacs-agent--sessions)
        (if (file-exists-p (romacs-agent-host-socket session))
            (romacs-agent--attach session 0)
          ;; No supervisor: what the journal says is all there is.
          (let ((romacs-agent-event-functions nil))
            (romacs-agent--read-journal session (lambda (record) (romacs-agent--record session record))))
          (unless (eq (romacs-agent-state session) 'ended)
            (setf (romacs-agent-state session) 'detached)))
        session))))

;;;###autoload
(defun romacs-agent-restore ()
  "Know every recorded session, and attach to those whose supervisor runs.
A server runs this at its start, so sessions survive it."
  (when (file-directory-p (romacs-agent--state-root))
    (dolist (dir (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-"))
      (romacs-agent--load dir))))

(defun romacs-agent-sessions ()
  "Every known session, newest first."
  (sort (hash-table-values romacs-agent--sessions)
        (lambda (a b) (string> (or (romacs-agent-dir a) "") (or (romacs-agent-dir b) "")))))

(defun romacs-agent-get (id)
  "The session ID names."
  (gethash id romacs-agent--sessions))

(romacs-mcp-register-method
 "romacs/hello"
 (lambda (peer params)
   ;; A session's relay says whom it serves; the tools record its calls.
   (process-put peer 'romacs-agent-session (gethash "session" params))
   ;; Its incarnation: a resumed harness's relay numbers its calls anew.
   (process-put peer 'romacs-agent-relay (gethash "relay" params))
   (romacs-mcp--object)))

(provide 'romacs-agent)
(require 'romacs-agent-ui)
(require 'romacs-agent-tools)
;;; romacs-agent.el ends here

;;; romacs-agent.el --- Agent sessions that outlive the editor -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent session is a harness (Claude Code's stream-json CLI, or Codex's
;; `codex exec --json', one run per turn) run by its
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

(defcustom romacs-agent-codex-program "codex"
  "The Codex CLI executable."
  :type 'string :group 'romacs-agent)

(defcustom romacs-agent-codex-arguments nil
  "Further arguments for `codex exec', passed through untouched, as
`romacs-agent-claude-arguments' are for Claude Code."
  :type '(repeat string) :group 'romacs-agent)

(defcustom romacs-agent-providers '(("claude" . "~/.claude"))
  "The harness instances sessions run under, the first being the default.
Each entry is (NAME . CONFIG-DIR), or (NAME :harness HARNESS :config-dir
CONFIG-DIR :program PROGRAM :arguments ARGUMENTS) for more.  HARNESS, when
omitted, is NAME's leading word (\"claude2\" and \"claude-work\" are
Claude Code, \"codex\" is Codex); PROGRAM and ARGUMENTS default to the
harness's own options (`romacs-agent-claude-program',
`romacs-agent-codex-arguments', ...).

CONFIG-DIR is the harness's configuration directory, where it keeps its
account, settings and conversations (Claude Code's CLAUDE_CONFIG_DIR,
Codex's CODEX_HOME).  It
is given to the harness alone; the harness's default directory, or none,
runs it with that variable unset, as the harness runs by default.  A
session records its instance's directory, and resumes there."
  :type '(repeat (choice (cons :tag "Name and directory" string directory)
                         (cons :tag "Name and options" string plist)))
  :group 'romacs-agent)

(defcustom romacs-agent-default-provider nil
  "The name of the instance `romacs-agent-start' offers first, or nil for
the first of `romacs-agent-providers'."
  :type '(choice (const nil) string) :group 'romacs-agent)

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
  id harness provider config-dir cwd dir host-socket connection (seq 0) (state 'starting)
  title model harness-session cost unit buffer
  ;; What it was started with: :model :effort :role :parent :request.
  options
  (approvals (make-hash-table :test #'equal))
  (streamed (make-hash-table :test #'equal))
  (tools (make-hash-table :test #'equal))
  message input (requests 0)
  ;; The last reply, the turns done, and messages waiting for a turn to
  ;; end: (TEXT DELIVERED VALID), DELIVERED called once TEXT reaches the
  ;; harness, and VALID, when given, asked first (nil drops the message).
  reply (turns 0) queue queue-tail cancelled
  ;; A delegated session's turn ends its parent has not had, newest
  ;; first: (SEQ RESULT FAILED).
  ends
  ;; The journal seq of its last turn's end, and whether that end failed
  ;; (an error, or the harness exiting mid-turn).
  (done-seq 0) failed
  ;; The journal's size when its last supervisor started.
  journal-start
  ;; What its views show, kept as its events arrive: the harness's permission
  ;; mode, the tokens of its last prompt and its model's window, the last
  ;; record's time, when the running turn began, whether the title is
  ;; generated (the first prompt replaces it), and the last turn end its
  ;; chat has shown.
  permission-mode context-used context-window last-time turn-start
  title-auto (seen-seq 0)
  ;; The harness's own transcript this session continues, when it was
  ;; imported from the harness's history: (:path PATH :size BYTES), the
  ;; bytes the harness had written before this session went on with it.
  imported)

(defvar romacs-agent--sessions (make-hash-table :test #'equal)
  "Session id to session.")

;;; Places

(defun romacs-agent--state-root ()
  (expand-file-name "romacs/agents/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))

(defun romacs-agent--runtime-root ()
  (expand-file-name "romacs/agents/" (or (getenv "XDG_RUNTIME_DIR") (format "/tmp/romacs%d" (user-uid)))))

(defun romacs-agent--short (id) (substring id 0 8))

(defun romacs-agent--key (id)
  "ID's name for its supervisor's socket directory and unit: its short
form, readable, and a digest of all of it, since ids made close together
\(Codex's time-ordered threads) share their first characters."
  (concat (romacs-agent--short id) "-" (substring (secure-hash 'sha256 id) 0 8)))

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

;;; Provider instances

(defconst romacs-agent--harnesses
  '((claude :variable "CLAUDE_CONFIG_DIR" :default "~/.claude"
            :program romacs-agent-claude-program :arguments romacs-agent-claude-arguments
            :argv romacs-agent--claude-argv :events romacs-agent--claude-events)
    (codex :variable "CODEX_HOME" :default "~/.codex"
           :program romacs-agent-codex-program :arguments romacs-agent-codex-arguments
           :argv romacs-agent--codex-argv :events romacs-agent--codex-events :per-turn t))
  "The harnesses sessions run: the variable naming a harness's
configuration directory, the directory it uses while that is unset, the
options holding its program and arguments, its command line and its
events.  A :per-turn harness runs once per turn, its prompt on stdin.")

(defun romacs-agent--harness-get (session key)
  (plist-get (alist-get (romacs-agent-harness session) romacs-agent--harnesses) key))

(defun romacs-agent--harness-of (name)
  "The harness an instance NAME names by its leading word."
  (and (string-match "\\`[[:alpha:]]+" name) (intern (downcase (match-string 0 name)))))

(defun romacs-agent--provider (entry)
  "ENTRY of `romacs-agent-providers' as a plist: :name :harness :config-dir
:program :arguments."
  (let* ((name (car entry))
         (options (if (listp (cdr entry)) (cdr entry) (list :config-dir (cdr entry))))
         (harness (or (plist-get options :harness) (romacs-agent--harness-of name)))
         (known (alist-get harness romacs-agent--harnesses)))
    (unless known
      (user-error "Agent provider %s: no harness %s (Romacs runs: %s)" name harness
                  (mapconcat (lambda (h) (symbol-name (car h))) romacs-agent--harnesses ", ")))
    (list :name name :harness harness :config-dir (plist-get options :config-dir)
          :program (if (plist-member options :program) (plist-get options :program)
                     (symbol-value (plist-get known :program)))
          :arguments (if (plist-member options :arguments) (plist-get options :arguments)
                       (symbol-value (plist-get known :arguments))))))

(defun romacs-agent--configured (name)
  "The configured instance NAME, or nil."
  (when-let* ((entry (assoc name romacs-agent-providers))) (romacs-agent--provider entry)))

(defun romacs-agent--instance (name harness)
  "The instance NAME as configured now, else HARNESS's defaults: what a
session recorded under NAME runs, its directory aside."
  (let ((provider (romacs-agent--configured name)))
    (if (eq (plist-get provider :harness) harness) provider
      (romacs-agent--provider (list name :harness harness)))))

(defun romacs-agent--config-dir (provider)
  "PROVIDER's configuration directory, absolute, or nil for its harness's
default (the harness then runs with its variable unset)."
  (let ((dir (plist-get provider :config-dir))
        (default (plist-get (alist-get (plist-get provider :harness) romacs-agent--harnesses) :default)))
    (when (and dir (not (string= (file-name-as-directory (expand-file-name dir))
                                 (file-name-as-directory (expand-file-name default)))))
      (directory-file-name (expand-file-name dir)))))

(defun romacs-agent--check-config-dir (provider dir)
  "Signal a clear error when PROVIDER's configuration directory DIR is
missing: the harness would start empty there, and could not resume."
  (when (and dir (not (file-directory-p (expand-file-name dir))))
    (user-error "Agent provider %s: configuration directory %s does not exist"
                provider (expand-file-name dir))))

(defun romacs-agent--environment-words (session)
  "The `env' words giving SESSION's harness its configuration directory,
whatever the editor's own environment and the user manager's say."
  (let ((variable (plist-get (alist-get (romacs-agent-harness session) romacs-agent--harnesses) :variable)))
    (if (romacs-agent-config-dir session)
        (list "env" (concat variable "=" (romacs-agent-config-dir session)))
      (list "env" "-u" variable))))

(defun romacs-agent--default-provider ()
  (or romacs-agent-default-provider (car (car romacs-agent-providers))
      (user-error "No agent provider: `romacs-agent-providers' is empty")))

(defun romacs-agent--read-provider ()
  "An instance's name: asked for when there is more than one."
  (let ((names (mapcar #'car romacs-agent-providers)))
    (if (cdr names)
        (completing-read (format-prompt "Provider" (romacs-agent--default-provider))
                         names nil t nil nil (romacs-agent--default-provider))
      (romacs-agent--default-provider))))

;;; Claude Code

(defun romacs-agent--claude-argv (session resume)
  "Claude Code's command line for SESSION, resuming its conversation when
RESUME is non-nil."
  (let ((dir (romacs-agent-dir session))
        (instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session))))
    (romacs-agent--write-json
     (expand-file-name "mcp.json" dir)
     (romacs-mcp--object
      "mcpServers" (romacs-mcp--object
                    "romacs" (romacs-mcp--object
                              "command" (romacs-agent--program "romacs-mcp")
                              "args" (vector "--socket" (romacs-agent--endpoint) "--reconnect"
                                             "--session" (romacs-agent-id session)
                                             "--reply-timeout" (number-to-string romacs-agent-reply-timeout))))))
    (append (romacs-agent--environment-words session)
            (list (plist-get instance :program) "-p" "--output-format" "stream-json"
                  "--input-format" "stream-json" "--verbose" "--include-partial-messages"
                  ;; The harness's own approvals come to this session's UI.
                  "--permission-prompt-tool" "stdio"
                  "--mcp-config" (expand-file-name "mcp.json" dir))
            (if resume (list "--resume" (romacs-agent-id session))
              (list "--session-id" (romacs-agent-id session)))
            (romacs-agent--option-words session "--model" "--effort")
            (plist-get instance :arguments))))

(defun romacs-agent--option-words (session model-flag effort-flag)
  "SESSION's model and effort as MODEL-FLAG and EFFORT-FLAG words."
  (let ((options (romacs-agent-options session)))
    (append (when-let* ((model (plist-get options :model))) (list model-flag model))
            (when-let* ((effort (plist-get options :effort))) (list effort-flag effort)))))

(defun romacs-agent--line (session object)
  "Write OBJECT as one line to SESSION's harness.  A detached session whose
supervisor still runs is attached first; one whose supervisor ended says
so: it resumes with `romacs-agent-resume'."
  (let ((connection (romacs-agent-connection session))
        (line (concat (json-serialize (romacs-mcp--object "write" (json-serialize object :false-object :false :null-object :null)))
                      "\n")))
    (cond ((process-live-p connection) (process-send-string connection line))
          ;; A stopped supervisor leaves its socket: only a connection says it listens.
          ((and (file-exists-p (romacs-agent-host-socket session)) (processp (romacs-agent--attach session 0)))
           (process-send-string (romacs-agent-connection session) line))
          (t (user-error "Session %s has ended: resume it (r) to go on" (romacs-agent-title session))))))

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
                             :permission-mode (gethash "permissionMode" line)
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
                ("text_delta" (list (list :type 'delta :text (gethash "text" delta) :parent parent
                                          :message (romacs-agent-message session))))
                ("thinking_delta" (list (list :type 'thinking :text (gethash "thinking" delta) :parent parent))))))
           (_ nil))))
      ("assistant"
       (let* ((message (gethash "message" line))
              (id (gethash "id" message))
              (streamed (gethash id (romacs-agent-streamed session)))
              (usage (gethash "usage" message))
              (text nil))
         (append
          ;; The prompt this call sent: the context it fills.
          (when (and (hash-table-p usage) (not parent))
            (list (list :type 'usage :context-used (+ (or (gethash "input_tokens" usage) 0)
                                                      (or (gethash "cache_read_input_tokens" usage) 0)
                                                      (or (gethash "cache_creation_input_tokens" usage) 0)))))
          (cl-loop for block across (gethash "content" message)
                   for btype = (gethash "type" block)
                   when (equal btype "text") do (setq text t)
                   when (and (equal btype "text") (not streamed))
                   collect (list :type 'delta :text (gethash "text" block) :parent parent :message id)
                   when (equal btype "tool_use")
                   collect (progn
                             (puthash (gethash "id" block) (or parent t) (romacs-agent-tools session))
                             (list :type 'tool :state 'started :id (gethash "id" block)
                                   :name (gethash "name" block) :input (gethash "input" block)
                                   :parent parent)))
          ;; The message's text is whole: its view may render it once.
          (when text (list (list :type 'message-end :message id :parent parent))))))
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
                   :error (eq (gethash "is_error" line) t) :text (gethash "result" line)
                   :duration (let ((ms (gethash "duration_ms" line))) (and (numberp ms) (/ ms 1000.0)))
                   :context-window (let ((usage (gethash "modelUsage" line)) (window nil))
                                     (when (hash-table-p usage)
                                       (maphash (lambda (_ model)
                                                  (let ((size (and (hash-table-p model) (gethash "contextWindow" model))))
                                                    (when (numberp size) (setq window (max size (or window 0))))))
                                                usage))
                                     window))))
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

;;; Codex

(defun romacs-agent--codex-argv (session resume)
  "`codex exec''s command line for one turn of SESSION, its prompt on
stdin; RESUME non-nil continues its thread."
  (let ((instance (romacs-agent--instance (romacs-agent-provider session) 'codex))
        (thread (romacs-agent-harness-session session)))
    (append (romacs-agent--environment-words session)
            (list (plist-get instance :program) "exec")
            (when (and resume thread) (list "resume" thread))
            (list "--json" "--skip-git-repo-check")
            ;; Romacs's tools, through the reconnecting relay.
            (romacs-agent--codex-mcp-words session)
            (let ((options (romacs-agent-options session)))
              (append (when-let* ((model (plist-get options :model))) (list "-m" model))
                      (when-let* ((effort (plist-get options :effort)))
                        (list "-c" (format "model_reasoning_effort=%S" effort)))))
            (plist-get instance :arguments)
            (list "-"))))

(defun romacs-agent--codex-mcp-words (session)
  (list "-c" (format "mcp_servers.romacs.command=%S" (romacs-agent--program "romacs-mcp"))
        "-c" (format "mcp_servers.romacs.args=[%s]"
                     (mapconcat (lambda (word) (format "%S" word))
                                (list "--socket" (romacs-agent--endpoint) "--reconnect"
                                      "--session" (romacs-agent-id session)
                                      "--reply-timeout" (number-to-string romacs-agent-reply-timeout))
                                ","))))

(defun romacs-agent--codex-tool (item)
  "An item's tool name and input, for the tool block."
  (pcase (gethash "type" item)
    ("command_execution" (list "Bash" (romacs-mcp--object "command" (gethash "command" item))))
    ("mcp_tool_call" (list (format "mcp__%s__%s" (gethash "server" item) (gethash "tool" item))
                           (gethash "arguments" item)))
    ("file_change" (list "Edit" (romacs-mcp--object "changes" (gethash "changes" item))))
    (type (list type nil))))

(defun romacs-agent--codex-output (item)
  (let ((output (or (gethash "aggregated_output" item) (gethash "result" item) (gethash "error" item) "")))
    (cond ((stringp output) output)
          ((and (hash-table-p output) (gethash "content" output)) (romacs-agent--text-of (gethash "content" output)))
          (t (json-serialize output :false-object :false :null-object :null)))))

(defun romacs-agent--codex-item (session item completed)
  "The events of ITEM, started or COMPLETED, in SESSION's current turn."
  (let ((type (gethash "type" item))
        (id (format "%d:%s" (romacs-agent-turns session) (gethash "id" item))))
    (pcase type
      ("agent_message"
       (when completed
         (setf (romacs-agent-reply session) (gethash "text" item))
         (list (list :type 'delta :text (concat (gethash "text" item) "\n") :message id)
               (list :type 'message-end :message id))))
      ("reasoning" (when completed (list (list :type 'thinking :text (or (gethash "text" item) "")))))
      ((or "todo_list" "error") nil)
      (_
       (let* ((started (unless (gethash id (romacs-agent-tools session))
                         (puthash id t (romacs-agent-tools session))
                         (pcase-let ((`(,name ,input) (romacs-agent--codex-tool item)))
                           (list (list :type 'tool :state 'started :id id :name name :input input)))))
              (failed (or (member (gethash "status" item) '("failed" "declined"))
                          (and (integerp (gethash "exit_code" item)) (/= 0 (gethash "exit_code" item))))))
         (append started
                 (when completed
                   (list (list :type 'tool :state (if failed 'failed 'completed) :id id
                               :output (romacs-agent--codex-output item))))))))))

(defun romacs-agent--codex-events (session line)
  "The events of LINE, one of `codex exec --json''s objects."
  (pcase (gethash "type" line)
    ("thread.started" (list (list :type 'session :harness-session (gethash "thread_id" line)
                                  :model (plist-get (romacs-agent-options session) :model))))
    ("turn.started" (setf (romacs-agent-reply session) nil) nil)
    ("item.started" (romacs-agent--codex-item session (gethash "item" line) nil))
    ("item.updated" nil)
    ("item.completed" (romacs-agent--codex-item session (gethash "item" line) t))
    ("turn.completed" (list (list :type 'done :text (romacs-agent-reply session) :usage (gethash "usage" line))))
    ("turn.failed" (list (list :type 'done :error t
                               :text (let ((error (gethash "error" line)))
                                       (or (and (hash-table-p error) (gethash "message" error)) "turn failed")))))
    ("error" (list (list :type 'error :text (gethash "message" line))))
    (_ (list (list :type 'unknown :raw line)))))

(defun romacs-agent--per-turn-p (session)
  (romacs-agent--harness-get session :per-turn))

(defun romacs-agent--turn (session text)
  "Run one turn of SESSION's per-turn harness on prompt TEXT.  The
supervisor journals TEXT and gives it to the harness itself, so the turn
goes on whatever becomes of this server."
  (romacs-agent--check-config-dir (romacs-agent-provider session) (romacs-agent-config-dir session))
  (let ((input (expand-file-name "input" (romacs-agent-dir session))))
    (let ((coding-system-for-write 'utf-8-unix))
      (write-region text nil input nil 'silent))
    (set-file-modes input #o600)
    (setf (romacs-agent-journal-start session)
          (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (romacs-agent-dir session)))) 0))
    (romacs-agent--launch session (romacs-agent--codex-argv session t) input))
  (setf (romacs-agent-state session) 'starting)
  (romacs-agent--attach session 50))

;;; Journal records

(defun romacs-agent--raise (session event)
  "Apply EVENT to SESSION's state and give it to the event functions."
  (when (plist-get event :time) (setf (romacs-agent-last-time session) (plist-get event :time)))
  (pcase (plist-get event :type)
    ('session (setf (romacs-agent-model session) (or (plist-get event :model) (romacs-agent-model session))
                    (romacs-agent-harness-session session) (plist-get event :harness-session)
                    (romacs-agent-state session) 'running)
              (when (plist-get event :permission-mode)
                (setf (romacs-agent-permission-mode session) (plist-get event :permission-mode))))
    ('usage (setf (romacs-agent-context-used session) (plist-get event :context-used)))
    ('input (setf (romacs-agent-state session) 'running
                  (romacs-agent-turn-start session) (plist-get event :time))
            (when (romacs-agent-title-auto session) (romacs-agent--title-from session (plist-get event :text))))
    ('approval (setf (romacs-agent-state session) 'waiting-approval))
    ('answered (setf (romacs-agent-state session) 'running))
    ('done (setf (romacs-agent-state session) 'idle
                 (romacs-agent-reply session) (plist-get event :text)
                 (romacs-agent-failed session) (plist-get event :error))
           (cl-incf (romacs-agent-turns session))
           (setf (romacs-agent-done-seq session) (or (plist-get event :seq) 0))
           (romacs-agent--note-end session)
           (when (plist-get event :context-window)
             (setf (romacs-agent-context-window session) (plist-get event :context-window)))
           ;; Claude's cost is the session's so far, for queries; no view shows it.
           (when (numberp (plist-get event :cost))
             (setf (romacs-agent-cost session) (plist-get event :cost))))
    ('exit
     (unless (eq (romacs-agent-state session) 'idle)
       ;; It ended mid-turn: that turn failed.
       (setf (romacs-agent-failed session) t
             (romacs-agent-reply session) (format "the harness exited (%s) before its turn ended" (plist-get event :status))
             (romacs-agent-done-seq session) (or (plist-get event :seq) 0))
       (romacs-agent--note-end session))
     (setf (romacs-agent-state session) (if (romacs-agent--per-turn-p session) 'idle 'ended))))
  (run-hook-with-args 'romacs-agent-event-functions session event))

(defun romacs-agent--title-from (session text)
  "Name SESSION, whose title was generated, after its first prompt TEXT."
  (let ((line (string-trim (car (split-string (or text "") "\n" t "[ \t]+")) )))
    (unless (string-empty-p line)
      (setf (romacs-agent-title session) (truncate-string-to-width line 60 nil nil "…")
            (romacs-agent-title-auto session) nil)
      (romacs-agent--update-manifest session))))

(defun romacs-agent--update-manifest (session)
  "Write SESSION's title back to its manifest."
  (let* ((file (expand-file-name "manifest.json" (romacs-agent-dir session)))
         ;; Read with null and false kept: they are written back.
         (manifest (and (file-readable-p file)
                        (with-temp-buffer
                          (insert-file-contents file)
                          (ignore-errors (json-parse-buffer :object-type 'hash-table
                                                            :null-object :null :false-object :false))))))
    (when (and manifest (not (equal (gethash "title" manifest) (romacs-agent-title session))))
      (puthash "title" (romacs-agent-title session) manifest)
      (puthash "titleAuto" :false manifest)
      (romacs-agent--write-json file manifest))))

(defun romacs-agent--note-end (session)
  "Keep SESSION's turn end for its parent, when it has one."
  (when (plist-get (romacs-agent-options session) :parent)
    (push (list (romacs-agent-done-seq session) (romacs-agent-reply session) (romacs-agent-failed session))
          (romacs-agent-ends session))))

(defun romacs-agent--record (session record)
  "Handle one journal RECORD of SESSION."
  (let ((seq (gethash "seq" record)))
    (when (and (integerp seq) (> seq (romacs-agent-seq session)))
      (setf (romacs-agent-seq session) seq)
      (let ((out (gethash "out" record :absent))
            (in (gethash "in" record :absent))
            (exit (gethash "exit" record :absent)))
        (let ((events (cond ((hash-table-p out) (funcall (romacs-agent--harness-get session :events) session out))
                            ((stringp out) (list (list :type 'unknown :raw out)))
                            ((hash-table-p in) (romacs-agent--input-events session in))
                            ;; A per-turn harness's prompt, written as text.
                            ((not (eq in :absent))
                             (list (list :type 'input :text (if (stringp in) in (json-serialize in)))))
                            ((not (eq exit :absent)) (list (list :type 'exit :status exit))))))
          ;; :last marks a record's last event: a view that has shown it has
          ;; shown the whole record.
          (while events
            (romacs-agent--raise session (append (car events) (list :seq seq :last (null (cdr events))
                                                                    :time (gethash "t" record))))
            (setq events (cdr events))))))))

(defun romacs-agent--read-journal (session function &optional from)
  "Call FUNCTION with each record of SESSION's journal, in order, from
byte FROM (a record's start) when given."
  (let ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
    (when (file-readable-p journal)
      (with-temp-buffer
        (insert-file-contents journal nil from)
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
        (if (and (romacs-agent--per-turn-p session) (eq (romacs-agent-state session) 'idle))
            (romacs-agent--run-ended session)
          (unless (eq (romacs-agent-state session) 'ended)
            (setf (romacs-agent-state session) 'detached))
          (romacs-agent--raise session (list :type 'status :text "detached")))))))

(defun romacs-agent--attach (session &optional tries steal)
  "Attach to SESSION's supervisor and replay what this server has not seen.
Retried for a while: a new supervisor takes a moment to listen."
  (condition-case nil
      (let ((process (make-network-process
                      :name (format "romacs-agent-%s" (romacs-agent--key (romacs-agent-id session)))
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
     (cond ((romacs-agent--caught-up-p session)
            ;; It ended before this server attached: its journal says it all.
            (romacs-agent--run-ended session))
           ((> (or tries 0) 0)
            (run-at-time 0.1 nil #'romacs-agent--attach session (1- tries) steal))
           (t (setf (romacs-agent-state session) 'detached)
              (romacs-agent--raise session (list :type 'status :text "supervisor not reachable"))
              nil)))))

(defun romacs-agent--caught-up-p (session)
  "Replay what SESSION's last supervisor journaled since it started, unseen
here; non-nil when that ends with the harness's exit."
  (when (romacs-agent-journal-start session)
    (let ((ended nil))
      (romacs-agent--read-journal
       session (lambda (record)
                 (unless (eq (gethash "exit" record :absent) :absent) (setq ended t))
                 (romacs-agent--record session record))
       (romacs-agent-journal-start session))
      ended)))

(defun romacs-agent--run-ended (session)
  "A per-turn harness's run ended: the next waiting message starts a turn."
  (when (and (romacs-agent--per-turn-p session) (eq (romacs-agent-state session) 'idle))
    (let ((next (pop (romacs-agent-queue session))))
      (while (and next (nth 2 next) (not (funcall (nth 2 next))))
        (setq next (pop (romacs-agent-queue session))))
      (when next
        (romacs-agent--turn session (car next))
        (when (nth 1 next) (funcall (nth 1 next)))))))

;;; Lifecycle

(defun romacs-agent--manifest (session argv)
  (romacs-agent--write-json
   (expand-file-name "manifest.json" (romacs-agent-dir session))
   (romacs-mcp--object "id" (romacs-agent-id session) "harness" (symbol-name (romacs-agent-harness session))
                       "provider" (romacs-agent-provider session)
                       "configDir" (or (romacs-agent-config-dir session) :null)
                       "cwd" (romacs-agent-cwd session) "title" (romacs-agent-title session)
                       "titleAuto" (if (romacs-agent-title-auto session) t :false)
                       "imported" (let ((imported (romacs-agent-imported session)))
                                    (if imported (romacs-mcp--object "path" (plist-get imported :path)
                                                                     "size" (plist-get imported :size))
                                      :null))
                       ;; A per-turn harness's thread, known before any turn when imported.
                       "harnessSession" (or (romacs-agent-harness-session session) :null)
                       "created" (float-time) "argv" (vconcat argv)
                       "unit" (or (romacs-agent-unit session) :null)
                       "options" (romacs-agent--options-object (romacs-agent-options session))
                       "hostSocket" (romacs-agent-host-socket session))))

(defconst romacs-agent--option-keys
  '((:model . "model") (:effort . "effort") (:role . "role") (:parent . "parent") (:request . "clientRequestId"))
  "A session's start options and their manifest names.")

(defun romacs-agent--options-object (options)
  (let ((object (romacs-mcp--object)))
    (pcase-dolist (`(,key . ,name) romacs-agent--option-keys)
      (when (plist-get options key) (puthash name (plist-get options key) object)))
    object))

(defun romacs-agent--options-of (object)
  (and (hash-table-p object)
       (cl-loop for (key . name) in romacs-agent--option-keys
                when (gethash name object) append (list key (gethash name object)))))

(defun romacs-agent--environment-arguments ()
  "systemd-run arguments giving a unit this session's environment, by
name: systemd-run takes each value from its own environment, so none
shows on a command line other users can read."
  (cl-loop for entry in process-environment
           when (string-match "\\`\\([A-Za-z_][A-Za-z0-9_]*\\)=" entry)
           collect (concat "--setenv=" (match-string 1 entry))))

(defun romacs-agent--launch (session argv &optional input)
  "Run SESSION's supervisor with harness ARGV, as a user unit when a user
manager answers, else detached.  INPUT is a file the supervisor gives the
harness as its whole stdin."
  (let* ((host (romacs-agent--program "romacs-agent-host"))
         (command (append (list host "--dir" (romacs-agent-dir session)
                                "--socket" (romacs-agent-host-socket session))
                          (and input (list "--input" input))
                          (cons "--" argv)))
         (unit (format "romacs-agent-%s" (romacs-agent--key (romacs-agent-id session))))
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
(defun romacs-agent-start (cwd &optional prompt title provider options known)
  "Start a session in CWD under instance PROVIDER (a name in
`romacs-agent-providers', by default `romacs-agent-default-provider'),
sending PROMPT when given.  OPTIONS is a plist: :model and :effort for
the harness, :role, :parent (the delegating session's id) and :request
\(its client request id) for the record.  KNOWN, when given, is called
with the session before its harness starts.  Return the session."
  (interactive (let* ((cwd (read-directory-name "Agent directory: " default-directory))
                      (provider (romacs-agent--read-provider)))
                 (list cwd (read-string "Prompt (empty for none): ") nil provider)))
  (let* ((name (or provider (romacs-agent--default-provider)))
         (instance (or (romacs-agent--configured name)
                       (user-error "No agent provider %s in `romacs-agent-providers'" name)))
         (session (progn
                    ;; The directory as configured: the harness's default too, when named.
                    (romacs-agent--check-config-dir name (plist-get instance :config-dir))
                    (romacs-agent--create
                     :id (romacs-agent--uuid) :harness (plist-get instance :harness) :provider name :options options
                     :config-dir (romacs-agent--config-dir instance) :cwd (expand-file-name cwd)
                     ;; The project's name until the first prompt names it.
                     :title (or title (file-name-nondirectory (directory-file-name (expand-file-name cwd))))
                     :title-auto (null title)))))
    (when known (funcall known session))
    (let ((prompt (and prompt (not (string-empty-p prompt)) prompt)))
      (if (romacs-agent--per-turn-p session)
          ;; No harness runs until a prompt starts a turn.
          (progn (setf (romacs-agent-state session) 'idle)
                 (romacs-agent--manifest session nil)
                 (when prompt (romacs-agent-send session prompt)))
        (romacs-agent--launch session (romacs-agent--claude-argv session nil))
        (romacs-agent--attach session 50)
        (when prompt
          (romacs-agent--when-attached session (lambda () (romacs-agent-send session prompt))))))
    (when (called-interactively-p 'any) (romacs-agent-show session))
    session))

(defun romacs-agent--create (&rest fields)
  "A new session of FIELDS (`romacs-agent--make''s, :id among them),
known, with its private directory."
  (let* ((id (plist-get fields :id))
         (session (apply #'romacs-agent--make
                         :dir (expand-file-name id (romacs-agent--state-root))
                         :host-socket (expand-file-name (concat (romacs-agent--key id) "/host.sock")
                                                        (romacs-agent--runtime-root))
                         fields)))
    (make-directory (romacs-agent-dir session) t)
    (set-file-modes (romacs-agent-dir session) #o700)
    (puthash id session romacs-agent--sessions)
    (romacs-agent--index session)
    session))

(defun romacs-agent-import (harness thread transcript cwd title provider config-dir)
  "A session going on with HARNESS's own conversation THREAD, kept in its
TRANSCRIPT, in CWD, titled TITLE, under instance PROVIDER run with
CONFIG-DIR (nil: the harness's default).  Claude Code's session takes
THREAD's id and resumes it at once; Codex's resumes THREAD with its
first message.  Romacs only reads TRANSCRIPT."
  (unless (string-match-p "\\`[0-9a-f]\\{8\\}-[0-9a-f-]\\{27\\}\\'" thread)
    (user-error "Not a conversation id: %s" thread))
  (romacs-agent--check-config-dir provider config-dir)
  ;; The session is named by THREAD: one directory per conversation, so
  ;; every server finds the one that goes on with it.
  (let* ((dir (expand-file-name thread (romacs-agent--state-root)))
         (existing (or (romacs-agent-get thread) (romacs-agent--load dir))))
    (if existing
        (romacs-agent--continue existing)
      (make-directory (romacs-agent--state-root) t)
      ;; Made once: a second server importing it meanwhile is told so.
      (condition-case nil (make-directory dir)
        (file-already-exists (user-error "Another Romacs is resuming this conversation: try again")))
      (let ((session (romacs-agent--create
                      :id thread :harness harness :provider provider :config-dir config-dir :cwd cwd :title title
                      :harness-session (unless (eq harness 'claude) thread)
                      :imported (list :path transcript :size (file-attribute-size (file-attributes transcript))))))
        (if (romacs-agent--per-turn-p session)
            (progn (setf (romacs-agent-state session) 'idle)
                   (romacs-agent--manifest session nil))
          (romacs-agent--launch session (romacs-agent--claude-argv session t))
          (romacs-agent--attach session 50))
        session))))

(defun romacs-agent--continue (session)
  "SESSION, going on: attached to its supervisor when one runs, else
resumed; a per-turn harness goes on with its next message."
  (cond ((process-live-p (romacs-agent-connection session)))
        ((romacs-agent--per-turn-p session))
        ;; A stopped supervisor leaves its socket: only a connection says it listens.
        ((and (file-exists-p (romacs-agent-host-socket session)) (processp (romacs-agent--attach session 0))))
        (t (romacs-agent-resume session)))
  session)

(defun romacs-agent--when-attached (session function &optional tries)
  "Call FUNCTION once SESSION is attached."
  (if (process-live-p (romacs-agent-connection session))
      (funcall function)
    (when (> (or tries 100) 0)
      (run-at-time 0.1 nil #'romacs-agent--when-attached session function (1- (or tries 100))))))

(defun romacs-agent-send (session text &optional delivered valid)
  "Send TEXT to SESSION's harness as the user's next message, then call
DELIVERED, when given.  A per-turn harness's message waits while a turn
runs (DELIVERED waits with it), then starts the next unless VALID, when
given, then says it is no longer wanted."
  (cond ((not (romacs-agent--per-turn-p session))
         (romacs-agent--line session (romacs-agent--claude-user text))
         (when delivered (funcall delivered)))
        ((or (process-live-p (romacs-agent-connection session))
             (memq (romacs-agent-state session) '(starting running)))
         (romacs-agent--enqueue session (list text delivered valid)))
        (t (romacs-agent--turn session text)
           (when delivered (funcall delivered)))))

(defun romacs-agent--enqueue (session item)
  (let ((cell (list item)))
    (if (romacs-agent-queue session)
        (setcdr (romacs-agent-queue-tail session) cell)
      (setf (romacs-agent-queue session) cell))
    (setf (romacs-agent-queue-tail session) cell)))

(defun romacs-agent-interrupt (session)
  "Interrupt SESSION's turn; queued messages stay.  A per-turn harness's
run ends (its supervisor journals the exit): the next message continues
its thread."
  (if (romacs-agent--per-turn-p session)
      ;; The harness alone: the supervisor goes on to journal its exit.
      (when-let* (((process-live-p (romacs-agent-connection session)))
                  (pid (romacs-agent-harness-pid session)))
        (signal-process pid 'TERM))
    (romacs-agent--line session (romacs-mcp--object
                               "type" "control_request"
                               "request_id" (format "romacs-interrupt-%d" (cl-incf (romacs-agent-requests session)))
                               "request" (romacs-mcp--object "subtype" "interrupt")))))

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
  (cond ((romacs-agent-unit session)
         (call-process "systemctl" nil 0 nil "--user" "stop" (romacs-agent-unit session)))
        ;; Detached, only while attached: the pids are then the live
        ;; supervisor's, not ones an ended one left for others to reuse.
        ((process-live-p (romacs-agent-connection session))
         (when-let* ((pid (romacs-agent--host-pid session))) (signal-process pid 'TERM))
         (when-let* ((pid (romacs-agent-harness-pid session))) (signal-process pid 'TERM)))
        ;; Just launched: stopped once attached.
        ((eq (romacs-agent-state session) 'starting)
         (romacs-agent--when-attached session (lambda () (romacs-agent-stop session))))))

(defun romacs-agent-resume (session)
  "Run SESSION's harness again on its conversation, after it ended, in the
configuration directory its instance had when it started: the harness
keeps the conversation there."
  (when (process-live-p (romacs-agent-connection session))
    (user-error "Session %s is still attached" (romacs-agent-title session)))
  (when (romacs-agent--per-turn-p session)
    (user-error "Session %s continues with its next message" (romacs-agent-title session)))
  (romacs-agent--check-config-dir (romacs-agent-provider session) (romacs-agent-config-dir session))
  ;; A turn the harness's end cut short goes on (Claude Code's own switch).
  (let ((process-environment (cons "CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1" process-environment)))
    (romacs-agent--launch session (romacs-agent--claude-argv session t)))
  (setf (romacs-agent-state session) 'starting)
  (romacs-agent--attach session 50))

;;; One question

(defvar romacs-agent--asks (make-hash-table :test #'equal)
  "A one-shot session's id to its `romacs-agent-ask' handler.")

(defun romacs-agent-ask (prompt resolve reject &rest options)
  "Ask PROMPT of a one-shot session; call RESOLVE with its reply, or
REJECT with why it has none, once.  OPTIONS: :provider (an instance of
`romacs-agent-providers'), :model and :effort (the instance's and the
harness's own defaults when absent), :cwd (default a private empty
directory, removed after) and :title.  The session stops once its first
turn ends.  Return a function that cancels: it stops the session, and
neither callback runs after it."
  (let* ((own (not (plist-get options :cwd)))
         (cwd (or (plist-get options :cwd) (make-temp-file "romacs-agent-ask-" t)))
         (session nil) (settled nil))
    (cl-flet ((settle (function value)
                (unless settled
                  (setq settled t)
                  (when session
                    (remhash (romacs-agent-id session) romacs-agent--asks)
                    (ignore-errors (romacs-agent-stop session)))
                  (when own (ignore-errors (delete-directory cwd t)))
                  (when function (funcall function value)))))
      (let ((handler (lambda (event)
                       (pcase (plist-get event :type)
                         ('done (if (or (plist-get event :error) (not (stringp (plist-get event :text))))
                                    (settle reject (or (plist-get event :text) "the harness answered nothing"))
                                  (settle resolve (plist-get event :text))))
                         ('exit (settle reject (format "the harness exited (%s) before answering"
                                                       (plist-get event :status))))))))
        (condition-case err
            (romacs-agent-start cwd prompt (or (plist-get options :title) "ask") (plist-get options :provider)
                                (list :model (plist-get options :model) :effort (plist-get options :effort)
                                      :role "ask")
                                ;; Known before it launches: a fast run's end is not missed.
                                (lambda (new)
                                  (setq session new)
                                  (puthash (romacs-agent-id new) handler romacs-agent--asks)))
          (error (settle nil nil) (signal (car err) (cdr err)))))
      (lambda () (settle nil nil)))))

(defun romacs-agent--ask-event (session event)
  (when-let* ((handler (gethash (romacs-agent-id session) romacs-agent--asks)))
    (funcall handler event)))

(add-hook 'romacs-agent-event-functions #'romacs-agent--ask-event)

;;; A new server

(defun romacs-agent--load (dir)
  "The session recorded in DIR, attached when its supervisor runs."
  (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
    (when (and manifest (not (gethash (gethash "id" manifest) romacs-agent--sessions)))
      (let ((session (romacs-agent--make
                      :id (gethash "id" manifest) :harness (intern (gethash "harness" manifest))
                      ;; Sessions recorded before instances ran the harness's default.
                      :provider (or (gethash "provider" manifest) (gethash "harness" manifest))
                      :config-dir (gethash "configDir" manifest)
                      :options (romacs-agent--options-of (gethash "options" manifest))
                      :imported (let ((imported (gethash "imported" manifest)))
                                  (and (hash-table-p imported)
                                       (list :path (gethash "path" imported) :size (gethash "size" imported))))
                      :harness-session (gethash "harnessSession" manifest)
                      :cancelled (file-exists-p (expand-file-name "cancelled" dir))
                      ;; Should attaching fail, the journal is read from its start.
                      :journal-start 0
                      :cwd (gethash "cwd" manifest) :title (gethash "title" manifest)
                      ;; Earlier records named a session "<dir> <id>": generated.
                      :title-auto (let ((auto (gethash "titleAuto" manifest :absent)))
                                    (if (eq auto :absent)
                                        (string-suffix-p (romacs-agent--short (gethash "id" manifest))
                                                         (or (gethash "title" manifest) ""))
                                      (eq auto t)))
                      :dir dir :unit (gethash "unit" manifest)
                      :host-socket (gethash "hostSocket" manifest) :state 'detached)))
        (puthash (romacs-agent-id session) session romacs-agent--sessions)
        (romacs-agent--index session)
        (if (file-exists-p (romacs-agent-host-socket session))
            (romacs-agent--attach session 0)
          ;; No supervisor: what the journal says is all there is.
          (let ((romacs-agent-event-functions nil))
            (romacs-agent--read-journal session (lambda (record) (romacs-agent--record session record))))
          (unless (or (eq (romacs-agent-state session) 'ended)
                      ;; A per-turn harness between turns is idle, not detached.
                      (and (romacs-agent--per-turn-p session) (eq (romacs-agent-state session) 'idle)))
            (setf (romacs-agent-state session) 'detached)))
        session))))

;;;###autoload
(defvar romacs-agent-restored-hook nil
  "Run after `romacs-agent-restore' has loaded every recorded session.")

(defun romacs-agent-restore ()
  "Know every recorded session, and attach to those whose supervisor runs.
A server runs this at its start, so sessions survive it."
  (when (file-directory-p (romacs-agent--state-root))
    (dolist (dir (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-"))
      (romacs-agent--load dir)))
  (run-hooks 'romacs-agent-restored-hook))

(defun romacs-agent-sessions ()
  "Every known session, newest first."
  (sort (hash-table-values romacs-agent--sessions)
        (lambda (a b) (string> (or (romacs-agent-dir a) "") (or (romacs-agent-dir b) "")))))

(defun romacs-agent-get (id)
  "The session ID names."
  (gethash id romacs-agent--sessions))

(defvar romacs-agent--requests (make-hash-table :test #'equal)
  "A delegating session's id and a client request id, to the session it
started: a request repeated after a lost reply finds its session.")

(defun romacs-agent--index (session)
  (let ((options (romacs-agent-options session)))
    (when (plist-get options :request)
      (puthash (cons (plist-get options :parent) (plist-get options :request))
               (romacs-agent-id session) romacs-agent--requests))))

(defun romacs-agent-requested (parent request)
  "The session PARENT started with client request id REQUEST, or nil."
  (when-let* ((id (gethash (cons parent request) romacs-agent--requests))) (romacs-agent-get id)))

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

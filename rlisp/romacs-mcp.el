;;; romacs-mcp.el --- Native owner MCP endpoint -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Explicitly start a dedicated private Unix endpoint, separate from server.el.
;; NDJSON is a custom local transport; the romacs-mcp stdio relay copies bytes.
;; Owner eval is unrestricted, synchronous and never promises rollback or CPU
;; preemption.  Filters frame and enqueue; one guarded timer admits tool calls.
;; See doc/misc/romacs-mcp.md for limits and the dual-era protocol contract.

;;; Code:
(require 'cl-lib)
(require 'json)
(require 'server)

(defgroup romacs-mcp nil
  "Native owner-controlled MCP endpoint."
  :group 'external)

(defcustom romacs-mcp-full-access t
  "Non-nil means MCP clients may evaluate arbitrary Lisp in this editor.
When non-nil, the `romacs_eval' tool is listed and callable.  It runs
code with the same privileges as the user running the editor, without
a sandbox or confirmation prompt.  When nil, that tool is neither
listed nor callable.  Other tools are not affected."
  :type 'boolean :group 'romacs-mcp)

(defcustom romacs-mcp-send-timeout 0.25
  "Seconds before closing an owned peer whose response send has not returned."
  :type 'number :group 'romacs-mcp)

(defconst romacs-mcp--frame-limit 131072)
(defconst romacs-mcp--output-limit 131072
  "Maximum bytes of one response line, including its newline.")
(defconst romacs-mcp--id-limit 1024
  "Maximum bytes of a JSON-encoded request ID.
Replies echo the ID, so bounding it keeps every error reply bounded.")
(defconst romacs-mcp--queue-limit 64)
(defconst romacs-mcp--peer-queue-limit 16)
(defconst romacs-mcp--peer-limit 32
  "Connections at once: clients, and one relay per agent session.")
(defconst romacs-mcp--scan-limit 8)
(defconst romacs-mcp--modern "2026-07-28")
(defconst romacs-mcp--legacy "2025-11-25")
(defconst romacs-mcp--full-access-tools '("romacs_eval")
  "Tools available only while `romacs-mcp-full-access' is non-nil.")
(defconst romacs-mcp--legacy-versions '("2025-11-25" "2025-06-18")
  "Supported handshake versions, newest first; both use legacy envelopes.")
(defvar romacs-mcp--boot nil)
(defvar romacs-mcp--generation 0)
(defvar romacs-mcp--listener nil)
(defvar romacs-mcp--socket nil)
(defvar romacs-mcp--socket-identity nil)
(defvar romacs-mcp--token nil
  "The line a loopback peer must send first, or nil for a local socket.")
(defvar romacs-mcp--peers nil)
(defvar romacs-mcp--queue nil)
(defvar romacs-mcp--queued (make-hash-table :test #'equal)
  "Each queued request with an id, by (PEER . ID): a relay withdraws one
without a scan.")

(defun romacs-mcp--unqueue (request)
  "REQUEST leaves the queue: forget it in the index."
  (when-let* ((message (plist-get request :message))
              ((hash-table-p message))
              (id (gethash "id" message nil)))
    (remhash (cons (plist-get request :peer) id) romacs-mcp--queued)))
(defvar romacs-mcp--active nil)
(defvar romacs-mcp--timer nil)
(defvar romacs-mcp--deferred nil
  "Admitted requests awaiting a deferred result, as (REQUEST ID POLL MODERN).")
(defvar romacs-mcp--deferred-timer nil)
(defvar romacs-mcp--deferred-index (make-hash-table :test #'equal)
  "(PEER . ID) of each deferred request to its entry, for withdrawal.")
(defvar romacs-mcp-tools nil
  "Plain alist of NAME and tool plist.
Each plist has :description, :schema, :handler and optional :annotations.
Use `romacs-mcp-register-tool' to replace an entry.  Handlers receive a JSON
object hash table and return a JSON value.  Changes are visible on next list;
no listChanged capability or subscriptions are advertised.")

(defvar romacs-mcp-methods (make-hash-table :test #'equal)
  "JSON-RPC methods beyond MCP's, by name: each a function of the peer and
the params object returning the result, or `romacs-mcp-hold''s handle's
promise to answer later.  They need no MCP initialization: clients of the
native endpoint (`romacs-server') call them directly.")
(defvar romacs-mcp-call-function (lambda (_peer _id _name run) (funcall run))
  "How a tool call runs: a function of the peer, the request id, the tool's
name and a function of no arguments that runs it and returns its value.
romacs-agent-tools.el records each session's calls through it.")
(defvar romacs-mcp-peer-close-functions nil
  "Functions called with a peer whose connection is retired.")

(define-error 'romacs-mcp-protocol-error "MCP protocol error")

(defun romacs-mcp-register-method (name handler)
  "Answer JSON-RPC method NAME with HANDLER, a function of peer and params."
  (puthash name handler romacs-mcp-methods))

(defun romacs-mcp-hold ()
  "From a method handler: answer this request later, and return its handle.
The handler's value is then ignored; `romacs-mcp-reply' answers it.  A
handle whose peer has gone answers nothing."
  (unless romacs-mcp--active (error "No request is being admitted"))
  (setf (plist-get romacs-mcp--active :held) t)
  romacs-mcp--active)

(defun romacs-mcp-reply (handle result &optional error-message)
  "Answer the held request HANDLE with RESULT, or with ERROR-MESSAGE."
  (let ((id (gethash "id" (plist-get handle :message) :null)))
    (romacs-mcp--send
     handle
     (if error-message
         (romacs-mcp--object "jsonrpc" "2.0" "id" id
                             "error" (romacs-mcp--object "code" -32000 "message" error-message))
       (romacs-mcp--object "jsonrpc" "2.0" "id" id "result" result)))))

(defun romacs-mcp--object (&rest pairs)
  "Return a JSON object populated by alternating key/value PAIRS."
  (let ((object (make-hash-table :test #'equal)))
    (while pairs (puthash (pop pairs) (pop pairs) object))
    object))

(defun romacs-mcp--fail (code message &optional data)
  "Signal protocol CODE with MESSAGE and optional DATA."
  (signal 'romacs-mcp-protocol-error (list code message data)))

(defun romacs-mcp-identity ()
  "Return the stable process identity, independent of endpoint restarts."
  (unless romacs-mcp--boot
    (setq romacs-mcp--boot
          (format "%s:%s:%s" (emacs-pid) (float-time) (random))))
  (romacs-mcp--object "instance" romacs-mcp--boot "pid" (emacs-pid)
                       "runtime" emacs-version "serverName" server-name
                       "endpointGeneration" romacs-mcp--generation))

(defun romacs-mcp--instance (arguments)
  "Refuse when ARGUMENTS name another process incarnation.  A call that
names none runs on the Romacs it reaches; naming one (romacs_identity)
refuses a call meant for a Romacs that has since restarted."
  (let ((given (gethash "instance" arguments :absent)))
    (unless (or (eq given :absent) (equal given (gethash "instance" (romacs-mcp-identity))))
      (user-error "Romacs instance mismatch"))))

(defun romacs-mcp-register-tool (name description schema handler &optional annotations)
  "Register a named tool with a JSON schema and handler.
NAME identifies the tool; DESCRIPTION documents it (a string, or a function
of no arguments giving it as things are when the tools are listed); SCHEMA
describes inputs.
HANDLER receives a JSON object and returns a JSON value.  Optional ANNOTATIONS
are descriptive hints, not permissions.  Replace the same name
 deterministically."
  (unless (and (stringp name) (string-match-p "\\`[A-Za-z0-9_.-]+\\'" name)
               (<= (length name) 128) (or (stringp description) (functionp description))
               (hash-table-p schema) (equal (gethash "type" schema) "object")
               (functionp handler))
    (error "Invalid MCP tool registration"))
  (setf (alist-get name romacs-mcp-tools nil nil #'equal)
        (list :description description :schema schema :handler handler
              :annotations annotations)))

(defun romacs-mcp--schema (properties required)
  "Return a closed object schema from PROPERTIES and REQUIRED field names."
  (romacs-mcp--object
   "type" "object" "properties"
   (apply #'romacs-mcp--object
          (cl-loop for (name . type) in properties
                   append (list name (romacs-mcp--object "type" type))))
   "required" (vconcat required) "additionalProperties" :false))

(defun romacs-mcp--arguments (schema arguments)
  "Validate ARGUMENTS against the supported object subset of SCHEMA.
Require fields, primitive types and closed properties; return ARGUMENTS.
Custom handlers own additional constraints, including nested JSON Schema."
  (unless (hash-table-p arguments) (romacs-mcp--fail -32602 "Arguments must be an object"))
  (mapc (lambda (key)
          (unless (not (eq (gethash key arguments :absent) :absent))
            (romacs-mcp--fail -32602 (concat "Missing argument: " key))))
        (gethash "required" schema))
  (maphash
   (lambda (key value)
     (let* ((property (gethash key (gethash "properties" schema)))
            (type (and property (gethash "type" property))))
       (when (and (not property) (eq (gethash "additionalProperties" schema) :false))
         (romacs-mcp--fail -32602 (concat "Unknown argument: " key)))
       (unless (pcase type
                 ("string" (stringp value)) ("integer" (integerp value))
                 ("number" (numberp value)) ("object" (hash-table-p value))
                 ("array" (vectorp value))
                 ("boolean" (memq value '(t :false))) (_ t))
         (romacs-mcp--fail -32602 (concat "Wrong argument type: " key)))))
   arguments)
  arguments)

(defun romacs-mcp--eval (arguments)
  "Evaluate all Lisp forms in ARGUMENTS after the instance fence.
Return a bounded printed value; effects are not rolled back on failure."
  (romacs-mcp--instance arguments)
  (romacs-mcp-eval-string (gethash "code" arguments)))

(defun romacs-mcp-eval-string (code)
  "Evaluate every form in the string CODE and return the last value printed.
The printed value is bounded; effects are not rolled back on failure."
  (let* ((wrapped (concat "(progn\n" code "\n)"))
         (parsed (read-from-string wrapped))
         (form (car parsed))
         (_complete
          (unless (= (cdr parsed) (length wrapped))
            (error "Malformed Lisp input; trailing forms outside wrapper")))
         (value (eval form t))
         (print-length 64) (print-level 16) (print-circle t)
         (print-escape-newlines t))
    (let ((text (prin1-to-string value)))
      (if (> (string-bytes text) 65536)
          (error "Eval result exceeds output limit; effects may have occurred")
        text))))

(defconst romacs-mcp-instructions
  "Romacs, a running Emacs-compatible editor. Start with romacs_identity \
(the instance a fenced call names), romacs_observe (what the frame shows) and \
romacs_eval (Emacs Lisp in the editor). Others: buffers and files \
(romacs_buffer_list/read/edit, romacs_file_write, romacs_visit); input \
(romacs_send_keys, romacs_wait_idle); agents (romacs_delegate_task, \
romacs_task_status/cancel, romacs_orchestrator_capabilities); restart \
(romacs_restart_request, romacs_call_outcome); gnosis_* (spaced-repetition \
practice and review). romacs_help lists them all, one line each."
  "What an MCP client is told at initialization: the entry points and the
tools' groups, short, since every session pays for it.")

(defconst romacs-mcp--tool-groups
  '(("core" "romacs_identity" "romacs_observe" "romacs_eval" "romacs_help")
    ("buffers and files" "romacs_buffer_list" "romacs_buffer_read" "romacs_buffer_edit" "romacs_file_write"
     "romacs_visit")
    ("input" "romacs_send_keys" "romacs_wait_idle")
    ("agents" "romacs_delegate_task" "romacs_task_status" "romacs_task_cancel"
     "romacs_orchestrator_capabilities")
    ("restart" "romacs_restart_request" "romacs_call_outcome")
    ("gnosis" . "gnosis_"))
  "The tools' groups for `romacs_help': names, or a prefix; the rest are other.")

(defun romacs-mcp--description (tool)
  "TOOL's description: a string, or a function giving it as things are now."
  (let ((description (plist-get tool :description)))
    (if (functionp description) (funcall description) description)))

(defun romacs-mcp--help (_arguments)
  "The tools offered, by group, each with its description's first sentence."
  (let ((groups (mapcar (lambda (g) (cons (car g) nil)) romacs-mcp--tool-groups))
        (other nil))
    (dolist (entry (romacs-mcp--available-tools))
      (let* ((name (car entry))
             (line (car (split-string (romacs-mcp--description (cdr entry)) "\\.\\( \\|$\\)\\|\n")))
             (group (car (cl-find-if (lambda (g) (if (stringp (cdr g)) (string-prefix-p (cdr g) name)
                                                    (member name (cdr g))))
                                     romacs-mcp--tool-groups))))
        (if group (push (cons name line) (alist-get group groups nil nil #'equal))
          (push (cons name line) other))))
    (mapconcat (lambda (g)
                 (concat (car g) ":\n"
                         (mapconcat (lambda (tool) (format "  %s  %s" (car tool) (cdr tool)))
                                    (sort (cdr g) (lambda (a b) (string< (car a) (car b)))) "\n")))
               (cl-remove-if-not #'cdr (append groups (and other (list (cons "other" other)))))
               "\n")))

(defun romacs-mcp--available-tools ()
  "Return the registered tools that clients may currently list and call."
  (if romacs-mcp-full-access
      romacs-mcp-tools
    (cl-remove-if (lambda (entry) (member (car entry) romacs-mcp--full-access-tools))
                  romacs-mcp-tools)))

(defun romacs-mcp--tool-list ()
  "Return deterministic descriptors for the registered tools."
  (vconcat
   (mapcar (lambda (entry)
             (let* ((tool (cdr entry))
                    (object (romacs-mcp--object
                             "name" (car entry) "description" (romacs-mcp--description tool)
                             "inputSchema" (plist-get tool :schema))))
               (when (plist-get tool :annotations)
                 (puthash "annotations" (plist-get tool :annotations) object))
               object))
           (sort (copy-sequence (romacs-mcp--available-tools))
                 (lambda (a b) (string-lessp (car a) (car b)))))))

(defun romacs-mcp-defer (poll)
  "Return a tool result that POLL supplies later, for a handler to return.
The endpoint answers other requests meanwhile and the command loop keeps
running: a timer calls POLL with no arguments every 0.05 seconds until it
returns non-nil, a list whose car is the tool's value.  An error POLL
signals is the tool's error.  A closed or cancelled request stops polling."
  (unless (functionp poll) (error "Invalid deferred MCP poll"))
  (list 'romacs-mcp--deferred poll))

(defun romacs-mcp--valid-char-p (char)
  "Whether CHAR is a Unicode scalar value (JSON can carry it)."
  (and (<= char #x10FFFF) (not (<= #xD800 char #xDFFF))))

(defun romacs-mcp--unicode (text)
  "TEXT as JSON can carry it: TEXT itself when it serializes (a native
check), else with each raw byte, surrogate or character past Unicode
replaced by U+FFFD, one for one, so a buffer's coordinates in a tool's
result stay true.  A unibyte TEXT is bytes, one character each, as its
buffer counts them: a byte past ASCII is a raw byte."
  (let ((text (if (multibyte-string-p text) text (string-to-multibyte text))))
    (if (ignore-errors (json-serialize text) t)
        text
      (concat (mapcar (lambda (char) (if (romacs-mcp--valid-char-p char) char #xFFFD)) text)))))

(defun romacs-mcp--unicode-tree (value &optional depth)
  "VALUE with every string in it, a key too, as `romacs-mcp--unicode'
makes it: hash tables, vectors, alists and plists.  A circular list, or
nesting deeper than JSON's limit, is left as it is: serializing it is
refused as before."
  (let ((depth (or depth 0)))
    (cond ((> depth 10000) value)
          ((stringp value) (romacs-mcp--unicode value))
          ((vectorp value) (vconcat (mapcar (lambda (item) (romacs-mcp--unicode-tree item (1+ depth))) value)))
          ((hash-table-p value)
           (let ((copy (make-hash-table :test (hash-table-test value) :size (hash-table-count value))))
             (maphash (lambda (key item)
                        (puthash (romacs-mcp--unicode-tree key (1+ depth))
                                 (romacs-mcp--unicode-tree item (1+ depth)) copy))
                      value)
             copy))
          ((and (consp value) (condition-case nil (progn (length value) nil) (circular-list t) (error nil)))
           value)
          ((consp value)
           (let ((items nil) (tail value))
             (while (consp tail)
               (push (romacs-mcp--unicode-tree (car tail) (1+ depth)) items)
               (setq tail (cdr tail)))
             (nconc (nreverse items) (romacs-mcp--unicode-tree tail (1+ depth)))))
          (t value))))

(defun romacs-mcp--unicode-value (value)
  "VALUE, a JSON value, as JSON can carry it: VALUE itself when it
serializes, else with its strings made Unicode (`romacs-mcp--unicode-tree')."
  (if (ignore-errors (json-serialize value :false-object :false :null-object :null) t)
      value
    (romacs-mcp--unicode-tree value)))

(defun romacs-mcp--result (value failed modern)
  "Return the MODERN or legacy tool envelope for VALUE, an error if FAILED.
Its text is Unicode: a raw byte in VALUE becomes U+FFFD, not a refusal."
  (let* ((text (romacs-mcp--unicode
                (if (stringp value) value
                  (decode-coding-string
                   (json-serialize (romacs-mcp--unicode-value value) :false-object :false :null-object :null)
                   'utf-8))))
         (result (romacs-mcp--object
                  "content" (vector (romacs-mcp--object "type" "text" "text" text))
                  "isError" (if failed t :false))))
    (when modern (puthash "resultType" "complete" result))
    result))

(defun romacs-mcp--call (params modern)
  "Call the tool named by PARAMS and return the MODERN or legacy envelope.
A handler's `romacs-mcp-defer' value is returned with MODERN appended."
  (let* ((tool (alist-get (gethash "name" params) (romacs-mcp--available-tools)
                          nil nil #'equal))
         (arguments (gethash "arguments" params (romacs-mcp--object))))
    (unless tool (romacs-mcp--fail -32602 "Unknown tool"))
    (romacs-mcp--arguments (plist-get tool :schema) arguments)
    (let* ((failed nil)
           (value (condition-case failure
                      ;; Quit is the call's error (`with-local-quit' would
                      ;; return nil for it): C-g or a deadline tells the caller.
                      (let ((inhibit-quit nil))
                        (funcall romacs-mcp-call-function
                                 (plist-get romacs-mcp--active :peer)
                                 (gethash "id" (plist-get romacs-mcp--active :message) nil)
                                 (gethash "name" params)
                                 (lambda () (funcall (plist-get tool :handler) arguments))))
                    ((error quit)
                     (setq failed t)
                     (error-message-string failure)))))
      (if (eq (car-safe value) 'romacs-mcp--deferred)
          (append value (list modern))
        (romacs-mcp--result value failed modern)))))

(defun romacs-mcp--modern-p (params)
  "Validate per-request modern metadata in PARAMS and return non-nil."
  (let* ((meta (gethash "_meta" params))
         (version (and (hash-table-p meta) (gethash "io.modelcontextprotocol/protocolVersion" meta))))
    (unless (and (stringp version)
                 (hash-table-p (gethash "io.modelcontextprotocol/clientCapabilities" meta)))
      (romacs-mcp--fail -32602 "Required protocol metadata is missing"))
    (unless (equal version romacs-mcp--modern)
      (romacs-mcp--fail -32022 "Unsupported protocol version"
                         (romacs-mcp--object
                          "supported" (vconcat (list romacs-mcp--modern)
                                              romacs-mcp--legacy-versions)
                          "requested" version)))
    t))

(defun romacs-mcp--dispatch (peer message)
  "Dispatch validated MESSAGE from PEER outside process filters."
  (let* ((method (gethash "method" message))
         (params (gethash "params" message (romacs-mcp--object)))
         (handler (gethash method romacs-mcp-methods))
         (meta (and (not handler) (gethash "_meta" params)))
         ;; Legacy progress and extension metadata do not select a new era.
         (modern (and (not handler) (not (equal method "initialize"))
                      (or (and (hash-table-p meta)
                               (cl-some
                                (lambda (key) (not (eq (gethash key meta :absent) :absent)))
                                '("io.modelcontextprotocol/protocolVersion"
                                  "io.modelcontextprotocol/clientCapabilities"
                                  "io.modelcontextprotocol/clientInfo")))
                          (not (process-get peer 'legacy)))
                      (romacs-mcp--modern-p params))))
    (if handler
        ;; A method's error is its reply: the client prints it.
        (condition-case failure
            (funcall handler peer params)
          (romacs-mcp-protocol-error (signal (car failure) (cdr failure)))
          ((error quit) (romacs-mcp--fail -32000 (error-message-string failure))))
    (pcase method
      ("initialize"
       (let ((version (gethash "protocolVersion" params)))
         (unless (and (stringp version) (> (length version) 0)
                      (hash-table-p (gethash "capabilities" params))
                      (hash-table-p (gethash "clientInfo" params))
                      (not (process-get peer 'legacy)))
           (romacs-mcp--fail -32602 "Expected fresh legacy initialization"))
         (process-put peer 'legacy 'initializing)
         ;; Echo supported offers; otherwise propose the newest handshake era.
         ;; Stateless modern requests never negotiate through initialize.
         (romacs-mcp--object
          "protocolVersion" (if (member version romacs-mcp--legacy-versions)
                                version romacs-mcp--legacy)
          "capabilities" (romacs-mcp--object "tools" (romacs-mcp--object))
          "serverInfo" (romacs-mcp--object "name" "Romacs" "version" "1")
          "instructions" romacs-mcp-instructions)))
      ("notifications/initialized"
       (unless (eq (process-get peer 'legacy) 'initializing)
         (romacs-mcp--fail -32600 "Unexpected initialized notification"))
       (process-put peer 'legacy 'ready) nil)
      (_
       (unless (or modern (eq (process-get peer 'legacy) 'ready))
         (romacs-mcp--fail -32600 "Legacy initialization is incomplete"))
       (pcase method
         ("ping" (if modern (romacs-mcp--object "resultType" "complete")
                   (romacs-mcp--object)))
         ("server/discover"
          (unless modern (romacs-mcp--fail -32601 "Method not found"))
          (romacs-mcp--object
           "resultType" "complete" "supportedVersions"
           (vconcat (list romacs-mcp--modern) romacs-mcp--legacy-versions)
           "capabilities" (romacs-mcp--object "tools" (romacs-mcp--object))
           "_meta" (romacs-mcp--object "io.modelcontextprotocol/serverInfo"
                                       (romacs-mcp--object "name" "Romacs" "version" "1"))
           "instructions" romacs-mcp-instructions
           "ttlMs" 0 "cacheScope" "private"))
         ("tools/list"
          (when (gethash "cursor" params) (romacs-mcp--fail -32602 "No pagination cursor is supported"))
          (let ((result (romacs-mcp--object "tools" (romacs-mcp--tool-list))))
            (when modern
              (puthash "resultType" "complete" result)
              (puthash "ttlMs" 0 result) (puthash "cacheScope" "private" result))
            result))
         ("tools/call" (romacs-mcp--call params modern))
         (_ (romacs-mcp--fail -32601 "Method not found"))))))))

(defun romacs-mcp--live-p (peer generation)
  "Return non-nil if PEER still belongs to endpoint GENERATION."
  (and (= generation romacs-mcp--generation)
       (memq peer romacs-mcp--peers) (process-live-p peer)))

(defvar romacs-mcp--closing nil
  "Peers whose close functions are running: a function that closes its
peer again does not run them twice.")

(defun romacs-mcp--close (peer)
  "Retire only PEER, its queue entries and owned send timer."
  (dolist (request romacs-mcp--queue)
    (when (eq peer (plist-get request :peer)) (romacs-mcp--unqueue request)))
  (setq romacs-mcp--peers (delq peer romacs-mcp--peers)
        romacs-mcp--queue
        (cl-remove peer romacs-mcp--queue :key (lambda (request) (plist-get request :peer))))
  (when-let* ((timer (process-get peer 'send-timer)))
    (cancel-timer timer) (process-put peer 'send-timer nil))
  (when (and romacs-mcp--active (eq peer (plist-get romacs-mcp--active :peer)))
    (setf (plist-get romacs-mcp--active :cancelled) t))
  (unless (memq peer romacs-mcp--closing)
    (let ((romacs-mcp--closing (cons peer romacs-mcp--closing)))
      (run-hook-with-args 'romacs-mcp-peer-close-functions peer)))
  ;; Like `delete-process', never kill PEER's buffer: the listener's
  ;; filter gives connections none, so any buffer is not ours.
  (when (process-live-p peer) (delete-process peer)))

(defun romacs-mcp--sentinel (peer _event)
  "Retire PEER when its native transport closes; ignore EVENT."
  (unless (process-live-p peer) (romacs-mcp--close peer)))

(defun romacs-mcp--encode (response)
  "Return RESPONSE encoded as JSON, or nil if it cannot be encoded."
  (condition-case nil
      (json-serialize response :false-object :false :null-object :null)
    (error nil)))

(defun romacs-mcp--wire (response)
  "Return RESPONSE as one line of JSON within `romacs-mcp--output-limit'.
If RESPONSE cannot be encoded, for example because it contains raw
bytes, or exceeds the limit, return a fixed-size error for its ID
instead, or for a null ID if even that error would not fit."
  (let* ((json (romacs-mcp--encode response))
         (message (cond ((null json) "Response cannot be encoded as JSON")
                        ((>= (string-bytes json) romacs-mcp--output-limit)
                         "Response exceeds the output limit"))))
    (when message
      (setq json (romacs-mcp--encode
                  (romacs-mcp--object
                   "jsonrpc" "2.0" "id" (gethash "id" response :null)
                   "error" (romacs-mcp--object "code" -32603 "message" message))))
      (unless (and json (< (string-bytes json) romacs-mcp--output-limit))
        (setq json (json-serialize
                    (romacs-mcp--object
                     "jsonrpc" "2.0" "id" :null
                     "error" (romacs-mcp--object "code" -32603 "message" message))))))
    (concat json "\n")))

(defun romacs-mcp--send (request response)
  "Bound serialization and sending RESPONSE for the exact REQUEST lifetime.
A response that cannot be encoded or exceeds the output limit is replaced
by an error for the same request.  Normal send return is not delivery
acknowledgement.  Close an unread peer."
  (let* ((peer (plist-get request :peer))
         (generation (plist-get request :generation))
         (wire (romacs-mcp--wire response)))
    (when (and (not (plist-get request :cancelled)) (romacs-mcp--live-p peer generation))
      (let ((timer (run-at-time
                    romacs-mcp-send-timeout nil
                    (lambda ()
                      (when (romacs-mcp--live-p peer generation)
                        (romacs-mcp--close peer))))))
        (process-put peer 'send-timer timer)
        (unwind-protect
            (condition-case nil
                (progn (process-send-string peer wire)
                       (romacs-mcp--live-p peer generation))
              (error (romacs-mcp--close peer)))
          (cancel-timer timer)
          (when (eq timer (process-get peer 'send-timer))
            (process-put peer 'send-timer nil)))))))

(defun romacs-mcp--unindex (entry)
  (let ((key (cons (plist-get (car entry) :peer) (nth 1 entry))))
    (when (eq (gethash key romacs-mcp--deferred-index) entry) (remhash key romacs-mcp--deferred-index))))

(defun romacs-mcp--poll-deferred ()
  "Answer each deferred request whose poll has a result; drop dead ones.
One pass: the entries that stay are kept as it goes, so retiring many is
linear."
  (let ((entries romacs-mcp--deferred) (generation romacs-mcp--generation) (kept nil))
    (dolist (entry entries)
      (pcase-let ((`(,request ,id ,poll ,modern) entry))
        (if (or (plist-get request :cancelled)
                (not (romacs-mcp--live-p (plist-get request :peer)
                                         (plist-get request :generation))))
            (romacs-mcp--unindex entry)
          (let* ((failed nil)
                 (done (condition-case failure
                           (with-local-quit (funcall poll))
                         ((error quit)
                          (setq failed t)
                          (list (error-message-string failure))))))
            (if (not done)
                (push entry kept)
              (romacs-mcp--unindex entry)
              (romacs-mcp--send request
                                (romacs-mcp--object
                                 "jsonrpc" "2.0" "id" id
                                 "result" (romacs-mcp--result (car done) failed modern))))))))
    ;; Entries deferred meanwhile are ahead of ENTRIES; a stop meanwhile
    ;; retired them all.
    (when (= generation romacs-mcp--generation)
      (setq romacs-mcp--deferred (nconc (cl-ldiff romacs-mcp--deferred entries) (nreverse kept)))))
  (unless (or romacs-mcp--deferred (not romacs-mcp--deferred-timer))
    (cancel-timer romacs-mcp--deferred-timer)
    (setq romacs-mcp--deferred-timer nil)))

(defun romacs-mcp--defer-request (request id poll modern)
  "Keep REQUEST with ID until POLL yields its MODERN or legacy result."
  (let ((entry (list request id poll modern)))
    (push entry romacs-mcp--deferred)
    (puthash (cons (plist-get request :peer) id) entry romacs-mcp--deferred-index))
  (unless romacs-mcp--deferred-timer
    (setq romacs-mcp--deferred-timer
          (run-at-time 0.05 0.05 #'romacs-mcp--poll-deferred))))

(defvar romacs-mcp-admission-paused nil
  "Non-nil holds every new request in its queue: none starts.  A restart
sets it once nothing is in flight, so none is cut off; clear it with
`romacs-mcp-resume-admission'.")

(defun romacs-mcp-resume-admission ()
  "Let queued requests start again."
  (setq romacs-mcp-admission-paused nil)
  (romacs-mcp--schedule))

(defun romacs-mcp-in-flight-p ()
  "Non-nil while a request is running, queued or awaiting a deferred answer."
  (or romacs-mcp--active romacs-mcp--queue romacs-mcp--deferred))

(defun romacs-mcp--schedule ()
  "Schedule a bounded drain unless another drain owns admission."
  (when (and romacs-mcp--queue (not romacs-mcp--active) (not romacs-mcp--timer)
             (not romacs-mcp-admission-paused))
    (setq romacs-mcp--timer (run-at-time 0.01 nil #'romacs-mcp--drain))))

(defun romacs-mcp--drain ()
  "Admit at most one request when human input is not pending.
Retire at most `romacs-mcp--scan-limit' cancelled or stale queue entries.
This is cooperative admission, not a handler, serialization or send deadline."
  (setq romacs-mcp--timer nil)
  (unless (or romacs-mcp--active romacs-mcp-admission-paused)
    ;; Guard admission across native input polling too.  The no-timers query
    ;; services native special input, but does not request timer execution.
    (setq romacs-mcp--active (list :admission t))
    (unwind-protect
        (unless (input-pending-p nil)
          (let ((scanned 0))
            (while (and romacs-mcp--queue (< scanned romacs-mcp--scan-limit)
                        (let ((request (car romacs-mcp--queue)))
                          (or (plist-get request :cancelled)
                              (not (romacs-mcp--live-p
                                    (plist-get request :peer)
                                    (plist-get request :generation))))))
              (romacs-mcp--unqueue (pop romacs-mcp--queue))
              (cl-incf scanned))
            (when (and romacs-mcp--queue (< scanned romacs-mcp--scan-limit))
              (let* ((request (romacs-mcp--admit (pop romacs-mcp--queue)))
                     (peer (plist-get request :peer))
                     (message (plist-get request :message))
                     (id (and message (gethash "id" message :notification)))
                     (response nil) (deferred nil))
                (setq romacs-mcp--active request)
                (condition-case failure
                    (if (plist-get request :error)
                        (signal 'romacs-mcp-protocol-error (plist-get request :error))
                      (let ((result (romacs-mcp--dispatch peer message)))
                        (if (eq (car-safe result) 'romacs-mcp--deferred)
                            (setq deferred t)
                          (setq response (romacs-mcp--object
                                          "jsonrpc" "2.0" "id" id "result" result)))
                        (when deferred
                          (romacs-mcp--defer-request request id (nth 1 result) (nth 2 result)))))
                  (romacs-mcp-protocol-error
                   (setq response
                         (romacs-mcp--object
                          "jsonrpc" "2.0" "id" (if (eq id :notification) :null (or id :null))
                          "error" (romacs-mcp--object "code" (nth 1 failure) "message" (nth 2 failure))))
                   (when (nth 3 failure) (puthash "data" (nth 3 failure) (gethash "error" response))))
                  ((error quit)
                   (setq response (romacs-mcp--object
                                   "jsonrpc" "2.0" "id" (or id :null) "error"
                                   (romacs-mcp--object "code" -32603 "message" "Internal error")))))
                (unless (or deferred (eq id :notification) (plist-get request :held))
                  (romacs-mcp--send request response))))))
      (setq romacs-mcp--active nil)
      (romacs-mcp--schedule))))

(defun romacs-mcp--enqueue (peer message &optional failure)
  "Enqueue validated MESSAGE or framing FAILURE for PEER with bounded quotas."
  (if (or (>= (length romacs-mcp--queue) romacs-mcp--queue-limit)
          (>= (cl-count peer romacs-mcp--queue :key (lambda (r) (plist-get r :peer)))
              romacs-mcp--peer-queue-limit))
      (romacs-mcp--close peer)
    (let ((request (list :peer peer :generation romacs-mcp--generation
                         :message message :error failure :cancelled nil :held nil
                         :arrived (float-time))))
      (setq romacs-mcp--queue (nconc romacs-mcp--queue (list request)))
      (when-let* (((hash-table-p message)) (id (gethash "id" message nil)))
        (puthash (cons peer id) request romacs-mcp--queued)))
    (romacs-mcp--schedule)))

(defun romacs-mcp--admit (request)
  "REQUEST leaves the queue to run; return it."
  (romacs-mcp--unqueue request)
  request)

(defun romacs-mcp--cancel (peer id)
  "Mark queued, active or deferred requests matching PEER and ID abandoned."
  (dolist (request (append (list romacs-mcp--active) romacs-mcp--queue
                           (mapcar #'car romacs-mcp--deferred)))
    (when (and request (eq peer (plist-get request :peer))
               (hash-table-p (plist-get request :message))
               (equal id (gethash "id" (plist-get request :message) :absent)))
      (setf (plist-get request :cancelled) t))))

(defun romacs-mcp--withdraw (peer id target)
  "Answer PEER's request ID: withdraw its queued request TARGET if it has
not started, and say whether it was.  A deferred TARGET (started, its
answer awaited) is retired too: the relay answered for it, so its poll
must not act as if its answer were delivered."
  (let ((request (gethash (cons peer target) romacs-mcp--queued)))
    (if request
        (progn (setf (plist-get request :cancelled) t)
               (romacs-mcp--unqueue request))
      (when-let* ((entry (gethash (cons peer target) romacs-mcp--deferred-index)))
        (setf (plist-get (car entry) :cancelled) t)))
    (romacs-mcp--send (list :peer peer :generation romacs-mcp--generation :cancelled nil)
                      (romacs-mcp--object "jsonrpc" "2.0" "id" id
                                          "result" (romacs-mcp--object "cancelled" (if request t :false))))))

(defun romacs-mcp--frame (peer line)
  "Validate LINE from PEER, then enqueue or mark cancellation without eval."
  (condition-case nil
      (let* ((decoded (decode-coding-string line 'utf-8))
             (_valid-utf8
              (unless (equal line (encode-coding-string decoded 'utf-8))
                (error "Invalid UTF-8")))
             (message (json-parse-string decoded :object-type 'hash-table :array-type 'array
                                        :null-object :null :false-object :false))
             (id (and (hash-table-p message) (gethash "id" message :absent)))
             (method (and (hash-table-p message) (gethash "method" message)))
             (params (and (hash-table-p message) (gethash "params" message (romacs-mcp--object)))))
        (cond
         ((not (and (hash-table-p message) (equal (gethash "jsonrpc" message) "2.0")
                    (stringp method) (hash-table-p params)
                    (or (eq id :absent) (stringp id) (integerp id))))
          (romacs-mcp--enqueue peer nil '(-32600 "Invalid request")))
         ((and (not (eq id :absent))
               (> (string-bytes (json-serialize id)) romacs-mcp--id-limit))
          ;; Over-long IDs are not echoed: reply once with a null ID.
          (romacs-mcp--enqueue peer nil '(-32600 "Request ID is too long")))
         ((and (eq id :absent) (equal method "notifications/cancelled"))
          (romacs-mcp--cancel peer (gethash "requestId" params :absent)))
         ;; A relay withdrawing a call it waited on too long, answered here,
         ;; not queued: whether it was still queued (and now never runs).
         ((and (not (eq id :absent)) (equal method "romacs/cancel"))
          (romacs-mcp--withdraw peer id (gethash "id" params :absent)))
         ((and (eq id :absent) (not (equal method "notifications/initialized")))
          nil)
         ((and (not (eq id :absent)) (string-prefix-p "notifications/" method))
          (romacs-mcp--enqueue peer message '(-32600 "Notification must not have an ID")))
         ((and (not (eq id :absent))
               (cl-some (lambda (r)
                          (and r (eq peer (plist-get r :peer))
                               (hash-table-p (plist-get r :message))
                               (equal id (gethash "id" (plist-get r :message) :absent))))
                        (cons romacs-mcp--active romacs-mcp--queue)))
          (romacs-mcp--close peer))
         (t (romacs-mcp--enqueue peer message))))
    (error (romacs-mcp--enqueue peer nil '(-32700 "Parse error")))))

(defun romacs-mcp--filter (peer chunk)
  "Frame bounded UTF-8 CHUNK from PEER; never invoke a tool in this callback."
  (when (romacs-mcp--live-p peer (process-get peer 'generation))
    (let ((text (concat (process-get peer 'input) chunk)) (frames 0))
      ;; A loopback peer's first line is the token; it is not a frame.
      (when (and (process-get peer 'token) (string-match "\n" text))
        (if (equal (substring text 0 (match-beginning 0)) (process-get peer 'token))
            (progn (process-put peer 'token nil)
                   (setq text (substring text (match-end 0))))
          (romacs-mcp--close peer)))
      (if (or (not (process-live-p peer)) (> (string-bytes text) romacs-mcp--frame-limit))
          (romacs-mcp--close peer)
        (while (and (process-live-p peer) (not (process-get peer 'token)) (string-match "\n" text)
                    (< frames romacs-mcp--peer-queue-limit))
          (let ((end (match-beginning 0)))
            (romacs-mcp--frame peer (substring text 0 end))
            (setq text (substring text (1+ end)) frames (1+ frames))))
        (if (and (not (process-get peer 'token)) (string-match-p "\n" text)) (romacs-mcp--close peer)
          (process-put peer 'input text))))))

(defun romacs-mcp--accept (listener peer _message)
  "Admit PEER from LISTENER, subject to the finite connection quota."
  (if (or (not (eq listener romacs-mcp--listener))
          (>= (length romacs-mcp--peers) romacs-mcp--peer-limit))
      (romacs-mcp--close peer)
    (push peer romacs-mcp--peers)
    (process-put peer 'generation romacs-mcp--generation)
    (process-put peer 'token romacs-mcp--token)
    (process-put peer 'input (encode-coding-string "" 'no-conversion))
    (set-process-query-on-exit-flag peer nil)
    (set-process-coding-system peer 'no-conversion 'no-conversion)
    (set-process-filter peer #'romacs-mcp--filter)
    (set-process-sentinel peer #'romacs-mcp--sentinel)))

(defun romacs-mcp--socket-id (socket)
  "Return the filesystem identity of SOCKET, excluding mutable timestamps."
  (when-let* ((attributes (file-attributes socket 'integer)))
    (list (file-attribute-inode-number attributes)
          (file-attribute-device-number attributes))))

(defun romacs-mcp--node-snapshot (attributes)
  "Return the node identity and status-change time in ATTRIBUTES.
Unlike `romacs-mcp--socket-id', this also distinguishes a new node that
reuses a deleted node's inode number."
  (and attributes
       (list (file-attribute-inode-number attributes)
             (file-attribute-device-number attributes)
             (file-attribute-status-change-time attributes))))

(defun romacs-mcp--owned-socket-snapshot (socket)
  "Return a snapshot of SOCKET if it is a socket the editor's owner owns.
Type, owner and identity come from one `file-attributes' call, which
does not follow symlinks, so they all describe the same node."
  (let ((attributes (file-attributes socket 'integer)))
    (and attributes
         (eq ?s (aref (file-attribute-modes attributes) 0))
         (eql (file-attribute-user-id attributes) (user-uid))
         (romacs-mcp--node-snapshot attributes))))

(defun romacs-mcp--stale-socket-id (socket)
  "Return a snapshot of SOCKET if it is an owned socket with no listener.
A symlink or any other node is never stale.  Like GNU `server-running-p',
probe by connecting, but stricter: only a refused connection means the
listener is gone; any other failure (such as permission denied on a live
socket) leaves the node alone.  The node snapshot is taken before the
probe, so the caller can check that the node it deletes is the one probed."
  (let ((identity (romacs-mcp--owned-socket-snapshot socket)))
    (and identity
         (condition-case err
             (progn
               (delete-process
                (make-network-process :name "romacs-mcp-probe" :family 'local
                                      :service socket :noquery t))
               nil)
           (file-error
            ;; ERR is (file-error CONTEXT REASON . CONTACT); the contact
            ;; names the socket, so read only the reason.  A localized
            ;; reason fails closed.
            (let ((reason (nth 2 err)))
              (and (eq (car err) 'file-error)
                   (stringp reason)
                   (string-match-p "refused" reason)
                   identity)))))))

;;;###autoload
(defun romacs-mcp-start (socket)
  "Start an owner-controlled MCP listener at explicit absolute SOCKET.
Require a private owner directory and do not change GNU server parsing.
Refuse any existing node except an owned socket that no listener accepts on,
which a dead editor left behind; like GNU `server-start', delete that one.
Loading the library alone never starts a listener."
  (interactive "FPrivate MCP socket: ")
  (unless (and (stringp socket) (file-name-absolute-p socket)
               (not (file-remote-p socket)))
    (user-error "MCP requires an absolute local socket"))
  (when romacs-mcp--listener (user-error "MCP is already started"))
  (server-ensure-safe-dir (file-name-directory socket))
  (when (or (file-exists-p socket) (file-symlink-p socket))
    (let ((stale (romacs-mcp--stale-socket-id socket)))
      (unless stale
        (user-error "MCP socket already exists"))
      ;; Another editor may have replaced the stale node with its own live
      ;; socket while this one probed; never delete a node other than the
      ;; one probed.  A replacement between this check and the deletion
      ;; is not excluded: no system call unlinks only a given node.
      (unless (equal stale (romacs-mcp--owned-socket-snapshot socket))
        (user-error "MCP socket changed while starting"))
      (let (delete-by-moving-to-trash)
        (delete-file socket))))
  (cl-incf romacs-mcp--generation)
  (let ((listener (make-network-process
                   :name "romacs-mcp" :family 'local :service socket
                   :server t :noquery t :coding 'no-conversion
                   ;; Like GNU `server-start': a listener with a filter
                   ;; gives accepted connections, which inherit it, no buffer.
                   :filter #'romacs-mcp--filter :sentinel #'romacs-mcp--sentinel
                   :log #'romacs-mcp--accept)))
    (setq romacs-mcp--listener listener romacs-mcp--socket socket
          romacs-mcp--socket-identity (romacs-mcp--socket-id socket))
    (add-hook 'kill-emacs-hook #'romacs-mcp-stop)
    (romacs-mcp-identity)))

(defun romacs-mcp-start-loopback (token file)
  "Start the endpoint on a loopback TCP port, for hosts without local sockets.
Write the port to FILE, in a private owner directory.  A peer's first
line must be TOKEN, a secret the caller shares with its clients through
that directory; a peer sending anything else is closed.  Frames follow as
on the local socket."
  (unless (and (stringp file) (file-name-absolute-p file) (not (file-remote-p file)))
    (user-error "MCP requires an absolute local port file"))
  (unless (and (stringp token) (>= (length token) 32) (not (string-search "\n" token)))
    (user-error "MCP loopback token must be one line of at least 32 characters"))
  (when romacs-mcp--listener (user-error "MCP is already started"))
  (server-ensure-safe-dir (file-name-directory file))
  (cl-incf romacs-mcp--generation)
  (let ((listener (make-network-process
                   :name "romacs-mcp" :family 'ipv4 :host "127.0.0.1" :service 0
                   :server t :noquery t :coding 'no-conversion
                   :filter #'romacs-mcp--filter :sentinel #'romacs-mcp--sentinel
                   :log #'romacs-mcp--accept)))
    (setq romacs-mcp--listener listener romacs-mcp--token token)
    (let ((temporary (concat file ".tmp")))
      (with-temp-file temporary
        (insert (format "%d\n" (process-contact listener :service))))
      (set-file-modes temporary #o600)
      (rename-file temporary file t))
    (add-hook 'kill-emacs-hook #'romacs-mcp-stop)
    (romacs-mcp-identity)))

;;;###autoload
(defun romacs-mcp-stop ()
  "Retire admission and close only this endpoint's connections and socket.
Keep boot identity and companion receipts until the editor process exits."
  (interactive)
  (cl-incf romacs-mcp--generation)
  (when romacs-mcp--timer (cancel-timer romacs-mcp--timer))
  (when romacs-mcp--deferred-timer (cancel-timer romacs-mcp--deferred-timer))
  (setq romacs-mcp--timer nil romacs-mcp--queue nil
        romacs-mcp--deferred nil romacs-mcp--deferred-timer nil)
  (clrhash romacs-mcp--deferred-index)
  (clrhash romacs-mcp--queued)
  (mapc #'romacs-mcp--close (copy-sequence romacs-mcp--peers))
  (when (process-live-p romacs-mcp--listener) (delete-process romacs-mcp--listener))
  (when (and romacs-mcp--socket
             (equal (romacs-mcp--socket-id romacs-mcp--socket) romacs-mcp--socket-identity)
             (file-exists-p romacs-mcp--socket))
    (delete-file romacs-mcp--socket))
  (setq romacs-mcp--listener nil romacs-mcp--socket nil romacs-mcp--socket-identity nil
        romacs-mcp--token nil)
  (remove-hook 'kill-emacs-hook #'romacs-mcp-stop))

(romacs-mcp-register-tool
 "romacs_identity" "Read the stable editor incarnation before fenced calls."
 (romacs-mcp--schema nil nil) (lambda (_) (romacs-mcp-identity))
 (romacs-mcp--object "readOnlyHint" t))
(romacs-mcp-register-tool
 "romacs_help" "Every tool, by group, with one line each."
 (romacs-mcp--schema nil nil) #'romacs-mcp--help
 (romacs-mcp--object "readOnlyHint" t))
(romacs-mcp-register-tool
 "romacs_eval" "Evaluate unrestricted owner Lisp. No rollback or CPU preemption."
 (romacs-mcp--schema '(("instance" . "string") ("code" . "string")) '("code"))
 #'romacs-mcp--eval)

(provide 'romacs-mcp)
;;; romacs-mcp.el ends here

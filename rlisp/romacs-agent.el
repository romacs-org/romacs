;;; romacs-agent.el --- Agent sessions that outlive the editor -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent session is a harness (Claude Code's stream-json CLI, or Codex's
;; JSON-RPC `codex app-server') run by its own supervisor, `romacs-agent-host', which owns the harness's stdio and
;; journals every line under $XDG_STATE_HOME/romacs/agents/ID/.  The
;; supervisor is nobody's child (a transient user unit), so the harness
;; and its turn survive a closed window, a server stop and a new build: the
;; next server attaches to the supervisor's socket and replays the journal.
;; The harness reaches Romacs's tools through `romacs-mcp --reconnect', which
;; keeps the harness's MCP session across server restarts.
;;
;; The harness runs with the user's own configuration and permission mode:
;; Romacs passes none.  When the harness asks the user to approve a tool
;; (Claude Code's `can_use_tool' request, Codex's approval requests), the session raises an `approval' event and
;; writes back exactly what the user answered.
;;
;; Events are plists with a :type (session status delta thinking tool
;; compacted approval done error input exit unknown), the session and the journal
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
  "Further arguments for `codex app-server', passed through untouched, as
`romacs-agent-claude-arguments' are for Claude Code."
  :type '(repeat string) :group 'romacs-agent)

(defcustom romacs-agent-providers '(("claude" . "~/.claude"))
  "The harness instances sessions run under, the first enabled being the
default.  Each entry is (NAME . CONFIG-DIR), or (NAME . PLIST) of
  :harness HARNESS   default NAME's leading word (\"claude2\" and
                     \"claude-work\" are Claude Code, \"codex\" is Codex)
  :display NAME      default NAME capitalised
  :config-dir DIR    the harness's configuration directory
  :enabled BOOLEAN   nil: not offered, not started; its sessions still resume
  :program PROGRAM, :arguments ARGUMENTS
                     default the harness's own options
                     (`romacs-agent-claude-program', ...)
  :model MODEL, :effort EFFORT
                     what its new sessions run, unless their start says
or the symbol `t3', or (t3 :settings FILE): T3 Code's instances, read
from its settings (`romacs-agent-t3-settings').  The first entry of a
name wins, so entries after `t3' are what runs without T3.

CONFIG-DIR is where the harness keeps its account, settings and
conversations (Claude Code's CLAUDE_CONFIG_DIR, Codex's CODEX_HOME).  It
is given to the harness alone; the harness's default directory, or none,
runs it with that variable unset, as the harness runs by default.  A
session records its instance's directory, and resumes there."
  :type '(repeat (choice (const :tag "T3 Code's instances" t3)
                         (cons :tag "Name and directory" string directory)
                         (cons :tag "Name and options" string plist)
                         (cons :tag "T3 Code's, from a file" (const t3) plist)))
  :group 'romacs-agent)

(defcustom romacs-agent-default-provider nil
  "The name of the instance `romacs-agent-start' offers first, or nil for
the first of `romacs-agent-providers'."
  :type '(choice (const nil) string) :group 'romacs-agent)

(defcustom romacs-agent-models
  '((claude ("claude-opus-5-5" :label "Opus 5.5" :efforts ("low" "medium" "high" "xhigh" "max")
             :default-effort "medium")
            ("claude-sonnet-5-5" :label "Sonnet 5.5" :efforts ("low" "medium" "high" "xhigh" "max")
             :default-effort "high")
            ("claude-fable-5-1" :label "Fable 5.1" :efforts ("low" "medium" "high" "xhigh" "max")
             :default-effort "medium")
            ("claude-haiku-5-5" :label "Haiku 5.5" :efforts ("low" "medium" "high" "xhigh" "max")
             :default-effort "medium"))
    (codex ("gpt-6.1-sol" :label "GPT-6.1 Sol" :efforts ("low" "medium" "high" "xhigh" "max" "ultra")
            :default-effort "low")
           ("gpt-6-astra" :label "GPT-6 Astra" :efforts ("low" "medium" "high" "xhigh" "max" "ultra")
            :default-effort "medium")
           ("gpt-6.1-terra" :label "GPT-6.1 Terra" :efforts ("low" "medium" "high" "xhigh" "max" "ultra")
            :default-effort "low")
           ("gpt-6-luna" :label "GPT-6 Luna" :efforts ("low" "medium" "high" "xhigh" "max")
            :default-effort "medium")))
  "The models each harness offers, as (HARNESS ROW...), a ROW being
\(ID :label LABEL :efforts EFFORTS :default-effort EFFORT).  What a harness
reports of itself goes before these, and an instance's own :models before
that (`romacs-agent--models').  Starting one names its ID to the harness;
no model named leaves the choice to the harness's configuration."
  :type '(alist :key-type symbol :value-type (repeat sexp)) :group 'romacs-agent)

(defcustom romacs-agent-select-timeout 10
  "Seconds a harness has to confirm a new model or effort before the
session is restarted on it instead."
  :type 'number :group 'romacs-agent)

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

(defvar romacs-agent-input-functions nil
  "Functions called with a session and a request id once a user's message
is written to its harness: before the harness has acted on it, whenever
its journal's record is read back.  The message's `input' event carries
the id as :request.")

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
  reply (turns 0) queue queue-tail (queue-length 0) dispatch cancelled
  ;; The seq of its last prompt, and whether its harness ended in the
  ;; middle of a turn (a resume then takes that turn up again).
  input-seq interrupted
  ;; The seq of its last turn's real end (a done; an exit mid-turn is none).
  (completed-seq 0)
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
  imported
  ;; Its harness's driver (`romacs-agent-define-harness'), looked up once.
  driver
  ;; The driver's startup state, folded from this run's journal records
  ;; (:run, the seq the run started at, then `handshake' events; an exit
  ;; clears it), the startup line written and
  ;; not yet journaled back, whether the records seen are the live ones
  ;; (the supervisor's replay acknowledged), and the lines written to the
  ;; harness so far, every run's (request ids count on from them).
  driver-state startup-pending caught-up (inputs 0) models-request pending-selection
  ;; Whether the last line was a compaction's boundary: its summary is next.
  compacting
  ;; What runs under it, for its views: its harness's own subagents by tool
  ;; id, then those and its delegated tasks as rows and as a line.
  subagents children-rows children-summary)

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

(defun romacs-agent--json (object)
  "OBJECT as JSON text: `json-serialize' gives UTF-8 bytes, this the
characters they encode, to put in a buffer, a string or more JSON."
  (decode-coding-string (json-serialize object :false-object :false :null-object :null) 'utf-8))

(defun romacs-agent--write-json (file object)
  "Write OBJECT as JSON to FILE in UTF-8, atomically, readable only by the user."
  (let ((temporary (concat file ".tmp")) (coding-system-for-write 'utf-8-unix))
    (with-temp-file temporary
      (insert (romacs-agent--json object)))
    (set-file-modes temporary #o600)
    (rename-file temporary file t)))

(defun romacs-agent--read-json (file)
  (when (file-readable-p file)
    (with-temp-buffer
      (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
      (ignore-errors (json-parse-buffer :object-type 'hash-table :null-object nil :false-object nil)))))

;; The endpoint harness relays reach.  A named server serves its own
;; name's socket (romacs-server.el); any other editor serves its own,
;; agents/editor/PID.sock.  Neither outlives its process, so a session's
;; relay names neither: it names the session's own route,
;; agents/route/KEY.sock, a link to the endpoint of the process attached to
;; that session.  Attachment is exclusive at the session's supervisor, so
;; exactly one live process leads each route, and the process attaching
;; after a restart (it restores at startup) leads it to itself.  Two
;; editors attached to different sessions each get their own sessions'
;; tools.  A relay an earlier build started names that editor's
;; romacs/session-PID/server.sock; the process attaching its session leads
;; that path to itself the same way.

(defun romacs-agent--own-socket ()
  "This editor's own endpoint, when it is no named server."
  (expand-file-name (format "editor/%d.sock" (emacs-pid)) (romacs-agent--runtime-root)))

(defun romacs-agent--route (session)
  "The path SESSION's relays name: a link to its attached process's endpoint,
named by the session's id (its record's directory name)."
  (expand-file-name (format "route/%s.sock" (romacs-agent-id session)) (romacs-agent--runtime-root)))

(defun romacs-agent--sessions-base ()
  "Where earlier builds kept per-process endpoints: romacs/session-PID/."
  (expand-file-name "romacs/" (or (getenv "XDG_RUNTIME_DIR") temporary-file-directory)))

(defun romacs-agent--private-directory (directory)
  (make-directory directory t)
  (set-file-modes directory #o700)
  directory)

(defun romacs-agent--endpoint ()
  "This process's endpoint, started when first needed: a named server's,
else this editor's own."
  (unless romacs-mcp--socket
    (let ((own (romacs-agent--own-socket)))
      (romacs-agent--private-directory (file-name-directory own))
      (romacs-mcp-start own)))
  romacs-mcp--socket)

(defun romacs-agent--listening-p (socket)
  "Whether something answers at SOCKET, a link followed."
  (condition-case nil
      (progn (delete-process (make-network-process :name "romacs-agent-probe" :family 'local
                                                   :service socket :noquery t))
             t)
    ;; Refused, absent, or a path no socket can have: nothing answers.
    (error nil)))

(defun romacs-agent--lead (path)
  "Make PATH a link to this process's endpoint, replacing a link there
atomically (rename(2)).  Another node there (a socket) is left alone."
  (let ((temporary (format "%s.%d" path (emacs-pid))) (delete-by-moving-to-trash nil))
    (when (or (file-symlink-p path) (not (file-exists-p path)))
      (when (file-symlink-p temporary) (delete-file temporary))
      (make-symbolic-link (romacs-agent--endpoint) temporary)
      (rename-file temporary path t))))

(defun romacs-agent--legacy-relays (session)
  "The romacs/session-PID/ endpoints SESSION's relays name, from an
earlier build."
  (let ((sockets nil))
    (dolist (file '("mcp.json" "manifest.json"))
      (let ((path (expand-file-name file (romacs-agent-dir session))))
        (when (file-readable-p path)
          (with-temp-buffer
            (let ((coding-system-for-read 'utf-8)) (insert-file-contents path))
            (while (re-search-forward "[^\"\\]*/session-[0-9]+/server\\.sock" nil t)
              (push (match-string 0) sockets))))))
    (delete-dups sockets)))

(defun romacs-agent--socket-pid (socket)
  "The process number an editor's own SOCKET path carries."
  (and (string-match "/\\(?:session-\\([0-9]+\\)/server\\|editor/\\([0-9]+\\)\\)\\.sock\\'" socket)
       (string-to-number (or (match-string 1 socket) (match-string 2 socket)))))

(defun romacs-agent--own-directory-p (directory)
  "Whether DIRECTORY is a private directory of this user, not a link."
  (let ((attributes (file-attributes directory 'integer)))
    (and (eq (file-attribute-type attributes) t)
         (eql (file-attribute-user-id attributes) (user-uid))
         (equal (file-attribute-modes attributes) "drwx------"))))

(defun romacs-agent--stale-p (path)
  "Whether PATH is a socket of ours its process left (that process gone,
nothing listening) or a link of ours to nothing that answers."
  (let ((attributes (file-attributes path 'integer)))
    (and attributes (eql (file-attribute-user-id attributes) (user-uid))
         (or (stringp (file-attribute-type attributes))
             (string-prefix-p "s" (file-attribute-modes attributes)))
         (not (process-attributes (or (romacs-agent--socket-pid path) 0)))
         (not (romacs-agent--listening-p path)))))

(defun romacs-agent--route-here (session)
  "SESSION is attached here: its route, and any earlier build's path its
relay names, lead to this process.  A legacy path keeps a live listener;
a socket its ended editor left is replaced."
  (romacs-agent--private-directory (file-name-directory (romacs-agent--route session)))
  (romacs-agent--lead (romacs-agent--route session))
  (dolist (socket (romacs-agent--legacy-relays session))
    (when (and (romacs-agent--own-directory-p (file-name-directory socket))
               (not (equal (file-symlink-p socket) romacs-mcp--socket)))
      (let ((delete-by-moving-to-trash nil))
        (when (and (file-exists-p socket) (not (file-symlink-p socket)) (romacs-agent--stale-p socket))
          (delete-file socket))
        (when (or (not (file-exists-p socket)) (romacs-agent--stale-p socket))
          (romacs-agent--lead socket))))))

(defun romacs-agent--drop-relays (session)
  "SESSION is no longer attached here: its relays' connections to this
endpoint close, so they reconnect through its route to the process that
attaches it next."
  (dolist (peer (copy-sequence romacs-mcp--peers))
    (when (equal (process-get peer 'romacs-agent-session) (romacs-agent-id session))
      (romacs-mcp--close peer))))

(defun romacs-agent--kept (session process)
  "PROCESS's first line from SESSION's supervisor that is no refusal: the
attachment is kept (a refusal is the only line a refused attach gets, and
a kept one always gets one: the replay starts at the last record seen, or
the acknowledgement {\"attached\": ID} a supervisor sends).  SESSION's
relays lead here from now."
  (unless (process-get process 'romacs-agent-kept)
    (process-put process 'romacs-agent-kept t)
    (when (eq (romacs-agent-connection session) process) (romacs-agent--route-here session))))

(defun romacs-agent--route-attached ()
  "This process's endpoint changed (a named server took over): every
session whose attachment here is kept leads here again."
  (maphash (lambda (_ session)
             (let ((connection (romacs-agent-connection session)))
               (when (and (process-live-p connection) (process-get connection 'romacs-agent-kept))
                 (romacs-agent--route-here session))))
           romacs-agent--sessions))

(defun romacs-agent--sweep ()
  "Remove what ended processes left: routes of sessions gone, other
editors' own sockets, and earlier builds' romacs/session-PID/ directories
no session's relay names.  Only nodes of this user whose process is gone
and that nothing answers at."
  (let ((runtime (romacs-agent--runtime-root))
        (legacy (make-hash-table :test #'equal)) (delete-by-moving-to-trash nil))
    (maphash (lambda (_ session)
               (dolist (socket (romacs-agent--legacy-relays session)) (puthash socket t legacy)))
             romacs-agent--sessions)
    ;; A route whose session's record is gone: its record is made before
    ;; its route (romacs-agent--create, then the launch), so a session
    ;; another process is creating keeps its route.  One that still answers
    ;; stays too: a process with another state root sees no records at all.
    (let ((routes (expand-file-name "route/" runtime)))
      (when (romacs-agent--own-directory-p routes)
        (dolist (route (directory-files routes t "\\.sock\\'"))
          (unless (file-directory-p (expand-file-name (file-name-base route) (romacs-agent--state-root)))
            ;; Another HOME's process sees no record of a live session here.
            (when (and (file-symlink-p route) (romacs-agent--stale-p route)) (delete-file route))))))
    (let ((editor (expand-file-name "editor/" runtime)))
      (when (romacs-agent--own-directory-p editor)
        (dolist (socket (directory-files editor t "\\`[0-9]+\\.sock\\'"))
          (when (and (not (equal socket romacs-mcp--socket)) (romacs-agent--stale-p socket))
            (delete-file socket)))))
    (let ((base (romacs-agent--sessions-base)))
      (dolist (directory (and (file-directory-p base) (directory-files base t "\\`session-[0-9]+\\'")))
        (let ((socket (expand-file-name "server.sock" directory)))
          (when (and (romacs-agent--own-directory-p directory) (not (gethash socket legacy))
                     (not (process-attributes (romacs-agent--socket-pid socket))))
            (when (romacs-agent--stale-p socket) (delete-file socket))
            ;; Only an emptied directory: anything else in it is not ours.
            (unless (or (file-exists-p socket) (file-symlink-p socket))
              (ignore-errors (delete-directory directory)))))))))

;;; Provider instances

;;; Harness drivers

(defvar romacs-agent--drivers nil
  "Each harness Romacs runs and its driver, (HARNESS . DRIVER), the last
defined first.")

(defvar romacs-agent--driver-table (make-hash-table :test #'eq)
  "Harness to its cell of `romacs-agent--drivers', the lookup by name.")

(defvar romacs-agent--driver-generation 0
  "Counts the drivers' definitions: what was derived from them (T3's
instances) is derived again when it changes.")

(defun romacs-agent-define-harness (harness &rest driver)
  "Define how Romacs runs HARNESS, by DRIVER's keys:

  :display NAME, :icon STRING   its name and its icon in views;
  :config-variable VARIABLE, :config-default DIRECTORY
      the variable naming its configuration directory, and the
      directory it uses while that is unset;
  :program-option, :arguments-option
      the options holding its program and its extra arguments;
  :scrub NAMES   variables it sets for the commands it runs (its own
      session's): no harness Romacs runs inherits them;
  :lifetime   `persistent' (one process reading lines) or `per-turn'
      (one run per turn, its prompt on stdin);
  :select (SESSION MODEL EFFORT) optional, for a persistent harness: how an
                           idle one takes a new model and effort: (ID .
                           LINE) pairs to write, each answered through
                           `romacs-agent--select-answered'; nil when its
                           next message carries them; else `restart':
                           resumed with the new option words
  :refresh-models (SESSION) optional: ask a new session's harness for its
                           models, `romacs-agent--keep-models' taking them
  :argv (SESSION RESUME)   its command line, RESUME going on with its
      conversation;
  :input (SESSION TEXT ID)   the object a persistent harness reads as the
      user's message, ID its request id;
  :interrupt (SESSION)   the object interrupting its turn, `signal' (the
      run ends by SIGTERM), or nil when there is no turn to interrupt;
  :respond (SESSION REQUEST-ID ALLOW MESSAGE)   the object answering an
      approval it asked for: ALLOW nil denies with MESSAGE, t allows as
      asked, a plist allows with :answers ((QUESTION . ANSWER) ...) to
      the questions its `approval' event's :questions asked, or with
      :always for the rest of the session;
  :events (SESSION OBJECT), :input-events (SESSION OBJECT)
      the events of an object it wrote, and of one written to it;
      `handshake' events (:state PLIST) fold into its startup state;
  :next (SESSION)   its next object to write (from
      `romacs-agent-driver-state'): a startup step, or once ready a line
      it owes (an answer to its harness's request); :ready once started
      up, or nil to wait for an answer; absent, it is ready at launch.
      Messages wait until ready;
  :resume-environment ENTRIES   added to its environment on a resume;
  :native-id-p (STRING)   whether STRING can name its conversation;
  :history PLIST   reading its transcripts: :files (DIRECTORY LIMIT),
      :keep, :describe and :events (`romacs-agent-history').

Defining it again replaces it."
  (let ((cell (gethash harness romacs-agent--driver-table)))
    (if cell
        (setcdr cell driver)
      (setq cell (cons harness driver))
      (push cell romacs-agent--drivers)
      (puthash harness cell romacs-agent--driver-table)))
  (cl-incf romacs-agent--driver-generation)
  harness)

(defun romacs-agent--driver-of (harness)
  (or (cdr (gethash harness romacs-agent--driver-table))
      (user-error "No harness %s (Romacs runs: %s)" harness
                  (mapconcat (lambda (h) (symbol-name (car h))) romacs-agent--drivers ", "))))

(defun romacs-agent--driver (session)
  "SESSION's driver, looked up once."
  (or (romacs-agent-driver session)
      (setf (romacs-agent-driver session) (romacs-agent--driver-of (romacs-agent-harness session)))))

(defun romacs-agent--harness-get (session key)
  (plist-get (romacs-agent--driver session) key))

(defun romacs-agent-request-id (session)
  "A new request id for SESSION's harness, above every line it was given."
  (setf (romacs-agent-requests session) (1+ (max (romacs-agent-requests session) (romacs-agent-inputs session)))))

;;; Provider instances

(defun romacs-agent--harness-of (name)
  "The harness an instance NAME names by its leading word."
  (and (string-match "\\`[[:alpha:]]+" name) (intern (downcase (match-string 0 name)))))

(defun romacs-agent--provider (entry)
  "ENTRY of `romacs-agent-providers', (NAME . DIRECTORY) or (NAME . PLIST),
as an instance: a plist of :name :display :harness :config-dir :key
:enabled :program :arguments :model :effort :models :options :source
\(and :t3-id for one of T3's)."
  (let* ((name (car entry))
         (options (if (listp (cdr entry)) (cdr entry) (list :config-dir (cdr entry))))
         (harness (or (plist-get options :harness) (romacs-agent--harness-of name)))
         (known (cdr (gethash harness romacs-agent--driver-table))))
    (unless known
      (user-error "Agent provider %s: no harness %s" name harness))
    (list :name name :harness harness
          :display (or (plist-get options :display) (capitalize name))
          :config-dir (plist-get options :config-dir)
          ;; Its continuation key (T3's): what a switch keeps or changes.
          :key (cons harness (romacs-agent--home harness (plist-get options :config-dir)))
          :enabled (if (plist-member options :enabled) (plist-get options :enabled) t)
          :program (if (plist-member options :program) (plist-get options :program)
                     (symbol-value (plist-get known :program-option)))
          :arguments (if (plist-member options :arguments) (plist-get options :arguments)
                       (symbol-value (plist-get known :arguments-option)))
          :model (plist-get options :model) :effort (plist-get options :effort)
          :models (plist-get options :models) :options (plist-get options :options)
          :source (or (plist-get options :source) 'lisp) :t3-id (plist-get options :t3-id)
          :t3-store (plist-get options :t3-store))))

(defvar romacs-agent-t3-settings)
(declare-function romacs-agent-t3-read "romacs-agent-t3" (file harness-of generation))
(declare-function romacs-agent-t3-store "romacs-agent-t3" (settings))

(defun romacs-agent--t3-harnesses ()
  "A function from T3 Code's driver name to the harness running it, or
nil: its index built once, when first asked."
  (let ((index nil))
    (lambda (driver)
      (unless index
        (setq index (make-hash-table :test #'equal))
        (dolist (cell romacs-agent--drivers)
          (when-let* ((name (plist-get (cdr cell) :t3-driver))) (puthash name (car cell) index))))
      (gethash driver index))))

(defun romacs-agent--walk-instances (stop)
  "`romacs-agent-providers' as (INSTANCES . CONSTRAINTS), as
`romacs-agent-instances' says.  A non-nil STOP ends the walk at the first
instance it accepts: finding that need not read a long list."
  (let* ((taken (make-hash-table :test #'equal)) (instances nil) (problems nil) (unknown nil)
         (add (lambda (instance)
                (push instance instances)
                (when (and stop (funcall stop instance)) (throw 'romacs-agent--walked nil)))))
    (catch 'romacs-agent--walked
      (dolist (entry romacs-agent-providers)
        (if (or (eq entry 't3) (eq (car-safe entry) 't3))
            (let ((settings (progn (require 'romacs-agent-t3)
                                   (or (plist-get (cdr-safe entry) :settings) romacs-agent-t3-settings))))
              (pcase-let ((`(,t3 . ,why)
                           (romacs-agent-t3-read settings (romacs-agent--t3-harnesses) romacs-agent--driver-generation)))
                (setq problems (append (reverse why) problems))
                (dolist (row t3)
                  (unless (gethash (car row) taken)
                    (puthash (car row) t taken)
                    ;; Its threads' titles are in the store beside these settings.
                    (funcall add (romacs-agent--provider
                                  (append row (list :t3-store (romacs-agent-t3-store settings)))))))))
          (unless (gethash (car entry) taken)
            (puthash (car entry) t taken)
            (condition-case err (funcall add (romacs-agent--provider entry))
              (user-error (push (list :source 'lisp :id (car entry) :reason (error-message-string err)) problems)
                          (setq unknown t)))))))
    ;; The harnesses there are, once.
    (when unknown
      (push (list :source 'lisp :id nil
                  :reason (format "Romacs runs: %s" (mapconcat (lambda (h) (symbol-name (car h))) romacs-agent--drivers ", ")))
            problems))
    (cons (nreverse instances) (nreverse problems))))

(defun romacs-agent-instances (&optional constraints)
  "The instances `romacs-agent-providers' names, in its order, each a plist
\=(`romacs-agent--provider'): the first entry of a name wins; the `t3'
entry, or (t3 :settings FILE), stands for T3 Code's instances there
\=(`romacs-agent-t3-settings').  Disabled ones are kept: they are not
offered, but their sessions still resume.  With CONSTRAINTS non-nil,
return (INSTANCES . CONSTRAINTS), the second saying, for each instance
Romacs cannot run, why."
  (let ((walked (romacs-agent--walk-instances nil)))
    (if constraints walked (car walked))))

(defun romacs-agent--configured (name)
  "The instance NAME, or whose T3 id is NAME, or nil."
  (let ((instances (romacs-agent-instances)))
    (or (cl-find name instances :key (lambda (i) (plist-get i :name)) :test #'equal)
        (cl-find name instances :key (lambda (i) (plist-get i :t3-id)) :test #'equal))))

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
        (default (plist-get (romacs-agent--driver-of (plist-get provider :harness)) :config-default)))
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
  (let ((variable (romacs-agent--harness-get session :config-variable)))
    (if (romacs-agent-config-dir session)
        (list "env" (concat variable "=" (romacs-agent-config-dir session)))
      (list "env" "-u" variable))))

(defun romacs-agent--default-provider ()
  (or romacs-agent-default-provider
      (let ((enabled (lambda (i) (plist-get i :enabled))))
        (plist-get (cl-find-if enabled (car (romacs-agent--walk-instances enabled))) :name))
      (user-error "No agent provider: `romacs-agent-providers' names no enabled instance")))

;;; Models

(defun romacs-agent--models-file (name)
  "Where the models instance NAME's harness reported are kept."
  (expand-file-name (concat "models/" (replace-regexp-in-string "[^[:alnum:]_.-]" "_" name) ".json")
                    (romacs-agent--state-root)))

(defun romacs-agent--models-stamp (instance)
  "What models kept for INSTANCE hold for: its program, resolved, and
that file's time.  Nil when it is not found."
  (when-let* ((found (executable-find (plist-get instance :program))))
    (let ((path (file-truename found)))
      (format "%s %s" path (format-time-string "%s.%N" (file-attribute-modification-time (file-attributes path)))))))

(defun romacs-agent--models-kept (instance)
  "The rows INSTANCE's harness reported, if its present program did."
  (when-let* ((kept (romacs-agent--read-json (romacs-agent--models-file (plist-get instance :name))))
              ((hash-table-p kept))
              ((equal (gethash "stamp" kept) (romacs-agent--models-stamp instance)))
              ((vectorp (gethash "models" kept))))
    (mapcar (lambda (row)
              (cons (gethash "id" row)
                    (list :label (gethash "label" row) :efforts (append (gethash "efforts" row) nil))))
            (gethash "models" kept))))

(defun romacs-agent--keep-models (session rows)
  "Keep ROWS, what SESSION's harness reports, for its instance."
  (let* ((instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session)))
         (file (romacs-agent--models-file (plist-get instance :name))))
    (when-let* ((rows) (stamp (romacs-agent--models-stamp instance)))
      (make-directory (file-name-directory file) t)
      (romacs-agent--write-json
       file (romacs-mcp--object
             "stamp" stamp
             "models" (vconcat (mapcar (lambda (row)
                                         (romacs-mcp--object "id" (car row) "label" (plist-get (cdr row) :label)
                                                             "efforts" (vconcat (plist-get (cdr row) :efforts))))
                                       rows)))))))

(defun romacs-agent--refresh-models (session)
  "Ask SESSION's harness for its models, when its driver can and its
instance has none kept for this program."
  (when-let* ((ask (romacs-agent--harness-get session :refresh-models))
              ((not (romacs-agent--models-kept
                     (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session))))))
    (funcall ask session)))

(defun romacs-agent--models (instance)
  "The models INSTANCE offers, each (ID . PLIST): its own :models, what
its harness reported, then `romacs-agent-models'.  The rows of one id
merge, the earlier's fields winning, and a default effort the efforts
that won do not hold is none."
  (let ((seen (make-hash-table :test #'equal)) (rows nil))
    (dolist (row (append (plist-get instance :models) (romacs-agent--models-kept instance)
                         (alist-get (plist-get instance :harness) romacs-agent-models)))
      (if-let* ((known (gethash (car row) seen)))
          (setcdr known (append (cdr known) (cdr row)))
        (push (puthash (car row) (cons (car row) (cdr row)) seen) rows)))
    (dolist (row rows)
      (let ((default (plist-get (cdr row) :default-effort)))
        (when (and default (not (member default (plist-get (cdr row) :efforts))))
          (setcdr row (append (list :default-effort nil) (cdr row))))))
    (nreverse rows)))

(defun romacs-agent--effort-fits-p (instance model effort)
  "Whether MODEL, as INSTANCE knows it, takes EFFORT: true of a model it
does not know, or of none named."
  (let ((row (and model (assoc model (romacs-agent--models instance)))))
    (or (null row) (member effort (plist-get (cdr row) :efforts)))))

;;; Choosing what a session runs on

(defun romacs-agent--efforts-of (instance model)
  "The efforts INSTANCE offers MODEL, its own model when nil; those of
every model it knows when it knows neither."
  (let* ((models (romacs-agent--models instance))
         (row (assoc (or model (plist-get instance :model)) models)))
    (if row (plist-get (cdr row) :efforts)
      (delete-dups (cl-loop for row in models append (copy-sequence (plist-get (cdr row) :efforts)))))))

(defun romacs-agent--instance-rows (instance label default)
  "The picker's rows of INSTANCE, called LABEL, the instance DEFAULT:
each (TEXT :provider NAME :model ID :efforts EFFORTS :default-effort EFFORT
:group HEADING :note NOTE).  The first takes the instance's own defaults."
  (let* ((models (romacs-agent--models instance))
         (model (plist-get instance :model)) (effort (plist-get instance :effort))
         (own (cdr (assoc model models)))
         (dir (or (plist-get instance :config-dir)
                  (plist-get (romacs-agent--driver-of (plist-get instance :harness)) :config-default)))
         (missing (and (plist-get instance :config-dir) (not (file-directory-p (expand-file-name dir)))
                       "dir missing"))
         (group (format "%s  %s%s" label (abbreviate-file-name dir) (if default "  default" "")))
         (common (list :provider (plist-get instance :name) :group group)))
    (cons (append (list (format "%s · default" label)
                        :efforts (romacs-agent--efforts-of instance nil)
                        :default-effort (plist-get own :default-effort)
                        :note (or missing (if (or model effort)
                                              (mapconcat #'identity (delq nil (list model effort)) " · ")
                                            "harness's own")))
                  common)
          (mapcar (lambda (row)
                    (let ((efforts (plist-get (cdr row) :efforts)))
                      (append (list (format "%s · %s" label (or (plist-get (cdr row) :label) (car row)))
                                    :model (car row) :efforts efforts
                                    :default-effort (plist-get (cdr row) :default-effort)
                                    :note (or missing
                                              (and efforts (format "effort %s...%s" (car efforts) (car (last efforts))))
                                              ""))
                              common)))
                  models))))

(defun romacs-agent--picker-rows ()
  "Every enabled instance's rows (`romacs-agent--instance-rows'), the
default instance's first."
  (let* ((default (plist-get (romacs-agent--configured (romacs-agent--default-provider)) :name))
         (labels (make-hash-table :test #'equal))
         (enabled (cl-remove-if-not (lambda (i) (plist-get i :enabled)) (romacs-agent-instances)))
         (first-p (lambda (i) (equal (plist-get i :name) default))))
    (cl-loop for instance in (append (cl-remove-if-not first-p enabled) (cl-remove-if first-p enabled))
             for display = (plist-get instance :display)
             ;; Two instances may be shown alike: rows must differ.
             for label = (if (gethash display labels) (format "%s (%s)" display (plist-get instance :name)) display)
             do (puthash display t labels)
             append (romacs-agent--instance-rows instance label (equal (plist-get instance :name) default)))))

(defun romacs-agent--ordered-table (candidates &rest metadata)
  "A completion table of CANDIDATES in their order, with METADATA."
  (lambda (string predicate action)
    (if (eq action 'metadata)
        `(metadata (display-sort-function . identity) (cycle-sort-function . identity) ,@metadata)
      (complete-with-action action candidates string predicate))))

(defun romacs-agent--pick-row (rows)
  "One of ROWS (`romacs-agent--picker-rows'), asked for: grouped by
instance, the first the default."
  (let ((by-text (make-hash-table :test #'equal)) (default (caar rows)))
    (dolist (row rows) (puthash (car row) (cdr row) by-text))
    (let ((text (completing-read
                 (format-prompt "Agent" default)
                 (romacs-agent--ordered-table
                  rows
                  (cons 'group-function
                        (lambda (text transform)
                          (if transform text (plist-get (gethash text by-text) :group))))
                  (cons 'affixation-function
                        (lambda (texts)
                          (mapcar (lambda (text)
                                    (list text "" (propertize (concat "  " (plist-get (gethash text by-text) :note))
                                                              'face 'completions-annotations)))
                                  texts))))
                 nil t nil nil default)))
      (cons text (gethash text by-text)))))

(defun romacs-agent--read-effort (efforts default)
  "One of EFFORTS, asked for, DEFAULT first; nil for none."
  (let ((answer (completing-read (format-prompt "Effort" default) (romacs-agent--ordered-table efforts)
                                 nil t nil nil default)))
    (and (not (string-empty-p answer)) answer)))

(defun romacs-agent--read-selection (ask-effort)
  "What a new session runs on, as (PROVIDER . OPTIONS): asked for when
more than one instance is enabled, or ASK-EFFORT.  Its effort is asked for
with ASK-EFFORT, or when nothing gives the chosen model one."
  (let* ((rows (romacs-agent--picker-rows))
         ;; Rows come by instance: its first and last differ with several.
         (several (not (equal (plist-get (cdar rows) :provider) (plist-get (cdar (last rows)) :provider))))
         (fields (cdr (if (or ask-effort several) (romacs-agent--pick-row rows) (car rows))))
         (provider (plist-get fields :provider))
         (model (plist-get fields :model))
         (efforts (plist-get fields :efforts))
         (own (plist-get (romacs-agent--configured provider) :effort))
         (effort (when (and efforts (or ask-effort (and model (not own) (not (plist-get fields :default-effort)))))
                   (romacs-agent--read-effort efforts (if (member own efforts) own (plist-get fields :default-effort))))))
    (cons provider (append (and model (list :model model)) (and effort (list :effort effort))))))

;;; What a harness is told of Romacs

(defconst romacs-agent--runtime-note
  "You are running inside Romacs, an Emacs-compatible editor, through its agent client.  \
The user reads your replies as Markdown in an editor buffer: name files by absolute path, \
which the user can follow as links.  The romacs_* tools act on the user's live editor \
session, its buffers, windows and state: treat it as their real work.  \
romacs_delegate_task starts a separate agent that sees only the brief you write."
  "What every session's harness is told of where it runs, beside its own
instructions.")

(defun romacs-agent--instructions (_session)
  "What a session's harness is told of Romacs, for its system prompt: the
same for every session, so a delegated one on the same model can reuse its
parent's cached prompt.  What a delegated one is told besides is in its
brief."
  romacs-agent--runtime-note)

;;; Writing to a harness

(defun romacs-agent--option-words (session model-flag effort-flag)
  "SESSION's model and effort as MODEL-FLAG and EFFORT-FLAG words."
  (let ((options (romacs-agent-options session)))
    (append (when-let* ((model (plist-get options :model))) (list model-flag model))
            (when-let* ((effort (plist-get options :effort))) (list effort-flag effort)))))

(defun romacs-agent--line (session object &optional journal)
  "Write OBJECT as one line to SESSION's harness; JOURNAL, when given, is
what the journal keeps of it.  A detached session whose supervisor still
runs is attached first; one whose supervisor ended says so: it resumes with
`romacs-agent-resume'."
  (let ((connection (romacs-agent-connection session))
        ;; The harness's line is text inside the envelope: serialized once.
        (line (concat (json-serialize
                       (if journal
                           (romacs-mcp--object "write" (romacs-agent--json object) "journal" (romacs-agent--json journal))
                         (romacs-mcp--object "write" (romacs-agent--json object))))
                      "\n")))
    (cond ((process-live-p connection) (process-send-string connection line))
          ;; A stopped supervisor leaves its socket: only a connection says it listens.
          ((and (file-exists-p (romacs-agent-host-socket session)) (processp (romacs-agent--attach session 0)))
           (process-send-string (romacs-agent-connection session) line))
          (t (user-error "Session %s has ended: resume it (r) to go on" (romacs-agent-title session))))))

(defun romacs-agent--image-caption (file)
  "The line telling a model where an attached image FILE is saved."
  (format "[Attached image saved at: %s]" file))

(defconst romacs-agent--image-caption-regexp
  "\\`\\[Attached image saved at: [^\n]*\\]\\'"
  "A text block that is an image's caption (`romacs-agent--image-caption'):
the image's, not the message's text.")

(defun romacs-agent--text-of (content)
  "The text of a message CONTENT: a string or a vector of blocks, images'
captions left out."
  (if (stringp content) content
    (mapconcat (lambda (block)
                 (let ((text (and (hash-table-p block) (gethash "text" block))))
                   (if (and text (not (string-match-p romacs-agent--image-caption-regexp text))) text "")))
               content "")))

(defcustom romacs-agent-image-max-bytes (* 20 1024 1024)
  "The largest image that may be attached or sent: one goes whole into a
message, on the main VM, and a harness refuses what is far beyond this."
  :type 'natnum :group 'romacs-agent)

(defun romacs-agent-image-file-p (file)
  "Whether FILE is a regular file with an image's name, of a size to send."
  (and (string-match-p "\\.\\(?:png\\|jpe?g\\|webp\\|gif\\)\\'" file)
       (file-regular-p file) (file-readable-p file)
       ;; The file a read would get, past any symbolic link.
       (<= (file-attribute-size (file-attributes (file-truename file))) romacs-agent-image-max-bytes)))

(defun romacs-agent--sendable-images (images)
  "IMAGES that may still be sent: a queued message's file can change while
it waits.  A file left out is said, as its message goes on without it."
  (seq-filter (lambda (file)
                (or (romacs-agent-image-file-p file)
                    (progn (message "Not sending %s: gone, or over `romacs-agent-image-max-bytes'" file) nil)))
              images))

(defun romacs-agent--value (value)
  "VALUE, with JSON null and false as nil."
  (unless (memq value '(:null :false)) value))

;;; Per-turn harnesses

(defun romacs-agent--per-turn-p (session)
  (eq (romacs-agent--harness-get session :lifetime) 'per-turn))

(defun romacs-agent--argv (session resume)
  "SESSION's harness's command line, RESUME going on with its conversation."
  (funcall (romacs-agent--harness-get session :argv) session resume))

(defvar romacs-agent--message-images nil
  "The images the user attached to the message being written to a harness,
for its driver.  Only what was attached: nothing is ever read from a
message's text, which a child's result or a quote can fill.")

(defun romacs-agent--input (session text &optional images)
  "Write TEXT to SESSION's persistent harness as the user's message, with
the attached IMAGES.  The journal keeps the message as it would be without
them, and their files: a replay reads every record, and no image's bytes.
Both are made with one request id: the journal's is the one written."
  (let* ((input (romacs-agent--harness-get session :input))
         (images (romacs-agent--sendable-images images))
         (id (romacs-agent-request-id session))
         (record (funcall input session text id)))
    ;; The journal's alone: the id names the message's turn.
    (puthash "romacsRequest" id record)
    (when images (puthash "images" (vconcat images) record))
    (romacs-agent--line session
                        (let ((romacs-agent--message-images images)) (funcall input session text id))
                        record)
    (run-hook-with-args 'romacs-agent-input-functions session id)))

(defun romacs-agent--uuid-p (id)
  "Whether ID is a UUID, as Claude Code's and Codex's conversations are named."
  (and (stringp id) (string-match-p "\\`[0-9a-f]\\{8\\}-[0-9a-f-]\\{27\\}\\'" id)))

(defun romacs-agent--native-id (session)
  "The harness's name for SESSION's conversation: an imported or a
per-turn harness's own id, else the session's."
  (or (romacs-agent-harness-session session) (romacs-agent-id session)))

(defun romacs-agent--home (harness config-dir)
  "HARNESS's configuration directory CONFIG-DIR (nil: its default), absolute."
  (directory-file-name (expand-file-name (or config-dir (plist-get (romacs-agent--driver-of harness) :config-default)))))

(defun romacs-agent--native-key (harness config-dir native)
  "The identity of HARNESS's conversation NATIVE in CONFIG-DIR: two
configuration directories can hold the same id."
  (format "%s\n%s\n%s" harness (romacs-agent--home harness config-dir) native))

(defun romacs-agent--import-id (harness config-dir native)
  "The session id of HARNESS's conversation NATIVE in CONFIG-DIR, imported:
a digest of its identity, UUID-shaped."
  (let ((hash (secure-hash 'sha256 (romacs-agent--native-key harness config-dir native))))
    (format "%s-%s-%s-%s-%s" (substring hash 0 8) (substring hash 8 12) (substring hash 12 16)
            (substring hash 16 20) (substring hash 20 32))))

(defun romacs-agent--native-session (harness config-dir native)
  "The session going on with HARNESS's conversation NATIVE in CONFIG-DIR."
  (let ((key (romacs-agent--native-key harness config-dir native)))
    (cl-find-if (lambda (session)
                  (and (eq (romacs-agent-harness session) harness)
                       (equal (romacs-agent--native-key harness (romacs-agent-config-dir session)
                                                        (romacs-agent--native-id session))
                              key)))
                (hash-table-values romacs-agent--sessions))))

(defun romacs-agent--turn (session text &optional images)
  "Run one turn of SESSION's per-turn harness on prompt TEXT with the
attached IMAGES.  The supervisor journals TEXT and gives it to the harness
itself, so the turn goes on whatever becomes of this server."
  (romacs-agent--check-config-dir (romacs-agent-provider session) (romacs-agent-config-dir session))
  (let ((input (expand-file-name "input" (romacs-agent-dir session)))
        (images (romacs-agent--sendable-images images)))
    (let ((coding-system-for-write 'utf-8-unix))
      (write-region text nil input nil 'silent))
    (set-file-modes input #o600)
    (setf (romacs-agent-journal-start session)
          (or (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" (romacs-agent-dir session)))) 0))
    (romacs-agent--launch session (let ((romacs-agent--message-images images)) (romacs-agent--argv session t))
                          input images))
  (setf (romacs-agent-state session) 'starting)
  (romacs-agent--attach session 50))

;;; Journal records

(defun romacs-agent--raise (session event)
  "Apply EVENT to SESSION's state and give it to the event functions."
  (when (plist-get event :time) (setf (romacs-agent-last-time session) (plist-get event :time)))
  (pcase (plist-get event :type)
    ('session (let ((native (plist-get event :harness-session)))
                ;; A harness naming its conversation after launch (ACP's
                ;; session/new): its manifest knows it for a resume.
                (when (and native (not (equal native (romacs-agent-harness-session session))))
                  (romacs-agent--manifest-put session "harnessSession" native)))
              (setf (romacs-agent-model session) (or (plist-get event :model) (romacs-agent-model session))
                    (romacs-agent-harness-session session) (plist-get event :harness-session)
                    (romacs-agent-state session) 'running)
              (when (plist-get event :permission-mode)
                (setf (romacs-agent-permission-mode session) (plist-get event :permission-mode))))
    ('usage (setf (romacs-agent-context-used session) (plist-get event :context-used)))
    ('selection (let ((model (plist-get (plist-get event :to) :model)))
                  (unless (equal model (plist-get (plist-get event :from) :model))
                    ;; The context held stays; its window was the last model's.
                    (setf (romacs-agent-context-window session) nil))
                  (when model (setf (romacs-agent-model session) model))))
    ;; A run starts (the supervisor's first record): its startup is its own,
    ;; and the record's seq names the run.
    ;; A run starts on its driver as defined now: one loaded since speaks
    ;; to the next run, never to the one going on.
    ('run (setf (romacs-agent-driver session) nil
                (romacs-agent-driver-state session) (list :run (plist-get event :seq))
                (romacs-agent-startup-pending session) nil))
    ('handshake (let ((state (romacs-agent-driver-state session)))
                  (cl-loop for (key value) on (plist-get event :state) by #'cddr
                           do (setq state (plist-put state key value)))
                  (setf (romacs-agent-driver-state session) state)))
    ('input (setf (romacs-agent-state session) 'running
                  (romacs-agent-turn-start session) (plist-get event :time)
                  (romacs-agent-input-seq session) (or (plist-get event :seq) (romacs-agent-input-seq session)))
            (when (romacs-agent-title-auto session) (romacs-agent--title-from session (plist-get event :text))))
    ('approval (setf (romacs-agent-state session) 'waiting-approval))
    ('answered (setf (romacs-agent-state session) 'running))
    ('done (setf (romacs-agent-state session) 'idle
                 (romacs-agent-reply session) (plist-get event :text)
                 (romacs-agent-failed session) (plist-get event :error))
           (cl-incf (romacs-agent-turns session))
           (setf (romacs-agent-done-seq session) (or (plist-get event :seq) 0)
                 (romacs-agent-completed-seq session) (or (plist-get event :seq) 0)
                 (romacs-agent-interrupted session) nil)
           (romacs-agent--note-end session)
           (when (plist-get event :context-window)
             (setf (romacs-agent-context-window session) (plist-get event :context-window)))
           ;; Claude's cost is the session's so far, for queries; no view shows it.
           (when (numberp (plist-get event :cost))
             (setf (romacs-agent-cost session) (plist-get event :cost)))
           ;; A long-lived harness's turn ended: a new selection applies,
           ;; then the first message queued for it is next (a per-turn
           ;; one's run ends with it).
           (when (and (or (romacs-agent-queue session) (romacs-agent-pending-selection session))
                      (not (romacs-agent--per-turn-p session))
                      (process-live-p (romacs-agent-connection session)))
             (run-at-time 0 nil #'romacs-agent--settle session)))
    ('exit
     ;; A run's startup is its own: the next run starts over.  What its
     ;; harness asked is no longer anyone's to answer.
     (setf (romacs-agent-driver-state session) nil (romacs-agent-startup-pending session) nil)
     (clrhash (romacs-agent-approvals session))
     ;; Ended with its last prompt's turn never done: cut, however many
     ;; runs (a resumed one journals no prompt) it took.
     (setf (romacs-agent-interrupted session)
           (and (not (romacs-agent--per-turn-p session))
                (> (or (romacs-agent-input-seq session) 0) (romacs-agent-completed-seq session))))
     (unless (eq (romacs-agent-state session) 'idle)
       ;; It ended mid-turn: that turn failed.
       (setf (romacs-agent-failed session) t
             (romacs-agent-reply session) (format "the harness exited (%s) before its turn ended" (plist-get event :status))
             (romacs-agent-done-seq session) (or (plist-get event :seq) 0))
       (romacs-agent--note-end session))
     (setf (romacs-agent-state session) (if (romacs-agent--per-turn-p session) 'idle 'ended))))
  (run-hook-with-args 'romacs-agent-event-functions session event))

(defcustom romacs-agent-preambles
  '("\\`Act as the [[:alnum:] -]+ sub-agent for this task\\.?\\'"
    "\\`Context handoff ([[:alnum:]_ -]+):?\\'")
  "Regexps of the first lines an orchestrator (T3 Code) puts before a
delegated task: a title is the task's first line after them."
  :type '(repeat regexp) :group 'romacs-agent)

(defun romacs-agent--task-line (text)
  "TEXT's first line that says what it is about: an orchestrator's
preamble lines (`romacs-agent-preambles') skipped, unless there is no
other; empty for none."
  (let ((lines (split-string (or text "") "\n" t "[ \t]+")))
    (while (and (cdr lines)
                (cl-some (lambda (regexp) (string-match-p regexp (car lines))) romacs-agent-preambles))
      (setq lines (cdr lines)))
    (or (car lines) "")))

(defun romacs-agent--title-from (session text)
  "Name SESSION, whose title was generated, after its first prompt TEXT."
  (let ((line (romacs-agent--task-line text)))
    (unless (string-empty-p line)
      (setf (romacs-agent-title session) (truncate-string-to-width line 60 nil nil t)
            (romacs-agent-title-auto session) nil)
      (romacs-agent--update-manifest session))))

(defun romacs-agent--manifest-put (session &rest fields)
  "Write FIELDS (NAME VALUE ...) into SESSION's manifest, when it differs."
  (let* ((file (expand-file-name "manifest.json" (romacs-agent-dir session)))
         ;; Read with null and false kept: they are written back.
         (manifest (and (file-readable-p file)
                        (with-temp-buffer
                          (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
                          (ignore-errors (json-parse-buffer :object-type 'hash-table
                                                            :null-object :null :false-object :false))))))
    (when (and manifest (cl-loop for (name value) on fields by #'cddr
                                 thereis (not (equal (gethash name manifest) value))))
      (cl-loop for (name value) on fields by #'cddr do (puthash name value manifest))
      (romacs-agent--write-json file manifest))))

(defun romacs-agent--update-manifest (session)
  "Write SESSION's title back to its manifest."
  (romacs-agent--manifest-put session "title" (romacs-agent-title session) "titleAuto" :false))

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
            (exit (gethash "exit" record :absent))
            (note (gethash "note" record :absent)))
        (unless (eq in :absent)
          (cl-incf (romacs-agent-inputs session))
          ;; The startup line written is journaled back: the next may go.
          ;; Another line written before it (an interrupt) is not it.
          (let ((pending (romacs-agent-startup-pending session)))
            (when (or (eq pending t) (equal pending (romacs-agent--step-id in)))
              (setf (romacs-agent-startup-pending session) nil))))
        (let ((events (cond ((hash-table-p out) (funcall (romacs-agent--harness-get session :events) session out))
                            ((stringp out) (list (list :type 'unknown :raw out)))
                            ;; A per-turn prompt: the supervisor's own record.
                            ((and (hash-table-p in) (stringp (gethash "prompt" in)))
                             (list (list :type 'input :text (gethash "prompt" in)
                                         :images (append (gethash "images" in) nil))))
                            ((hash-table-p in)
                             (let ((request (gethash "romacsRequest" in)))
                               (mapcar (lambda (event)
                                         (if (and request (eq (plist-get event :type) 'input))
                                             (append event (list :request request))
                                           event))
                                       (funcall (or (romacs-agent--harness-get session :input-events) #'ignore)
                                                session in))))
                            ;; A per-turn harness's prompt, written as text.
                            ((not (eq in :absent))
                             (list (list :type 'input :text (if (stringp in) in (romacs-agent--json in)))))
                            ((not (eq exit :absent)) (list (list :type 'exit :status exit)))
                            ((hash-table-p note)
                             (pcase (gethash "type" note)
                               ("run" (list (list :type 'run)))
                               ("selection" (list (list :type 'selection
                                                        :from (romacs-agent--selection-of (gethash "from" note))
                                                        :to (romacs-agent--selection-of (gethash "to" note)))))))))) 
          ;; :last marks a record's last event: a view that has shown it has
          ;; shown the whole record.
          (while events
            (romacs-agent--raise session (append (car events) (list :seq seq :last (null (cdr events))
                                                                    :time (gethash "t" record))))
            (setq events (cdr events)))
          (when (romacs-agent-caught-up session) (romacs-agent--step session)))))))

(defun romacs-agent--step-id (object)
  "What names a driver line written, to know its echo by: its request id
\=(JSON-RPC's \"id\", a control message's \"request_id\", or that of the
response it carries); t for one with none, whose echo is the next line
journaled."
  (or (and (hash-table-p object)
           (or (gethash "id" object) (gethash "request_id" object)
               (let ((response (gethash "response" object)))
                 (and (hash-table-p response) (gethash "request_id" response)))))
      t))

(defun romacs-agent--ready-p (session)
  "Whether SESSION's harness takes messages: started up, when its driver
has startup steps."
  (or (not (romacs-agent--harness-get session :next))
      (plist-get (romacs-agent-driver-state session) :ready)))

(defun romacs-agent--step (session)
  "Write SESSION's harness's next driver line (a startup step, or a line it
owes), once its records are the live ones and the last one written is
journaled back (a replay never sends one again); once ready, what waited
goes."
  (when-let* ((next (romacs-agent--harness-get session :next))
              ;; The live session only: a copy replaying the journal
              ;; (a chat drawn again) writes nothing.
              ((eq (romacs-agent-get (romacs-agent-id session)) session))
              ((not (romacs-agent-startup-pending session)))
              ((process-live-p (romacs-agent-connection session)))
              (step (funcall next session)))
    (cond
     ((not (eq step :ready))
      (setf (romacs-agent-startup-pending session) (romacs-agent--step-id step))
      (romacs-agent--line session step))
     ((plist-get (romacs-agent-driver-state session) :ready))
     (t
      (setf (romacs-agent-driver-state session) (plist-put (romacs-agent-driver-state session) :ready t))
      ;; Ready between turns: idle, and the first message waiting goes.
      (when (and (memq (romacs-agent-state session) '(starting running))
                 (<= (or (romacs-agent-input-seq session) 0) (romacs-agent-completed-seq session)))
        (setf (romacs-agent-state session) 'idle))
      (when (romacs-agent-queue session) (romacs-agent--send-next session t))))))

(defun romacs-agent--read-journal (session function &optional from)
  "Call FUNCTION with each record of SESSION's journal, in order, from
byte FROM (a record's start) when given."
  (let ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
    (when (file-readable-p journal)
      (with-temp-buffer
        (let ((coding-system-for-read 'utf-8)) (insert-file-contents journal nil from))
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
                ((gethash "attached" record)
                 (romacs-agent--kept session process)
                 ;; The replay is over: what comes now is live.
                 (setf (romacs-agent-caught-up session) t)
                 (romacs-agent--step session)
                 ;; A selection a restart found armed, with no turn to end.
                 (when (romacs-agent-pending-selection session) (romacs-agent--settle session)))
                (t (romacs-agent--kept session process)
                   (condition-case err (romacs-agent--record session record)
                     (error (message "romacs-agent: %s" (error-message-string err)))))))))
    (process-put process 'pending (substring text start))))

(defun romacs-agent--sentinel (process _event)
  (unless (process-live-p process)
    (let ((session (process-get process 'romacs-agent)))
      (when (eq (romacs-agent-connection session) process)
        (setf (romacs-agent-connection session) nil)
        (romacs-agent--drop-relays session)
        ;; A run so short its supervisor left before answering the
        ;; attachment: its journal has what was not sent.
        (when (and (romacs-agent--per-turn-p session) (not (eq (romacs-agent-state session) 'idle)))
          (romacs-agent--caught-up-p session))
        (if (and (romacs-agent--per-turn-p session) (eq (romacs-agent-state session) 'idle))
            (romacs-agent--run-ended session)
          (unless (eq (romacs-agent-state session) 'ended)
            (setf (romacs-agent-state session) 'detached))
          (romacs-agent--raise session (list :type 'status :text "detached"))
          ;; The harness gone (a turn cut, a restart asked for): a selection
          ;; waiting for it takes effect.
          (when (romacs-agent-pending-selection session) (romacs-agent--settle session)))))))

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
        (setf (romacs-agent-connection session) process (romacs-agent-caught-up session) nil)
        ;; From the last record seen, not the one after: a kept attachment
        ;; always hears a line back (that record, seen and skipped, or the
        ;; acknowledgement), the proof its relays may lead here.
        (process-send-string
         process (concat (json-serialize
                          (romacs-mcp--object "attach" (romacs-mcp--object "from" (max 1 (romacs-agent-seq session))
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
  "A per-turn harness's run ended: a new selection applies, then the next
waiting message starts a turn."
  (when (and (romacs-agent--per-turn-p session) (eq (romacs-agent-state session) 'idle))
    (romacs-agent--settle session)))

;;; A new model or effort

(defun romacs-agent--selection-of (object)
  "OBJECT, a journal's or manifest's selection, as a plist."
  (and (hash-table-p object)
       (list :model (romacs-agent--value (gethash "model" object))
             :effort (romacs-agent--value (gethash "effort" object)))))

(defun romacs-agent--selection-object (selection)
  (romacs-mcp--object "model" (or (plist-get selection :model) :null)
                      "effort" (or (plist-get selection :effort) :null)))

(defun romacs-agent--selection (session)
  "SESSION's model and effort now, as a plist."
  (let ((options (romacs-agent-options session)))
    (list :model (plist-get options :model) :effort (plist-get options :effort))))

(defun romacs-agent--wanted (session)
  "SESSION's selection to come: the one waiting, else the one running."
  (let ((switch (romacs-agent-pending-selection session)))
    (if switch (list :model (plist-get switch :model) :effort (plist-get switch :effort))
      (romacs-agent--selection session))))

(defun romacs-agent--efforts (session)
  "The efforts SESSION's instance offers the model it is to run."
  (romacs-agent--efforts-of (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session))
                            (plist-get (romacs-agent--wanted session) :model)))

(defun romacs-agent--switch-object (session)
  "SESSION's waiting selection for its manifest, or null.  One that took
effect and only holds messages until the harness is back waits for nothing
a restart must do."
  (let ((switch (romacs-agent-pending-selection session)))
    (if (and switch (not (eq (plist-get switch :state) 'resuming)))
        (romacs-agent--selection-object switch)
      :null)))

(defun romacs-agent--keep-switch (session)
  "Write SESSION's waiting selection to its manifest: it survives a restart."
  (romacs-agent--manifest-put session "switch" (romacs-agent--switch-object session)))

(defun romacs-agent-select (session model effort)
  "Run SESSION on MODEL at EFFORT from its next turn on; nil for either
leaves it to the harness.  A running turn ends as it began, and messages
wait until the change is in effect.  The last selection asked for wins.
Return whether anything changes."
  (let* ((target (list :model model :effort effort))
         (switch (romacs-agent-pending-selection session))
         (state (plist-get switch :state)))
    (unless (equal target (romacs-agent--wanted session))
      (romacs-agent--cancel-select session)
      (setf (romacs-agent-pending-selection session)
            (cond
             ;; Back to what runs, before anything was sent.
             ((and (eq state 'armed) (equal target (romacs-agent--selection session))) nil)
             ;; A restart under way goes on, to this selection.
             ((eq state 'restarting) (plist-put (plist-put switch :model model) :effort effort))
             (t (append (list :state 'armed) target))))
      (romacs-agent--keep-switch session)
      (romacs-agent--raise session (list :type 'status :text "selection armed"))
      ;; Requests already sent leave the harness between the two: only a
      ;; new one is sure to be on this selection.
      (if (eq state 'applying) (romacs-agent--restart session) (romacs-agent--settle session))
      t)))

(defun romacs-agent--harness-live-p (session)
  "Whether SESSION's persistent harness runs and is attached here."
  (and (not (romacs-agent--per-turn-p session)) (not (eq (romacs-agent-state session) 'ended))
       (process-live-p (romacs-agent-connection session))))

(defun romacs-agent--harness-free-p (session)
  "Whether nothing stands between SESSION's harness and a new selection:
no turn runs, a per-turn harness's run has retired, and a supervisor that
may run has been heard from."
  (let ((live (process-live-p (romacs-agent-connection session))))
    (pcase (romacs-agent-state session)
      ((or 'running 'waiting-approval) nil)
      ;; Waiting for its first message, unless it is taking up a cut turn.
      ('starting (and live (romacs-agent-caught-up session) (not (romacs-agent--per-turn-p session))
                      (not (romacs-agent-interrupted session))))
      ('detached (not (or live (file-exists-p (romacs-agent-host-socket session)))))
      (_ (if live (and (romacs-agent-caught-up session) (not (romacs-agent--per-turn-p session))) t)))))

(defun romacs-agent--settle (session)
  "A point where SESSION's new selection may apply; with none waiting, its
next message goes."
  (let ((switch (romacs-agent-pending-selection session)))
    (pcase (plist-get switch :state)
      ('nil (romacs-agent--send-next session))
      ('armed (when (romacs-agent--harness-free-p session) (romacs-agent--apply-switch session)))
      ('restarting (romacs-agent--restarted session))
      ;; Attached to the new harness: what waited goes, after a cut turn
      ;; it takes up, if any.
      ('resuming (setf (romacs-agent-pending-selection session) nil)
                 (romacs-agent--send-next session (not (romacs-agent-interrupted session)))))))

(defun romacs-agent--apply-switch (session)
  "Put SESSION's armed selection in effect."
  (let* ((switch (romacs-agent-pending-selection session))
         (select (romacs-agent--harness-get session :select))
         (how (cond ((not (romacs-agent--harness-live-p session)) nil)
                    (select (funcall select session (plist-get switch :model) (plist-get switch :effort)))
                    (t 'restart))))
    (cond ((null how) (romacs-agent--commit-switch session))
          ((eq how 'restart) (romacs-agent--restart session))
          (t (setf (romacs-agent-pending-selection session)
                   (append (list :state 'applying :waiting (mapcar #'car how)
                                 :timer (run-at-time romacs-agent-select-timeout nil
                                                     #'romacs-agent--restart session))
                           (cl-loop for (key value) on switch by #'cddr
                                    unless (eq key :state) append (list key value))))
             (dolist (entry how) (romacs-agent--line session (cdr entry)))))))

(defun romacs-agent--select-answered (session id ok)
  "The harness answered ID of SESSION's selection, confirming it when OK;
return whether ID was one.  A refusal restarts SESSION on the selection."
  (let ((switch (romacs-agent-pending-selection session)))
    (when (member id (plist-get switch :waiting))
      (if (not ok)
          (romacs-agent--restart session)
        (let ((waiting (remove id (plist-get switch :waiting))))
          (setf (romacs-agent-pending-selection session) (plist-put switch :waiting waiting))
          (unless waiting (romacs-agent--commit-switch session))))
      t)))

(defun romacs-agent--cancel-select (session)
  (when-let* ((timer (plist-get (romacs-agent-pending-selection session) :timer)))
    (cancel-timer timer)))

(defun romacs-agent--commit-switch (session)
  "SESSION's selection is in effect: recorded, noted, and its messages go."
  (romacs-agent--cancel-select session)
  (let* ((from (romacs-agent--selection session))
         (switch (romacs-agent-pending-selection session))
         (to (list :model (plist-get switch :model) :effort (plist-get switch :effort)))
         (options (romacs-agent-options session)))
    (setf (romacs-agent-options session)
          (append (cl-loop for key in '(:model :effort)
                           when (plist-get to key) append (list key (plist-get to key)))
                  (cl-loop for (key value) on options by #'cddr
                           unless (memq key '(:model :effort)) append (list key value)))
          (romacs-agent-pending-selection session) nil)
    (romacs-agent--manifest-put session "options" (romacs-agent--options-object (romacs-agent-options session))
                                "switch" :null)
    (romacs-agent--note session (romacs-mcp--object "type" "selection"
                                                    "from" (romacs-agent--selection-object from)
                                                    "to" (romacs-agent--selection-object to)))
    ;; With no harness to take them, messages wait for the resume.
    (when (or (romacs-agent--per-turn-p session) (romacs-agent--harness-live-p session))
      (romacs-agent--send-next session))))

(defun romacs-agent--note (session note)
  "Journal NOTE in SESSION's journal, in its place: through its supervisor
when one runs; with none, the journal is the server's to add to."
  (let ((connection (romacs-agent-connection session)))
    (if (romacs-agent--harness-live-p session)
        (process-send-string connection (concat (json-serialize (romacs-mcp--object "note" note)) "\n"))
      (let* ((seq (1+ (romacs-agent-seq session)))
             (record (romacs-mcp--object "seq" seq "t" (float-time) "note" note))
             (coding-system-for-write 'utf-8-unix))
        (write-region (concat (romacs-agent--json record) "\n") nil
                      (expand-file-name "journal.jsonl" (romacs-agent-dir session)) t 'silent)
        (romacs-agent--record session record)))))

(defun romacs-agent--restart (session)
  "Stop SESSION's harness to go on with its selection in a new one."
  (romacs-agent--cancel-select session)
  (setf (romacs-agent-pending-selection session)
        (plist-put (plist-put (romacs-agent-pending-selection session) :state 'restarting) :waiting nil))
  (if-let* ((unit (romacs-agent-unit session)))
      ;; The next run takes the unit's name: this one must be gone.
      (make-process :name "romacs-agent-restart" :noquery t :connection-type 'pipe
                    :command (list "systemctl" "--user" "stop" unit)
                    :sentinel (lambda (process _event)
                                (unless (process-live-p process)
                                  (setf (romacs-agent-pending-selection session)
                                        (plist-put (romacs-agent-pending-selection session) :stopped t))
                                  (romacs-agent--restarted session))))
    (setf (romacs-agent-pending-selection session) (plist-put (romacs-agent-pending-selection session) :stopped t))
    (romacs-agent-stop session)
    ;; Gone already, no sentinel is to come.
    (romacs-agent--restarted session)))

(defun romacs-agent--restarted (session)
  "Resume SESSION on its selection, once its harness and its unit are gone."
  (when (and (plist-get (romacs-agent-pending-selection session) :stopped)
             (not (process-live-p (romacs-agent-connection session))))
    (romacs-agent--commit-switch session)
    ;; Messages go on waiting until the new supervisor is there to take them.
    (setf (romacs-agent-pending-selection session) (list :state 'resuming))
    (romacs-agent-resume session)))

(defun romacs-agent--send-next (session &optional force)
  "Send the first message waiting for SESSION's turn to end, unless a
turn runs again (FORCE: none can, the harness just resumed): a per-turn
harness's starts a turn.  One dispatch at a time: a turn that ends while
its message is being sent (a run that ends before it is attached) asks
for the next, sent once this one is off the queue."
  (cond
   ;; Nothing reaches a harness while its selection changes.
   ((romacs-agent-pending-selection session))
   ((romacs-agent-dispatch session)
    (setf (romacs-agent-dispatch session) 'again))
   (t
    (setf (romacs-agent-dispatch session) 'busy)
    (unwind-protect
        (let ((force force) (more t))
          (while more
            (setq more nil)
            (unless (and (not force) (memq (romacs-agent-state session) '(starting running waiting-approval)))
              (cl-flet ((drop () (pop (romacs-agent-queue session))
                          (cl-decf (romacs-agent-queue-length session))
                          (unless (romacs-agent-queue session) (setf (romacs-agent-queue-tail session) nil))))
                (let ((next (car (romacs-agent-queue session))))
                  (while (and next (nth 2 next) (not (funcall (nth 2 next))))
                    (drop)
                    (setq next (car (romacs-agent-queue session))))
                  (when (and next (or (romacs-agent--per-turn-p session) (romacs-agent--ready-p session)))
                    ;; Taken off the queue once sent: a send that fails (the
                    ;; harness gone meanwhile) keeps it for the next turn end
                    ;; or resume.
                    (if (romacs-agent--per-turn-p session)
                        (romacs-agent--turn session (car next) (nth 3 next))
                      (romacs-agent--input session (car next) (nth 3 next)))
                    (drop)
                    (when (nth 1 next) (funcall (nth 1 next)))
                    (setq more (eq (romacs-agent-dispatch session) 'again) force nil)
                    (setf (romacs-agent-dispatch session) 'busy)))))))
      (setf (romacs-agent-dispatch session) nil)))))

(defun romacs-agent-send-later (session text &optional first images)
  "Send TEXT to SESSION, with the attached IMAGES, once its running turn
ends, now when none runs; FIRST puts it ahead of the messages already
waiting."
  (if (not (memq (romacs-agent-state session) '(starting running waiting-approval)))
      (romacs-agent-send session text nil nil images)
    (if (and first (romacs-agent-queue session))
        (progn (push (list text nil nil images) (romacs-agent-queue session))
               (cl-incf (romacs-agent-queue-length session)))
      (romacs-agent--enqueue session (list text nil nil images)))))

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
                       "switch" (romacs-agent--switch-object session)
                       "hostSocket" (romacs-agent-host-socket session))))

(defconst romacs-agent--option-keys
  '((:model . "model") (:effort . "effort") (:role . "role") (:parent . "parent") (:request . "clientRequestId")
    (:wake . "wake") (:worktree . "worktree") (:branch . "branch"))
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

(defconst romacs-agent--editor-variables
  '("ROMACS_RUNTIME_ROOT" "ROMACS_LISP_DIR" "ROMACS_USAGE_LOG" "ROMACS_GUI_FONT" "ROMACS_GUI_FONT_SIZE")
  "The editor's own launch variables: they describe this editor's run (its
build, its usage log, its font), so no harness or command it runs
inherits them.")

(defconst romacs-agent-git-variables '("GIT_DIR" "GIT_WORK_TREE" "GIT_INDEX_FILE")
  "What points git at another repository than the directory it runs in:
an editor's, inherited, would send a harness's git (and Romacs's own) to
the editor's checkout.")

(defconst romacs-agent-orchestrator-variables '("T3CODE_THREAD_ID" "T3CODE_CLI_PATH")
  "Variables an orchestrator running this editor sets for its own child:
no harness Romacs runs is that child.")

(defun romacs-agent--dropped-variables ()
  "What no harness inherits: the editor's launch variables, an
orchestrator's, git's repository pointers, and every harness's own (its configuration variable, and
what it sets for the commands it runs): a harness parent's would
otherwise reach any child, a Claude Code parent's a Codex child too.  A
session's own configuration variable is given again, by `env'."
  (delete-dups (append romacs-agent--editor-variables romacs-agent-orchestrator-variables romacs-agent-git-variables
                       (cl-loop for (_ . driver) in romacs-agent--drivers
                                append (cons (plist-get driver :config-variable) (plist-get driver :scrub))))))

(defun romacs-agent--harness-environment ()
  "The editor's environment without what no harness inherits
\=(`romacs-agent--dropped-variables'): a list of its own, so a `setenv'
under it never changes the editor's."
  (let ((dropped (make-hash-table :test #'equal)))
    (dolist (name (romacs-agent--dropped-variables)) (puthash name t dropped))
    (copy-sequence
     (cl-remove-if (lambda (entry)
                     (let ((end (string-search "=" entry)))
                       (and end (gethash (substring entry 0 end) dropped))))
                   process-environment))))

(defun romacs-agent--environment-arguments ()
  "systemd-run arguments giving a unit this session's environment, by
name: systemd-run takes each value from its own environment, so none
shows on a command line other users can read."
  (cl-loop for entry in process-environment
           when (string-match "\\`\\([A-Za-z_][A-Za-z0-9_]*\\)=" entry)
           collect (concat "--setenv=" (match-string 1 entry))))

(defun romacs-agent--launch (session argv &optional input images)
  "Run SESSION's supervisor with harness ARGV, as a user unit when a user
manager answers, else detached.  INPUT is a file the supervisor gives the
harness as its whole stdin, and IMAGES what the journal keeps as attached
to it."
  ;; The process launching it attaches it: its relays lead here from the start.
  (romacs-agent--route-here session)
  (let* ((host (romacs-agent--program "romacs-agent-host"))
         (command (append (list host "--dir" (romacs-agent-dir session)
                                "--socket" (romacs-agent-host-socket session)
                                ;; Journaled first: where this run's records start.
                                "--note" "{\"type\":\"run\"}")
                          (and input (list "--input" input))
                          (and images (list "--input-images" (json-serialize (vconcat images))))
                          (cons "--" argv)))
         (unit (format "romacs-agent-%s" (romacs-agent--key (romacs-agent-id session))))
         (default-directory (file-name-as-directory (romacs-agent-cwd session))))
    (make-directory (file-name-directory (romacs-agent-host-socket session)) t)
    (set-file-modes (file-name-directory (romacs-agent-host-socket session)) #o700)
    ;; Arguments (paths, a directory) go out as UTF-8, whatever the locale:
    ;; the supervisor reads UTF-8.  The harness gets the editor's
    ;; environment but for the editor's own launch variables.
    (if (zerop (let ((coding-system-for-write 'utf-8-unix)
                     (process-environment (romacs-agent--harness-environment)))
                 (apply #'call-process "systemd-run" nil nil nil
                      (append (list "--user" "--quiet" "--collect" (concat "--unit=" unit)
                                    "--slice=romacs-agents.slice"
                                    (concat "--working-directory=" (expand-file-name default-directory)))
                              (mapcar (lambda (p) (concat "--property=" p)) romacs-agent-unit-properties)
                              ;; Nor from the user manager's own environment.
                              (list (concat "--property=UnsetEnvironment="
                                            (string-join (romacs-agent--dropped-variables) " ")))
                              (romacs-agent--environment-arguments)
                              (cons "--" command)))))
        (setf (romacs-agent-unit session) unit)
      (let ((coding-system-for-write 'utf-8-unix)
            (process-environment (romacs-agent--harness-environment)))
        (apply #'call-process "setsid" nil 0 nil "--fork" command))
      (setf (romacs-agent-unit session) nil))
    (romacs-agent--manifest session argv)))

(defun romacs-agent--project-default ()
  "The directory a new session works in by default: the current
project's root, else `default-directory'.  The harness's own
configuration directory comes from its instance, never from here."
  (let ((project (project-current nil)))
    (if project (project-root project) default-directory)))

(defun romacs-agent--start-arguments (directory)
  "`romacs-agent-start''s arguments, asked for, DIRECTORY offered.
A chat opens with no initial prompt."
  (let* ((cwd (read-directory-name "Project directory: " directory))
         (selection (romacs-agent--read-selection current-prefix-arg)))
    (list cwd nil nil (car selection) (cdr selection))))

(defun romacs-agent--start-instance (provider)
  "The instance a session started under PROVIDER (nil: the default) runs
on, or a `user-error' saying why none can start there: unknown, disabled,
or its configuration directory missing."
  (let* ((name (or provider (romacs-agent--default-provider)))
         (instance (or (romacs-agent--configured name)
                       (user-error "No agent provider %s; configured: %s" name
                                   (mapconcat #'identity (cl-loop for i in (romacs-agent-instances)
                                                                  when (plist-get i :enabled) collect (plist-get i :name))
                                              ", ")))))
    (unless (plist-get instance :enabled)
      (user-error "Agent provider %s is disabled" (plist-get instance :name)))
    ;; The directory as configured: the harness's default too, when named.
    (romacs-agent--check-config-dir (plist-get instance :name) (plist-get instance :config-dir))
    instance))

;;;###autoload
(defun romacs-agent-start (cwd &optional prompt title provider options known)
  "Start a session working in the project directory CWD under instance
PROVIDER (a name in `romacs-agent-providers', by default
`romacs-agent-default-provider'), sending PROMPT when given.  OPTIONS is a plist: :model and :effort for
the harness, :role, :parent (the delegating session's id) and :request
\(its client request id) for the record.  KNOWN, when given, is called
with the session before its harness starts.  Return the session.
Interactively, open the chat without sending an initial prompt."
  (interactive (romacs-agent--start-arguments (romacs-agent--project-default)))
  (let* ((instance (romacs-agent--start-instance provider))
         ;; A T3 id names its instance.
         (name (plist-get instance :name))
         (session (progn
                    (romacs-agent--create
                     :id (romacs-agent--uuid) :harness (plist-get instance :harness) :provider name
                     ;; What the start asks for, else the instance's defaults.
                     :options (append (and (not (plist-get options :model)) (plist-get instance :model)
                                           (list :model (plist-get instance :model)))
                                      (and (not (plist-get options :effort)) (plist-get instance :effort)
                                           (romacs-agent--effort-fits-p instance (plist-get options :model)
                                                                        (plist-get instance :effort))
                                           (list :effort (plist-get instance :effort)))
                                      options)
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
        (romacs-agent--launch session (romacs-agent--argv session nil))
        (romacs-agent--attach session 50)
        (romacs-agent--when-attached session (lambda ()
                                               (romacs-agent--refresh-models session)
                                               (when prompt (romacs-agent-send session prompt))))))
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
CONFIG-DIR (nil: the harness's default).  A persistent harness resumes
THREAD at once; a per-turn one with its first message.  Romacs only
reads TRANSCRIPT."
  (unless (funcall (or (plist-get (romacs-agent--driver-of harness) :native-id-p) #'stringp) thread)
    (user-error "Not a conversation id: %s" thread))
  (romacs-agent--check-config-dir provider config-dir)
  ;; Named by the conversation's identity: one directory per conversation,
  ;; so every server finds the one that goes on with it.  Sessions imported
  ;; before took THREAD itself as their id.
  (let* ((id (romacs-agent--import-id harness config-dir thread))
         (dir (expand-file-name id (romacs-agent--state-root)))
         (legacy (expand-file-name thread (romacs-agent--state-root)))
         (existing (or (romacs-agent--native-session harness config-dir thread)
                       (romacs-agent--load dir)
                       (and (file-directory-p legacy) (romacs-agent--load legacy)
                            (romacs-agent--native-session harness config-dir thread)))))
    (if existing
        (romacs-agent--continue existing)
      (make-directory (romacs-agent--state-root) t)
      ;; Made once: a second server importing it meanwhile is told so.
      (condition-case nil (make-directory dir)
        (file-already-exists (user-error "Another Romacs is resuming this conversation: try again")))
      (let ((session (romacs-agent--create
                      :id id :harness harness :provider provider :config-dir config-dir :cwd cwd :title title
                      :harness-session thread
                      :imported (list :path transcript :size (file-attribute-size (file-attributes transcript))))))
        (if (romacs-agent--per-turn-p session)
            (progn (setf (romacs-agent-state session) 'idle)
                   (romacs-agent--manifest session nil))
          (romacs-agent--launch session (romacs-agent--argv session t))
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

(defun romacs-agent-send (session text &optional delivered valid images)
  "Send TEXT to SESSION's harness as the user's next message, then call
DELIVERED, when given.  A per-turn harness's message waits while a turn
runs (DELIVERED waits with it), then starts the next unless VALID, when
given, then says it is no longer wanted.  IMAGES are the files the user
attached to it."
  (cond ((romacs-agent-pending-selection session)
         (romacs-agent--enqueue session (list text delivered valid images)))
        ((and (not (romacs-agent--per-turn-p session)) (romacs-agent--ready-p session))
         (romacs-agent--input session text images)
         (when delivered (funcall delivered)))
        ;; Starting up: it goes once the harness is ready.  An ended one's
        ;; is refused by the write (it resumes first).
        ((not (romacs-agent--per-turn-p session))
         (if (or (process-live-p (romacs-agent-connection session)) (eq (romacs-agent-state session) 'starting))
             (romacs-agent--enqueue session (list text delivered valid images))
           (romacs-agent--input session text images)
           (when delivered (funcall delivered))))
        ((or (process-live-p (romacs-agent-connection session))
             (memq (romacs-agent-state session) '(starting running)))
         (romacs-agent--enqueue session (list text delivered valid images)))
        (t (romacs-agent--turn session text images)
           (when delivered (funcall delivered)))))

(defun romacs-agent--enqueue (session item)
  (cl-incf (romacs-agent-queue-length session))
  (let ((cell (list item)))
    (if (romacs-agent-queue session)
        (setcdr (romacs-agent-queue-tail session) cell)
      (setf (romacs-agent-queue session) cell))
    (setf (romacs-agent-queue-tail session) cell)))

(defun romacs-agent-interrupt (session)
  "Interrupt SESSION's turn; queued messages stay.  A per-turn harness's
run ends (its supervisor journals the exit): the next message continues
its thread."
  (let ((how (funcall (romacs-agent--harness-get session :interrupt) session)))
    (pcase how
      ;; The harness alone: the supervisor goes on to journal its exit.
      ('signal (when-let* (((process-live-p (romacs-agent-connection session)))
                           (pid (romacs-agent-harness-pid session)))
                 (signal-process pid 'TERM)))
      ('nil (user-error "Session %s has no turn to interrupt yet" (romacs-agent-title session)))
      (_ (romacs-agent--line session how)))))

(defun romacs-agent-respond (session request-id allow &optional message)
  "Answer SESSION's approval REQUEST-ID: ALLOW, as a driver's :respond
takes it, allows (t, or a plist of :answers or :always), else it is denied
with MESSAGE."
  (unless (gethash request-id (romacs-agent-approvals session)) (user-error "No such approval request"))
  (romacs-agent--line session (funcall (romacs-agent--harness-get session :respond) session request-id allow message)))

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
  (let ((process-environment (append (romacs-agent--harness-get session :resume-environment) process-environment)))
    (romacs-agent--launch session (romacs-agent--argv session t)))
  (setf (romacs-agent-state session) 'starting (romacs-agent-driver-state session) nil
        (romacs-agent-startup-pending session) nil)
  (romacs-agent--attach session 50)
  ;; What waited for its turn's end when the harness went goes now, unless
  ;; the harness ended in the middle of a turn: the resumed harness takes
  ;; that turn up again, and the queue goes at its end (a done), as known
  ;; from how the last run ended, not from when attaching completes.
  (when (and (romacs-agent-queue session) (not (romacs-agent-interrupted session)))
    (romacs-agent--when-attached session (lambda () (romacs-agent--send-next session t)))))

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
                      :pending-selection (when-let* ((switch (romacs-agent--selection-of (gethash "switch" manifest))))
                                (append (list :state 'armed) switch))
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
            (setf (romacs-agent-state session) 'detached))
          (when (romacs-agent-pending-selection session) (romacs-agent--settle session)))
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
  ;; A session whose supervisor runs may call a tool any moment: the
  ;; endpoint listens now, where its relay looks.
  (when (cl-some (lambda (session) (file-exists-p (romacs-agent-host-socket session)))
                 (hash-table-values romacs-agent--sessions))
    (romacs-agent--endpoint))
  (romacs-agent--sweep)
  (run-hooks 'romacs-agent-restored-hook))

;; At startup, cheaply, without loading this library unless a supervisor
;; runs: then its sessions are known and its tools answer with no command.
;;;###autoload
(progn
  (defun romacs-agent--restore-at-startup ()
    "Once started, restore agent sessions when any supervisor runs."
    (unless noninteractive
      (let ((root (expand-file-name "romacs/agents/" (or (getenv "XDG_RUNTIME_DIR")
                                                         (format "/tmp/romacs%d" (user-uid))))))
        (when (and (file-directory-p root) (file-expand-wildcards (concat root "*/host.sock")))
          (run-at-time 0 nil (lambda () (require 'romacs-agent) (romacs-agent-restore)))))))
  (add-hook 'after-init-hook #'romacs-agent--restore-at-startup))

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

(defvar romacs-agent--children (make-hash-table :test #'equal)
  "A session's id to the sessions delegated from it.")

(defun romacs-agent-children (session)
  "The sessions SESSION delegated."
  (cl-remove-if-not (lambda (child) (eq (romacs-agent-get (romacs-agent-id child)) child))
                    (gethash (romacs-agent-id session) romacs-agent--children)))

(defun romacs-agent--index (session)
  (let ((options (romacs-agent-options session)))
    (when-let* ((parent (plist-get options :parent)))
      (push session (gethash parent romacs-agent--children)))
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
   ;; A known session another process is attached to: its relay belongs
   ;; there, through its route, and retries until the route leads there.
   (when-let* ((session (romacs-agent-get (gethash "session" params))))
     (unless (process-live-p (romacs-agent-connection session))
       (run-at-time 0 nil (lambda () (when (memq peer romacs-mcp--peers) (romacs-mcp--close peer))))))
   (romacs-mcp--object)))

(provide 'romacs-agent)

;; The harnesses Romacs runs.
(require 'romacs-agent-claude)
(require 'romacs-agent-codex)
(require 'romacs-agent-ui)
(require 'romacs-agent-tools)
;;; romacs-agent.el ends here

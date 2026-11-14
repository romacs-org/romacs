;;; romacs-agent-tools.el --- What agent sessions call in Romacs -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; MCP tools for agent sessions (romacs-agent.el), on the native endpoint
;; (romacs-mcp.el).  They do what they are asked: Romacs has no permission
;; layer of its own (the harness's is the user's).  What keeps the user's
;; data safe is engineering: an edit is one undo group, the first change a
;; session makes to a file keeps that file's earlier content (its pre-image,
;; under the session's directory), and nothing is saved unless asked.
;;
;; Every call a session makes is recorded in its directory's call log
;; before it runs and again when it ends (a deferred call when its answer
;; is ready), so after a server restart the harness can ask
;; `romacs_call_outcome' whether a call whose reply was lost ran: `not
;; executed', `unknown' (it was running, or is too old to be known) or
;; `done' with the result.  A call is never run again on its own.

;;; Code:
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-mcp-editor)
(require 'romacs-mcp-session)
(require 'romacs-mcp-dev)
(require 'romacs-agent)
(require 'romacs-agent-worktree)

(defcustom romacs-agent-eval-budget 5000
  "Milliseconds an agent's `romacs_eval' may run before it is quit, as
by C-g, at its next safepoint.  0 means no budget."
  :type 'integer :group 'romacs-agent)

;; A session's calls are a log, calls.jsonl in its directory: one line per
;; state a call reaches, appended (no rewrite per call).  A new server
;; reads it once; past `romacs-agent-tools--kept' calls it keeps the newest
;; and remembers that older ones were forgotten, so a call it no longer
;; knows answers `unknown', never `not executed'.

(defconst romacs-agent-tools--kept 1024
  "Calls a new server keeps from a session's log.")

(defvar romacs-agent-tools--calls (make-hash-table :test #'equal)
  "Session id to its calls: a table from a call's key to its record, and
`:forgotten' t when older calls were dropped.")

(defvar romacs-agent-tools--generation 0
  "The last evaluation's deadline generation.")

;;; Call records

(defun romacs-agent-tools--dir (session-id)
  (expand-file-name session-id (romacs-agent--state-root)))

(defun romacs-agent-tools--log-file (session-id)
  (expand-file-name "calls.jsonl" (romacs-agent-tools--dir session-id)))

(defun romacs-agent-tools--load (session-id)
  "SESSION-ID's calls as its log left them, compacted when long."
  (let ((table (make-hash-table :test #'equal)) (order nil)
        (file (romacs-agent-tools--log-file session-id)))
    (when (file-readable-p file)
      (with-temp-buffer
        (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
        (goto-char (point-min))
        (while (not (eobp))
          (when-let* ((line (ignore-errors (json-parse-string (buffer-substring (point) (line-end-position))
                                                              :object-type 'hash-table)))
                      (key (gethash "key" line)))
            (if (equal key "forgotten")
                (puthash :forgotten t table)
              (unless (gethash key table) (push key order))
              (puthash key line table)))
          (forward-line 1)))
      (when (> (length order) romacs-agent-tools--kept)
        (dolist (key (nthcdr romacs-agent-tools--kept order)) (remhash key table))
        (puthash :forgotten t table)
        ;; A private file renamed over the log: an interrupted compaction
        ;; leaves the log as it was.
        (let ((temporary (concat file ".tmp")))
          (let ((coding-system-for-write 'utf-8-unix))
            (with-temp-file temporary
              (insert (romacs-agent--json (romacs-mcp--object "key" "forgotten")) "\n")
              (dolist (key (reverse (seq-take order romacs-agent-tools--kept)))
                (insert (romacs-agent--json (gethash key table)) "\n"))))
          (set-file-modes temporary #o600)
          (rename-file temporary file t))))
    table))

(defun romacs-agent-tools--table (session-id)
  (or (gethash session-id romacs-agent-tools--calls)
      (puthash session-id (romacs-agent-tools--load session-id) romacs-agent-tools--calls)))

(defun romacs-agent-tools--note (session-id key record)
  "Record RECORD for call KEY of SESSION-ID: in memory, and one line
appended to its log."
  (puthash "key" key record)
  (puthash key record (romacs-agent-tools--table session-id))
  (let ((dir (romacs-agent-tools--dir session-id)))
    (when (file-directory-p dir)
      (let ((coding-system-for-write 'utf-8-unix) (file (romacs-agent-tools--log-file session-id)))
        (write-region (concat (json-serialize record :false-object :false :null-object :null) "\n")
                      nil file t 'silent)
        (set-file-modes file #o600)))))

(defun romacs-agent-tools--key (peer id)
  "A call's key: the relay that sent it (its incarnation) and its request
ID, so a resumed harness reusing ids never reads another's record."
  (concat (or (process-get peer 'romacs-agent-relay) "") ":" (romacs-agent--json id)))

(defun romacs-agent-tools--text (value)
  "VALUE as the log keeps it: Unicode text (a raw byte U+FFFD), cut short."
  (romacs-mcp--unicode (truncate-string-to-width (if (stringp value) value (format "%S" value)) 1024)))

(defun romacs-agent-tools--finish (session key name start value failed)
  (romacs-agent-tools--note session key (romacs-mcp--object
                                         "state" "done" "tool" name "ms" (round (* 1000 (- (float-time) start)))
                                         "failed" (if failed t :false)
                                         "result" (romacs-agent-tools--text value))))

(defun romacs-agent-tools--call (peer id name run)
  "Run the tool call RUN for PEER, recorded when PEER is a session's
relay: `running' on disk first, then `done' with its result.  A deferred
call stays running until its answer is ready."
  (let ((session (and (processp peer) (process-get peer 'romacs-agent-session))))
    (if (not (and session id))
        (funcall run)
      (let ((key (romacs-agent-tools--key peer id)) (start (float-time)) (done nil))
        (romacs-agent-tools--note session key (romacs-mcp--object "state" "running" "tool" name "at" start))
        (unwind-protect
            (let ((value (funcall run)))
              (setq done t)
              (if (eq (car-safe value) 'romacs-mcp--deferred)
                  (let ((poll (nth 1 value)))
                    (list 'romacs-mcp--deferred
                          (lambda ()
                            (let ((ok nil))
                              (unwind-protect
                                  (let ((answer (funcall poll)))
                                    (setq ok t)
                                    (when answer (romacs-agent-tools--finish session key name start (car answer) nil))
                                    answer)
                                (unless ok (romacs-agent-tools--finish session key name start "signalled an error" t)))))))
                (romacs-agent-tools--finish session key name start value nil)
                value))
          (unless done (romacs-agent-tools--finish session key name start "signalled an error" t)))))))

(setq romacs-mcp-call-function #'romacs-agent-tools--call)

(defun romacs-agent-tools--outcome (arguments)
  (let* ((peer (plist-get romacs-mcp--active :peer))
         (session (process-get peer 'romacs-agent-session))
         (table (and session (romacs-agent-tools--table session)))
         (record (and table (gethash (romacs-agent-tools--key peer (gethash "id" arguments)) table))))
    (cond ((not session) (romacs-mcp--object "outcome" "unknown" "reason" "not an agent session"))
          ((and (not record) (gethash :forgotten table))
           (romacs-mcp--object "outcome" "unknown" "reason" "older calls are no longer recorded"))
          ((not record) (romacs-mcp--object "outcome" "not executed"))
          ((equal (gethash "state" record) "running")
           (romacs-mcp--object "outcome" "unknown" "reason" "it was running when Romacs stopped"))
          (t (romacs-mcp--object "outcome" "done" "failed" (gethash "failed" record :false)
                                 "result" (gethash "result" record))))))

;;; Editing

(defun romacs-agent-tools--session-dir ()
  (when-let* ((session (process-get (plist-get romacs-mcp--active :peer) 'romacs-agent-session)))
    (romacs-agent-tools--dir session)))

(defun romacs-agent-tools--pre-image (buffer)
  "Keep BUFFER's content as it was before this session first changed it."
  (when-let* ((dir (romacs-agent-tools--session-dir))
              (name (or (buffer-file-name buffer) (buffer-name buffer)))
              (file (expand-file-name (concat (secure-hash 'sha1 name) ".pre") (expand-file-name "pre-images" dir))))
    (unless (file-exists-p file)
      (make-directory (file-name-directory file) t)
      (with-current-buffer buffer
        (save-restriction
          (widen)
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (point-min) (point-max) file nil 'silent))))
      (set-file-modes file #o600)
      (with-temp-file (concat file ".name") (insert name)))))

(defun romacs-agent-tools--live-buffer (name)
  (or (get-buffer name) (user-error "No buffer named %s" name)))

(defun romacs-agent-tools--buffer-edit (arguments)
  "Apply every edit, last first, as one undo group, or none."
  (romacs-mcp--instance arguments)
  (let* ((buffer (romacs-agent-tools--live-buffer (gethash "name" arguments)))
         (tick (gethash "expectedTick" arguments))
         (edits (append (gethash "edits" arguments) nil)))
    (with-current-buffer buffer
      (when (and (integerp tick) (/= tick (buffer-modified-tick)))
        (user-error "Buffer changed (tick %d, expected %d): read it again" (buffer-modified-tick) tick))
      (when buffer-read-only (user-error "Buffer %s is read-only" (buffer-name)))
      (let ((sorted (sort (mapcar (lambda (edit)
                                    (let ((start (gethash "start" edit)) (end (gethash "end" edit))
                                          (text (gethash "text" edit)))
                                      (unless (and (integerp start) (integerp end) (stringp text)
                                                   (<= (point-min) start end (point-max)))
                                        (user-error "Bad edit: start and end in %d..%d, and text" (point-min) (point-max)))
                                      (list start end text)))
                                  edits)
                          (lambda (a b) (> (car a) (car b))))))
        (cl-loop for (a b) on sorted while b
                 when (> (cadr b) (car a)) do (user-error "Edits overlap"))
        (romacs-agent-tools--pre-image buffer)
        (undo-boundary)
        ;; All or none: a failing edit (read-only text) undoes the others.
        (atomic-change-group
          (save-excursion
            (dolist (edit sorted)
              (goto-char (car edit))
              (delete-region (car edit) (cadr edit))
              (insert (nth 2 edit)))))
        (undo-boundary)
        (romacs-mcp--object "name" (buffer-name) "tick" (buffer-modified-tick) "size" (buffer-size))))))

(defun romacs-agent-tools--visit (arguments)
  (let* ((path (expand-file-name (gethash "path" arguments)))
         (buffer (find-file-noselect path))
         (line (gethash "line" arguments)))
    (with-current-buffer buffer
      (when (integerp line) (goto-char (point-min)) (forward-line (1- line)))
      (romacs-mcp--object "name" (buffer-name) "tick" (buffer-modified-tick) "modified" (if (buffer-modified-p) t :false)))))

(defun romacs-agent-tools--file-write (arguments)
  "Replace a file's buffer text; save only when asked."
  (romacs-mcp--instance arguments)
  (let* ((path (expand-file-name (gethash "path" arguments)))
         (buffer (find-file-noselect path)))
    (with-current-buffer buffer
      (when buffer-read-only (user-error "Buffer %s is read-only" (buffer-name)))
      (romacs-agent-tools--pre-image buffer)
      (undo-boundary)
      (atomic-change-group
        (save-restriction
          (widen)
          (erase-buffer)
          (insert (gethash "text" arguments))))
      (undo-boundary)
      (when (eq (gethash "save" arguments) t) (save-buffer))
      (romacs-mcp--object "name" (buffer-name) "tick" (buffer-modified-tick)
                          "saved" (if (buffer-modified-p) :false t)))))

;;; Evaluation

(defun romacs-agent-tools--bounded (arguments run)
  "Call RUN within ARGUMENTS' budget: quit, as by C-g, when it runs out."
  (let ((budget (let ((b (gethash "budgetMs" arguments))) (if (integerp b) b romacs-agent-eval-budget)))
        (generation (cl-incf romacs-agent-tools--generation)))
    (when (> budget 0) (romacs--call-deadline generation budget))
    (unwind-protect (funcall run)
      (romacs--call-deadline nil))))

(defun romacs-agent-tools--eval (arguments)
  "`romacs_eval' with a budget."
  (romacs-mcp--instance arguments)
  (romacs-agent-tools--bounded
   arguments
   ;; A command the agent's code starts gets no editor launch variable
   ;; either (`romacs-agent--editor-variable-p').
   (lambda ()
     (let ((process-environment (romacs-agent--harness-environment)))
       (romacs-mcp-eval-string (gethash "code" arguments))))))

;;; Delegation (T3 Code's delegate_task, task_status, task_cancel)
;;
;; A session delegates a task to a child session under any instance of any
;; harness: `wait' answers when the child's turn ends (deferred: the editor
;; keeps running), `async' answers at once and wakes the parent with the
;; child's result when it ends, as T3 Code wakes a parent thread.  A wake
;; is sent once per child turn: the child's directory records the journal
;; seq last delivered once the parent's harness has it (`woken'), and each
;; end a `wait' answered with (`answered'), so a server restarted meanwhile
;; delivers what it missed, and a replay never repeats one.

(defun romacs-agent-tools--caller ()
  "The id of the session calling the current tool, or nil."
  (let ((peer (plist-get romacs-mcp--active :peer)))
    (and (processp peer) (process-get peer 'romacs-agent-session))))

(defun romacs-agent-tools--status (session)
  "SESSION's task status, in T3 Code's words."
  (let ((state (romacs-agent-state session)))
    (cond ((romacs-agent-cancelled session) "cancelled")
          ((eq state 'waiting-approval) "waiting")
          ;; Never started: failed, whatever waits for it, its state saying
          ;; so (not detached).
          ((eq state 'unstarted) "failed")
          ((or (memq state '(starting running)) (romacs-agent-queue session)) "running")
          ((romacs-agent-failed session) "failed")
          ((> (romacs-agent-turns session) 0) "completed")
          ((eq state 'ended) "failed")
          (t (symbol-name state)))))

(defun romacs-agent-tools--running-next (session)
  "What a caller is told to do about SESSION, a task still running."
  (pcase (romacs-agent-tools--wake-rule session)
    ("none" "It will not wake you: read its result with romacs_task_status when you need it.")
    ("failure" "Only its failure wakes you, so end your turn now: do not wait on it or poll it.")
    ("all" "You are woken once every task you delegated with wake all has ended, so end your turn now: do not wait or poll.")
    (_ "Its end wakes you with its result, so end your turn now: do not wait on it or poll it.")))

(defun romacs-agent-tools--task (session)
  "SESSION as a task record; one still running says what to do next."
  (let* ((options (romacs-agent-options session)) (null (lambda (v) (or v :null)))
         (status (romacs-agent-tools--status session)))
    (romacs-mcp--object
     "taskId" (romacs-agent-id session) "status" status
     "state" (symbol-name (romacs-agent-state session)) "title" (romacs-agent-title session)
     "provider" (romacs-agent-provider session) "harness" (symbol-name (romacs-agent-harness session))
     "model" (funcall null (or (romacs-agent-model session) (plist-get options :model)))
     "role" (funcall null (plist-get options :role)) "parent" (funcall null (plist-get options :parent))
     "turns" (romacs-agent-turns session) "result" (funcall null (romacs-agent-reply session))
     "cost" (funcall null (romacs-agent-cost session))
     "cwd" (funcall null (romacs-agent-cwd session)) "conversation" (romacs-agent--native-id session)
     "worktree" (funcall null (plist-get options :worktree)) "branch" (funcall null (plist-get options :branch))
     "pendingApproval" (if (> (hash-table-count (romacs-agent-approvals session)) 0) t :false)
     "limit" (romacs-agent-tools--limit-object session)
     "next" (if (equal status "running") (romacs-agent-tools--running-next session) :null))))

(defun romacs-agent-tools--known (arguments)
  (or (romacs-agent-get (gethash "taskId" arguments)) (user-error "No task %s" (gethash "taskId" arguments))))

(defun romacs-agent-tools--woken-file (session)
  (expand-file-name "woken" (romacs-agent-dir session)))

(defun romacs-agent-tools--woken (session)
  "The journal seq of SESSION's last turn end delivered to its parent."
  (or (ignore-errors (with-temp-buffer (insert-file-contents (romacs-agent-tools--woken-file session))
                                       (string-to-number (buffer-string))))
      0))

(defun romacs-agent-tools--mark-woken (session seq)
  "SESSION's turn ends through SEQ are delivered (never moving back): the
parent's harness takes them in order."
  (when (> seq (romacs-agent-tools--woken session))
    (with-temp-file (romacs-agent-tools--woken-file session) (insert (number-to-string seq)))))

(defconst romacs-agent-tools--wake-limit 16000
  "The most of a child's result a wake carries: the rest is
romacs_task_status's.")

(defun romacs-agent-tools--wake-result (id result)
  "RESULT, of task ID, as a wake carries it: whole, or its start and where
the rest is."
  (if (<= (length result) romacs-agent-tools--wake-limit) result
    (format "%s\n[Result cut at %d of %d characters: romacs_task_status with taskId %s has it whole.]"
            (substring result 0 romacs-agent-tools--wake-limit) romacs-agent-tools--wake-limit (length result) id)))

(defun romacs-agent-tools--limit-object (session)
  "SESSION's last end at its instance's usage limit, with the instances
that have room, as a task record says it; or null."
  (if-let* ((limited (romacs-agent-limited session)))
      (romacs-mcp--object
       "provider" (plist-get limited :provider) "model" (or (plist-get limited :model) :null)
       "text" (or (plist-get limited :text) :null)
       "window" (or (plist-get limited :window) :null) "resets" (or (plist-get limited :resets) :null)
       "alternatives" (vconcat (mapcar (lambda (a) (romacs-mcp--object "provider" (plist-get a :provider)
                                                                       "lossless" (if (plist-get a :lossless) t :false)))
                                       (romacs-agent-alternatives session))))
    :null))

(defun romacs-agent-tools--limit-note (session)
  "What a wake adds to SESSION's end at its usage limit: where it could go on."
  (when-let* ((limited (romacs-agent-limited session)))
    (let ((alternatives (romacs-agent-alternatives session))
          (model (plist-get limited :model)))
      (format "\n[Usage limit %s%s. %s]"
              (if model (format "for %s on %s" model (plist-get limited :provider))
                (format "on %s" (plist-get limited :provider)))
              (if (plist-get limited :resets)
                  (format-time-string " (resets %b %-d %H:%M)" (seconds-to-time (plist-get limited :resets))) "")
              (if model
                  (let ((lossless (cl-remove-if-not (lambda (a) (plist-get a :lossless)) alternatives))
                        (handoffs (cl-remove-if (lambda (a) (plist-get a :lossless)) alternatives)))
                    (format "Its other models run: romacs_task_switch with taskId and model goes on in place, \
the same conversation%s%s."
                            (if lossless
                                (format ", or with provider (%s) on %s"
                                        (mapconcat (lambda (a) (plist-get a :provider)) lossless ", ") model)
                              "")
                            (if handoffs
                                (format "; %s take a handoff on a brief, on a model of their own"
                                        (mapconcat (lambda (a) (plist-get a :provider)) handoffs ", "))
                              "")))
              (if alternatives
                  (concat "With room: "
                          (mapconcat (lambda (a) (format "%s (%s)" (plist-get a :provider)
                                                         (if (plist-get a :lossless) "the same conversation moves" "a handoff on a brief")))
                                     alternatives ", ")
                          ". romacs_task_switch with that provider goes on there.")
                "No instance has room."))))))

(defun romacs-agent-tools--wake-result-of (session end)
  "What the wake of SESSION's turn END says of its result."
  (romacs-agent-tools--wake-result
   (romacs-agent-id session)
   (concat (or (nth 1 end) "") (and (nth 2 end) (romacs-agent-tools--limit-note session)))))

(defun romacs-agent-tools--wake-message (session end)
  "The record (`romacs-agent-make-message') of the wake of SESSION's turn END:
a child's result, answering the message its turn began with."
  (let ((result (romacs-agent-tools--wake-result-of session end)))
    (append (romacs-agent-make-message (concat "m-" (romacs-agent--uuid)) "child"
                                  (list :session (romacs-agent-id session)
                                        :name (or (romacs-agent-title session) (romacs-agent-id session)))
                                  (nth 3 end) result)
            (list :wake (list :state (if (nth 2 end) "failed" "completed") :chars (length result))))))

(defun romacs-agent-tools--wake-text (session end)
  "The parent's message for SESSION's turn END, (SEQ RESULT FAILED).  Its
head says how long its result is: wakes go together, and a result may hold
anything."
  (let ((result (romacs-agent-tools--wake-result-of session end)))
    (format "Delegated task %s (%s) [%s, %d chars%s]:\n%s" (romacs-agent-id session) (romacs-agent-title session)
            (if (nth 2 end) "failed" "completed") (length result)
            ;; The turn answered a message: the reply names it.
            (if (nth 3 end) (format ", re %s" (nth 3 end)) "")
            result)))

(defconst romacs-agent-tools--child-note
  "You are a delegated task: your parent sees only your final reply.  Do the work yourself, \
start no delegated tasks unless your task says to, and end with a self-contained result.  A later \
turn your parent need not act on (no landing, decision or blocker) may end with a reply starting \
[quiet]: it wakes no one, and your parent can still read it."
  "What a delegated session is told after its task: in its first message,
not its system prompt, which stays its parent's.")

(defun romacs-agent-tools--brief (role task)
  "What a child is started on: TASK, as the ROLE sub-agent when one is
named, then `romacs-agent-tools--child-note'; an empty TASK stays empty."
  (if (string-empty-p (string-trim task)) task
   (concat (if (and role (not (string-empty-p role))) (format "Act as the %s sub-agent for this task.\n\n" role) "")
          task "\n\n" romacs-agent-tools--child-note)))

(defconst romacs-agent-tools--wake-head
  "Delegated task \\([0-9a-f-]\\{36\\}\\) (\\(.*\\)) \\[\\(completed\\|failed\\|sent\\)\\(?:, \\([0-9]+\\) chars\\)?\\(?:, re \\(m-[0-9a-f-]+\\)\\)?\\]:\n"
  "A wake's head: its task, title, state (\"sent\": a copy of its message to
another task of its parent), when it says, its result's length, and the
message the turn answered.")

(defun romacs-agent-tools--wakes-parse (text)
  "TEXT read back as the wakes it carries, in order, each a plist of :id
:title :state :reply-to (the message the turn answered) and :result, or
nil; after the last, the lines saying how to answer each.  A head's
length takes its result whatever it holds; one with none (a wake of
before) takes the rest of TEXT.  The harness takes wakes as a plain
message, so its journal keeps only this text.  It is read in a buffer,
each place where it is: nothing is searched for further on, nor reached
by counting characters from the start."
  (with-temp-buffer
    (insert text)
    (goto-char (point-min))
    (let ((wakes nil) (ok t))
      (while (and ok (not (eobp)))
        (cond
         ((looking-at romacs-agent-tools--wake-head)
          (let* ((from (match-end 0))
                 (size (match-string 4))
                 (to (if size (+ from (string-to-number size)) (point-max))))
            (if (> to (point-max)) (setq ok nil)
              (push (list :id (match-string 1) :title (match-string 2) :state (match-string 3)
                          :reply-to (match-string 5) :result (buffer-substring-no-properties from to))
                    wakes)
              (goto-char to)
              (unless (eobp) (if (looking-at "\n\n") (forward-char 2) (setq ok nil))))))
         ;; The lines saying how to answer each, after the last.
         ((and wakes (looking-at "\\(?:Reply to [^\n]*\\(?:\n\\|\\'\\)\\)+\\'"))
          (goto-char (point-max)))
         (t (setq ok nil))))
      (and ok (nreverse wakes)))))

(defun romacs-agent-tools--wake-parse (text)
  "TEXT read back as one wake (`romacs-agent-tools--wakes-parse'), or nil."
  (let ((wakes (romacs-agent-tools--wakes-parse text)))
    (and (= (length wakes) 1) (car wakes))))

(defvar romacs-agent-tools--answered (make-hash-table :test #'equal)
  "A child's id to a table of its turn ends' seqs a `wait' answered with,
read once from its `answered' file.")

(defun romacs-agent-tools--answered-table (session)
  (or (gethash (romacs-agent-id session) romacs-agent-tools--answered)
      (let ((table (make-hash-table)) (file (expand-file-name "answered" (romacs-agent-dir session))))
        (when (file-readable-p file)
          (dolist (line (split-string (with-temp-buffer (insert-file-contents file) (buffer-string)) "\n" t))
            (puthash (string-to-number line) t table)))
        (puthash (romacs-agent-id session) table romacs-agent-tools--answered))))

(defun romacs-agent-tools--answer (session seq)
  "A `wait' answered with SESSION's turn end SEQ: its parent has it, and
earlier ends still on their way stay so."
  (puthash seq t (romacs-agent-tools--answered-table session))
  (write-region (format "%d\n" seq) nil (expand-file-name "answered" (romacs-agent-dir session)) t 'silent))

(defun romacs-agent-tools--wanted-p (session seq)
  "Whether SESSION's turn end SEQ is still to be given to its parent."
  (and (not (romacs-agent-cancelled session))
       (> seq (romacs-agent-tools--woken session))
       (not (gethash seq (romacs-agent-tools--answered-table session)))))

(defconst romacs-agent-tools--wake-rules '("end" "failure" "all" "none")
  "When a delegated task's end wakes its parent: each end; only a failed
one; once every task of the parent's delegated with \"all\" has ended; never
\(its result is romacs_task_status's).")

(defun romacs-agent-tools--wake-rule (child)
  (or (plist-get (romacs-agent-options child) :wake) "end"))

(defvar romacs-agent-tools--pending (make-hash-table :test #'equal)
  "A parent's id to the ends of its children waiting to wake it, oldest
first, each (CHILD . END): the ends that come while it works, or close
together, wake it once.")

(defconst romacs-agent-tools--quiet "[quiet]"
  "What a child's reply starts with when its parent need not act on it:
the end wakes no one, and romacs_task_status still has it.")

(defun romacs-agent-tools--wakes-p (child end)
  "Whether CHILD's turn END, (SEQ RESULT FAILED), wakes its parent: a
failure, unless its rule is \"none\"; another end by its rule, unless
its result is empty (a resumed harness's own turn) or quiet
\(`romacs-agent-tools--quiet')."
  (let ((result (string-trim (or (nth 1 end) ""))))
    (pcase (romacs-agent-tools--wake-rule child)
      ("none" nil)
      ("failure" (nth 2 end))
      (_ (or (nth 2 end)
             (not (or (string-empty-p result) (string-prefix-p romacs-agent-tools--quiet result))))))))

(defvar romacs-agent-tools--moving (make-hash-table :test #'equal)
  "A task moved on its own at its usage limit, its id to (SEQ . FROM):
that end's wake waits for the move's outcome.")

(defun romacs-agent-tools--wake (child)
  "Hand CHILD's new turn ends to its parent's next wake, as CHILD's wake
rule says; an end no rule lets wake it is done with.  Each end is handed
on once; none while CHILD moves on its own at its usage limit."
  (unless (gethash (romacs-agent-id child) romacs-agent-tools--moving)
    (romacs-agent-tools--hand-on child)))

(defvar romacs-agent-tools--handed (make-hash-table :test #'eq :weakness 'key)
  "A child to the seq of its last turn end handed to its parent's wake by
this process.  Its ends stay in it until delivered
\(`romacs-agent-tools--wanted-p'): a restart before then, its state kept,
hands them on again.")

(defun romacs-agent-tools--unhanded (child)
  "CHILD's turn ends not yet handed on by this process, oldest first."
  (let ((handed (gethash child romacs-agent-tools--handed 0)))
    (cl-remove-if-not (lambda (end) (> (car end) handed)) (reverse (romacs-agent-ends child)))))

(defun romacs-agent-tools--hand-on (child)
  (let ((parent (romacs-agent-get (plist-get (romacs-agent-options child) :parent)))
        (ends (romacs-agent-tools--unhanded child)))
    ;; Those delivered or done with are let go.
    (setf (romacs-agent-ends child)
          (cl-remove-if-not (lambda (end) (romacs-agent-tools--wanted-p child (car end))) (romacs-agent-ends child)))
    (when parent
      (dolist (end ends)
        (puthash child (car end) romacs-agent-tools--handed)
        (when (romacs-agent-tools--wanted-p child (car end))
          (if (romacs-agent-tools--wakes-p child end)
              ;; Newest first: an end is added in constant time.
              (push (cons child end) (gethash (romacs-agent-id parent) romacs-agent-tools--pending))
            ;; Done with on its own: an earlier end still waiting stays wanted.
            (romacs-agent-tools--answer child (car end)))))
      (romacs-agent-tools--wake-soon parent))))

(defvar romacs-agent-tools--waking (make-hash-table :test #'equal)
  "Parents whose wake is scheduled.")

(defun romacs-agent-tools--wake-soon (parent)
  "Wake PARENT a moment from now with what waits for it: ends that come
meanwhile go with it."
  (unless (gethash (romacs-agent-id parent) romacs-agent-tools--waking)
    (puthash (romacs-agent-id parent) t romacs-agent-tools--waking)
    (run-at-time 0.5 nil (lambda ()
                           (remhash (romacs-agent-id parent) romacs-agent-tools--waking)
                           (romacs-agent-tools--flush parent)))))

(defun romacs-agent-tools--all-pending-p (parent)
  "Whether a task PARENT delegated with the \"all\" rule still runs, or has
ended without its end handed on yet."
  (cl-some (lambda (child) (and (equal (romacs-agent-tools--wake-rule child) "all")
                                (or (member (romacs-agent-tools--status child) '("running" "waiting"))
                                    (romacs-agent-tools--unhanded child)
                                    (gethash (romacs-agent-id child) romacs-agent-tools--scheduled))))
           (romacs-agent-children parent)))

(defun romacs-agent-tools--flush (parent)
  "Wake PARENT once with the ends waiting for it, unless it is at work: its
turn's end wakes it then.  Ends of \"all\" tasks wait for the last of them."
  (unless (memq (romacs-agent-state parent) '(starting running waiting-approval))
    (let* ((hold (romacs-agent-tools--all-pending-p parent))
           (copies (gethash (romacs-agent-id parent) romacs-agent-tools--copies))
           (held nil) (ready nil))
      ;; One pass, oldest first: the ends still wanted, held or ready.
      (dolist (item (reverse (gethash (romacs-agent-id parent) romacs-agent-tools--pending)))
        (when (romacs-agent-tools--wanted-p (car item) (cadr item))
          (if (and hold (equal (romacs-agent-tools--wake-rule (car item)) "all"))
              (push item held)
            (push item ready))))
      (setq ready (nreverse ready))
      (puthash (romacs-agent-id parent) held romacs-agent-tools--pending)
      (when ready
        (romacs-agent-tools--deliver
         parent
         (mapconcat #'identity
                    (append (mapcar (lambda (item) (romacs-agent-tools--wake-text (car item) (cdr item))) ready)
                            ;; Its tasks' messages to each other since the last: copies, never a wake.
                            (mapcar (lambda (copy) (romacs-agent-tools--copy-text (car copy) (cdr copy))) copies))
                    "\n\n")
         ;; Delivered only once the parent's harness has it; the copies kept till then.
         (lambda ()
           (dolist (item ready) (romacs-agent-tools--mark-woken (car item) (cadr item)))
           (puthash (romacs-agent-id parent)
                    (cl-remove-if (lambda (copy) (memq copy copies))
                                  (gethash (romacs-agent-id parent) romacs-agent-tools--copies))
                    romacs-agent-tools--copies))
         (lambda () (cl-some (lambda (item) (romacs-agent-tools--wanted-p (car item) (cadr item))) ready))
         ;; Each is a message from another session: the chat reads these.
         (append (mapcar (lambda (item) (romacs-agent-tools--wake-message (car item) (cdr item))) ready)
                 (mapcar (lambda (copy) (romacs-agent-tools--copy-message (car copy) (cdr copy))) copies)))))))

(defun romacs-agent-tools--deliver (session text delivered &optional valid messages)
  "Give SESSION's harness TEXT as its next message, then call DELIVERED.
A per-turn harness takes it as its next turn (queued behind a running
one); another once its supervisor is attached.  VALID, when given, is
asked first: nil drops the message.  MESSAGES say whose it is
\(`romacs-agent--input')."
  (let ((deliver (lambda () (when (or (null valid) (funcall valid))
                              (romacs-agent-send session text delivered valid nil messages)))))
    (if (romacs-agent--per-turn-p session) (funcall deliver)
      ;; A parent whose harness was stopped or ended is resumed to take it.
      (unless (or (process-live-p (romacs-agent-connection session)) (eq (romacs-agent-state session) 'starting))
        (romacs-agent--continue session))
      (romacs-agent--when-attached session deliver))))

(defvar romacs-agent-tools--scheduled (make-hash-table :test #'equal)
  "Children whose wake is scheduled.")

(defun romacs-agent-tools--forget-ends (child)
  "Forget what was delivered or answered of CHILD's ends, and its ends
waiting for its parent: a session with no journal has had no end but its
launches' failures, each numbered as the first, so the next one wakes
its parent again."
  (delete-file (romacs-agent-tools--woken-file child))
  (delete-file (expand-file-name "answered" (romacs-agent-dir child)))
  (remhash (romacs-agent-id child) romacs-agent-tools--answered)
  (remhash child romacs-agent-tools--handed)
  (when-let* ((parent (plist-get (romacs-agent-options child) :parent)))
    (puthash parent (cl-remove child (gethash parent romacs-agent-tools--pending) :key #'car)
             romacs-agent-tools--pending)))

(defun romacs-agent-tools--on-event (session event)
  ;; A launch failed: its end is new, whatever an earlier failure's was.
  (when (eq (plist-get event :type) 'unstarted)
    (romacs-agent-tools--forget-ends session))
  ;; A parent's turn ended: what waited while it worked wakes it now.
  (when (and (eq (plist-get event :type) 'done) (gethash (romacs-agent-id session) romacs-agent-tools--pending))
    (romacs-agent-tools--wake-soon session))
  (when (and (plist-get (romacs-agent-options session) :parent)
             (or (memq (plist-get event :type) '(done unstarted))
                 (and (eq (plist-get event :type) 'exit) (romacs-agent-failed session)
                      (= (romacs-agent-done-seq session) (or (plist-get event :seq) -1)))))
    ;; A moment later: a `wait' answering with this end (its poll runs
    ;; every 0.05 s) marks it answered first; an abandoned one does not.
    ;; One timer per child: the ends that come meanwhile go with it.
    (unless (gethash (romacs-agent-id session) romacs-agent-tools--scheduled)
      (puthash (romacs-agent-id session) t romacs-agent-tools--scheduled)
      (run-at-time 0.5 nil (lambda ()
                             (remhash (romacs-agent-id session) romacs-agent-tools--scheduled)
                             (romacs-agent-tools--wake session))))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--on-event)

(defun romacs-agent-tools--wake-missed ()
  "After a restart: wake the parents of children that ended meanwhile."
  (dolist (session (romacs-agent-sessions))
    (when (plist-get (romacs-agent-options session) :parent) (romacs-agent-tools--wake session))))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--wake-missed)

;;; Reaping finished tasks

(defcustom romacs-agent-reap-after '(("review" . 300))
  "How long a delegated task may sit idle after a turn before its harness
is stopped, by role: an alist of a role (a string, or t for any other)
and seconds.  Its conversation stays: a message to it (romacs_task_send)
resumes it.  Stopping a task ends what its harness started too, so a
lane that waits on its own child processes is better left out."
  :type '(alist :key-type (choice string (const :tag "Any other role" t)) :value-type number)
  :group 'romacs-agent)

(defun romacs-agent-tools--reap-delay (session)
  "Seconds SESSION may sit idle, or nil to keep it: a session with a role
was delegated (a user's own has none), whoever delegated it."
  (when-let* ((role (plist-get (romacs-agent-options session) :role)))
    (cdr (or (assoc role romacs-agent-reap-after) (assq t romacs-agent-reap-after)))))

(defvar romacs-agent-tools--reaping (make-hash-table :test #'equal)
  "Tasks whose look is scheduled: a replay's many ends schedule one.")

(defun romacs-agent-tools--reap-later (session &optional delay)
  "Look at SESSION once it may have sat idle long enough to be reaped:
after DELAY seconds, by default its role's time."
  (when-let* ((delay (or delay (romacs-agent-tools--reap-delay session)))
              ((not (gethash (romacs-agent-id session) romacs-agent-tools--reaping))))
    (puthash (romacs-agent-id session) t romacs-agent-tools--reaping)
    (run-at-time delay nil #'romacs-agent-tools--reap session)))

(defun romacs-agent-tools--idle-p (session)
  "Whether SESSION's harness runs and has nothing to do: no turn, nothing
waiting to be sent or written and not yet journaled back, no question
unanswered."
  (and (eq (romacs-agent-state session) 'idle)
       (null (romacs-agent-queue session))
       (<= (romacs-agent-requests session) (romacs-agent-inputs session))
       (zerop (hash-table-count (romacs-agent-approvals session)))
       (or (process-live-p (romacs-agent-connection session)) (romacs-agent-unit session))))

(defun romacs-agent-tools--reap (session)
  "Stop SESSION's harness when it has sat idle its role's time since its
last record; when it has not yet, look again then."
  (remhash (romacs-agent-id session) romacs-agent-tools--reaping)
  (when-let* (((eq (romacs-agent-get (romacs-agent-id session)) session))
              ((not (romacs-agent-cancelled session)))
              (delay (romacs-agent-tools--reap-delay session))
              ;; Its replay not yet over, what it does is not yet known.
              ((or (romacs-agent--per-turn-p session) (romacs-agent-caught-up session)
                   (not (process-live-p (romacs-agent-connection session)))
                   (progn (romacs-agent-tools--reap-later session delay) nil)))
              ((romacs-agent-tools--idle-p session)))
    (let ((left (- (+ (or (romacs-agent-last-time session) 0) delay) (float-time))))
      (if (> left 0)
          (romacs-agent-tools--reap-later session left)
        (romacs-agent-stop session)))))

(defun romacs-agent-tools--reap-on-event (session event)
  ;; A turn's end: its task may be done.
  (when (eq (plist-get event :type) 'done) (romacs-agent-tools--reap-later session)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--reap-on-event)

(defun romacs-agent-tools--reap-restored ()
  "After a restart: look at every task that may be idle already."
  (mapc #'romacs-agent-tools--reap-later (romacs-agent-sessions)))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--reap-restored)

(defun romacs-agent-tools--wait (child timeout-ms arrived)
  "A deferred answer: CHILD's task record once its turn ends, asks for an
approval, or TIMEOUT-MS passes (the child goes on; its end then wakes
the parent).  The wait ends within the session relay's reply timeout,
counted from the request's ARRIVED time, which would otherwise withdraw
the call (and retire this wait) and drop its answer."
  (let ((deadline (min (+ (float-time) (/ (or timeout-ms 600000) 1000.0))
                       (+ arrived (max 1 (- romacs-agent-reply-timeout 5))))))
    (romacs-mcp-defer
     (lambda ()
       (let ((status (romacs-agent-tools--status child)))
         (cond ((member status '("completed" "failed" "cancelled" "waiting"))
                (when (member status '("completed" "failed"))
                  (romacs-agent-tools--answer child (romacs-agent-done-seq child)))
                (list (romacs-agent-tools--task child)))
               ((> (float-time) deadline)
                (let ((task (romacs-agent-tools--task child))) (puthash "waitTimedOut" t task) (list task)))))))))

(defun romacs-agent-tools--delegate (arguments)
  "Start a child session on ARGUMENTS' task, or find the one this caller
started with the same clientRequestId.  The child runs on the instance
its target names, else on its delegating session's own (claude to
claude, codex to codex); a caller that is no agent session gets the
default instance.  With isolation \"worktree\" it runs in a new
worktree of its directory's repository (`romacs-agent-worktree-create');
with resume it goes on with that conversation of the instance's
\(`romacs-agent-tools--adopt')."
  (romacs-mcp--instance arguments)
  (let* ((parent (romacs-agent-tools--caller))
         (request (gethash "clientRequestId" arguments))
         (target (gethash "target" arguments (romacs-mcp--object)))
         (options (gethash "options" target (romacs-mcp--object)))
         (provider (or (gethash "provider" target)
                       (and parent (romacs-agent-get parent) (romacs-agent-provider (romacs-agent-get parent)))))
         ;; Refused before anything starts, or any worktree is made.
         (wake (let ((wake (gethash "wake" arguments)))
                 (if (member wake romacs-agent-tools--wake-rules) wake
                   (and wake (user-error "wake must be one of %s"
                                         (string-join romacs-agent-tools--wake-rules ", "))))))
         (cwd (or (gethash "cwd" arguments)
                  (and parent (romacs-agent-get parent) (romacs-agent-cwd (romacs-agent-get parent)))
                  default-directory))
         ;; This request's arrival: a deferred answer runs with none active.
         (arrived (or (plist-get romacs-mcp--active :arrived) (float-time)))
         (start-options (list :model (gethash "model" target)
                              :effort (or (gethash "reasoningEffort" options) (gethash "effort" options))
                              :role (gethash "role" arguments) :parent parent :request request :wake wake))
         (start (lambda (cwd &rest worktree)
                  (romacs-agent-start
                   cwd (romacs-agent-tools--brief (gethash "role" arguments) (gethash "task" arguments))
                   (gethash "title" arguments) provider (append start-options worktree))))
         (answer (lambda (child)
                   (romacs-agent-tools--started
                    child (lambda ()
                            (if (equal (gethash "mode" arguments) "wait")
                                (romacs-agent-tools--wait child (gethash "timeoutMs" arguments) arrived)
                              (romacs-agent-tools--task child))))))
         (known (and request (romacs-agent-requested parent request))))
    (cond (known (funcall answer known))
          ((gethash "resume" arguments)
           (funcall answer (romacs-agent-tools--adopt arguments provider start-options)))
          ((equal (gethash "isolation" arguments) "worktree")
           ;; A child that cannot start leaves no worktree behind.
           (romacs-agent--start-instance provider)
           (romacs-agent-tools--isolated (romacs-agent-tools--isolating (cons parent request) cwd start) answer))
          (t (funcall answer (funcall start cwd))))))

(defun romacs-agent-tools--own (session caller what)
  "SESSION, when CALLER delegated it (a caller that is no agent session
owns the sessions none delegated); else a `user-error' saying only its
parent may do WHAT."
  (let ((parent (plist-get (romacs-agent-options session) :parent)))
    (unless (equal parent caller)
      (user-error "%s (%s) is %s: only the session that delegated it may %s"
                  (romacs-agent-id session) (romacs-agent-title session)
                  (if parent (format "task of session %s" parent) "no delegated task") what))
    session))

(defun romacs-agent-tools--tell (child text &optional mode valid messages)
  "Send TEXT to CHILD as its next message by the session's own send:
now when it is idle, once its running turn ends, once its harness is
back when it had ended.  MODE `steer' writes it into the running turn,
as the chat's steer does (a harness run per turn takes it after);
`interrupt' interrupts the turn and sends it first.  VALID, when given,
is asked before a waiting message goes: nil drops it.  MESSAGES, when
given, say it comes from other sessions (`romacs-agent--input').  Its
next turn end wakes its parent by its wake rule, as its first did.  An
empty TEXT sends nothing."
  (when (romacs-agent-cancelled child)
    (user-error "Task %s (%s) was cancelled" (romacs-agent-id child) (romacs-agent-title child)))
  (unless (string-empty-p (string-trim text))
    (let ((busy (memq (romacs-agent-state child) '(running waiting-approval))))
      (cond ((and busy (eq mode 'steer) (not (romacs-agent--per-turn-p child)))
             (romacs-agent-send child text nil valid nil messages))
            ((and busy (eq mode 'interrupt))
             ;; First in line before the cut: a turn not yet under way
             ;; (Codex's before it names it) cannot be cut, and it goes after.
             (romacs-agent-send-later child text t nil valid messages)
             (ignore-error user-error (romacs-agent-interrupt child)))
            (t
             ;; One starting is going on already: attaching it again would take its
             ;; supervisor from the attachment under way.
             (unless (eq (romacs-agent-state child) 'starting) (romacs-agent--continue child))
             (romacs-agent-send-later child text nil nil valid messages)))))
  child)

(defconst romacs-agent-tools--send-modes '("queue" "steer" "replace" "interrupt")
  "How romacs_task_send delivers: after the running turn, into it, in place
of what the sender has waiting, or cutting the turn short.")

(defvar romacs-agent-tools--generations (make-hash-table :test #'equal)
  "(TASK-ID . SENDER-ID) to the generation of the sender's messages to the
task: a `replace' starts a new one, and the waiting messages of an older
one are dropped as they come up.")

(defun romacs-agent-tools--sibling-p (session caller)
  "Whether CALLER is another task of SESSION's parent."
  (let ((parent (plist-get (romacs-agent-options session) :parent)))
    (and parent caller (not (equal caller (romacs-agent-id session)))
         (when-let* ((sender (romacs-agent-get caller)))
           (equal (plist-get (romacs-agent-options sender) :parent) parent)))))

(defvar romacs-agent-tools--copies (make-hash-table :test #'equal)
  "A parent's id to the copies of its tasks' messages to each other, oldest
first, each (SENDER TARGET MODE TEXT): they go with its next wake, never
waking it.")

(defun romacs-agent-tools--copy-body (copy)
  "What a COPY, (TARGET MODE TEXT), says: its task, how, and the message."
  (pcase-let ((`(,target ,mode ,text) copy))
    (format "To task %s (%s), %s:\n%s" (romacs-agent-id target) (romacs-agent-title target) mode text)))

(defun romacs-agent-tools--copy-text (sender copy)
  "SENDER's message COPY, (TARGET MODE TEXT), as a wake its parent reads
\(state \"sent\")."
  (let ((body (romacs-agent-tools--copy-body copy)))
    (format "Delegated task %s (%s) [sent, %d chars]:\n%s"
            (romacs-agent-id sender) (romacs-agent-title sender) (length body) body)))

(defun romacs-agent-tools--copy-message (sender copy)
  "The record of SENDER's message COPY, (TARGET MODE TEXT), to its parent:
a sibling's message, sent to TARGET."
  (pcase-let ((`(,target ,_mode ,text) copy))
    (append (romacs-agent-make-message (concat "m-" (romacs-agent--uuid)) "sibling"
                                  (list :session (romacs-agent-id sender)
                                        :name (or (romacs-agent-title sender) (romacs-agent-id sender)))
                                  nil text)
            (list :wake (list :state "sent" :chars (length text) :to (romacs-agent-id target))))))

(defun romacs-agent-tools--message (kind sender &optional in-reply-to by-user)
  "A new message (`romacs-agent-make-message'): from SENDER, a session (nil: one
outside Romacs), what it is to the receiver KIND, answering message
IN-REPLY-TO; BY-USER, its user wrote it in SENDER's chat."
  (romacs-agent-make-message (concat "m-" (romacs-agent--uuid)) kind
                        (and sender (list :session (romacs-agent-id sender)
                                          :name (or (romacs-agent-title sender) (romacs-agent-id sender))
                                          :by-user by-user))
                        in-reply-to))

(defun romacs-agent-tools--kind (sender target)
  "What SENDER, a session or nil (outside Romacs), is to TARGET: parent,
child, sibling, or external."
  (let ((parent-of (lambda (session) (plist-get (romacs-agent-options session) :parent))))
    (cond ((null sender) "external")
          ((equal (funcall parent-of target) (romacs-agent-id sender)) "parent")
          ((equal (funcall parent-of sender) (romacs-agent-id target)) "child")
          ((and (funcall parent-of target) (equal (funcall parent-of target) (funcall parent-of sender))) "sibling")
          (t "external"))))

(defun romacs-agent-tools--message-to (sender target text mode &optional in-reply-to by-user)
  "Send TEXT from SENDER (a session, or nil) to TARGET as a message by MODE
\(`romacs-agent-tools--send-modes'), answering IN-REPLY-TO when given;
BY-USER, SENDER's user wrote it.  A sibling's parent gets a copy with
its next wake.  Answer TARGET's task record and the message's id."
  (when (string-empty-p (string-trim (or text ""))) (user-error "message is empty"))
  (unless (member mode romacs-agent-tools--send-modes)
    (user-error "mode must be one of %s" (string-join romacs-agent-tools--send-modes ", ")))
  (let* ((caller (and sender (romacs-agent-id sender)))
         (kind (romacs-agent-tools--kind sender target))
         (message (romacs-agent-tools--message kind sender in-reply-to by-user))
         (key (cons (romacs-agent-id target) caller))
         (generation (if (equal mode "replace")
                         (puthash key (1+ (gethash key romacs-agent-tools--generations 0))
                                  romacs-agent-tools--generations)
                       (gethash key romacs-agent-tools--generations 0)))
         (valid (lambda () (eql generation (gethash key romacs-agent-tools--generations 0)))))
    (romacs-agent-tools--tell target text (intern mode) valid (list message))
    (when (equal kind "sibling")
      (let ((parent (plist-get (romacs-agent-options target) :parent)))
        (puthash parent (append (gethash parent romacs-agent-tools--copies) (list (list sender target mode text)))
                 romacs-agent-tools--copies)))
    (let ((record (romacs-agent-tools--task target)))
      (puthash "messageId" (plist-get message :id) record)
      record)))

(defun romacs-agent-tools--send (arguments)
  "Send ARGUMENTS' message to task taskId: the caller's own task, or
another task of the caller's parent, its parent then copied."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (task (romacs-agent-tools--known arguments))
         (sibling (romacs-agent-tools--sibling-p task caller)))
    (unless sibling (romacs-agent-tools--own task caller "send it messages (or another of its parent's tasks)"))
    (romacs-agent-tools--message-to (and caller (romacs-agent-get caller)) task (gethash "message" arguments)
                                    (or (gethash "mode" arguments) "queue") (gethash "inReplyTo" arguments))))

(defun romacs-agent-answer (session id text &optional mode by-user)
  "Answer the message ID SESSION received with TEXT, routed to its sender
whatever it is to SESSION, by MODE (`romacs-agent-tools--send-modes',
default queue), linked to ID.  BY-USER: SESSION's user wrote it (its
chat), said so to the sender, and SESSION's journal keeps that it was
answered.  Return the sender's task record with the answer's messageId;
a `user-error' for a message SESSION did not receive, one from outside
Romacs (its own channel's reply line says how), or a sender gone."
  (let* ((message (or (romacs-agent-received-message session id)
                      (user-error "No message %s among the ones this session received" id)))
         (sender-id (plist-get (plist-get message :from) :session))
         (sender (cond ((null sender-id)
                        ;; Its own channel's, when it came by one: the call to copy.
                        (user-error "Message %s came from outside Romacs. %s" id
                                    (romacs-agent--message-reply-line message)))
                       ((romacs-agent-get sender-id))
                       (t (user-error "Message %s's sender, session %s, is gone" id sender-id))))
         (record (romacs-agent-tools--message-to session sender text (or mode "queue") id by-user)))
    ;; An agent's answer is its tool call, in its journal; its user's is
    ;; this note.
    (when by-user
      (romacs-agent--note session (romacs-mcp--object "type" "replied" "message" id "reply" (gethash "messageId" record)
                                                      "byUser" t)))
    record))

(defun romacs-agent-tools--reply (arguments)
  "Answer the message messageId the caller received (`romacs-agent-answer')."
  (romacs-mcp--instance arguments)
  (romacs-agent-answer (or (when-let* ((id (romacs-agent-tools--caller))) (romacs-agent-get id))
                           (user-error "romacs_task_reply answers a message an agent session received: you are none"))
                       (or (gethash "messageId" arguments) (user-error "messageId is required"))
                       (gethash "text" arguments) (gethash "mode" arguments)))

(defun romacs-agent-tools--same-directory-p (a b)
  (equal (file-name-as-directory (file-truename a)) (file-name-as-directory (file-truename b))))

(defun romacs-agent-tools--adopt (arguments provider options)
  "The caller's task going on with the conversation ARGUMENTS' resume
names, of instance PROVIDER, in its own directory cwd, started with
OPTIONS: imported (`romacs-agent-import') with task as its next message,
as is, no brief around it.  A conversation a session already goes on
with is that task, sent nothing, when the caller delegated it, and
refused otherwise.  Its transcript is found as the history finds one."
  (require 'romacs-agent-history)
  (let* ((thread (gethash "resume" arguments))
         (cwd (or (gethash "cwd" arguments)
                  (user-error "resume needs cwd: the directory conversation %s was held in" thread)))
         (instance (romacs-agent--start-instance provider))
         (harness (plist-get instance :harness))
         (config-dir (romacs-agent--config-dir instance))
         (known (romacs-agent--conversation harness config-dir thread)))
    (when (equal (gethash "isolation" arguments) "worktree")
      (user-error "resume goes on in the conversation's own directory: isolation worktree does not apply"))
    (if known
        (romacs-agent-tools--own known (plist-get options :parent) "take it over")
      (let ((item (or (romacs-agent-history-find harness config-dir thread)
                      (user-error "No %s conversation %s in %s: name the instance that ran it as target.provider"
                                  (plist-get (romacs-agent--driver-of harness) :display) thread
                                  (romacs-agent--home harness config-dir)))))
        (unless (plist-get item :cwd)
          (user-error "Conversation %s names no directory it was held in: it cannot be resumed" thread))
        (unless (romacs-agent-tools--same-directory-p (plist-get item :cwd) cwd)
          (user-error "Conversation %s was held in %s: give that directory as cwd" thread (plist-get item :cwd)))
        (romacs-agent-tools--tell
         (romacs-agent-import harness thread (plist-get item :path) (expand-file-name cwd)
                              (or (gethash "title" arguments) (plist-get item :title) (romacs-agent--short thread))
                              (plist-get instance :name) config-dir
                              (romacs-agent--instance-options instance options))
         (gethash "task" arguments))))))

(defconst romacs-agent-tools--row-result 300
  "The most of a task's result a row of romacs_task_list carries.")

(defun romacs-agent-tools--row (session)
  "SESSION's task record as romacs_task_list shows it: its result's
head and length, and no advice."
  (let* ((task (romacs-agent-tools--task session)) (result (gethash "result" task)))
    (remhash "next" task)
    (when (stringp result)
      (puthash "resultChars" (length result) task)
      (when (> (length result) romacs-agent-tools--row-result)
        (puthash "result" (concat (substring result 0 romacs-agent-tools--row-result) "…") task)))
    task))

(defun romacs-agent-tools--list (arguments)
  "The caller's tasks, or with all every session's, most recently active
first, as rows (`romacs-agent-tools--row').  The records are the
sessions' own on disk: a restarted server lists them all."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (all (eq (gethash "all" arguments) t))
         (tasks (cl-remove-if-not
                 (lambda (session) (let ((parent (plist-get (romacs-agent-options session) :parent)))
                                     (or (equal parent caller) (and all parent))))
                 (romacs-agent-sessions))))
    (romacs-mcp--object
     "tasks" (vconcat (mapcar #'romacs-agent-tools--row
                              (sort tasks (lambda (a b) (> (or (romacs-agent-last-time a) 0)
                                                           (or (romacs-agent-last-time b) 0)))))))))

(defun romacs-agent-tools--started (child then)
  "A deferred answer: THEN's, once CHILD's supervisor is attached (or a
per-turn harness, which has none between turns, is created); the call's
failure when it never started, an end then answered, so it wakes no one
besides."
  (let ((next nil))
    (romacs-mcp-defer
     (lambda ()
       (cond (next (funcall next))
             ((eq (romacs-agent-state child) 'unstarted)
              (romacs-agent-tools--answer child (romacs-agent-done-seq child))
              (user-error "Task %s (%s) did not start: %s" (romacs-agent-id child) (romacs-agent-title child)
                          (romacs-agent-reply child)))
             ((or (process-live-p (romacs-agent-connection child)) (romacs-agent--per-turn-p child)
                  (not (eq (romacs-agent-state child) 'starting)))
              (let ((value (funcall then)))
                (if (eq (car-safe value) 'romacs-mcp--deferred)
                    (funcall (setq next (cadr value)))
                  (list value)))))))))

;;; Going on under another instance

(defcustom romacs-agent-switch-on-limit nil
  "What a delegated task whose turn ends at its instance's usage limit
does: nil, its parent is woken with where it could go on; `lossless', it
moves to an instance of its harness with room, its conversation and all,
and goes on there, its parent not woken for it."
  :type '(choice (const :tag "Wake the parent" nil) (const :tag "Move to an instance with room" lossless))
  :group 'romacs-agent)

(defconst romacs-agent-tools--go-on
  "Your last turn stopped at %s's usage limit; you now run on %s, the same conversation. Go on from where you were."
  "What a task moved on its own is told.")

(defun romacs-agent-tools--switch-on-limit (session event)
  "A delegated SESSION's turn ended at its usage limit (EVENT): moved on
`romacs-agent-switch-on-limit's say; that end wakes its parent only if
the move fails (`romacs-agent-tools--moved')."
  (when-let* (((plist-get event :limit))
              ;; A live end: one replayed was handled when it came.
              ((romacs-agent-caught-up session))
              ((eq romacs-agent-switch-on-limit 'lossless))
              ((plist-get (romacs-agent-options session) :parent))
              (target (cl-find-if (lambda (a) (plist-get a :lossless)) (romacs-agent-alternatives session))))
    (let ((wanted (romacs-agent--wanted session)))
      (puthash (romacs-agent-id session) (cons (romacs-agent-done-seq session) (romacs-agent-provider session))
               romacs-agent-tools--moving)
      (condition-case nil
          (romacs-agent-select session (plist-get wanted :model) (plist-get wanted :effort) (plist-get target :provider))
        (error (remhash (romacs-agent-id session) romacs-agent-tools--moving))))))

(defun romacs-agent-tools--moved (session event)
  "SESSION's move at its usage limit took effect (EVENT, its selection's
note) or did not: moved, the end that caused it is handed on to no one
and it is told to go on; not, its parent is woken with that end."
  (when-let* ((moving (gethash (romacs-agent-id session) romacs-agent-tools--moving)))
    (remhash (romacs-agent-id session) romacs-agent-tools--moving)
    (let ((to (plist-get (plist-get event :to) :provider)))
      (if (equal to (cdr moving))
          (romacs-agent-tools--wake session)
        (romacs-agent-tools--answer session (car moving))
        ;; Its harness is still to come back: the message waits for it.
        (romacs-agent--enqueue session (list (format romacs-agent-tools--go-on (cdr moving) to) nil nil nil))))))

(defun romacs-agent-tools--limit-on-event (session event)
  (pcase (plist-get event :type)
    ('done (romacs-agent-tools--switch-on-limit session event))
    ('selection (romacs-agent-tools--moved session event))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--limit-on-event)

(defun romacs-agent-tools--switch (arguments)
  "Go on with the task taskId (the caller's own session by default) under
instance provider (default its own: a model or effort changes in place):
of its harness, the same session and conversation, once its running turn
ends; of another, a handoff (a new task on a brief of the thread, with
its parent and wake rule)."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (id (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no session")))
         (task (or (romacs-agent-get id) (user-error "No task %s" id)))
         (provider (or (gethash "provider" arguments) (romacs-agent-provider task)))
         (instance (or (romacs-agent--configured provider) (user-error "No agent provider %s" provider)))
         (model (gethash "model" arguments)) (effort (gethash "effort" arguments))
         (next (gethash "next" arguments)))
    (unless (equal id caller) (romacs-agent-tools--own task caller "move it"))
    (if (eq (plist-get instance :harness) (romacs-agent-harness task))
        (let ((wanted (romacs-agent--wanted task))
              (verb (if (equal provider (romacs-agent-provider task)) "change" "move")))
          (romacs-agent-select task (or model (plist-get wanted :model)) (or effort (plist-get wanted :effort)) provider)
          (when (and next (not (string-empty-p (string-trim next))))
            (if (equal id caller)
                ;; Its own next step: its own words.
                (romacs-agent-tools--tell task next)
              (romacs-agent-tools--message-to (romacs-agent-get caller) task next "queue")))
          (let ((record (romacs-agent-tools--task task)))
            (puthash "next" (if (equal id caller)
                                (format "You %s once this turn ends, the same conversation: end your turn now." verb)
                              (format "It %ss once its running turn ends, the same conversation; its next end wakes you as before."
                                      verb))
                     record)
            record))
      (romacs-agent-tools--handed-off task provider model effort next))))

(defun romacs-agent-tools--handed-off (task provider model effort next)
  "A deferred answer: the new task going on with TASK's thread under
PROVIDER (another harness) on a brief, keeping TASK's parent, wake rule,
role and worktree."
  (require 'romacs-agent-handoff)
  (let ((cell (list nil)))
    (romacs-agent-events-async
     task (lambda (events)
            (setcar cell (condition-case err
                             (list :child (romacs-agent-handoff
                                           task (romacs-agent-handoff-thread task events) provider model
                                           (or next "Go on from where this thread left off.")
                                           (and effort (list :effort effort))))
                           (error (list :error (error-message-string err)))))))
    (romacs-mcp-defer
     (lambda ()
       (let ((made (car cell)))
         (cond ((null made) nil)
               ((plist-get made :error) (user-error "%s" (plist-get made :error)))
               (t (let ((record (romacs-agent-tools--task (plist-get made :child))))
                    (puthash "handedOffFrom" (romacs-agent-id task) record)
                    (list record)))))))))

(defvar romacs-agent-tools--making (make-hash-table :test #'equal)
  "A caller's (PARENT . CLIENT-REQUEST-ID) to its isolated task being made:
the same request repeated meanwhile waits for that one.")

(defun romacs-agent-tools--isolating (key cwd start)
  "The making of KEY's isolated task, a cell whose car becomes (:child
SESSION) or (:error MESSAGE): CWD's worktree made in the background, then
the child START starts in it, whether or not anyone still waits."
  (or (and (cdr key) (gethash key romacs-agent-tools--making))
      (let ((cell (list nil)))
        (when (cdr key) (puthash key cell romacs-agent-tools--making))
        (romacs-agent-worktree-create
         cwd (lambda (made)
               (remhash key romacs-agent-tools--making)
               (setcar cell
                       (if (plist-get made :error) made
                         (condition-case err
                             (list :child (funcall start (plist-get made :cwd) :worktree (plist-get made :path)
                                                   :branch (plist-get made :branch)))
                           (error (list :error (format "Worktree %s (branch %s) was made, but the child did not start: %s"
                                                       (plist-get made :path) (plist-get made :branch)
                                                       (error-message-string err)))))))))
        cell)))

(defun romacs-agent-tools--isolated (cell answer)
  "A deferred answer: ANSWER's value for the child CELL comes to hold, or
CELL's error as the call's."
  (let ((next nil))
    (romacs-mcp-defer
     (lambda ()
       (let ((made (car cell)))
         (cond (next (funcall next))
               ((null made) nil)
               ((plist-get made :error) (user-error "%s" (plist-get made :error)))
               (t (let ((value (funcall answer (plist-get made :child))))
                    (if (eq (car-safe value) 'romacs-mcp--deferred)
                        (funcall (setq next (cadr value)))
                      (list value))))))))))

(defun romacs-agent-tools--cancel (arguments)
  (romacs-mcp--instance arguments)
  (let ((child (romacs-agent-tools--known arguments)))
    (setf (romacs-agent-cancelled child) t (romacs-agent-queue child) nil
          (romacs-agent-queue-tail child) nil (romacs-agent-queue-length child) 0)
    ;; A restarted server knows it was cancelled.
    (with-temp-file (expand-file-name "cancelled" (romacs-agent-dir child)))
    ;; An idle one has no end of a run to tell its views.
    (romacs-agent--raise child (list :type 'status :text "cancelled"))
    (romacs-agent-stop child)
    (romacs-agent-tools--task child)))

(defun romacs-agent-tools--effort-option (instance)
  "The name INSTANCE's harness's effort goes by in a target's options."
  (if (eq (plist-get instance :harness) 'codex) "reasoningEffort" "effort"))

(defun romacs-agent-tools--model (instance row)
  "ROW of `romacs-agent--models' as T3 Code lists a model."
  (let ((efforts (plist-get (cdr row) :efforts)))
    (romacs-mcp--object
     "id" (car row) "label" (or (plist-get (cdr row) :label) (car row))
     "options" (if efforts
                   (vector (romacs-mcp--object "id" (romacs-agent-tools--effort-option instance)
                                               "values" (vconcat efforts)
                                               "default" (or (plist-get (cdr row) :default-effort) :null)))
                 []))))

(defun romacs-agent-tools--capabilities (arguments)
  "Each provider instance (`romacs-agent-instances'): its harness,
directory, program, models, whether it is enabled and where it comes
from; and each one Romacs cannot run, with why."
  (romacs-mcp--instance arguments)
  (pcase-let ((`(,instances . ,constraints) (romacs-agent-instances t)))
    (romacs-mcp--object
     "default" (or (ignore-errors (romacs-agent--default-provider)) :null)
     "providers"
     (vconcat
      (mapcar (lambda (instance)
                (let ((dir (or (plist-get instance :config-dir)
                               (plist-get (romacs-agent--driver-of (plist-get instance :harness)) :config-default))))
                  (romacs-mcp--object
                   "providerInstanceId" (plist-get instance :name) "displayName" (plist-get instance :display)
                   "harness" (symbol-name (plist-get instance :harness))
                   "enabled" (if (plist-get instance :enabled) t :false)
                   "source" (symbol-name (plist-get instance :source))
                   "t3InstanceId" (or (plist-get instance :t3-id) :null)
                   "configDir" (expand-file-name dir) "configDirExists" (if (file-directory-p dir) t :false)
                   "program" (plist-get instance :program)
                   "programFound" (if (executable-find (plist-get instance :program)) t :false)
                   "options" (vector (romacs-agent-tools--effort-option instance))
                   "models" (vconcat (mapcar (lambda (row) (romacs-agent-tools--model instance row))
                                             (romacs-agent--models instance))))))
              instances))
     "constraints"
     (vconcat (mapcar (lambda (c) (romacs-mcp--object "source" (symbol-name (plist-get c :source))
                                                      "id" (or (plist-get c :id) :null) "reason" (plist-get c :reason)))
                      constraints)))))

;;; Restart requests (the agents-develop-Romacs-live loop)

(defvar warm-restart-last-result)
(declare-function warm-restart-request "warm-restart")
(declare-function romacs-server--build-label "romacs-server" (true))

(defun romacs-agent-tools--restart-wake-file (session)
  (expand-file-name "restart-wake" (romacs-agent-dir session)))

(defun romacs-agent-tools--restart-ready (id)
  "A restart request's READY for session ID: its turn has ended."
  (lambda ()
    (when-let* ((session (romacs-agent-get id))
                ((memq (romacs-agent-state session) '(starting running waiting-approval))))
      (list (format "agent session %s's turn is running" (romacs-agent-title session))))))

(defun romacs-agent-tools--restart (arguments)
  "Schedule a restart after the calling session's turn; tell it after."
  (romacs-mcp--instance arguments)
  (require 'warm-restart)
  (let* ((id (romacs-agent-tools--caller))
         (session (and id (romacs-agent-get id))))
    (warm-restart-request
     :ready (and session (romacs-agent-tools--restart-ready id))
     :target (gethash "target" arguments)
     :delay (gethash "delay" arguments)
     :expiry (gethash "expiry" arguments)
     :force (eq (gethash "force" arguments) t)
     ;; Written just before the restart: the successor tells the session.
     :before (and session (lambda () (write-region "" nil (romacs-agent-tools--restart-wake-file session) nil 'silent)))
     :failed (and session (lambda (why)
                            (ignore-errors (delete-file (romacs-agent-tools--restart-wake-file session)))
                            (romacs-agent-tools--deliver session (format "Romacs restart failed, this session goes on: %s" why)
                                                         #'ignore))))
    (romacs-mcp--object
     "status" (symbol-name (plist-get warm-restart-last-result :status))
     "target" (or (gethash "target" arguments) :null)
     "waitsFor" (if session "this session's turn to end, then nothing in the way" "nothing in the way"))))

(defun romacs-agent-tools--build-text ()
  "Which build this process runs, for the restarted sessions' message."
  (let ((root (getenv "ROMACS_RUNTIME_ROOT")))
    (format "Romacs restarted into %s"
            (or (and root (fboundp 'romacs-server--build-label)
                     (when-let* ((label (romacs-server--build-label root))) (format "%s (%s)" label root)))
                (and root (format "the build at %s" root))
                (format "the build at %s" invocation-directory)))))

(defun romacs-agent-tools--restart-wakes ()
  "After a restart: tell each session that asked for it, once."
  (let ((text nil))
    (dolist (session (romacs-agent-sessions))
      (let ((file (romacs-agent-tools--restart-wake-file session)))
        (when (file-exists-p file)
          (romacs-agent-tools--deliver session (or text (setq text (romacs-agent-tools--build-text)))
                                       (lambda () (ignore-errors (delete-file file)))
                                       (lambda () (file-exists-p file))))))))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--restart-wakes)

;;; Options

(declare-function romacs-option-describe "romacs-option" (symbol))
(declare-function romacs-option-search "romacs-option" (query &rest keys))
(declare-function romacs-option-set "romacs-option" (symbol value))
(declare-function romacs-option-save "romacs-option" (symbol value))

(defconst romacs-agent-tools--flags '(:risky :safe :local :setter :obsolete :loaded)
  "The keys of an option's record that are true or false.")

(defun romacs-agent-tools--json (value)
  "An option record's VALUE as JSON: a symbol by its name, a list as an array."
  (cond ((null value) :null)
        ((eq value t) t)
        ((symbolp value) (symbol-name value))
        ((listp value) (vconcat (mapcar #'romacs-agent-tools--json value)))
        (t value)))

(defun romacs-agent-tools--object (record)
  "RECORD, a plist of `romacs-option.el', as a JSON object."
  (let ((object (romacs-mcp--object)))
    (cl-loop for (key value) on record by #'cddr
             do (puthash (substring (symbol-name key) 1)
                         (cond ((memq key romacs-agent-tools--flags) (if value t :false))
                               (t (romacs-agent-tools--json value)))
                         object))
    object))

(defun romacs-agent-tools--option-symbol (arguments)
  "The symbol ARGUMENTS name, which must exist: no tool interns a client's name."
  (let ((name (gethash "name" arguments)))
    (or (intern-soft name) (user-error "Unknown option: %s" name))))

(defun romacs-agent-tools--option-value (arguments)
  "The value ARGUMENTS give, Lisp syntax read and never evaluated."
  (let* ((text (gethash "value" arguments))
         (parsed (condition-case failure (read-from-string text)
                   (error (user-error "Value does not read as Lisp: %s"
                                      (error-message-string failure)))))
         (rest (substring text (cdr parsed))))
    (unless (string-match-p "\\`[ \t\n]*\\'" rest)
      (user-error "Text after the value: %s" rest))
    (car parsed)))

(defun romacs-agent-tools--option-describe (arguments)
  (romacs-mcp--instance arguments)
  (require 'romacs-option)
  (romacs-agent-tools--object (romacs-option-describe (romacs-agent-tools--option-symbol arguments))))

(defun romacs-agent-tools--option-search (arguments)
  (romacs-mcp--instance arguments)
  (require 'romacs-option)
  (let* ((name (gethash "group" arguments)) (kind (gethash "kind" arguments))
         (group (and name (intern-soft name)))
         (rows (progn
                 (unless (member kind '(nil "variable" "face")) (user-error "Unknown kind: %s" kind))
                 (when (and name (not (and group (get group 'custom-group))))
                   (user-error "Unknown group: %s" name))
                 (romacs-option-search (gethash "query" arguments)
                                       :group group
                                       :kind (and kind (intern kind))
                                       :docs (eq (gethash "docs" arguments) t)
                                       :limit (gethash "limit" arguments 50)))))
    (romacs-mcp--object
     "scope" "loaded"
     "options" (vconcat (mapcar (lambda (row)
                                  (romacs-mcp--object "name" (symbol-name (car row))
                                                      "kind" (symbol-name (nth 1 row))
                                                      "summary" (or (nth 2 row) :null)))
                                rows)))))

(defun romacs-agent-tools--option-write (write verb arguments)
  "Call WRITE, `romacs-option-set' or `-save', for ARGUMENTS, and say so:
the user sees what an agent changed.  VERB names it."
  (romacs-mcp--instance arguments)
  (require 'romacs-option)
  (let ((outcome (romacs-agent-tools--bounded
                  arguments
                  (lambda () (funcall write (romacs-agent-tools--option-symbol arguments)
                                      (romacs-agent-tools--option-value arguments))))))
    (message "Agent %s %s %s: %s" (or (romacs-agent-tools--caller) "over MCP") verb
             (plist-get outcome :option) (plist-get outcome :value))
    (romacs-agent-tools--object outcome)))

;;; Registration

(romacs-mcp-register-tool
 "romacs_eval"
 "Evaluate Emacs Lisp in the running editor (its buffers, frames and state) and return the last value printed. \
Runs at a wait point of the editor's command loop; C-g or budgetMs (default 5000, 0 for none) quits it at its next \
safepoint. instance (from romacs_identity) is optional: given, a call that would reach a restarted Romacs is \
refused. Never safe to repeat after an unknown outcome: on `Romacs instance mismatch', call romacs_identity, \
then romacs_call_outcome for your in-flight ids."
 (romacs-mcp--schema '(("instance" . "string") ("code" . "string") ("budgetMs" . "integer")) '("code"))
 #'romacs-agent-tools--eval)

(romacs-mcp-register-tool
 "romacs_buffer_edit"
 "Replace ranges of a buffer's text, atomically and as one undo group. Positions are Emacs positions (1 is the \
first character); edits must not overlap. With expectedTick the edit is refused if the buffer changed since you \
read it (romacs_buffer_read gives the tick). Safe to repeat with expectedTick."
 (romacs-mcp--schema '(("instance" . "string") ("name" . "string") ("expectedTick" . "integer") ("edits" . "array"))
                     '("name" "edits"))
 #'romacs-agent-tools--buffer-edit)

(romacs-mcp-register-tool
 "romacs_file_write"
 "Make a file's buffer text TEXT (visiting it first), as one undo group; save it only when save is true. The \
file's earlier content is kept for this session."
 (romacs-mcp--schema '(("instance" . "string") ("path" . "string") ("text" . "string") ("save" . "boolean"))
                     '("path" "text"))
 #'romacs-agent-tools--file-write)

(romacs-mcp-register-tool
 "romacs_visit"
 "Visit a file in a buffer (not shown), optionally at a line, and return the buffer's name and tick."
 (romacs-mcp--schema '(("path" . "string") ("line" . "integer")) '("path"))
 #'romacs-agent-tools--visit)

(romacs-mcp-register-tool
 "romacs_call_outcome"
 "After a Romacs restart answered a call `outcome unknown': what happened to the call with request id ID: \
`not executed' (safe to repeat), `unknown' (it was running; read the state again before repeating) or `done' \
with its result."
 (romacs-mcp--object "type" "object" "properties"
                     (romacs-mcp--object "id" (romacs-mcp--object))
                     "required" ["id"] "additionalProperties" :false)
 #'romacs-agent-tools--outcome
 (romacs-mcp--object "readOnlyHint" t))

(defun romacs-agent-tools--delegate-description ()
  "romacs_delegate_task's description, naming the instances it accepts now."
  (concat
   "Start a child agent session on TASK, as T3 Code's delegate_task. The child gets only TASK (and ROLE): none of \
your conversation, so write a complete brief: the goal, the files and facts it needs, constraints, and what done \
means. target {provider, model, options {reasoningEffort|effort}}, role, cwd (default the caller's), title. \
provider defaults to your own session's instance \
(claude to claude, codex to codex); name another to run there: "
   (or (ignore-errors
         (mapconcat #'identity (cl-loop for i in (romacs-agent-instances) when (plist-get i :enabled)
                                        collect (plist-get i :name))
                    ", "))
       "see romacs_orchestrator_capabilities")
   " (romacs_orchestrator_capabilities has their models and options). mode async (default) answers at once with \
the taskId and wakes you with the child's result when its turn ends: end your turn then, do not poll. Ends that come \
together or while you work wake you once; an empty result, or one a child starts with [quiet] (nothing for you to \
act on: no landing, decision or blocker), wakes no one and stays readable by romacs_task_status. wake: end (default), failure (only a failure wakes you), all (one wake when \
every task you delegated with all has ended), none (read it with romacs_task_status). mode wait answers when it ends, asks for an \
approval, or timeoutMs (default 600000) passes, and the child goes on. The same clientRequestId returns the task it \
started before. isolation \"worktree\" gives the child its own checkout of cwd's repository, on a new branch at your \
HEAD: it commits there, and you merge the branch and remove the worktree (the task's worktree and branch); Romacs \
never removes one. Use it for children that edit files at the same time. resume takes over an existing conversation \
of the provider (a Claude Code session id or a Codex thread id) as your task: cwd is required, the directory it was \
held in; task is sent to it as is, as its next message (empty: none). Taking over one you already own returns its \
task and sends nothing; one another session owns is refused. Talk to a running task with romacs_task_send; \
romacs_task_list finds your tasks again."))

(romacs-mcp-register-tool
 "romacs_delegate_task"
 #'romacs-agent-tools--delegate-description
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "task" (romacs-mcp--object "type" "string") "role" (romacs-mcp--object "type" "string")
                "mode" (romacs-mcp--object "type" "string" "enum" ["async" "wait"])
                "wake" (romacs-mcp--object "type" "string" "enum" (vconcat romacs-agent-tools--wake-rules))
                "target" (romacs-mcp--object
                          "type" "object"
                          "properties" (romacs-mcp--object
                                        "provider" (romacs-mcp--object
                                                    "type" "string"
                                                    "description" "An instance (providerInstanceId, or a T3 instance id); default your own session's.")
                                        "model" (romacs-mcp--object "type" "string")
                                        "options" (romacs-mcp--object "type" "object")))
                "cwd" (romacs-mcp--object "type" "string") "title" (romacs-mcp--object "type" "string")
                "isolation" (romacs-mcp--object "type" "string" "enum" ["shared" "worktree"])
                "timeoutMs" (romacs-mcp--object "type" "integer")
                "clientRequestId" (romacs-mcp--object "type" "string")
                "resume" (romacs-mcp--object
                          "type" "string"
                          "description" "A conversation id of the provider's to go on with, in cwd, instead of a new one."))
  "required" ["task"] "additionalProperties" :false)
 #'romacs-agent-tools--delegate)

(romacs-mcp-register-tool
 "romacs_task_status"
 "A delegated task's status (running, waiting, completed, failed, cancelled), provider, model, turns, cost and its \
last result."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string")) '("taskId"))
 (lambda (arguments) (romacs-mcp--instance arguments) (romacs-agent-tools--task (romacs-agent-tools--known arguments)))
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_task_send"
 "Send a message to a task you delegated, or to another task of your own parent (your parent gets a copy with its \
next wake, never woken for it); any other session is refused. It reads it as a <romacs-message> naming you, and \
answers with romacs_task_reply or its turn's reply; inReplyTo links this one to a message it answers. mode queue (default): at \
once when the task is idle, after its running turn when it is busy, resuming its harness when it had ended; steer: \
into its running turn; replace: in place of your messages still waiting for it; interrupt: its turn cut short, \
then this first. A task's next turn end wakes its parent with its reply by its wake rule: end your turn then, do \
not poll. Answers its task record."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string") "taskId" (romacs-mcp--object "type" "string")
                "message" (romacs-mcp--object "type" "string")
                "mode" (romacs-mcp--object "type" "string" "enum" (vconcat romacs-agent-tools--send-modes))
                "inReplyTo" (romacs-mcp--object "type" "string"
                                                "description" "The id of a message this one answers."))
  "required" ["taskId" "message"] "additionalProperties" :false)
 #'romacs-agent-tools--send)

(romacs-mcp-register-tool
 "romacs_task_reply"
 "Answer a message another session sent you (a <romacs-message>, by its id): routed back to its sender, whatever \
it is to you (parent, child, sibling), linked to the message it answers. A message from your parent is answered \
by your turn's reply too: use this to answer before your turn ends, or anyone else. mode as romacs_task_send's. \
Answers the sender's task record and the reply's messageId."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "messageId" (romacs-mcp--object "type" "string") "text" (romacs-mcp--object "type" "string")
                "mode" (romacs-mcp--object "type" "string" "enum" (vconcat romacs-agent-tools--send-modes)))
  "required" ["messageId" "text"] "additionalProperties" :false)
 #'romacs-agent-tools--reply)

(romacs-mcp-register-tool
 "romacs_task_list"
 "The tasks you delegated (with all, every session's), most recently active first: each task's record with the \
first 300 characters of its result and resultChars, its whole length. They are kept on disk, so a restarted Romacs \
or a lead that lost its context finds them again. Read one whole with romacs_task_status; talk to one with \
romacs_task_send."
 (romacs-mcp--schema '(("instance" . "string") ("all" . "boolean")) nil)
 #'romacs-agent-tools--list
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_task_switch"
 "Change a task's model or effort, or go on with it under another provider instance (taskId, default your own \
session), e.g. when its model or instance hits a usage limit. The same session and conversation go on, once its \
running turn ends (your own: end your turn): in place with model or effort alone (provider defaults to its own), \
or moved as is to an instance of the same harness (claude to claude2); on another harness, a new task starts on a \
brief of the thread, with the same parent and wake rule. model and effort default to the task's; next is a message \
it is given. A task's record names its usage limit and the instances with room."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string") ("provider" . "string") ("model" . "string")
                       ("effort" . "string") ("next" . "string"))
                     nil)
 #'romacs-agent-tools--switch)

(romacs-mcp-register-tool
 "romacs_task_cancel"
 "Cancel a delegated task: its session stops, with everything its harness started."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string")) '("taskId"))
 #'romacs-agent-tools--cancel)

(romacs-mcp-register-tool
 "romacs_restart_request"
 "Restart Romacs into a new process once your turn ends, carrying the session (buffers, windows, histories) \
and its agent sessions: after an install, the new build runs. target is a build directory (one holding \
romacs-vm-gui, lisp/gnu and lisp/romacs, e.g. the launcher's current) or a binary; omitted, the session's restart \
command or its own binary. It waits until your turn has ended, nothing is BLOCKED (unsaved work that cannot be \
carried), no Romacs tool call is in flight and the editor is between commands, for delay seconds (default 2), up \
to expiry seconds (default 900). A process that would end (a shell) is in the way unless force. After the \
restart this session is told \"Romacs restarted into <commit>\"; if it fails, it is told why."
 (romacs-mcp--schema '(("instance" . "string") ("target" . "string") ("delay" . "number")
                       ("expiry" . "number") ("force" . "boolean"))
                     nil)
 #'romacs-agent-tools--restart)

(romacs-mcp-register-tool
 "romacs_orchestrator_capabilities"
 "The provider instances romacs_delegate_task can start: harness, configuration directory, program and options."
 (romacs-mcp--schema '(("instance" . "string")) nil)
 #'romacs-agent-tools--capabilities
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_option_search"
 "Find user options and faces by words in their names, or in their documentation with docs. group keeps one \
customization group, kind `variable' or `face' one kind, limit (default 50) bounds the rows. Only options whose \
library is loaded are known: load it first to find the rest. Obsolete options are left out."
 (romacs-mcp--schema '(("instance" . "string") ("query" . "string") ("group" . "string") ("kind" . "string")
                       ("docs" . "boolean") ("limit" . "integer"))
                     '("query"))
 #'romacs-agent-tools--option-search
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_option_describe"
 "One option or face: documentation, groups, its raw :type as Lisp, standard and current values printed as Lisp, \
state (GNU's: standard, saved, set, themed, changed, rogue), the themes setting it, and whether it is risky. An \
autoloaded option whose library is not loaded says loaded false and little else."
 (romacs-mcp--schema '(("instance" . "string") ("name" . "string")) '("name"))
 #'romacs-agent-tools--option-describe
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_option_set"
 "Set a user option for this session, through its :set as GNU does. value is Lisp syntax, read and not evaluated: \
72, t, nil, \"text\", (a b). A value that does not fit :type is refused and nothing changes. The answer gives \
previous, value and state: set previous back to restore the value, though what a :set function did stays done. \
A value is null when it has no printed form or is past 8 KB. A setter that waits for input quits at budgetMs \
(default 5000). Faces are read-only here. Loads the option's library when it must."
 (romacs-mcp--schema '(("instance" . "string") ("name" . "string") ("value" . "string") ("budgetMs" . "integer"))
                     '("name" "value"))
 (lambda (arguments) (romacs-agent-tools--option-write #'romacs-option-set "set" arguments)))

(romacs-mcp-register-tool
 "romacs_option_save"
 "As romacs_option_set, and keep the value for future sessions, in the user's saved settings, never their init. \
A later saved value replaces this one; the entry itself stays."
 (romacs-mcp--schema '(("instance" . "string") ("name" . "string") ("value" . "string") ("budgetMs" . "integer"))
                     '("name" "value"))
 (lambda (arguments) (romacs-agent-tools--option-write #'romacs-option-save "saved" arguments))
 (romacs-mcp--object "destructiveHint" t))

(romacs-mcp-enable-editor-tools)
;; Gnosis's agent API, when gnosis is installed (it loads at a tool's first call).
(when (locate-library "gnosis-agent") (require 'romacs-agent-gnosis))
(romacs-mcp-enable-session-tools)
(romacs-mcp-enable-dev-tools)

(provide 'romacs-agent-tools)
;;; romacs-agent-tools.el ends here

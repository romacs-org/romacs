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
(require 'romacs-agent-summary)
(require 'romacs-agent-worktree)
(require 'romacs-agent-renew)
(require 'romacs-agent-issues)
(require 'romacs-agent-attachments)
(require 'romacs-agent-limits)


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
        (let ((coding-system-for-write 'utf-8-unix))
          (romacs-mcp-write-private
           file (lambda ()
                  (insert (romacs-agent--json (romacs-mcp--object "key" "forgotten")) "\n")
                  (dolist (key (reverse (seq-take order romacs-agent-tools--kept)))
                    (insert (romacs-agent--json (gethash key table)) "\n")))))))
    table))

(defun romacs-agent-tools--table (session-id)
  (or (gethash session-id romacs-agent-tools--calls)
      (puthash session-id (romacs-agent-tools--load session-id) romacs-agent-tools--calls)))

(defun romacs-agent-tools--note (session-id key record)
  "Record RECORD for call KEY of SESSION-ID: in memory, and one line
appended to its log.  Past twice `romacs-agent-tools--kept' calls in
memory, its table is let go: the next call reads the log back, its
newest kept and the log compacted (`romacs-agent-tools--load'), so a
long run holds and appends a bounded number."
  (puthash "key" key record)
  (let ((table (romacs-agent-tools--table session-id)))
    (puthash key record table)
    (let ((dir (romacs-agent-tools--dir session-id)))
      (when (file-directory-p dir)
        (let ((coding-system-for-write 'utf-8-unix) (file (romacs-agent-tools--log-file session-id)))
          (write-region (concat (json-serialize record :false-object :false :null-object :null) "\n")
                        nil file t 'silent)
          (set-file-modes file #o600))
        ;; Written: the log has every record the table had.
        (when (> (hash-table-count table) (* 2 romacs-agent-tools--kept))
          (remhash session-id romacs-agent-tools--calls))))))

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
                    (romacs-mcp-defer
                     (lambda ()
                       (let ((ok nil))
                         (unwind-protect
                             (let ((answer (funcall poll)))
                               (setq ok t)
                               (when answer (romacs-agent-tools--finish session key name start (car answer) nil))
                               answer)
                           (unless ok (romacs-agent-tools--finish session key name start "signalled an error" t)))))
                     (nth 2 value)))
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

(defun romacs-agent-tools--status (summary)
  "The task status of the session SUMMARY says (`romacs-agent-summary'), in
T3 Code's words: a plan it offers waits on its user as an approval does."
  (let ((state (romacs-agent-summary-state summary)))
    (cond ((romacs-agent-summary-cancelled summary) "cancelled")
          ((or (eq state 'waiting-approval) (romacs-agent-summary-plan-offer-waits summary)) "waiting")
          ;; Never started: failed, whatever waits for it, its state saying
          ;; so (not detached).
          ((eq state 'unstarted) "failed")
          ((or (memq state '(starting running)) (> (romacs-agent-summary-queue-length summary) 0)) "running")
          ((romacs-agent-summary-failed summary) "failed")
          ;; Its turn over, the tasks it delegated still at work: its own
          ;; task is not done (T3 Code's waiting_for_children).
          ((romacs-agent-tools--delegates-working-p summary) "running")
          ((> (romacs-agent-summary-turns summary) 0) "completed")
          ((eq state 'ended) "failed")
          (t (symbol-name state)))))

(defun romacs-agent-tools--delegates-working-p (summary)
  "Whether a task the session SUMMARY says delegated will yet wake that
session with its end: it is at work, or its end waits to be delivered.
A turn of the session's ending then is not its own task's end.  Only a
task whose every end wakes it (rule end or all) counts: one that wakes
it only on failure, or never, may end and leave it asleep."
  (cl-some (lambda (row)
             (when-let* ((id (plist-get row :id)) (child (romacs-agent-summary id)))
               (and (member (or (romacs-agent-summary-wake-rule child) "end") '("end" "all"))
                    (or (member (romacs-agent-tools--status child) '("running" "waiting"))
                        (and (eq (romacs-agent-summary-state child) 'detached)
                             (not (romacs-agent-summary-cancelled child)))
                        ;; Ended, its end not yet in this session's harness.
                        (romacs-agent-summary-undelivered child)))))
           (romacs-agent-summary-children-rows summary)))

(defun romacs-agent-tools--running-next (summary)
  "What a caller is told to do about the task SUMMARY says, still running."
  (pcase (romacs-agent-summary-wake-rule summary)
    ("none" "It will not wake you: read its result with romacs_task_status when you need it.")
    ("failure" "Only its failure wakes you, so end your turn now: do not wait on it or poll it.")
    ("all" "You are woken once every task you delegated with wake all has ended, so end your turn now: do not wait or poll.")
    (_ "Its end wakes you with its result, so end your turn now: do not wait on it or poll it.")))

(defun romacs-agent-tools--task (summary)
  "The session SUMMARY says (`romacs-agent-summary') as a task record; one
still running says what to do next, and one whose plan offer waits on its
user says so (planOffered)."
  (let* ((null (lambda (v) (or v :null)))
         (status (romacs-agent-tools--status summary))
         (task (romacs-mcp--object
                "taskId" (romacs-agent-summary-id summary) "status" status
                "state" (symbol-name (romacs-agent-summary-state summary)) "title" (romacs-agent-summary-title summary)
                "provider" (romacs-agent-summary-provider summary)
                "harness" (symbol-name (romacs-agent-summary-harness summary))
                "model" (funcall null (or (romacs-agent-summary-model summary)
                                          (romacs-agent-summary-requested-model summary)))
                "role" (funcall null (romacs-agent-summary-role summary))
                "parent" (funcall null (romacs-agent-summary-parent summary))
                "turns" (romacs-agent-summary-turns summary) "result" (funcall null (romacs-agent-summary-reply summary))
                "cost" (funcall null (romacs-agent-summary-cost summary))
                "cwd" (funcall null (romacs-agent-summary-cwd summary))
                "conversation" (romacs-agent-summary-conversation summary)
                "worktree" (funcall null (romacs-agent-summary-worktree summary))
                "branch" (funcall null (romacs-agent-summary-branch summary))
                "pendingApproval" (if (> (romacs-agent-summary-waiting-count summary) 0) t :false)
                "limit" (romacs-agent-tools--limit-object summary)
                "next" (if (equal status "running") (romacs-agent-tools--running-next summary) :null))))
    (when (romacs-agent-summary-plan-offer-waits summary) (puthash "planOffered" t task))
    task))

(defconst romacs-agent-tools--ack-keys
  '("taskId" "status" "state" "title" "provider" "harness" "model" "worktree" "branch" "limit" "next"
    "planOffered")
  "The fields of a task's record (`romacs-agent-tools--task') an action on
it answers with: what its caller acts on next.")

(defun romacs-agent-tools--ack (session &optional result)
  "SESSION's task record as an action on it answers (a message, a reply,
a delegation, a switch, a renewal, a cancel), from its summary as the
action left it: `romacs-agent-tools--ack-keys' and resultChars, its last
result's length, never that result again (it would repeat into the
caller's context each time).  RESULT, a result the caller has not had
yet, goes with it when given.  The whole record is romacs_task_status's."
  (let* ((summary (romacs-agent-summary (romacs-agent-id session)))
         (task (romacs-agent-tools--task summary)) (ack (romacs-mcp--object)))
    (dolist (key romacs-agent-tools--ack-keys)
      (when-let* ((value (gethash key task))) (puthash key value ack)))
    ;; A limit's text is the turn's reply: its result again.
    (when (hash-table-p (gethash "limit" ack)) (remhash "text" (gethash "limit" ack)))
    (puthash "resultChars" (length (or (romacs-agent-summary-reply summary) "")) ack)
    (when result (puthash "result" result ack))
    ack))

(defun romacs-agent-tools--listed-p (summary caller all)
  "Whether the task SUMMARY says is listed for CALLER, including other
delegated tasks with ALL."
  (let ((parent (romacs-agent-summary-parent summary)))
    (or (equal parent caller) (and all parent))))

(defun romacs-agent-tools--visible-prefix (prefix caller)
  "Return stored ids with PREFIX that CALLER can see in the full task list."
  (when (>= (length prefix) 8)
    (cl-remove-if-not
     (lambda (id)
       (when-let* ((summary (romacs-agent-summary id)))
         (romacs-agent-tools--listed-p summary caller t)))
     (romacs-agent-store-prefixed-ids prefix))))

(defun romacs-agent-tools--ask-owner (id &optional then)
  "Ask session ID's owner for it by id, loading it: nil when no session is
ID's, `folding' while a worker VM still folds its records (with THEN,
that fold goes first, and THEN is called once it is done, never when it
fails: `romacs-agent--when-folded'), else t, its summary
\(`romacs-agent-summary') saying all of it, not what its row alone holds."
  (when-let* ((session (romacs-agent-get id)))
    (if (not (romacs-agent--folding-p session)) t
      (when then (romacs-agent--when-folded session then))
      'folding)))

(defun romacs-agent-tools--resolve (id)
  "The id of the task named by ID, exact or a unique prefix the caller
sees, asked of its owner (`romacs-agent-tools--ask-owner').  A
`user-error' when none is, or ID names several."
  (if (romacs-agent-tools--ask-owner id) id
    (let ((matches (romacs-agent-tools--visible-prefix id (romacs-agent-tools--caller))))
      (when (cdr matches)
        (user-error "Ambiguous task id %s. Use one of these full ids: %s"
                    id (string-join matches ", ")))
      (or (and matches (romacs-agent-tools--ask-owner (car matches)) (car matches))
          (user-error "No task %s. Call romacs_task_list for task ids." id)))))

(defun romacs-agent-tools--told (id answer)
  "ANSWER's value, a function of ID, once session ID's owner has all of
it: at once, or, while a worker VM folds its records, a deferred answer
\(`romacs-mcp-defer') given once that fold is done, hurried; a fold that
fails for good is the answer's error, never what its row alone says."
  (let ((folded nil))
    (if (not (eq (romacs-agent-tools--ask-owner id (lambda () (setq folded t))) 'folding))
        (funcall answer id)
      (romacs-mcp-defer
       (lambda ()
         (cond ((eq (romacs-agent-tools--ask-owner id) 'folding) nil)
               (folded (list (funcall answer id)))
               (t (user-error "Task %s's records could not be read by a worker; its status is unknown until a load reads them"
                              id))))))))

(defun romacs-agent-tools--known (id)
  "Return the task with exact ID or a unique visible prefix matching ID
\(`romacs-agent-tools--resolve'), to act on."
  (romacs-agent-get (romacs-agent-tools--resolve id)))

(defun romacs-agent-tools--woken-file (session)
  (expand-file-name "woken" (romacs-agent-dir session)))

(defvar romacs-agent-tools--woken-seqs (make-hash-table :test #'eq :weakness 'key)
  "Each session's last turn end delivered to its parent, as its file was
read or written here: an end is checked against it without reading it.")

(defun romacs-agent-tools--woken (session)
  "The journal seq of SESSION's last turn end delivered to its parent."
  (or (gethash session romacs-agent-tools--woken-seqs)
      (puthash session
               (or (ignore-errors (with-temp-buffer (insert-file-contents (romacs-agent-tools--woken-file session))
                                                    (string-to-number (buffer-string))))
                   0)
               romacs-agent-tools--woken-seqs)))

(defun romacs-agent-tools--mark-woken (session seq)
  "SESSION's turn ends through SEQ are delivered (never moving back): the
parent's harness takes them in order."
  (when (> seq (romacs-agent-tools--woken session))
    (with-temp-file (romacs-agent-tools--woken-file session) (insert (number-to-string seq)))
    (puthash session seq romacs-agent-tools--woken-seqs)
    (romacs-agent-tools--owed-paid session)
    ;; Its summary says whether an end waits to be delivered.
    (romacs-agent--summary-stale (romacs-agent-id session))))

(defun romacs-agent-tools--owed-paid (session)
  "SESSION's `owed' mark goes once none of its ends is still to be given
to its parent (`romacs-agent-tools--hand-on'): a newer end answered
leaves an older one still on its way marked."
  (unless (or (gethash session romacs-agent-tools--held)
              (cl-some (lambda (end) (romacs-agent-tools--wanted-p session (car end))) (romacs-agent-ends session)))
    (romacs-agent-mark (romacs-agent-id session) 'owed nil)))

(defconst romacs-agent-tools--wake-limit 16000
  "The most of a child's result a wake carries: the rest is
romacs_task_status's.")

(defun romacs-agent-tools--wake-result (id result)
  "RESULT, of task ID, as a wake carries it: whole, or its start and where
the rest is."
  (if (<= (length result) romacs-agent-tools--wake-limit) result
    (format "%s\n[Result cut at %d of %d characters: romacs_task_status with taskId %s has it whole.]"
            (substring result 0 romacs-agent-tools--wake-limit) romacs-agent-tools--wake-limit (length result) id)))

(defun romacs-agent-tools--limit-object (summary)
  "The last end at its instance's usage limit of the session SUMMARY says,
with the instances that have room, as a task record says it; or null."
  (if-let* ((limited (romacs-agent-summary-limited summary)))
      (romacs-mcp--object
       "provider" (plist-get limited :provider) "model" (or (plist-get limited :model) :null)
       "text" (or (plist-get limited :text) :null)
       "window" (or (plist-get limited :window) :null) "resets" (or (plist-get limited :resets) :null)
       "alternatives" (vconcat (mapcar (lambda (a) (romacs-mcp--object "provider" (plist-get a :provider)
                                                                       "lossless" (if (plist-get a :lossless) t :false)))
                                       (romacs-agent-summary-alternatives summary))))
    :null))

(defun romacs-agent-tools--limit-note (summary)
  "What a wake adds to the end at its usage limit of the session SUMMARY
says: where it could go on."
  (when-let* ((limited (romacs-agent-summary-limited summary)))
    (let ((alternatives (romacs-agent-summary-alternatives summary))
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

(defun romacs-agent-tools--wake-result-of (summary end)
  "What the wake of the turn END of the session SUMMARY says, says of its
result."
  (romacs-agent-tools--wake-result
   (romacs-agent-summary-id summary)
   (concat (or (nth 1 end) "") (and (nth 2 end) (romacs-agent-tools--limit-note summary)))))

(defun romacs-agent-tools--end-state (summary end)
  "What a wake says of the turn END, (SEQ RESULT FAILED), of the session
SUMMARY says, as its status says it: failed; waiting, its last end while
the plan it offers waits on its user; else completed."
  (cond ((nth 2 end) "failed")
        ((and (romacs-agent-summary-plan-offer-waits summary)
              (eql (car end) (romacs-agent-summary-done-seq summary)))
         "waiting")
        (t "completed")))

(defun romacs-agent-tools--wake-message (summary end)
  "The record (`romacs-agent-make-message') of the wake of the turn END of
the session SUMMARY says: a child's result, answering the message its
turn began with."
  (let ((result (romacs-agent-tools--wake-result-of summary end))
        (id (romacs-agent-summary-id summary)))
    (append (romacs-agent-make-message (concat "m-" (romacs-agent--uuid)) "child"
                                  (list :session id :name (or (romacs-agent-summary-title summary) id))
                                  (nth 3 end) result)
            (list :wake (list :state (romacs-agent-tools--end-state summary end) :chars (length result))))))

(defun romacs-agent-tools--wake-text (summary end)
  "The parent's message for the turn END, (SEQ RESULT FAILED), of the
session SUMMARY says.  Its head says how long its result is: wakes go
together, and a result may hold anything."
  (let ((result (romacs-agent-tools--wake-result-of summary end)))
    (format "Delegated task %s (%s) [%s, %d chars%s]:\n%s"
            (romacs-agent-summary-id summary) (romacs-agent-summary-title summary)
            (romacs-agent-tools--end-state summary end) (length result)
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
  "Delegated task \\([0-9a-f-]\\{36\\}\\) (\\(.*\\)) \\[\\(completed\\|failed\\|waiting\\|sent\\)\\(?:, \\([0-9]+\\) chars\\)?\\(?:, re \\(m-[0-9a-f-]+\\)\\)?\\]:\n"
  "A wake's head: its task, title, state (\"sent\": a copy of its message to
another task of its parent; \"waiting\": a plan offered waits on its user),
when it says, its result's length, and the message the turn answered.")

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

(defconst romacs-agent--parent-reply-line
  "Your turn's reply answers this message from your parent."
  "The line after a message from the receiver's parent when the
receiver's turn's reply reaches it (`romacs-agent--turn-answers-p'): the
one channel that answers it, so no call is offered to answer it twice.
Beside the regexp that reads it back, which needs it as it loads.")

(defconst romacs-agent--reply-line-regexp
  (concat "\n\\(?:"
          "Reply to this message with SendMessage(to=\"[^\"\n]*\", message=\\.\\.\\.)\\."
          "\\|Reply to this \\(?:message\\|result\\) with romacs_task_reply(messageId=\"[^\"\n]*\", text=\\.\\.\\.)\\."
          "\\|Its sender is outside Romacs and has no reply route: it reads only your turn's reply\\."
          "\\|" (regexp-quote romacs-agent--parent-reply-line)
          "\\)[ \t]*\\(?:\n[ \t\n]*\\|\\'\\)")
  "The line `romacs-agent--message-reply-line' writes after an envelope,
each of its forms whole, and the space after it.")

(defun romacs-agent--envelope-at (text at)
  "The whole envelope of TEXT at AT, the line its sender wrote saying how
to answer it after it (`romacs-agent--message-reply-line'), as (MESSAGE
. END), MESSAGE a plist of :from, :state and :text; nil if none is
there.  An envelope without that line is a user's example."
  (when (eq (string-match "<romacs-message\\( [^>]*\\)?>\n?" text at) at)
    (let* ((attributes (or (match-string 1 text) ""))
           (open-end (match-end 0))
           (attribute (lambda (name)
                        (and (string-match (format " %s=\"\\([^\"]*\\)\"" name) attributes)
                             (match-string 1 attributes))))
           (from (funcall attribute "from"))
           (kind (funcall attribute "kind"))
           (close (string-search "</romacs-message>" text open-end))
           (body (and close (string-trim-right (substring text open-end close) "\n")))
           (after (and close (+ close (length "</romacs-message>")))))
      (when (and after (eq (string-match romacs-agent--reply-line-regexp text after) after))
        (cons (list :from (or from "a session") :state kind :text body) (match-end 0))))))

(defun romacs-agent--envelopes (text)
  "The messages of TEXT when it is whole envelopes and space alone, in
order; nil otherwise."
  (let ((at (progn (string-match "\\`[ \t\n]*" text) (match-end 0))) (found nil) (ok t))
    (while (and ok (< at (length text)))
      (if-let* ((envelope (romacs-agent--envelope-at text at)))
          (setq found (cons (car envelope) found) at (cdr envelope))
        (setq ok (string-match-p "\\`[ \t\n]*\\'" (substring text at)) at (length text))))
    (and ok found (nreverse found))))

(defun romacs-agent-incoming-of (text)
  "TEXT, written to an agent as a user's turn, read as what came from
elsewhere: other sessions' messages (whole <romacs-message> envelopes,
`romacs-agent--message-envelope', each with the line saying how to
answer it) or a delegated task's wakes.  A list of plists of :from
\(whom it came from), :state (the message's kind, or the task's state)
and :text (its body), one per message or wake; nil for anything else,
the user's own words among them."
  (cond ((not (stringp text)) nil)
        ((string-match-p "\\`[ \t\n]*<romacs-message[ >]" text) (romacs-agent--envelopes text))
        ((string-prefix-p "Delegated task " text)
         (mapcar (lambda (wake) (list :from (plist-get wake :title) :state (plist-get wake :state)
                                      :text (plist-get wake :result)))
                 (romacs-agent-tools--wakes-parse text)))))

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

(defun romacs-agent-tools--write-answered (session seq)
  "Keep that SESSION's turn end SEQ is answered, in its `answered' file."
  (write-region (format "%d\n" seq) nil (expand-file-name "answered" (romacs-agent-dir session)) t 'silent))

(defun romacs-agent-tools--mark-answered (session seq &optional unwritten)
  "SESSION's turn end SEQ is answered: its parent has it, and earlier
ends still on their way stay so.  UNWRITTEN: not kept yet, its caller
writes it (`romacs-agent-tools--write-answered')."
  (puthash seq t (romacs-agent-tools--answered-table session))
  (romacs-agent--summary-stale (romacs-agent-id session))
  (unless unwritten
    (romacs-agent-tools--write-answered session seq)
    ;; Kept first: the mark goes only once a restart reads it answered.
    (romacs-agent-tools--owed-paid session)))

(defun romacs-agent-tools--answer (session seq &optional unwritten)
  "A `wait' answered with SESSION's turn end SEQ: marked so
\(`romacs-agent-tools--mark-answered', UNWRITTEN as it says), it goes from
SESSION's ends now, no delivery may follow.  The ends are walked only
down to it: a wait answers its newest end."
  (romacs-agent-tools--mark-answered session seq unwritten)
  ;; Newest first: past an older seq it is not there (gone already).
  (let ((ends (romacs-agent-ends session)))
    (if (eql (car (car ends)) seq)
        (setf (romacs-agent-ends session) (cdr ends))
      (let ((cell ends))
        (while (and (cdr cell) (> (car (cadr cell)) seq)) (setq cell (cdr cell)))
        (when (and (cdr cell) (eql (car (cadr cell)) seq)) (setcdr cell (cddr cell)))))))

(defun romacs-agent-tools--drop-answered (child handed answered)
  "Let go of CHILD's ends whose seqs ANSWERED (a table) holds, all newer
than HANDED: only its ends newer than that are walked."
  (let ((kept nil) (rest (romacs-agent-ends child)))
    (while (and rest (> (car (car rest)) handed))
      (unless (gethash (car (car rest)) answered) (push (car rest) kept))
      (setq rest (cdr rest)))
    (setf (romacs-agent-ends child) (nconc (nreverse kept) rest))))

(defun romacs-agent-tools--wanted-p (session seq &optional summary)
  "Whether SESSION's turn end SEQ is still to be given to its parent: it
is not cancelled, as its SUMMARY says (read when not given), and the end
is not delivered or answered yet."
  (and (not (romacs-agent-summary-cancelled (or summary (romacs-agent-tools--summary-of session))))
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

(defun romacs-agent-tools--wakes-p (child end &optional summary)
  "Whether CHILD's turn END, (SEQ RESULT FAILED), wakes its parent: a
failure, unless its rule, as its SUMMARY says (read when not given), is
\"none\"; another end by its rule, unless its result is empty (a resumed
harness's own turn) or quiet (`romacs-agent-tools--quiet').  One whose
own tasks still work is held (`romacs-agent-tools--hand-on')."
  (let ((result (string-trim (or (nth 1 end) "")))
        (summary (or summary (romacs-agent-tools--summary-of child))))
    (pcase (romacs-agent-summary-wake-rule summary)
      ("none" nil)
      ("failure" (nth 2 end))
      (_ (or (nth 2 end)
             (not (or (string-empty-p result) (string-prefix-p romacs-agent-tools--quiet result))))))))

(defun romacs-agent-tools--wake-held-p (limited replaying)
  "Whether a child's ends wait to go to its parent, the one rule: at its
usage limit (LIMITED, `romacs-agent-limited'), it waits to move on
\=(`:moving', for a bounded time: `romacs-agent-tools--hand-over'), or its
records are still REPLAYING, so whether it moves is not yet known (till
it catches up or its attachment goes: `romacs-agent-tools--on-event')."
  (and limited (or (plist-get limited :moving) replaying) t))

(defun romacs-agent-tools--wake (child)
  "Hand CHILD's new turn ends to its parent's next wake, as CHILD's wake
rule says; an end no rule lets wake it is done with.  Each end is handed
on once; none while CHILD may move on at its usage limit
\=(`romacs-agent-tools--wake-held-p')."
  (unless (romacs-agent-tools--wake-held-p
           (romacs-agent-limited child)
           (and (process-live-p (romacs-agent-connection child)) (not (romacs-agent-caught-up child))))
    (romacs-agent-tools--hand-on child)))

(defvar romacs-agent-tools--handed (make-hash-table :test #'eq :weakness 'key)
  "A child to the seq of its last turn end handed to its parent's wake by
this process.  Its ends stay in it until delivered
\(`romacs-agent-tools--wanted-p'): a restart before then, its state kept,
hands them on again.")

(defvar romacs-agent-tools--pruned (make-hash-table :test #'eq :weakness 'key)
  "A child to the delivered seq its ends were last pruned at: they are
pruned again only once more of them are delivered.  An end answered goes
as it is (`romacs-agent-tools--answer').")

(defun romacs-agent-tools--unhanded (child)
  "CHILD's turn ends not yet handed on by this process, oldest first: the
newest of its ends, newest first, down to the last handed on."
  (let ((handed (gethash child romacs-agent-tools--handed 0)) (new nil))
    (cl-loop for end in (romacs-agent-ends child) while (> (car end) handed) do (push end new))
    new))

(defun romacs-agent-tools--prune-ends (child &optional summary)
  "Let go of CHILD's ends its parent has, once more of them are delivered
since they were last pruned (and the first time, what a restart kept): a
parent at work while ends pile up costs nothing per end.  SUMMARY is
CHILD's, when read already."
  (let ((woken (romacs-agent-tools--woken child)))
    (unless (eql woken (gethash child romacs-agent-tools--pruned))
      (puthash child woken romacs-agent-tools--pruned)
      (setf (romacs-agent-ends child)
            (cl-remove-if-not (lambda (end) (romacs-agent-tools--wanted-p child (car end) summary))
                              (romacs-agent-ends child))))))

(defun romacs-agent-tools--hand-on (child)
  ;; Its parent, loaded when only listed: a task's end reaches it all the same.
  (let ((parent (romacs-agent-load-session (plist-get (romacs-agent-options child) :parent)))
        (handed (gethash child romacs-agent-tools--handed 0))
        (ends (romacs-agent-tools--unhanded child))
        ;; Read once, when it has ends: what each end's handing on reads of it.
        (summary (and (romacs-agent-ends child) (romacs-agent-tools--summary-of child)))
        (answered nil))
    (romacs-agent-tools--prune-ends child summary)
    (when parent
      (dolist (end ends)
        (puthash child (car end) romacs-agent-tools--handed)
        ;; A newer end of it: one held before is superseded.
        (remhash child romacs-agent-tools--held)
        (when (romacs-agent-tools--wanted-p child (car end) summary)
          (cond
           ((not (romacs-agent-tools--wakes-p child end summary))
            ;; Done with on its own: an earlier end still waiting stays wanted.
            (romacs-agent-tools--mark-answered child (car end))
            (puthash (car end) t (or answered (setq answered (make-hash-table)))))
           ;; A coordinator whose own tasks still work: its turn after
           ;; theirs ends its task; held till then, should none wake it.
           ((and (not (nth 2 end)) (romacs-agent-tools--delegates-working-p summary))
            (romacs-agent-mark (romacs-agent-id child) 'owed t)
            (puthash child end romacs-agent-tools--held))
           (t
            ;; Owed to its parent till delivered: a restart meanwhile
            ;; loads it (its mark), and hands it on again.
            (romacs-agent-mark (romacs-agent-id child) 'owed t)
            ;; Newest first: an end is added in constant time.
            (push (cons child end) (gethash (romacs-agent-id parent) romacs-agent-tools--pending))))))
      ;; They go at once, in one walk of the new ends alone.
      (when answered (romacs-agent-tools--drop-answered child handed answered))
      (romacs-agent-tools--wake-soon parent)
      ;; CHILD's end may have been its coordinator's last task's.
      (romacs-agent-tools--release-held parent))))

(defvar romacs-agent-tools--held (make-hash-table :test #'eq :weakness 'key)
  "A coordinator to its end held while the tasks it delegated worked
\(`romacs-agent-tools--hand-on').  A restart loses none: the end stays
unanswered and its `owed' mark loads it, handed on anew.")

(defun romacs-agent-tools--release-held (coordinator)
  "Hand on COORDINATOR's held end to its parent once its tasks are done
and none of their ends woke it: it runs no turn after them, so its held
end is its task's."
  (when-let* ((end (gethash coordinator romacs-agent-tools--held))
              ((not (gethash (romacs-agent-id coordinator) romacs-agent-tools--pending)))
              ((not (memq (romacs-agent-state coordinator) '(starting running waiting-approval))))
              ((not (romacs-agent-tools--delegates-working-p (romacs-agent-tools--summary-of coordinator))))
              (parent (romacs-agent-load-session (plist-get (romacs-agent-options coordinator) :parent))))
    (remhash coordinator romacs-agent-tools--held)
    (push (cons coordinator end) (gethash (romacs-agent-id parent) romacs-agent-tools--pending))
    (romacs-agent-tools--wake-soon parent)))

(defvar romacs-agent-tools--waking (make-hash-table :test #'equal)
  "Parents whose wake is scheduled.")

(defun romacs-agent-tools--wake-soon (parent)
  "Wake PARENT a moment from now with what waits for it: ends that come
meanwhile go with it.  A settled parent's ends stay pending."
  (let ((id (romacs-agent-id parent)))
    (when (and (romacs-agent--goes-on-p parent) (not (gethash id romacs-agent-tools--waking)))
      (puthash id t romacs-agent-tools--waking)
      (run-at-time 0.5 nil (lambda ()
                             (remhash id romacs-agent-tools--waking)
                             (when (and (eq (gethash id romacs-agent--sessions) parent)
                                        (romacs-agent--goes-on-p parent))
                               (romacs-agent-tools--flush parent)))))))

(defun romacs-agent-tools--summary-of (session)
  "The summary of SESSION, by its id (`romacs-agent-summary-held'): what
a status read of another session reads of it."
  (romacs-agent-summary-held session))

(defun romacs-agent-tools--at-work-p (summary)
  "Whether the task SUMMARY says still runs or waits on its user: an
\"all\" task that does holds its parent's wake."
  (member (romacs-agent-tools--status summary) '("running" "waiting")))

(defun romacs-agent-tools--all-pending-p (parent)
  "Whether a task PARENT delegated with the \"all\" rule is at work, or
has ended without its end handed on yet."
  (cl-some (lambda (child)
             (let ((summary (romacs-agent-tools--summary-of child)))
               (and (equal (romacs-agent-summary-wake-rule summary) "all")
                    (or (romacs-agent-tools--at-work-p summary)
                        (romacs-agent-tools--unhanded child)
                        (gethash (romacs-agent-id child) romacs-agent-tools--scheduled)))))
           (romacs-agent-children parent)))

(defun romacs-agent-tools--flush (parent)
  "Wake PARENT once with the ends waiting for it, unless it is at work: its
turn's end wakes it then.  Ends of \"all\" tasks wait for the last of them."
  (unless (memq (romacs-agent-summary-state (romacs-agent-tools--summary-of parent))
                '(starting running waiting-approval))
    (let* ((hold (romacs-agent-tools--all-pending-p parent))
           (copies (gethash (romacs-agent-id parent) romacs-agent-tools--copies))
           ;; Each child's summary, read once this pass.
           (summaries (make-hash-table :test #'eq))
           (summary (lambda (child) (with-memoization (gethash child summaries)
                                      (romacs-agent-tools--summary-of child))))
           (held nil) (ready nil))
      ;; One pass, oldest first: the ends still wanted, held or ready.
      (dolist (item (reverse (gethash (romacs-agent-id parent) romacs-agent-tools--pending)))
        (when (romacs-agent-tools--wanted-p (car item) (cadr item) (funcall summary (car item)))
          (if (and hold (equal (romacs-agent-summary-wake-rule (funcall summary (car item))) "all"))
              (push item held)
            (push item ready))))
      (setq ready (nreverse ready))
      (puthash (romacs-agent-id parent) held romacs-agent-tools--pending)
      (when ready
        (romacs-agent-tools--deliver
         parent
         (mapconcat #'identity
                    (append (mapcar (lambda (item) (romacs-agent-tools--wake-text (funcall summary (car item)) (cdr item)))
                                    ready)
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
         (append (mapcar (lambda (item) (romacs-agent-tools--wake-message (funcall summary (car item)) (cdr item)))
                         ready)
                 (mapcar (lambda (copy) (romacs-agent-tools--copy-message (car copy) (cdr copy))) copies))
         ;; A turn of its own, after the one running.
         'next)))))

(defun romacs-agent-tools--deliver (session text delivered &optional valid messages kind)
  "Give SESSION's harness TEXT as its next message, then call DELIVERED;
KIND as `romacs-agent-item' says (`now' by default: into a turn running).
A per-turn harness takes it as its next turn (queued behind a running
one); another once its supervisor is attached.  VALID, when given, is
asked first: nil drops the message.  MESSAGES say whose it is
\(`romacs-agent--input')."
  (if (romacs-agent--folding-p session)
      ;; Whether it runs is known once a worker has folded it.
      (romacs-agent--when-folded session (lambda () (romacs-agent-tools--deliver session text delivered valid messages kind)))
    ;; A parent whose harness was stopped or ended is resumed to take it.
    ;; A resume that fails leaves it admitted all the same: it waits in the
    ;; queue for the parent's next run, never dropped with the resume.
    (unless (or (process-live-p (romacs-agent-connection session)) (eq (romacs-agent-state session) 'starting))
      (condition-case err (romacs-agent--continue session)
        (error (romacs-agent--raise session (list :type 'error :text (error-message-string err))))))
    ;; No sender keeps it: kept for the resume, should it be refused.
    (romacs-agent--admit session (romacs-agent-item text :messages messages :delivered delivered :valid valid
                                                    :kind (or kind 'now))
                         nil t)))

(defvar romacs-agent-tools--scheduled (make-hash-table :test #'equal)
  "Children whose wake is scheduled.")

(defun romacs-agent-tools--forget-ends (child)
  "Forget what was delivered or answered of CHILD's ends, and its ends
waiting for its parent: a session with no journal has had no end but its
launches' failures, each numbered as the first, so the next one wakes
its parent again."
  (delete-file (romacs-agent-tools--woken-file child))
  (remhash child romacs-agent-tools--woken-seqs)
  (romacs-agent-mark (romacs-agent-id child) 'owed nil)
  (delete-file (expand-file-name "answered" (romacs-agent-dir child)))
  (remhash (romacs-agent-id child) romacs-agent-tools--answered)
  (remhash child romacs-agent-tools--handed)
  (remhash child romacs-agent-tools--pruned)
  (when-let* ((parent (plist-get (romacs-agent-options child) :parent)))
    (puthash parent (cl-remove child (gethash parent romacs-agent-tools--pending) :key #'car)
             romacs-agent-tools--pending)))

(defun romacs-agent-tools--on-event (session event)
  ;; A launch failed: its end is new, whatever an earlier failure's was.
  (when (eq (plist-get event :type) 'unstarted)
    (romacs-agent-tools--forget-ends session))
  ;; A parent finished its turn or was brought back: its waiting ends can go.
  (when (and (memq (plist-get event :type) '(done settled))
             (gethash (romacs-agent-id session) romacs-agent-tools--pending))
    (romacs-agent-tools--wake-soon session))
  (when (and (plist-get (romacs-agent-options session) :parent)
             (or (memq (plist-get event :type) '(done unstarted))
                 (and (eq (plist-get event :type) 'exit) (romacs-agent-failed session)
                      (= (romacs-agent-done-seq session) (or (plist-get event :seq) -1)))
                 ;; At its limit, its attachment gone or back: ends held
                 ;; while it replayed (`romacs-agent-tools--wake-held-p') go now.
                 (and (eq (plist-get event :type) 'status) (romacs-agent-limited session)
                      (romacs-agent-tools--unhanded session))))
    ;; A moment later: a `wait' answering with this end (its poll runs
    ;; every 0.05 s) marks it answered first; an abandoned one does not.
    ;; One timer per child: the ends that come meanwhile go with it.
    (unless (gethash (romacs-agent-id session) romacs-agent-tools--scheduled)
      (puthash (romacs-agent-id session) t romacs-agent-tools--scheduled)
      (run-at-time 0.5 nil (lambda ()
                             (remhash (romacs-agent-id session) romacs-agent-tools--scheduled)
                             (romacs-agent-tools--wake session))))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--on-event)

(defun romacs-agent-tools--delegate-gone (session event)
  "SESSION, a delegated task, stopped, ended or was cancelled (EVENT):
its parent's end held while it worked may go now
\(`romacs-agent-tools--release-held')."
  (when-let* (((or (eq (plist-get event :type) 'exit)
                   (and (eq (plist-get event :type) 'status)
                        (member (plist-get event :text) '("stopped" "cancelled")))))
              (parent-id (plist-get (romacs-agent-options session) :parent))
              (parent (romacs-agent-loaded parent-id))
              ((gethash parent romacs-agent-tools--held)))
    (romacs-agent-tools--release-held parent)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--delegate-gone)

(defconst romacs-agent-tools--streaming-events
  '(delta thinking tool tool-output record-state usage cache rate-limit message-end handshake)
  "The events of a turn under way, which never end a task's work.")

(defun romacs-agent-tools--released (session event)
  "Wake SESSION's parent soon when SESSION, delegated with the \"all\" rule,
is no longer at work after EVENT (`romacs-agent-tools--at-work-p': a plan
offer withdrawn or settled, an ask gone, its run ended) while ends wait
for that parent: the ends it held may go now."
  (when-let* (((not (memq (plist-get event :type) romacs-agent-tools--streaming-events)))
              (parent (plist-get (romacs-agent-options session) :parent))
              ((gethash parent romacs-agent-tools--pending))
              (summary (romacs-agent-tools--summary-of session))
              ((equal (romacs-agent-summary-wake-rule summary) "all"))
              ((not (romacs-agent-tools--at-work-p summary)))
              (waiting (romacs-agent-loaded parent)))
    (romacs-agent-tools--wake-soon waiting)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--released)

(defun romacs-agent-tools--say-unsure (session event)
  "Tell SESSION's parent, when it has one, EVENT's error saying messages
kept for SESSION may not have reached it (`romacs-agent-store--sent-of'):
a turn of its own, nothing sent again."
  (when-let* (((eq (plist-get event :type) 'error)) ((plist-get event :unsure))
              (parent (romacs-agent-load-session (plist-get (romacs-agent-options session) :parent))))
    (romacs-agent-tools--deliver parent (plist-get event :text) #'ignore nil nil 'next)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--say-unsure)

(defun romacs-agent-tools--wake-missed ()
  "After a restart: wake the parents of children that ended meanwhile."
  (dolist (session (romacs-agent-sessions))
    (when (plist-get (romacs-agent-options session) :parent) (romacs-agent-tools--wake session))))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--wake-missed)

;;; A task's question to its parent (T3 Code's delegated question)
;;
;; A request only a user answers (a question, a plan to proceed with,
;; a call under a policy that asks) blocks a delegated task.  Its parent
;; is told, a turn of its own, and may answer it (romacs_task_answer) as
;; its user would; the user may still answer in the task's chat.

(defun romacs-agent-tools--question-text (child event)
  "What CHILD's request EVENT asks, as its parent is told it."
  (let ((questions (plist-get event :questions)))
    (concat (format "Delegated task %s (%s) waits on an answer, request %s:\n"
                    (romacs-agent-title child) (romacs-agent-id child) (plist-get event :request-id))
            (cond (questions
                   (mapconcat (lambda (q)
                                (concat "- " (car q)
                                        (when-let* ((options (plist-get (cdr q) :options)))
                                          (concat " [" (string-join options " | ") "]"))))
                              questions "\n"))
                  ((plist-get event :plan) "It presents its plan: proceed?")
                  (t (format "It asks to run %s." (plist-get event :tool))))
            "\nAnswer it with romacs_task_answer, or leave it to the user in its chat.")))

(defvar romacs-agent-tools--asked-parents (make-hash-table :test #'equal)
  "(TASK-ID . REQUEST-ID) its parent was told of this process: once.")

(defun romacs-agent-tools--ask-parent (session event)
  "Tell SESSION's parent the request EVENT, live, that waits on SESSION's
user, once, unless SESSION's rule wakes no one or the parent is settled
\(the user sees it in SESSION's chat): dropped once it is answered.  A
request answered or withdrawn is forgotten here too."
  (when (memq (plist-get event :type) '(answered approval-withdrawn))
    (remhash (cons (romacs-agent-id session) (plist-get event :request-id)) romacs-agent-tools--asked-parents))
  (when-let* (((eq (plist-get event :type) 'approval))
              (parent-id (plist-get (romacs-agent-options session) :parent))
              ((romacs-agent--live-p session))
              (id (plist-get event :request-id))
              (key (cons (romacs-agent-id session) id))
              ((not (gethash key romacs-agent-tools--asked-parents)))
              ((romacs-agent--user-answers-p session id))
              ((not (equal (romacs-agent-tools--wake-rule session) "none")))
              (parent (romacs-agent-load-session parent-id))
              ((romacs-agent--goes-on-p parent)))
    (puthash key t romacs-agent-tools--asked-parents)
    (romacs-agent-tools--deliver parent (romacs-agent-tools--question-text session event) #'ignore
                                 (lambda () (and (gethash id (romacs-agent-approvals session)) t))
                                 nil 'next)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--ask-parent)

(defun romacs-agent-tools--ask-parent-pending (session)
  "SESSION caught up with its records: its parent is told the requests
still waiting on its user that came while it replayed."
  (dolist (id (romacs-agent-asks session))
    (when-let* ((wire (gethash id (romacs-agent-approvals session))))
      (romacs-agent-tools--ask-parent
       session (append (list :type 'approval :request-id id) (romacs-agent-request-event session id wire))))))

(add-hook 'romacs-agent-caught-up-functions #'romacs-agent-tools--ask-parent-pending)

(defun romacs-agent-tools--answer-request (arguments)
  "Answer the request of a task the caller delegated, as its user would:
allow (with answers to its questions) or deny with a message."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (child (romacs-agent-tools--own (romacs-agent-tools--known (gethash "taskId" arguments)) caller "answer it"))
         (answers (gethash "answers" arguments))
         (allow (not (eq (gethash "allow" arguments) :false))))
    (romacs-agent-respond child (gethash "requestId" arguments)
                          (if (and allow (hash-table-p answers) (> (hash-table-count answers) 0))
                              (list :answers (let ((alist nil))
                                               (maphash (lambda (question answer) (push (cons question answer) alist))
                                                        answers)
                                               (nreverse alist)))
                            allow)
                          (gethash "message" arguments)
                          (list :kind 'user :login (user-login-name) :client (format "task %s" caller)))
    (romacs-agent-tools--ack child)))

;;; Reaping finished tasks
;;
;; A delegated task whose turn has ended keeps no processes it does not
;; use: after `romacs-agent-reap-after' idle its harness is stopped to
;; sleep (`romacs-agent-stop' REAPED), and it reads idle; its next
;; message, a romacs_task_send or what a watch tells it, resumes its
;; conversation.  One its harness still works for (a shell it started
;; runs on, a build in the background) is kept.  A user's own session,
;; which no session delegated, is never reaped.  One timer looks at every
;; task, and only while a task's harness runs: never a timer a task.

(defcustom romacs-agent-reap-after 600
  "Seconds a delegated task may sit idle after a turn before its harness
is stopped to sleep, or nil to keep every task's harness.  Its
conversation stays, and its next message resumes it."
  :type '(choice (const :tag "Never" nil) number)
  :group 'romacs-agent)

(defconst romacs-agent-tools--shells '("bash" "sh" "zsh" "dash" "fish" "ksh")
  "What a harness runs its commands in: one running means work goes on.")

(defun romacs-agent-tools--reap-delay (session)
  "Seconds SESSION may sit idle, or nil to keep it: only a delegated task
is reaped, never a user's own session."
  (and (plist-get (romacs-agent-options session) :parent) romacs-agent-reap-after))

(defun romacs-agent-tools--idle-p (session)
  "Whether SESSION's harness runs and has nothing to do: no turn, nothing
waiting to be sent or written and not yet journaled back, no question
unanswered."
  (and (eq (romacs-agent-state session) 'idle)
       (not (romacs-agent-stopped session))
       (null (romacs-agent-queue session))
       (<= (romacs-agent-requests session) (romacs-agent-inputs session))
       (zerop (hash-table-count (romacs-agent-approvals session)))
       (or (process-live-p (romacs-agent-connection session)) (romacs-agent-unit session))))

(defun romacs-agent-tools--processes (session pid)
  "The pids SESSION's supervisor PID runs beside: its unit's (its cgroup)
when it runs as its own unit, else PID's descendants; a supervisor run
detached shares the cgroup it was started in with others."
  (let* ((unit (romacs-agent-unit session))
         (line (and unit (ignore-errors (with-temp-buffer (insert-file-contents (format "/proc/%d/cgroup" pid))
                                                          (buffer-string)))))
         (group (and line (string-match (format "^0::\\(.*/%s\\.service\\)$" (regexp-quote unit)) line)
                     (concat "/sys/fs/cgroup" (match-string 1 line)))))
    (if group
        (mapcan (lambda (procs) (mapcar #'string-to-number (split-string (with-temp-buffer (insert-file-contents procs)
                                                                                           (buffer-string)))))
                (directory-files-recursively group "\\`cgroup\\.procs\\'"))
      (let ((children (make-hash-table)) (found (list pid)) (todo (list pid)))
        (dolist (p (list-system-processes))
          (when-let* ((parent (alist-get 'ppid (process-attributes p)))) (push p (gethash parent children))))
        (while todo
          (dolist (child (gethash (pop todo) children)) (push child found) (push child todo)))
        found))))

(defun romacs-agent-tools--working-p (session)
  "Whether SESSION's harness still works for something after its turn: a
shell it started runs among its processes."
  (when-let* ((pid (romacs-agent--host-pid session)))
    (cl-some (lambda (p) (member (alist-get 'comm (process-attributes p)) romacs-agent-tools--shells))
             (romacs-agent-tools--processes session pid))))

(defconst romacs-agent-tools--reap-every 60
  "Seconds between the reaper's looks, while a task's harness runs.")

(defvar romacs-agent-tools--reaper nil
  "The reaper, one timer for every task: it runs while some task's harness
does (`romacs-agent-tools--reap-sweep'), and none runs at rest.")

(defun romacs-agent-tools--reap-watched-p (session)
  "Whether SESSION is a task the reaper watches: delegated, its harness's
supervisor attached, not cancelled nor already stopped.  A unit's name
kept by an ended or detached session is no harness running."
  (and (romacs-agent-tools--reap-delay session)
       (not (romacs-agent-cancelled session))
       (not (romacs-agent-stopped session))
       (process-live-p (romacs-agent-connection session))))

(defun romacs-agent-tools--reap-due-p (session)
  "Whether SESSION's harness is to stop now: its replay over (what it does
is known), nothing to do, idle its time since its last record, and no
shell of its at work."
  (and (or (romacs-agent--per-turn-p session) (romacs-agent-caught-up session)
           (not (process-live-p (romacs-agent-connection session))))
       (romacs-agent-tools--idle-p session)
       (>= (- (float-time) (or (romacs-agent-last-time session) 0)) (romacs-agent-tools--reap-delay session))
       (not (romacs-agent-tools--working-p session))))

(defun romacs-agent-tools--quiet-due (id)
  "Whether the quiet session ID (unloaded, its supervisor running:
`romacs-agent--quiet') is a task idle its time by its row: loaded then,
so the reaper sees it attached.  Nil for one to look at again later, and
`never' for one no reaper stops (a user's own session)."
  (let* ((row (romacs-agent-store-row-session id))
         (delay (and row (romacs-agent-tools--reap-delay row))))
    (cond ((not delay) 'never)
          ((>= (- (float-time) (or (romacs-agent-last-time row) 0)) delay) t))))

(defun romacs-agent-tools--reap-sweep ()
  "Stop to sleep the harness of each watched task that is due; with no
other task's harness left running, stop looking.  A quiet task (a
restart left it unloaded, its supervisor running) idle its time is
loaded, attached, and stopped by a look after."
  (let ((due nil) (watched 0))
    (maphash (lambda (_id session)
               (when (romacs-agent-tools--reap-watched-p session)
                 (if (romacs-agent-tools--reap-due-p session) (push session due) (cl-incf watched))))
             romacs-agent--sessions)
    (dolist (id (hash-table-keys romacs-agent--quiet))
      (pcase (ignore-error romacs-agent-store-unread (romacs-agent-tools--quiet-due id))
        ('never nil)
        ('nil (cl-incf watched))
        (_ (when (romacs-agent-load-session id) (cl-incf watched)))))
    (dolist (session due) (romacs-agent-stop session t))
    (when (and (zerop watched) (timerp romacs-agent-tools--reaper))
      (cancel-timer romacs-agent-tools--reaper)
      (setq romacs-agent-tools--reaper nil))))

(defun romacs-agent-tools--reap-watch ()
  "Run the reaper, unless it runs."
  (unless (timerp romacs-agent-tools--reaper)
    (setq romacs-agent-tools--reaper
          (run-at-time romacs-agent-tools--reap-every romacs-agent-tools--reap-every
                       #'romacs-agent-tools--reap-sweep))))

(defun romacs-agent-tools--reap-on-event (session event)
  "A task's turn ended: it may be done, so the reaper runs."
  (when (and (eq (plist-get event :type) 'done) (romacs-agent-tools--reap-delay session))
    (romacs-agent-tools--reap-watch)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--reap-on-event)
;; After a restart, tasks may be idle already: its first look finds them.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--reap-watch)

(defun romacs-agent-tools--wait (child timeout-ms arrived)
  "A deferred answer: CHILD's task record (`romacs-agent-tools--ack') once
its turn ends, asks for an approval, or TIMEOUT-MS passes (the child goes
on; its end then wakes the parent).  An end its parent has not had yet
is this answer's to give, in place of its wake: its result goes with
the record, cut as a wake's.  The wait ends within the session relay's
reply timeout, counted from the request's ARRIVED time, which would
otherwise withdraw the call (and retire this wait) and drop its answer."
  (let ((deadline (min (+ (float-time) (/ (or timeout-ms 600000) 1000.0))
                       (+ arrived (max 1 (- romacs-agent-reply-timeout 5))))))
    (romacs-mcp-defer
     (lambda ()
       (let* ((summary (romacs-agent-tools--summary-of child))
              (status (romacs-agent-tools--status summary)) (seq (romacs-agent-summary-done-seq summary)))
         (cond ((member status '("completed" "failed"))
                (let ((new (romacs-agent-tools--wanted-p child seq)))
                  (when new (romacs-agent-tools--answer child seq))
                  (list (romacs-agent-tools--ack
                         child (and new (romacs-agent-tools--wake-result
                                         (romacs-agent-id child) (or (romacs-agent-summary-reply summary) "")))))))
               ((member status '("cancelled" "waiting")) (list (romacs-agent-tools--ack child)))
               ((> (float-time) deadline)
                (let ((ack (romacs-agent-tools--ack child))) (puthash "waitTimedOut" t ack) (list ack))))))
     deadline)))

(defun romacs-agent-tools--wake-waits (_session _event)
  "A wait answers when its child's status changes: every session event
asks the woken deferred requests again (`romacs-mcp-defer')."
  (romacs-mcp-deferred-wake))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--wake-waits)

(defun romacs-agent-tools--delegated (child arguments arrived prompted)
  "What romacs_delegate_task ARGUMENTS, arrived at ARRIVED, answer with
its task CHILD: its record once started (PROMPTED as
`romacs-agent-tools--started' says), or its end for mode wait.  One
cancelled, taken over again, is sent nothing: its record at once, its
note saying so."
  (if (romacs-agent-summary-cancelled (romacs-agent-tools--summary-of child))
      (let ((ack (romacs-agent-tools--ack child)))
        (puthash "note" (format "Task %s is cancelled: nothing was sent to it." (romacs-agent-id child)) ack)
        ack)
    (romacs-agent-tools--started
     child (lambda ()
             (if (equal (gethash "mode" arguments) "wait")
                 (romacs-agent-tools--wait child (gethash "timeoutMs" arguments) arrived)
               (romacs-agent-tools--ack child)))
     prompted)))

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
         (delegating (and parent (romacs-agent-summary parent)))
         (request (gethash "clientRequestId" arguments))
         (target (gethash "target" arguments (romacs-mcp--object)))
         (options (gethash "options" target (romacs-mcp--object)))
         ;; New work draws on its delegating session's harness's pool; a
         ;; conversation resumed is its own account's.
         (provider (if (gethash "provider" target) (romacs-agent-pool-name (gethash "provider" target))
                     (and delegating (romacs-agent-summary-harness delegating))))
         (pinned (or (gethash "provider" target) (and delegating (romacs-agent-summary-provider delegating))))
         ;; Refused before anything starts, or any worktree is made.
         (wake (let ((wake (gethash "wake" arguments)))
                 (if (member wake romacs-agent-tools--wake-rules) wake
                   (and wake (user-error "wake must be one of %s"
                                         (string-join romacs-agent-tools--wake-rules ", "))))))
         (cwd (or (gethash "cwd" arguments)
                  (and delegating (romacs-agent-summary-cwd delegating))
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
         (answer (lambda (child &optional prompted)
                   (romacs-agent-tools--delegated child arguments arrived prompted)))
         (known (and request (romacs-agent-requested parent request)))
         ;; A brief is sent: its answer waits for the task to have it.
         (briefed (let ((task (gethash "task" arguments))) (and (stringp task) (not (string-empty-p task))))))
    (cond (known (funcall answer known))
          ((gethash "resume" arguments)
           (funcall answer (romacs-agent-tools--adopt arguments pinned start-options)))
          ((equal (gethash "isolation" arguments) "worktree")
           ;; A child that cannot start leaves no worktree behind.
           (romacs-agent--start-instance provider)
           (romacs-agent-tools--isolated (romacs-agent-tools--isolating (cons parent request) cwd start)
                                         (lambda (child) (funcall answer child briefed))))
          (t (funcall answer (funcall start cwd) briefed)))))

(defun romacs-agent-tools--cannot-receive (id)
  "Why session ID cannot go on with a message as it is set up now, or nil
when it can: its own check (`romacs-agent--check-continuation'), asked of
it by id, as what it looks at (its supervisor's socket, its directories)
is no fact its summary keeps."
  (condition-case err
      (progn (romacs-agent--check-continuation (or (romacs-agent-get id) (user-error "No session %s" id)))
             nil)
    (user-error (error-message-string err))))

(defun romacs-agent-tools--not-receiving (summary)
  "Why the session SUMMARY says (`romacs-agent-summary') cannot take a
queued message on its current setup, or nil when it can: cancelled; one
starting can (no continuation is checked then, as
`romacs-agent--check-continuation' says); else as its owner says
\(`romacs-agent-tools--cannot-receive')."
  (cond ((romacs-agent-summary-cancelled summary) "it was cancelled")
        ((eq (romacs-agent-summary-state summary) 'starting) nil)
        (t (romacs-agent-tools--cannot-receive (romacs-agent-summary-id summary)))))

(defun romacs-agent-tools--reply-route (sender target)
  "Give SENDER, the calling session, a reply route to the session the
summary TARGET says when a message it received from it permits it."
  (when-let* ((message (and sender target
                            (cl-find (romacs-agent-summary-id target) (romacs-agent-received sender) :test #'equal
                                     :key (lambda (item) (plist-get (plist-get item :from) :session)))))
              ((not (romacs-agent-tools--not-receiving target))))
    (format "Use romacs_task_reply with messageId %s." (plist-get message :id))))

(defun romacs-agent-tools--owner-route (parent caller)
  "Tell CALLER how to ask the task owner PARENT, or the user: the owner,
asked for by id (`romacs-agent-tools--ask-owner'), and the caller as a
sender, read from their summaries.  When the owner cannot take a
message, why."
  (let* ((owner (and (romacs-agent-tools--ask-owner parent) (romacs-agent-summary parent)))
         (sender (and caller (romacs-agent-summary caller)))
         (why (and owner (eq (romacs-agent-tools--send-route owner caller sender) 'send)
                   (or (romacs-agent-tools--not-receiving owner) 'none))))
    (cond ((eq why 'none) "Please ask that session with romacs_task_send.")
          ;; Its own messages received: the caller's record.
          ((romacs-agent-tools--reply-route (and caller (romacs-agent-get caller)) owner))
          (why (format "It cannot take a message now (%s). Please ask the user to arrange it."
                       (string-trim-right why "[ \t\n.]+")))
          (t "Please ask the user to arrange it."))))

(defun romacs-agent-tools--own (session caller what)
  "SESSION, when CALLER delegated it (a caller that is no agent session
owns the sessions none delegated); else a `user-error' saying who may do WHAT."
  (let ((parent (plist-get (romacs-agent-options session) :parent)))
    (unless (equal parent caller)
      (if parent
          (user-error "Task %s belongs to session %s, and among agents only the session that delegated it may %s. %s"
                      (romacs-agent-id session) parent what (romacs-agent-tools--owner-route parent caller))
        (user-error "Session %s is a root session. Ask the user to %s."
                    (romacs-agent-id session) what)))
    session))

(defun romacs-agent-tools--check-message-target (session)
  "Refuse to send to a cancelled SESSION."
  (when (romacs-agent-cancelled session)
    (user-error "Task %s (%s) was cancelled" (romacs-agent-id session) (romacs-agent-title session))))

(defun romacs-agent-tools--tell (child text &optional mode valid messages)
  "Send TEXT to CHILD as its next message by the session's own send:
now when it is idle, once its running turn ends, once its harness is
back when it had ended.  MODE `steer' writes it into the running turn,
as the chat's steer does (a harness run per turn takes it after);
`interrupt' interrupts the turn and sends it first.  VALID, when given,
is asked before a waiting message goes: nil drops it.  MESSAGES, when
given, say it comes from other sessions (`romacs-agent--input').  Its
next turn end wakes its parent by its wake rule, as its first did.  An
empty TEXT sends nothing.  Return how it went (`romacs-agent-tools--delivery'),
`queued' for an interrupt the turn could not take yet; nil for nothing sent."
  (romacs-agent-tools--check-message-target child)
  (unless (string-empty-p (string-trim text))
    (pcase (romacs-agent-tools--delivery child mode)
      ('steered
       ;; Into the turn only when it was written there now (a Codex turn
       ;; named, no switch pending); else it waits, as queued.
       (let ((item (romacs-agent-send child text nil valid nil messages)))
         (if (or (not (memq item (romacs-agent-queue child)))
                 (memq (romacs-agent-item-held item) '(sending written)))
             'steered 'queued)))
      ('interrupting
       ;; First in line before the cut: a turn not yet under way
       ;; (Codex's before it names it) cannot be cut, and it goes after.
       (romacs-agent-send-later child text t nil valid messages)
       (condition-case nil (progn (romacs-agent-interrupt child) 'interrupting)
         (user-error 'queued)))
      (delivery
       ;; One starting is going on already: attaching it again would take its
       ;; supervisor from the attachment under way.
       (unless (eq (romacs-agent-state child) 'starting) (romacs-agent--continue child))
       ;; It waits for the turn running: that turn's result is made before it.
       (when (eq delivery 'queued) (romacs-agent-issues-told child))
       (romacs-agent-send-later child text nil nil valid messages)
       delivery))))

(defun romacs-agent-tools--delivery (child mode)
  "How a message by MODE reaches CHILD as it is now: `steered' into its
running turn, `interrupting' that turn, `queued' behind it, or `sent' as
its next turn at once.  A steer a turn cannot take (none runs, or its
harness runs a process a turn) is queued or sent."
  (let ((busy (memq (romacs-agent-state child) '(running waiting-approval))))
    (cond ((and busy (eq mode 'steer) (not (romacs-agent--per-turn-p child))) 'steered)
          ((and busy (eq mode 'interrupt)) 'interrupting)
          ;; Starting: it waits for the harness, as behind a turn.
          ((or busy (eq (romacs-agent-state child) 'starting)) 'queued)
          (t 'sent))))

(defconst romacs-agent-tools--send-modes '("queue" "replace" "interrupt")
  "How romacs_task_send delivers, as its schema names them: after the running
turn (into it with steer), in place of what the sender has waiting, or
cutting the turn short.  `steer' is taken as a mode too, as it was one.")

(defun romacs-agent-tools--sent-by-p (item caller)
  "Whether ITEM, waiting, is CALLER's own messages (a session id, or nil
for outside Romacs) and nothing else: no task's result, no other sender's."
  (let ((messages (romacs-agent-item-messages item)))
    (and messages
         (cl-every (lambda (message)
                     (and (not (plist-get message :wake))
                          (equal (plist-get (plist-get message :from) :session) caller)))
                   messages))))

(defvar romacs-agent-tools--replacing nil
  "The session a `replace' admits into, its messages held meanwhile.")

(defun romacs-agent-tools--replacing-p (session)
  "Whether a `replace' holds SESSION's messages (`romacs-agent-hold-functions')."
  (eq session romacs-agent-tools--replacing))

(add-hook 'romacs-agent-hold-functions #'romacs-agent-tools--replacing-p)

(defun romacs-agent-tools--waiting-from (target caller)
  "CALLER's messages waiting for TARGET that a `replace' takes the place
of: not one being written already."
  ;; A copy: the queue's tail, shared, grows as the replacement is queued.
  (cl-remove-if-not (lambda (item) (and (romacs-agent-tools--sent-by-p item caller)
                                        (not (memq (romacs-agent-item-held item) '(sending written)))))
                    (copy-sequence (romacs-agent-queue target))))

(defun romacs-agent-tools--send-route (target caller sender)
  "Whether CALLER may send to the session the summary TARGET says, needs a
reply, or needs its owner.  SENDER is the calling session's summary, if
known."
  (let ((parent (romacs-agent-summary-parent target))
        (sender-parent (and sender (romacs-agent-summary-parent sender)))
        (id (romacs-agent-summary-id target)))
    (cond ((and caller (or (null parent) (equal id sender-parent))) 'reply)
          ((or (equal parent caller)
               (and parent sender (not (equal caller id)) (equal parent sender-parent)))
           'send)
          (t 'owner))))

(defvar romacs-agent-tools--copies (make-hash-table :test #'equal)
  "A parent's id to the copies of its tasks' messages to each other, oldest
first, each (SENDER TARGET MODE TEXT): they go with its next wake, never
waking it.")

(defun romacs-agent-tools--copy-body (copy)
  "What a COPY, (TARGET MODE TEXT), says: its task, how, and the message."
  (pcase-let ((`(,target ,mode ,text) copy))
    (format "To task %s (%s), %s:\n%s" (romacs-agent-id target)
            (romacs-agent-summary-title (romacs-agent-tools--summary-of target)) mode text)))

(defun romacs-agent-tools--copy-text (sender copy)
  "SENDER's message COPY, (TARGET MODE TEXT), as a wake its parent reads
\(state \"sent\")."
  (let ((body (romacs-agent-tools--copy-body copy)))
    (format "Delegated task %s (%s) [sent, %d chars]:\n%s"
            (romacs-agent-id sender) (romacs-agent-summary-title (romacs-agent-tools--summary-of sender))
            (length body) body)))

(defun romacs-agent-tools--copy-message (sender copy)
  "The record of SENDER's message COPY, (TARGET MODE TEXT), to its parent:
a sibling's message, sent to TARGET."
  (pcase-let ((`(,target ,_mode ,text) copy))
    (append (romacs-agent-make-message (concat "m-" (romacs-agent--uuid)) "sibling"
                                  (list :session (romacs-agent-id sender)
                                        :name (or (romacs-agent-summary-title (romacs-agent-tools--summary-of sender))
                                                  (romacs-agent-id sender)))
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
its next wake.  Answer TARGET's task record (`romacs-agent-tools--ack')
and the message's id."
  (when (string-empty-p (string-trim (or text ""))) (user-error "message is empty"))
  (unless (member mode (cons "steer" romacs-agent-tools--send-modes))
    (user-error "mode must be one of %s" (string-join romacs-agent-tools--send-modes ", ")))
  (let* ((caller (and sender (romacs-agent-id sender)))
         (kind (romacs-agent-tools--kind sender target))
         (message (romacs-agent-tools--message kind sender in-reply-to by-user))
         (delivery nil))
    (let ((replaced (and (equal mode "replace") (romacs-agent-tools--waiting-from target caller))))
      (if (not replaced)
          (setq delivery (romacs-agent-tools--tell target text (intern mode) nil (list message)))
        ;; Nothing goes meanwhile, so no message it replaces is written
        ;; first; its replacement admitted, they go: now, not at dispatch,
        ;; so each stays all data the store keeps; a refusal keeps them.
        (let ((romacs-agent-tools--replacing target))
          (setq delivery (romacs-agent-tools--tell target text (intern mode) nil (list message)))
          (dolist (item replaced) (romacs-agent--dequeue target item)))
        (condition-case err
            (romacs-agent--dispatch target)
          (error
           (unless (cl-some (lambda (item) (memq message (romacs-agent-item-messages item)))
                            (romacs-agent-queue target))
             (signal (car err) (cdr err)))
           (setq delivery 'queued)
           (romacs-agent--raise target (list :type 'error :text (error-message-string err)))))))
    ;; Text no harness takes is never copied: it would ride every wake after.
    (when (and (equal kind "sibling") (romacs-agent--encodable-p (romacs-agent-item text)))
      (let ((parent (plist-get (romacs-agent-options target) :parent)))
        (puthash parent (append (gethash parent romacs-agent-tools--copies) (list (list sender target mode text)))
                 romacs-agent-tools--copies)))
    (let ((record (romacs-agent-tools--ack target)))
      (puthash "messageId" (plist-get message :id) record)
      ;; What became of it, a steer asked for included: T3 Code's delivery.
      (puthash "delivery" (symbol-name delivery) record)
      record)))

(defun romacs-agent-tools--send (arguments)
  "Send ARGUMENTS' message to task taskId: the caller's own task, or
another task of the caller's parent, its parent then copied."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (task (romacs-agent-tools--known (gethash "taskId" arguments)))
         (sender (and caller (romacs-agent-get caller)))
         (target (romacs-agent-tools--summary-of task)))
    (pcase (romacs-agent-tools--send-route target caller (and sender (romacs-agent-tools--summary-of sender)))
      ('reply (user-error "You cannot send messages to session %s. %s"
                          (romacs-agent-id task)
                          (or (romacs-agent-tools--reply-route sender target) "Please ask the user.")))
      ('owner (romacs-agent-tools--own task caller "send it messages")))
    (romacs-agent-tools--message-to sender task (gethash "message" arguments)
                                    (romacs-agent-tools--send-mode arguments) (gethash "inReplyTo" arguments))))

(defun romacs-agent-tools--reply-ids (prefix received)
  "Return up to eight ids from RECEIVED, putting PREFIX matches first.
Order matches and the remaining ids by most recent first."
  (let ((groups (seq-group-by (lambda (id) (string-prefix-p prefix id))
                              (mapcar (lambda (message) (plist-get message :id)) received))))
    (seq-take (append (alist-get t groups) (alist-get nil groups)) 8)))

(defun romacs-agent-answer (session id text &optional mode by-user)
  "Answer the message ID SESSION received with TEXT, routed to its sender
whatever it is to SESSION, by MODE (`romacs-agent-tools--send-modes',
default queue), linked to ID.  BY-USER: SESSION's user wrote it (its
chat), said so to the sender, and SESSION's journal keeps that it was
answered.  Return the sender's task record with the answer's messageId;
a `user-error' for a message SESSION did not receive, one from outside
Romacs (its own channel's reply line says how), or a sender gone."
  (let* ((message (or (romacs-agent-received-message session id)
                      (if-let* ((received (romacs-agent-received session)))
                          (user-error "No received message %s. Use a full messageId: %s" id
                                      (string-join (romacs-agent-tools--reply-ids id received) ", "))
                        (user-error "No message %s. This session has no received message ids." id))))
         (sender-id (plist-get (plist-get message :from) :session))
         (sender (cond ((null sender-id)
                        ;; Its own channel's, when it came by one: the call to copy.
                        (user-error "Message %s came from outside Romacs. %s" id
                                    (romacs-agent--message-reply-line message)))
                       ;; One a start left a row: loaded, to be answered.
                       ((romacs-agent-get sender-id))
                       (t (user-error "Message %s's sender, session %s, is gone" id sender-id))))
         ;; Its note first able to go: never an answer sent and its note lost.
         (record (progn (when by-user (romacs-agent--check-note session))
                        (romacs-agent-tools--message-to session sender text (or mode "queue") id by-user))))
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
                       (gethash "text" arguments) (romacs-agent-tools--send-mode arguments)))

(defun romacs-agent-tools--send-mode (arguments)
  "The mode ARGUMENTS ask to deliver by: their mode (queue by default), or
steer when their steer is true, which goes with mode queue alone."
  (let ((mode (or (gethash "mode" arguments) "queue")))
    (if (not (eq (gethash "steer" arguments) t)) mode
      (unless (equal mode "queue") (user-error "steer goes with mode queue, not %s" mode))
      "steer")))

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
        (let ((child (romacs-agent-import harness thread (plist-get item :path) (expand-file-name cwd)
                                          (or (gethash "title" arguments) (plist-get item :title) (romacs-agent--short thread))
                                          (plist-get instance :name) config-dir
                                          (romacs-agent--instance-options instance options))))
          (romacs-agent-tools--tell child (gethash "task" arguments))
          child)))))

(defconst romacs-agent-tools--row-result 300
  "The most of a task's result a row of romacs_task_list carries.")

(defun romacs-agent-tools--row (summary)
  "The task record of the session SUMMARY says, as romacs_task_list shows
it: its result's head and length, and no advice."
  (let* ((task (romacs-agent-tools--task summary)) (result (gethash "result" task)))
    (remhash "next" task)
    (when (stringp result)
      (puthash "resultChars" (length result) task)
      (when (> (length result) romacs-agent-tools--row-result)
        (puthash "result" (concat (substring result 0 romacs-agent-tools--row-result) "…") task)))
    task))

(defun romacs-agent-tools--list (arguments)
  "The caller's tasks, or with all every session's, most recently active
first, as rows (`romacs-agent-tools--row'), from their summaries
\(`romacs-agent-summary'): a restarted server lists them all, those left
unloaded as their rows have them."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (all (eq (gethash "all" arguments) t))
         (limit (max 1 (min 200 (or (gethash "limit" arguments) romacs-agent-tools--list-page))))
         (offset (max 0 (or (gethash "offset" arguments) 0)))
         (tasks (cl-remove-if-not (lambda (summary) (romacs-agent-tools--listed-p summary caller all))
                                  (romacs-agent-tools--listing)))
         ;; Stable: those as recent stay in the listing's order.
         (sorted (sort tasks (lambda (a b) (> (or (romacs-agent-summary-last-time a) 0)
                                              (or (romacs-agent-summary-last-time b) 0)))))
         (total (length sorted))
         ;; Rows made for the page alone: a long history stays one answer.
         (page (seq-take (nthcdr offset sorted) limit)))
    (romacs-mcp--object
     "tasks" (vconcat (mapcar #'romacs-agent-tools--row page))
     "total" total
     "nextOffset" (if (< (+ offset limit) total) (+ offset limit) :null))))

(defconst romacs-agent-tools--list-page 50
  "The rows romacs_task_list answers by default: a page, not every task.")

(defun romacs-agent-tools--listing ()
  "Every session's summary in the order the task list takes them before
it sorts by recency: the loaded ones by their directories, newest first,
as `romacs-agent-sessions' orders them, then those known by their rows,
as the store lists them."
  (let ((loaded nil) (rows nil))
    (dolist (summary (romacs-agent-summary nil))
      (if (gethash (romacs-agent-summary-id summary) romacs-agent--sessions) (push summary loaded) (push summary rows)))
    (nconc (sort (nreverse loaded) (lambda (a b) (string> (or (romacs-agent-summary-dir a) "")
                                                          (or (romacs-agent-summary-dir b) ""))))
           (nreverse rows))))

(defun romacs-agent-tools--started (child then &optional prompted)
  "A deferred answer: THEN's, once CHILD's supervisor is attached (or a
per-turn harness, which has none between turns, is created), and, when
PROMPTED (a task started with its brief), once its supervisor has the
brief: an editor stopped right after leaves no task that never had it.
The call's failure when it never started, an end then answered, so it
wakes no one besides."
  (let ((next nil))
    (romacs-mcp-defer
     (lambda ()
       (cond (next (funcall next))
             ((eq (romacs-agent-state child) 'unstarted)
              (romacs-agent-tools--answer child (romacs-agent-done-seq child))
              (user-error "Task %s (%s) did not start: %s" (romacs-agent-id child) (romacs-agent-title child)
                          (romacs-agent-reply child)))
             ;; Its harness gone before it had its brief: it never will.
             ((and prompted (not (romacs-agent--per-turn-p child)) (not (romacs-agent-input-seq child))
                   (eq (romacs-agent-state child) 'ended)
                   (not (process-live-p (romacs-agent-connection child))))
              ;; The caller has the failure: its end wakes no one besides.
              (romacs-agent-tools--answer child (romacs-agent-done-seq child))
              (user-error "Task %s (%s) ended before it had its brief: %s" (romacs-agent-id child)
                          (romacs-agent-title child) (or (romacs-agent-reply child) "its harness exited")))
             ((and (or (process-live-p (romacs-agent-connection child)) (romacs-agent--per-turn-p child)
                       (not (eq (romacs-agent-state child) 'starting)))
                   (or (not prompted) (romacs-agent--per-turn-p child) (romacs-agent-input-seq child)))
              (let ((value (funcall then)))
                (if (eq (car-safe value) 'romacs-mcp--deferred)
                    (progn (romacs-mcp-deferred-retime (nth 2 value)) (funcall (setq next (cadr value))))
                  (list value)))))))))

;;; Going on under another instance

(defcustom romacs-agent-switch-on-limit 'lossless
  "What a session whose turn ends at its account's usage limit does:
`lossless', after `romacs-agent-switch-delay' seconds it moves to the next
account of its harness's pool with room (`romacs-agent-move-on'), its
conversation as is, and goes on there; a delegated task's parent is not
woken for that end.  nil, it stays, and a delegated task's parent is
woken with where it could go on; the chat's actions menu still moves it
by hand.  Each limit moves it once at most, never to an account
at its own limit, and only between turns: one turn's requests are never
split across accounts."
  :type '(choice (const :tag "Move to the next account with room" lossless) (const :tag "Stay" nil))
  :group 'romacs-agent)

(defcustom romacs-agent-switch-delay 30
  "Seconds a session waits at its account's limit before it moves on
\(`romacs-agent-switch-on-limit')."
  :type 'number :group 'romacs-agent)

(defun romacs-agent-tools--spent (session)
  "Journal that SESSION stays at its limit, every account of its pool
spent, and when the first resets."
  (romacs-agent--note session (romacs-mcp--object
                               "type" "handoff" "from" (romacs-agent-provider session) "to" :null
                               "why" (format "every %s account is at its limit" (romacs-agent-harness session))
                               "resets" (or (romacs-agent--earliest-reset (romacs-agent-harness session)) :null))))

(defun romacs-agent-tools--lossless-p (session)
  "Whether SESSION has an account of its own harness with room to move to."
  (and (cl-find-if (lambda (a) (plist-get a :lossless)) (romacs-agent-alternatives session)) t))

(defun romacs-agent-tools--moving-p (session)
  "The turn end SESSION waits at its usage limit from to move on, or nil
\=(`romacs-agent-tools--arm-move')."
  (plist-get (romacs-agent-limited session) :moving))

(defun romacs-agent-tools--arm-move (session)
  "SESSION, at its usage limit, moves on after `romacs-agent-switch-delay':
its `limited' says `:moving' with the turn end it waits from, and while
it does its messages wait (`romacs-agent--open-p') and so do its ends
\=(`romacs-agent-tools--wake')."
  (let ((seq (romacs-agent-done-seq session)))
    (setf (romacs-agent-limited session)
          (append (list :moving seq :moving-since (float-time))
                  (romacs-agent--without (romacs-agent-limited session) '(:moving :moving-since))))
    (run-at-time romacs-agent-switch-delay nil #'romacs-agent-tools--hand-over session seq)))

(defun romacs-agent-tools--at-limit (session)
  "SESSION waits at its usage limit, as `romacs-agent-switch-on-limit'
says: moving on, once (`romacs-agent-tools--arm-move'); with every account
spent, said and resumed at the first reset, unless that is waited for
already: its ends go to its parent."
  (cond ((romacs-agent-tools--moving-p session))
        ((romacs-agent-tools--lossless-p session) (romacs-agent-tools--arm-move session))
        ((not (romacs-agent-resume-time session))
         (romacs-agent-tools--spent session)
         (romacs-agent-tools--schedule-resume session))))

(defun romacs-agent-tools--switch-on-limit (session event)
  "SESSION's turn ended at its account's usage limit (EVENT): it waits
there (`romacs-agent-tools--at-limit')."
  (when (and (plist-get event :limit) (not (romacs-agent-settled-p session))
             ;; A live end: one replayed is seen to as its replay ends.
             (romacs-agent-caught-up session)
             (eq romacs-agent-switch-on-limit 'lossless))
    (romacs-agent-tools--at-limit session)))

(defun romacs-agent-tools--hand-over (session seq)
  "Move SESSION on, still waiting at its limit from turn end SEQ; with a
selection pending, once that is in effect, waiting at most
`romacs-agent-start-deadline' since the wait began; with no account left,
say so: its end then goes to its parent as it would have."
  ;; A session no longer the one loaded (a load replaced it) is not this
  ;; one's to move or wake: the one loaded waits for its own.
  (when (and (eql (romacs-agent-tools--moving-p session) seq)
             (eq (romacs-agent-loaded (romacs-agent-id session)) session))
    (cond ((or (romacs-agent-cancelled session) (romacs-agent-settled-p session))
           (romacs-agent-tools--not-moved session nil))
          ;; One asked meanwhile (romacs_task_switch, or a move a restart
          ;; cut): it decides first, unless it never comes to it.
          ((romacs-agent-pending-selection session)
           (if (< (- (float-time) (or (plist-get (romacs-agent-limited session) :moving-since) 0))
                  romacs-agent-start-deadline)
               (run-at-time romacs-agent-switch-delay nil #'romacs-agent-tools--hand-over session seq)
             (romacs-agent-tools--withdraw-move session)
             (romacs-agent-tools--not-moved session "Its move to another account did not take effect")))
          ((not (romacs-agent-tools--lossless-p session))
           (romacs-agent-tools--spent session)
           (romacs-agent-tools--schedule-resume session)
           (romacs-agent-tools--not-moved session nil))
          (t (condition-case err (romacs-agent-move-on session "usage limit")
               (error (romacs-agent-tools--not-moved session (error-message-string err))))))))

(defun romacs-agent-tools--withdraw-move (session)
  "Take back the selection SESSION waits on: selected as it runs now, as
a user would (`romacs-agent-select'), a stop under way for it ends on the
account it has, so no late commit moves it after its parent was told."
  (let ((now (romacs-agent--selection session)))
    (condition-case err
        (romacs-agent-select session (plist-get now :model) (plist-get now :effort) (plist-get now :provider))
      (error (romacs-agent--raise session (list :type 'error :text (error-message-string err)))))))

(defun romacs-agent-tools--moves-again-p (limited idle asked room)
  "Whether a session its replay leaves at its usage limit waits there
again, a restart having cut that: LIMITED (`romacs-agent-limited'), IDLE,
nothing ASKED of it since, and no ROOM known back on that limit's account
\=(none known is none).  A selection pending is waited out
\=(`romacs-agent-tools--hand-over')."
  (and limited idle (not asked) (not room)))

(defun romacs-agent-tools--room-back-p (provider since)
  "Whether instance PROVIDER is known to have room again since SINCE, its
limit's time: its limit reset, or a report of room newer than that.  A
limit of unknown time (kept by an older Romacs) knows only its reset."
  (when-let* ((room (romacs-agent-room-of provider)) ((romacs-agent-room provider)))
    (let ((resets (plist-get room :resets)) (time (plist-get room :time)))
      (or (and (numberp resets) (> (float-time) resets))
          (and (numberp since) (numberp time) (> time since))))))

(defun romacs-agent-tools--limit-caught-up (session)
  "SESSION's replay is over: still at its usage limit as a restart left it,
it waits there (`romacs-agent-tools--at-limit'); else the ends it held
from its parent while it replayed go on (`romacs-agent-tools--wake')."
  (when (and (romacs-agent--live-p session)
             (eq romacs-agent-switch-on-limit 'lossless) (not (romacs-agent-settled-p session))
             (romacs-agent-tools--moves-again-p
              (romacs-agent-limited session) (eq (romacs-agent-state session) 'idle)
              (> (or (romacs-agent-input-seq session) 0) (romacs-agent-done-seq session))
              (romacs-agent-tools--room-back-p (plist-get (romacs-agent-limited session) :provider)
                                               (plist-get (romacs-agent-limited session) :time))))
    (romacs-agent-tools--at-limit session))
  (when (and (plist-get (romacs-agent-options session) :parent) (romacs-agent-tools--unhanded session))
    (romacs-agent-tools--wake session)))

(add-hook 'romacs-agent-caught-up-functions #'romacs-agent-tools--limit-caught-up)

(defun romacs-agent-tools--limit-loaded (session _manifest)
  "SESSION, loaded at its usage limit with a move to another account still
pending (a restart cut it), waits for it as a live move does: its ends
held, for a bounded time (`romacs-agent-tools--arm-move')."
  (let ((switch (romacs-agent-pending-selection session)))
    (when (and (romacs-agent-limited session) (not (romacs-agent-tools--moving-p session))
               (eq romacs-agent-switch-on-limit 'lossless)
               (plist-get switch :provider) (not (equal (plist-get switch :provider) (romacs-agent-provider session)))
               (not (romacs-agent-settled-p session)) (not (romacs-agent-cancelled session)))
      (romacs-agent-tools--arm-move session))))

(add-hook 'romacs-agent-loaded-functions #'romacs-agent-tools--limit-loaded)

;;; Resuming at a limit's reset

(defcustom romacs-agent-resume-at-reset t
  "Whether a session stopped at its limit, with no account of its pool
left with room, goes on by itself once the first of their limits resets
\(plus `romacs-agent-resume-margin'), on the first account with room."
  :type 'boolean :group 'romacs-agent)

(defcustom romacs-agent-resume-margin 60
  "Seconds after a limit's reported reset a session resumes at."
  :type 'number :group 'romacs-agent)

(defvar romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key)
  "Each session waiting to resume to (TIME . TIMER).")

(defvar romacs-agent-tools--resets-used (make-hash-table :test #'eq :weakness 'key)
  "Each session to the reset its last resume was scheduled for: another
needs a newer one reported, so a limit that stays never schedules again.")

(defun romacs-agent-resume-time (session)
  "When SESSION resumes by itself at its limit's reset, or nil; for a
session made from its row (`romacs-agent-store-row-session'), as the row
says."
  (or (car (gethash session romacs-agent-tools--resumes))
      (plist-get (romacs-agent-options session) :resume-at)))

(defun romacs-agent-tools--keep-resume (session time)
  "Record in SESSION's manifest that it resumes at TIME (nil: no longer):
its own record, written as asked, so a restarted server arms it again."
  ;; Marked before it is kept, unmarked after: a stop between leaves a mark
  ;; too many, never a resume unmarked.
  (when time (romacs-agent-mark (romacs-agent-id session) 'resume t))
  (romacs-agent--manifest-put session "resumeAt" (or time :null))
  (unless time (romacs-agent-mark (romacs-agent-id session) 'resume nil)))

(defun romacs-agent-tools--put-resume (session pending)
  "Make PENDING, (TIME . TIMER) or nil for none, SESSION's resume at its
limit's reset (`romacs-agent-tools--resumes'): its summary's
`romacs-agent-resume-time' follows."
  (if pending (puthash session pending romacs-agent-tools--resumes) (remhash session romacs-agent-tools--resumes))
  (romacs-agent--summary-stale (romacs-agent-id session)))

(defun romacs-agent-tools--arm-resume (session time)
  (unless (romacs-agent-settled-p session)
    (romacs-agent-tools--put-resume
     session (cons time (run-at-time (max 1 (- time (float-time))) nil #'romacs-agent-tools--resume session time)))
    (romacs-agent--changed session)))

(defun romacs-agent-tools--schedule-resume (session)
  "SESSION waits at its limit, every account spent: it resumes by itself
at the earliest reset plus the margin, journaled, unless that is unknown
or `romacs-agent-resume-at-reset' is off."
  (when-let* (((and romacs-agent-resume-at-reset (not (romacs-agent-settled-p session))
                    (not (romacs-agent-resume-time session))))
              (resets (romacs-agent--earliest-reset (romacs-agent-harness session)))
              ((> resets (gethash session romacs-agent-tools--resets-used 0))))
    (puthash session resets romacs-agent-tools--resets-used)
    (let ((time (max (+ resets romacs-agent-resume-margin) (+ (float-time) romacs-agent-switch-delay))))
      (romacs-agent-tools--arm-resume session time)
      (romacs-agent-tools--keep-resume session time)
      (romacs-agent--note session (romacs-mcp--object "type" "resume" "state" "scheduled" "at" time)))))

(defun romacs-agent-tools--cancel-resume (session &optional why)
  "SESSION no longer resumes by itself; WHY, when given, is journaled."
  (when-let* ((pending (gethash session romacs-agent-tools--resumes)))
    (cancel-timer (cdr pending))
    (romacs-agent-tools--put-resume session nil)
    (romacs-agent-tools--keep-resume session nil)
    (when why (romacs-agent--note session (romacs-mcp--object "type" "resume" "state" "cancelled" "why" why)))
    (romacs-agent--changed session)))

(defun romacs-agent-tools--resume (session time)
  "SESSION's resume at TIME is due: still stopped at its limit, it goes on
under the first account of its pool with room, first of what waits
\=(`romacs-agent--limit-continuation'); else nothing.
An account still spent (its report not yet renewed) waits for the next
reset, once."
  (cond
   ((not (equal (romacs-agent-resume-time session) time)))
   ;; Turned off since: forgotten.
   ((or (not romacs-agent-resume-at-reset) (romacs-agent-settled-p session))
    (romacs-agent-tools--put-resume session nil)
    (romacs-agent-tools--keep-resume session nil))
   ;; Its journal still replaying: what it is at is not yet known.
   ((and (process-live-p (romacs-agent-connection session)) (not (romacs-agent-caught-up session)))
    (romacs-agent-tools--arm-resume session time))
   (t
    (romacs-agent-tools--put-resume session nil)
    (romacs-agent-tools--keep-resume session nil)
    (when (and (eq (romacs-agent-loaded (romacs-agent-id session)) session) (not (romacs-agent-cancelled session))
               (not (romacs-agent-stopped session)) (romacs-agent-limited session)
               (<= (or (romacs-agent-input-seq session) 0) (romacs-agent-done-seq session)))
      (let ((target (romacs-agent-pool-pick (romacs-agent-harness session))))
        (if (not (and target (romacs-agent-room (plist-get target :name))))
            (romacs-agent-tools--schedule-resume session)
          (romacs-agent--note session (romacs-mcp--object "type" "resume" "state" "fired" "on" (plist-get target :name)))
          (if (equal (plist-get target :name) (romacs-agent-provider session))
              ;; An ended harness, resumed to take it.
              (romacs-agent--admit (romacs-agent--continue session)
                                   (romacs-agent--limit-continuation (plist-get target :name) (plist-get target :name)
                                                                     (romacs-agent-done-seq session))
                                   t)
            (romacs-agent-move-on session "limit reset"))))))))

(defun romacs-agent-tools--rearm-resume (session manifest)
  "SESSION, loaded by a new server, resumes at the time its MANIFEST
kept, if any; with `romacs-agent-resume-at-reset' off, that is forgotten."
  (when-let* ((time (gethash "resumeAt" manifest)) ((numberp time)))
    (if (and romacs-agent-resume-at-reset (not (romacs-agent-settled-p session)))
        (romacs-agent-tools--arm-resume session time)
      (romacs-agent-tools--keep-resume session nil))))

(add-hook 'romacs-agent-loaded-functions #'romacs-agent-tools--rearm-resume)

(defun romacs-agent-tools--room-back (provider)
  "Instance PROVIDER has room again before the reset sessions wait for:
each waiting to resume on its harness's pool resumes now."
  (let ((harness (plist-get (cl-find provider (romacs-agent-instances) :key (lambda (i) (plist-get i :name))
                                     :test #'equal)
                            :harness))
        (due nil))
    (maphash (lambda (session pending)
               (when (eq (romacs-agent-harness session) harness) (push (cons session (car pending)) due)))
             romacs-agent-tools--resumes)
    (pcase-dolist (`(,session . ,time) due)
      (cancel-timer (cdr (gethash session romacs-agent-tools--resumes)))
      ;; Its reset not reached: a limit there again waits for it again.
      (remhash session romacs-agent-tools--resets-used)
      (condition-case err
          (romacs-agent-tools--resume session time)
        ;; It waits on as it did, its own failure said.
        (error (unless (romacs-agent-settled-p session)
                 (romacs-agent-tools--arm-resume session time)
                 ;; Kept as it can be: the others still go on.
                 (ignore-errors (romacs-agent-tools--keep-resume session time)))
               (message "Resuming %s: %s" (romacs-agent-id session) (error-message-string err)))))))

(add-hook 'romacs-agent-room-functions #'romacs-agent-tools--room-back)

(defun romacs-agent-tools--not-moved (session why)
  "SESSION stays where its limit stopped it, WHY when said: no longer
`:moving', its messages go, and its end goes to its parent."
  (when-let* ((limited (romacs-agent-limited session)))
    (setf (romacs-agent-limited session) (romacs-agent--without limited '(:moving :moving-since))))
  (when why (romacs-agent--raise session (list :type 'status :text why)))
  (romacs-agent-tools--wake session)
  (romacs-agent--dispatch-soon session))

(defun romacs-agent-tools--moved (session from to limited &optional kept)
  "SESSION, at its usage limit LIMITED, was put on another account: moved
off FROM, the ends from the one its move waited from (else its last, a
move a restart applied) are answered by its going on, handed on to no
one, and kept so once its continuation is (KEPT, as
`romacs-agent-moved-functions' says); not (TO is FROM), it stays
\=(`romacs-agent-tools--not-moved')."
  (if (equal to from) (romacs-agent-tools--not-moved session nil)
    ;; Every end from the limit on, not the newest alone: a limit reported
    ;; again (a duplicate completion) ends a turn too, and the turn going on
    ;; answers them all.  Those there now: the going on's own come later.
    (let* ((from-seq (or (plist-get limited :moving) (romacs-agent-done-seq session)))
           (seqs (cl-loop for end in (romacs-agent-ends session) when (>= (car end) from-seq) collect (car end))))
      (dolist (seq seqs) (romacs-agent-tools--answer session seq t))
      ;; Written only once the continuation is kept: a stop before that
      ;; leaves the end to its parent, never neither.
      (funcall (or kept (lambda (then) (funcall then '(ok nil))))
               (lambda (answer)
                 (when (eq (car answer) 'ok)
                   (dolist (seq seqs) (romacs-agent-tools--write-answered session seq))))))))

(add-hook 'romacs-agent-moved-functions #'romacs-agent-tools--moved)

(defun romacs-agent-tools--limit-on-event (session event)
  (pcase (plist-get event :type)
    ('done (romacs-agent-tools--switch-on-limit session event))
    ;; A message after the limit: a resume is not its (none goes while it
    ;; moves).  Its turn may end at the limit again: that end schedules a
    ;; resume anew, the reset not yet used by one that fired.
    ('input (when (and (romacs-agent-caught-up session) (not (romacs-agent-pending-selection session)))
              (when (romacs-agent-resume-time session) (remhash session romacs-agent-tools--resets-used))
              (romacs-agent-tools--cancel-resume session "a message came")))
    ('settled
     (when (romacs-agent-settled-p session)
       (romacs-agent-tools--cancel-resume session)
       (when (romacs-agent-tools--moving-p session)
         (romacs-agent-tools--not-moved session nil))))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-tools--limit-on-event)

(defun romacs-agent-tools--switch (arguments)
  "Go on with the task taskId (the caller's own session by default) under
instance provider (default its own: a model or effort changes in place):
of its harness, the same session and conversation, once its running turn
ends; of another, the same session in a new conversation there, briefed
on this one (`romacs-agent-switch-harness-soon')."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (task (romacs-agent-tools--known
                (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no session"))))
         (id (romacs-agent-id task))
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
          (let ((record (romacs-agent-tools--ack task)))
            (puthash "next" (if (equal id caller)
                                (format "You %s once this turn ends, the same conversation: end your turn now." verb)
                              (format "It %ss once its running turn ends, the same conversation; its next end wakes you as before."
                                      verb))
                     record)
            record))
      ;; Another harness: the same task, in a new conversation there.
      (let* ((when (romacs-agent-switch-harness-soon task (romacs-agent-pool-name provider) model effort next))
             (record (romacs-agent-tools--ack task)))
        (puthash "next" (cond ((equal id caller) "You switch once this turn ends: end your turn now. Your next turn there starts on a brief of this one.")
                              ((eq when 'now) "It switches now: the same task, a new conversation there on a brief of this one; its next end wakes you as before.")
                              (t "It switches once its running turn ends, the same task; its next end wakes you as before."))
                 record)
        record))))

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
                       (cond
                        ((plist-get made :error) made)
                        ;; Its parent cancelled meanwhile: no writer starts under it.
                        ((when-let* ((parent (and (car key) (romacs-agent-loaded (car key)))))
                           (romacs-agent-cancelled parent))
                         (list :error (format "Worktree %s (branch %s) was made, but the task that asked for it was cancelled"
                                              (plist-get made :path) (plist-get made :branch))))
                        (t
                         (condition-case err
                             (list :child (funcall start (plist-get made :cwd) :worktree (plist-get made :path)
                                                   :branch (plist-get made :branch)))
                           (error (list :error (format "Worktree %s (branch %s) was made, but the child did not start: %s"
                                                       (plist-get made :path) (plist-get made :branch)
                                                       (error-message-string err))))))))))
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
                        (progn (romacs-mcp-deferred-retime (nth 2 value)) (funcall (setq next (cadr value))))
                      (list value))))))))))

(defun romacs-agent-tools--renew (arguments)
  "Renew the task taskId (the caller's own session by default): now, or
once its running turn ends (`romacs-agent-renew-soon')."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (task (romacs-agent-tools--known
                (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no session"))))
         (own (equal (romacs-agent-id task) caller)))
    (unless own (romacs-agent-tools--own task caller "renew it"))
    (let ((when (romacs-agent-renew-soon task (or (gethash "why" arguments) (if own "its own" "its parent's"))))
          (record (romacs-agent-tools--ack task)))
      (puthash "next" (cond (own "You renew once this turn ends: end your turn now. Your next message carries a brief.")
                            ((eq when 'now) "It renews now; your next message to it carries a brief of its conversation.")
                            (t "It renews once its running turn ends; its next end wakes you as before."))
               record)
      record)))

(defun romacs-agent-tools--subtree (session)
  "SESSION and the tasks it delegated, theirs too, all the way down, not
yet cancelled.  The lineage is walked whatever each one's state: a task
may still run under a coordinator whose own harness ended.  Loaded ones
by the index, unloaded ones by their rows, loaded to be acted on only
when their supervisor still runs: one whose supervisor is gone runs
nothing."
  (let ((todo (list (romacs-agent-id session))) (seen nil) (found nil))
    (while todo
      (let ((id (pop todo)))
        (unless (member id seen)
          (push id seen)
          (when-let* ((at (if (equal id (romacs-agent-id session)) session
                            (or (romacs-agent-loaded id)
                                (and (file-exists-p (romacs-agent--host-socket id)) (romacs-agent-load-session id)))))
                      ((not (romacs-agent-cancelled at))))
            (push at found))
          (setq todo (append (mapcar #'romacs-agent-id (gethash id romacs-agent--children))
                             (mapcar #'car (romacs-agent-store-unloaded-rows id))
                             todo)))))
    (nreverse found)))

(defun romacs-agent-tools--cancel (arguments)
  "Cancel the task ARGUMENTS name and every task under it: all marked
first, so none's end wakes a parent being cancelled, then each stopped."
  (romacs-mcp--instance arguments)
  (let* ((child (romacs-agent-tools--known (gethash "taskId" arguments)))
         (tree (progn (romacs-agent-tools--own child (romacs-agent-tools--caller) "cancel it")
                      (romacs-agent-tools--subtree child))))
    (dolist (task tree)
      (setf (romacs-agent-cancelled task) t)
      (romacs-agent--clear-queue task)
      ;; A restarted server knows it was cancelled.
      (with-temp-file (expand-file-name "cancelled" (romacs-agent-dir task))))
    (dolist (task tree)
      ;; An idle one has no end of a run to tell its views.
      (romacs-agent--raise task (list :type 'status :text "cancelled"))
      (romacs-agent-stop task))
    (romacs-agent-tools--ack child)))

(defun romacs-agent-tools--effort-option (instance)
  "The name INSTANCE's harness's effort goes by in a target's options."
  (or (plist-get (romacs-agent--driver-of (plist-get instance :harness)) :effort-option) "effort"))

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

(defun romacs-agent-tools--pools ()
  "Each harness's pool (`romacs-agent-pool'): its accounts in order, each
with its last limit report and the sessions at work on it, and the one
new work draws on now."
  (let ((live (make-hash-table :test #'equal)))
    ;; This editor's loaded sessions, by their summaries.
    (maphash (lambda (id _)
               (when-let* ((summary (romacs-agent-summary id))
                           ((memq (romacs-agent-summary-state summary) '(starting running waiting-approval))))
                 (cl-incf (gethash (romacs-agent-summary-provider summary) live 0))))
             romacs-agent--sessions)
    (cl-loop for (harness) in romacs-agent--drivers
             for pool = (romacs-agent-pool harness)
             when pool
             collect (romacs-mcp--object
                      "harness" (symbol-name harness)
                      "next" (plist-get (romacs-agent-pool-pick harness) :name)
                      "accounts"
                      (vconcat (mapcar (lambda (i)
                                         (let ((room (romacs-agent-room-of (plist-get i :name))))
                                           (romacs-mcp--object
                                            "providerInstanceId" (plist-get i :name)
                                            "room" (if (romacs-agent-room (plist-get i :name)) t :false)
                                            "status" (if room (symbol-name (plist-get room :status)) :null)
                                            "reason" (or (plist-get room :reason) :null)
                                            "used" (or (plist-get room :used) :null)
                                            "resets" (or (plist-get room :resets) :null)
                                            "window" (or (plist-get room :window) :null)
                                            "live" (gethash (plist-get i :name) live 0))))
                                       pool))))))

(defun romacs-agent-tools--capabilities (arguments)
  "Each provider instance (`romacs-agent-instances'): its harness,
directory, program, models, whether it is enabled and where it comes
from; and each one Romacs cannot run, with why.  The models are those
known: a harness not asked yet in this run is asked in the background."
  (romacs-mcp--instance arguments)
  (pcase-let ((`(,instances . ,constraints) (romacs-agent-instances t)))
    (romacs-agent--want-models (cl-remove-if-not (lambda (i) (plist-get i :enabled)) instances))
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
     "pools" (vconcat (romacs-agent-tools--pools))
     "constraints"
     (vconcat (mapcar (lambda (c) (romacs-mcp--object "source" (symbol-name (plist-get c :source))
                                                      "id" (or (plist-get c :id) :null) "reason" (plist-get c :reason)))
                      constraints)))))

;;; Restart requests (the agents-develop-Romacs-live loop)

(defvar warm-restart-last-result)
(declare-function warm-restart-request "warm-restart")
(declare-function romacs-session-reset-target "romacs-live" ())
(declare-function romacs-server--build-label "romacs-server" (true))

(defun romacs-agent-tools--restart-wake-file (session)
  (expand-file-name "restart-wake" (romacs-agent-dir session)))

(defvar romacs-restart-requester)
(declare-function romacs-restart-record "warm-restart" (&optional id))

(defun romacs-agent-tools--restart (arguments)
  "Schedule a supervised restart once this call's reply is out and the
editor is between commands; tell the calling session its verdict."
  (romacs-mcp--instance arguments)
  (require 'warm-restart)
  (let* ((id (romacs-agent-tools--caller))
         (session (and id (romacs-agent-get id)))
         (target (or (gethash "target" arguments)
                     (and (eq (gethash "fresh" arguments) t) (romacs-session-reset-target)))))
    (warm-restart-request
     :target target
     :fresh (and (eq (gethash "fresh" arguments) t)
                 (if (eq (gethash "keepBuffers" arguments) t) 'buffers 'unsaved))
     :delay (gethash "delay" arguments)
     :expiry (gethash "expiry" arguments)
     ;; What the image cannot carry ends, or comes back with its package:
     ;; no process of the user's holds a restart.
     :force t
     ;; Just before the restart: the supervisor's verdict is told this
     ;; session; a build without verdicts (a rollback to an older one)
     ;; tells it from the wake file.
     :before (and session (lambda ()
                            (setq romacs-restart-requester id)
                            (romacs-agent-mark id 'wake t)
                            (write-region "verdict" nil (romacs-agent-tools--restart-wake-file session) nil 'silent)))
     :failed (and session (lambda (why)
                            (setq romacs-restart-requester nil)
                            (ignore-errors (delete-file (romacs-agent-tools--restart-wake-file session)))
                            (romacs-agent-mark id 'wake nil)
                            (romacs-agent-tools--deliver session (format "Romacs restart did not start, this session goes on: %s" why)
                                                         #'ignore))))
    (romacs-mcp--object
     "status" (symbol-name (plist-get warm-restart-last-result :status))
     "target" (or target :null)
     "waitsFor" "this call's reply, then the editor between commands"
     "verdict" "told this session when the new Romacs is judged; romacs_restart_status reads the record")))

(defun romacs-agent-tools--restart-status (arguments)
  "The record of a restart: its plan, its phases with times, its verdict."
  (romacs-mcp--instance arguments)
  (require 'warm-restart)
  (or (romacs-restart-record (gethash "restart" arguments))
      (romacs-mcp--object "state" "none" "restart" (or (gethash "restart" arguments) :null)
                          "message" "No restart record")))

(defun romacs-agent-tools--build-text ()
  "Which build this process runs, for the restarted sessions' message."
  (let ((root (getenv "ROMACS_RUNTIME_ROOT")))
    (format "Romacs restarted into %s"
            (or (and root (fboundp 'romacs-server--build-label)
                     (when-let* ((label (romacs-server--build-label root))) (format "%s (%s)" label root)))
                (and root (format "the build at %s" root))
                (format "the build at %s" invocation-directory)))))

(defun romacs-agent-tools--restart-wakes ()
  "After a restart: tell each session that asked for it, once.  One whose
wake file says `verdict' is told by the restart's supervisor instead
\(`romacs-restart-verdict')."
  (let ((text nil))
    (dolist (session (romacs-agent-sessions))
      (let ((file (romacs-agent-tools--restart-wake-file session)))
        (when (and (file-exists-p file)
                   (not (equal (with-temp-buffer (insert-file-contents file) (buffer-string)) "verdict")))
          (romacs-agent-tools--deliver session (or text (setq text (romacs-agent-tools--build-text)))
                                       (lambda () (ignore-errors (delete-file file))
                                         (romacs-agent-mark (romacs-agent-id session) 'wake nil))
                                       (lambda () (file-exists-p file))))))))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-tools--restart-wakes)
;; A session that asked to be told of a restart is loaded as a restore
;; runs, so it is told: one left a row would never be.
(add-to-list 'romacs-agent-mark-kinds
             (cons 'wake (lambda (dir _manifest) (file-exists-p (expand-file-name "restart-wake" dir)))))
;; A session due to resume when its limit resets is loaded as a restore
;; runs, so its resume is set again.
(add-to-list 'romacs-agent-mark-kinds
             (cons 'resume (lambda (_dir manifest) (numberp (gethash "resumeAt" manifest)))))

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
task and sends nothing; one another session owns is refused. Answers the task's short record: taskId, status, \
title, provider, model, worktree and branch, next, and resultChars, its last result's length; a wait's carries the \
result it waited for (cut as a wake's), never one you had. romacs_task_status reads a task's whole record and \
result. Talk to a running task with romacs_task_send; romacs_task_list finds your tasks again."))

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
last result, whole: the tools that act on a task answer only its length (resultChars)."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string")) '("taskId"))
 (lambda (arguments)
   (romacs-mcp--instance arguments)
   (romacs-agent-tools--told (romacs-agent-tools--resolve (gethash "taskId" arguments))
                             (lambda (id) (romacs-agent-tools--task (romacs-agent-summary id)))))
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_task_send"
 "Delivery: queued until the task's turn ends unless steer is true, which reaches a running task at its next tool \
boundary. Send a message to a task you delegated, or to another task of your own parent (your parent gets a copy with its \
next wake, never woken for it); any other session is refused. It reads it as a <romacs-message> naming you, and \
answers as its last line says: your own task by its turn's reply when that wakes you (wake end or all), else by romacs_task_reply; inReplyTo links this one to a message it answers. An idle task gets \
it at once; one whose harness ended is resumed for it. mode queue (default); replace: in place of your messages still \
waiting for it; interrupt: its turn cut short, then this first. A task's next turn end wakes its parent with its reply \
by its wake rule: end your turn then, do \
not poll. Answers the messageId, its delivery (steered into the running turn, interrupting it, queued behind it, or \
sent as its next turn now: a steer no turn can take is queued or sent) and the task's short record (taskId, status, \
title, provider, model, next, resultChars), never its last result: romacs_task_status reads that."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string") "taskId" (romacs-mcp--object "type" "string")
                "message" (romacs-mcp--object "type" "string")
                "steer" (romacs-mcp--object "type" "boolean"
                                            "description" "Into the running turn, at its next tool boundary (default false).")
                "mode" (romacs-mcp--object "type" "string" "enum" (vconcat romacs-agent-tools--send-modes))
                "inReplyTo" (romacs-mcp--object "type" "string"
                                                "description" "The id of a message this one answers."))
  "required" ["taskId" "message"] "additionalProperties" :false)
 #'romacs-agent-tools--send)

(romacs-mcp-register-tool
 "romacs_task_reply"
 "Answer a message another session sent you (a <romacs-message>, by its id): routed back to its sender, whatever \
it is to you (parent, child, sibling), linked to the message it answers. When a message's last line says your \
turn's reply answers it, use this only for an interim note to its sender while you work. mode as romacs_task_send's. \
Answers the reply's messageId and the sender's short record, as romacs_task_send's."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "messageId" (romacs-mcp--object "type" "string") "text" (romacs-mcp--object "type" "string")
                "steer" (romacs-mcp--object "type" "boolean"
                                            "description" "Into the running turn, at its next tool boundary (default false).")
                "mode" (romacs-mcp--object "type" "string" "enum" (vconcat romacs-agent-tools--send-modes)))
  "required" ["messageId" "text"] "additionalProperties" :false)
 #'romacs-agent-tools--reply)

(romacs-mcp-register-tool
 "romacs_task_list"
 "The tasks you delegated (with all, every session's), most recently active first: each task's record with the \
first 300 characters of its result and resultChars, its whole length. They are kept on disk, so a restarted Romacs \
or a lead that lost its context finds them again. A page at a time: limit rows (default 50, at most 200) from offset; \
total counts them all and nextOffset reads on. Read one whole with romacs_task_status; talk to one with \
romacs_task_send."
 (romacs-mcp--schema '(("instance" . "string") ("all" . "boolean") ("limit" . "integer") ("offset" . "integer")) nil)
 #'romacs-agent-tools--list
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_task_switch"
 "Change a task's model or effort, or go on with it under another provider instance (taskId, default your own \
session), e.g. when its model or instance hits a usage limit. The same session and conversation go on, once its \
running turn ends (your own: end your turn): in place with model or effort alone (provider defaults to its own), \
or moved as is to an instance of the same harness (claude to claude2); on another harness, the same task goes on \
there in a new conversation that starts on a brief of this one. model and effort default to the task's; next is a message \
it is given. A task's record names its usage limit and the instances with room."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string") ("provider" . "string") ("model" . "string")
                       ("effort" . "string") ("next" . "string"))
                     nil)
 #'romacs-agent-tools--switch)

(romacs-mcp-register-tool
 "romacs_task_renew"
 "Go on with a task (taskId, default your own session) in a new conversation of its harness, to free its \
context: the same task, parent, wake rule and worktree; its next message carries a brief of the conversation so \
far (its first request, the latest turns, its tool calls). Now when it is idle, else once its running turn ends \
(your own: end your turn). The old conversation stays in its journal."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string") ("why" . "string")) nil)
 #'romacs-agent-tools--renew)

(romacs-mcp-register-tool
 "romacs_task_cancel"
 "Cancel a delegated task: its session stops, with everything its harness started."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string")) '("taskId"))
 #'romacs-agent-tools--cancel)

(romacs-mcp-register-tool
 "romacs_task_answer"
 "Answer a request a task you delegated waits on (a question it asks, a plan it presents, a call it asks to run), \
as its user would: you are told of each with its requestId. allow (default true) with answers, an object from each \
question's text to the answer chosen; allow false denies, message saying why. The user may answer it in the task's \
chat instead; a request answered already is refused."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "taskId" (romacs-mcp--object "type" "string") "requestId" (romacs-mcp--object "type" "string")
                "allow" (romacs-mcp--object "type" "boolean") "answers" (romacs-mcp--object "type" "object")
                "message" (romacs-mcp--object "type" "string"))
  "required" ["taskId" "requestId"] "additionalProperties" :false)
 #'romacs-agent-tools--answer-request)

(romacs-mcp-register-tool
 "romacs_restart"
 "Restart Romacs into a new process, supervised: the session (buffers, windows, histories) and its agent sessions \
carry over, and the new process must show its socket, a window, its init done and its restore done within \
deadlines, or the old build is started again with the same session. target is a build directory (one holding \
romacs-vm-gui and lisp/) or a binary; omitted, this build again. It starts once this call's reply is out and the \
editor is between commands. This session is told the verdict (verified, rolled back and why) when it is known; \
romacs_restart_status reads the record."
 (romacs-mcp--schema '(("instance" . "string") ("target" . "string") ("delay" . "number") ("expiry" . "number")) nil)
 #'romacs-agent-tools--restart)

(romacs-mcp-register-tool
 "romacs_restart_status"
 "A restart's record, the newest when restart (its id) is omitted: its plan, each phase with its time, the \
checks that failed and the verdict (verified, rolled-back, rollback-failed)."
 (romacs-mcp--schema '(("instance" . "string") ("restart" . "string")) nil)
 #'romacs-agent-tools--restart-status
 (romacs-mcp--object "readOnlyHint" t))

(romacs-mcp-register-tool
 "romacs_restart_request"
 "Restart Romacs into a new process once your turn ends, carrying the session (buffers, windows, histories) \
and its agent sessions: after an install, the new build runs. target is a build directory (one holding \
romacs-vm-gui and lisp, e.g. the launcher's current) or a binary; omitted, the session's restart \
command or its own binary. fresh true makes it a reset (romacs-session-reset): onto the installed build \
when target is omitted, carrying no live Lisp files, kill ring, registers, histories or kept globals, only \
unsaved text and agent sessions; keepBuffers true carries the file buffers and their windows too. It waits until your turn has ended, nothing is BLOCKED (unsaved work that cannot be \
carried), no Romacs tool call is in flight and the editor is between commands, for delay seconds (default 2), up \
to expiry seconds (default 900). The same supervised restart as romacs_restart, whose verdict this session is \
told."
 (romacs-mcp--schema '(("instance" . "string") ("target" . "string") ("delay" . "number")
                       ("expiry" . "number") ("force" . "boolean") ("fresh" . "boolean")
                       ("keepBuffers" . "boolean"))
                     nil)
 #'romacs-agent-tools--restart)

(romacs-mcp-register-tool
 "romacs_orchestrator_capabilities"
 "The provider instances romacs_delegate_task can start: harness, configuration directory, program and options; \
and each harness's pool of accounts in order, each with its last usage-limit report (status, used, resets) and \
the sessions at work on it. A target provider naming a harness (\"claude\") draws on its pool: the first account \
with room, `next'; naming an instance pins it."
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

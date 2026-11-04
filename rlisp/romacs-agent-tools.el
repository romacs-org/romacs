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
(require 'romacs-agent)

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

(defun romacs-agent-tools--eval (arguments)
  "`romacs_eval' with a budget: quit, as by C-g, when it runs out."
  (romacs-mcp--instance arguments)
  (let ((budget (let ((b (gethash "budgetMs" arguments))) (if (integerp b) b romacs-agent-eval-budget)))
        (generation (cl-incf romacs-agent-tools--generation)))
    (when (> budget 0) (romacs--call-deadline generation budget))
    (unwind-protect
        (romacs-mcp-eval-string (gethash "code" arguments))
      (romacs--call-deadline nil))))

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
          ((or (memq state '(starting running)) (romacs-agent-queue session)) "running")
          ((romacs-agent-failed session) "failed")
          ((> (romacs-agent-turns session) 0) "completed")
          ((eq state 'ended) "failed")
          (t (symbol-name state)))))

(defun romacs-agent-tools--task (session)
  "SESSION as a task record."
  (let ((options (romacs-agent-options session)) (null (lambda (v) (or v :null))))
    (romacs-mcp--object
     "taskId" (romacs-agent-id session) "status" (romacs-agent-tools--status session)
     "state" (symbol-name (romacs-agent-state session)) "title" (romacs-agent-title session)
     "provider" (romacs-agent-provider session) "harness" (symbol-name (romacs-agent-harness session))
     "model" (funcall null (or (romacs-agent-model session) (plist-get options :model)))
     "role" (funcall null (plist-get options :role)) "parent" (funcall null (plist-get options :parent))
     "turns" (romacs-agent-turns session) "result" (funcall null (romacs-agent-reply session))
     "cost" (funcall null (romacs-agent-cost session))
     "pendingApproval" (if (> (hash-table-count (romacs-agent-approvals session)) 0) t :false))))

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

(defun romacs-agent-tools--wake-text (session end)
  "The parent's message for SESSION's turn END, (SEQ RESULT FAILED)."
  (format "Delegated task %s (%s) [%s]:\n%s" (romacs-agent-id session) (romacs-agent-title session)
          (if (nth 2 end) "failed" "completed") (or (nth 1 end) "")))

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

(defun romacs-agent-tools--wake (child)
  "Give CHILD's parent CHILD's new turn ends, in order, each with its own
result.  Each end is handed on once: it then waits in the parent's queue
(or for the parent's attaching) until the parent's harness has it."
  (let ((parent (romacs-agent-get (plist-get (romacs-agent-options child) :parent)))
        (ends (reverse (romacs-agent-ends child))))
    (setf (romacs-agent-ends child) nil)
    (when parent
      (dolist (end ends)
        (when (romacs-agent-tools--wanted-p child (car end))
          (let* ((text (romacs-agent-tools--wake-text child end))
                 ;; Delivered only once the parent's harness has it.
                 (delivered (lambda () (romacs-agent-tools--mark-woken child (car end))))
                 (valid (lambda () (romacs-agent-tools--wanted-p child (car end)))))
            (romacs-agent-tools--deliver parent text delivered valid)))))))

(defun romacs-agent-tools--deliver (session text delivered &optional valid)
  "Give SESSION's harness TEXT as its next message, then call DELIVERED.
A per-turn harness takes it as its next turn (queued behind a running
one); another once its supervisor is attached.  VALID, when given, is
asked first: nil drops the message."
  (let ((deliver (lambda () (when (or (null valid) (funcall valid))
                              (romacs-agent-send session text delivered valid)))))
    (if (romacs-agent--per-turn-p session) (funcall deliver)
      (romacs-agent--when-attached session deliver))))

(defvar romacs-agent-tools--scheduled (make-hash-table :test #'equal)
  "Children whose wake is scheduled.")

(defun romacs-agent-tools--on-event (session event)
  (when (and (plist-get (romacs-agent-options session) :parent)
             (or (eq (plist-get event :type) 'done)
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
started with the same clientRequestId."
  (romacs-mcp--instance arguments)
  (let* ((parent (romacs-agent-tools--caller))
         (request (gethash "clientRequestId" arguments))
         (target (gethash "target" arguments (romacs-mcp--object)))
         (options (gethash "options" target (romacs-mcp--object)))
         (child (or (and request (romacs-agent-requested parent request))
                    (romacs-agent-start
                     (or (gethash "cwd" arguments)
                         (and parent (romacs-agent-get parent) (romacs-agent-cwd (romacs-agent-get parent)))
                         default-directory)
                     (gethash "task" arguments) (gethash "title" arguments) (gethash "provider" target)
                     (list :model (gethash "model" target)
                           :effort (or (gethash "reasoningEffort" options) (gethash "effort" options))
                           :role (gethash "role" arguments) :parent parent :request request)))))
    (if (equal (gethash "mode" arguments) "wait")
        (romacs-agent-tools--wait child (gethash "timeoutMs" arguments)
                                  (or (plist-get romacs-mcp--active :arrived) (float-time)))
      (romacs-agent-tools--task child))))

(defun romacs-agent-tools--cancel (arguments)
  (romacs-mcp--instance arguments)
  (let ((child (romacs-agent-tools--known arguments)))
    (setf (romacs-agent-cancelled child) t (romacs-agent-queue child) nil)
    ;; A restarted server knows it was cancelled.
    (with-temp-file (expand-file-name "cancelled" (romacs-agent-dir child)))
    (romacs-agent-stop child)
    (romacs-agent-tools--task child)))

(defun romacs-agent-tools--capabilities (arguments)
  "Each provider instance: its harness, directory and program."
  (romacs-mcp--instance arguments)
  (romacs-mcp--object
   "default" (romacs-agent--default-provider)
   "providers"
   (vconcat
    (mapcar (lambda (entry)
              (let* ((instance (romacs-agent--provider entry))
                     (dir (or (plist-get instance :config-dir)
                              (plist-get (alist-get (plist-get instance :harness) romacs-agent--harnesses) :default))))
                (romacs-mcp--object
                 "providerInstanceId" (plist-get instance :name) "harness" (symbol-name (plist-get instance :harness))
                 "configDir" (expand-file-name dir) "configDirExists" (if (file-directory-p dir) t :false)
                 "program" (plist-get instance :program)
                 "programFound" (if (executable-find (plist-get instance :program)) t :false)
                 "options" (if (eq (plist-get instance :harness) 'codex) ["reasoningEffort"] ["effort"]))))
            romacs-agent-providers))))

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

(romacs-mcp-register-tool
 "romacs_delegate_task"
 "Start a child agent session on TASK, as T3 Code's delegate_task: target {provider (a providerInstanceId from \
romacs_orchestrator_capabilities), model, options {reasoningEffort|effort}}, role, cwd (default the caller's), title. \
mode async (default) answers at once with the taskId and wakes you with the child's result when its turn ends; mode \
wait answers when it ends, asks for an approval, or timeoutMs (default 600000) passes, and the child goes on. The \
same clientRequestId returns the task it started before."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "task" (romacs-mcp--object "type" "string") "role" (romacs-mcp--object "type" "string")
                "mode" (romacs-mcp--object "type" "string" "enum" ["async" "wait"])
                "target" (romacs-mcp--object "type" "object")
                "cwd" (romacs-mcp--object "type" "string") "title" (romacs-mcp--object "type" "string")
                "timeoutMs" (romacs-mcp--object "type" "integer")
                "clientRequestId" (romacs-mcp--object "type" "string"))
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
 "romacs_task_cancel"
 "Cancel a delegated task: its session stops, with everything its harness started."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string")) '("taskId"))
 #'romacs-agent-tools--cancel)

(romacs-mcp-register-tool
 "romacs_restart_request"
 "Restart Romacs into a new process once your turn ends, carrying the session (buffers, windows, histories) \
and its agent sessions: after an install, the new build runs. target is a build directory (one holding \
romacs-vm-gui, gnu-lisp and rlisp, e.g. the launcher's current) or a binary; omitted, the session's restart \
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

(romacs-mcp-enable-editor-tools)
;; Gnosis's agent API, when gnosis is installed (it loads at a tool's first call).
(when (locate-library "gnosis-agent") (require 'romacs-agent-gnosis))
(romacs-mcp-enable-session-tools)

(provide 'romacs-agent-tools)
;;; romacs-agent-tools.el ends here

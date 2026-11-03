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
        (insert-file-contents file)
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
          (with-temp-file temporary
            (insert (json-serialize (romacs-mcp--object "key" "forgotten")) "\n")
            (dolist (key (reverse (seq-take order romacs-agent-tools--kept)))
              (insert (json-serialize (gethash key table)) "\n")))
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
  (concat (or (process-get peer 'romacs-agent-relay) "") ":" (json-serialize id)))

(defun romacs-agent-tools--text (value)
  (truncate-string-to-width (if (stringp value) value (format "%S" value)) 1024))

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

;;; Registration

(romacs-mcp-register-tool
 "romacs_eval"
 "Evaluate Emacs Lisp in the running editor (its buffers, frames and state) and return the last value printed. \
Runs at a wait point of the editor's command loop; C-g or budgetMs (default 5000, 0 for none) quits it at its next \
safepoint. Never safe to repeat after an unknown outcome: on `Romacs instance mismatch', call romacs_identity, \
then romacs_call_outcome for your in-flight ids."
 (romacs-mcp--schema '(("instance" . "string") ("code" . "string") ("budgetMs" . "integer")) '("instance" "code"))
 #'romacs-agent-tools--eval)

(romacs-mcp-register-tool
 "romacs_buffer_edit"
 "Replace ranges of a buffer's text, atomically and as one undo group. Positions are Emacs positions (1 is the \
first character); edits must not overlap. With expectedTick the edit is refused if the buffer changed since you \
read it (romacs_buffer_read gives the tick). Safe to repeat with expectedTick."
 (romacs-mcp--schema '(("instance" . "string") ("name" . "string") ("expectedTick" . "integer") ("edits" . "array"))
                     '("instance" "name" "edits"))
 #'romacs-agent-tools--buffer-edit)

(romacs-mcp-register-tool
 "romacs_file_write"
 "Make a file's buffer text TEXT (visiting it first), as one undo group; save it only when save is true. The \
file's earlier content is kept for this session."
 (romacs-mcp--schema '(("instance" . "string") ("path" . "string") ("text" . "string") ("save" . "boolean"))
                     '("instance" "path" "text"))
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

(romacs-mcp-enable-editor-tools)
(romacs-mcp-enable-session-tools)

(provide 'romacs-agent-tools)
;;; romacs-agent-tools.el ends here

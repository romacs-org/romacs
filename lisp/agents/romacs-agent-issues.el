;;; romacs-agent-issues.el --- Orchestration problems, logged as they happen  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What goes wrong between agent sessions is logged as it happens, so it
;; can be counted and fixed: a tool call refused or failing, a task's
;; result that crossed messages its parent sent meanwhile, a task ending
;; with no result or with one its role's shape does not fit (a review
;; with no verdict), usage limits and the moves they cause, a supervisor
;; that never came up, a watch that could not tell how its unit ended.
;;
;; One JSON line an issue, appended to issues.jsonl in the agents' state
;; root: its time, kind, session and details.  Only live events are
;; logged, never a replay.  romacs_orchestration_issues counts them by
;; kind, with the latest of each.

;;; Code:

(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-agent)

(defcustom romacs-agent-issues-role-shapes
  '(("review" . "^VERDICT: \\(?:APPROVE\\|CHANGES\\)"))
  "Each role's result shape: a delegated task of the role whose turn ends
with a result this regexp does not find is an issue (`result-shape')."
  :type '(alist :key-type string :value-type regexp) :group 'romacs-agent)

(defconst romacs-agent-issues--limit (* 4 1024 1024)
  "The log's size past which it starts again, the old one kept beside it.")

(defconst romacs-agent-issues--text-limit 300
  "The most characters of one detail's text an issue keeps.")

(defun romacs-agent-issues--file ()
  (expand-file-name "issues.jsonl" (romacs-agent--state-root)))

(defun romacs-agent-issues--value (value)
  "VALUE fit for an issue's details: text cut short, a symbol its name."
  (cond ((stringp value) (truncate-string-to-width value romacs-agent-issues--text-limit nil nil "…"))
        ((and value (symbolp value) (not (eq value t))) (symbol-name value))
        ((null value) :null)
        (t value)))

(defun romacs-agent-issues-record (kind session &rest details)
  "Log an issue of KIND (a symbol) about SESSION (a session, an id or nil),
DETAILS a plist of names and values.  It never signals: logging never
stands in the way of what it logs."
  (with-demoted-errors "romacs-agent-issues: %S"
    (romacs-agent-issues--write kind session details)))

(defun romacs-agent-issues--write (kind session details)
  (let* ((file (romacs-agent-issues--file))
         (id (if (stringp session) session (and session (romacs-agent-id session))))
         (object (romacs-mcp--object "t" (float-time) "kind" (symbol-name kind) "session" (or id :null)))
         (fields (romacs-mcp--object)))
    (when-let* ((session (if (stringp session) (romacs-agent-get session) session))
                (title (romacs-agent-title session)))
      (puthash "title" (romacs-agent-issues--value title) object))
    (cl-loop for (name value) on details by #'cddr
             do (puthash (substring (symbol-name name) 1) (romacs-agent-issues--value value) fields))
    (puthash "details" fields object)
    (make-directory (file-name-directory file) t)
    (when (> (or (file-attribute-size (file-attributes file)) 0) romacs-agent-issues--limit)
      (rename-file file (concat file ".1") t))
    (let ((coding-system-for-write 'utf-8-unix))
      (write-region (concat (json-serialize object :null-object :null :false-object :false) "\n")
                    nil file t 'silent))))

;;; What sessions' events say

(defvar romacs-agent-issues--judged (make-hash-table :test #'eq :weakness 'key)
  "Delegated sessions to the seq of the prompt their last turn end
answered: an end answers a prompt only when one came since.  Nothing
judged yet in this process means its last prompt is unanswered: a
session restored mid-turn has that prompt in its kept state.")

(defun romacs-agent-issues--asked (session)
  "Whether SESSION's turn ending now answers a prompt none answered yet;
it is answered from now on."
  (let ((prompt (romacs-agent-input-seq session)))
    (prog1 (and prompt (not (eql prompt (gethash session romacs-agent-issues--judged))))
      (puthash session prompt romacs-agent-issues--judged))))

(defvar romacs-agent-issues--told (make-hash-table :test #'eq :weakness 'key)
  "Delegated sessions to how many messages their parent sent while their
turn ran, waiting for its end: that turn's result was made before them.")

(defun romacs-agent-issues-told (session)
  "SESSION's parent sent it a message that waits for its running turn."
  (puthash session (1+ (gethash session romacs-agent-issues--told 0)) romacs-agent-issues--told))

(defconst romacs-agent-issues--quiet "[quiet]"
  "What a task's reply starts with when its parent need not act on it: an
acknowledgement, not a result to judge.")

(defun romacs-agent-issues--end (session event)
  "The issues of a delegated SESSION's turn end, EVENT a `done'.  No result
text is kept: only its size."
  (let* ((result (string-trim (or (and (stringp (plist-get event :text)) (plist-get event :text)) "")))
         (role (plist-get (romacs-agent-options session) :role))
         (seq (plist-get event :seq))
         (told (gethash session romacs-agent-issues--told))
         (asked (romacs-agent-issues--asked session)))
    (remhash session romacs-agent-issues--told)
    (when told (romacs-agent-issues-record 'crossed session :seq seq :waiting told :chars (length result)))
    (cond ((plist-get event :limit)
           (romacs-agent-issues-record 'limit session :seq seq :provider (romacs-agent-provider session)
                                       :limit (plist-get event :limit)))
          ((plist-get event :error)
           (romacs-agent-issues-record 'failed session :seq seq :reason (plist-get event :reason)))
          ;; Only a turn that answers a message is judged.
          ((not asked))
          ((string-empty-p result) (romacs-agent-issues-record 'no-result session :seq seq :how "done"))
          ((string-prefix-p romacs-agent-issues--quiet result))
          ((when-let* ((shape (cdr (assoc role romacs-agent-issues-role-shapes))))
             (not (string-match-p shape result)))
           (romacs-agent-issues-record 'result-shape session :seq seq :role role :chars (length result))))))

(defun romacs-agent-issues--on-event (session event)
  "Log what EVENT, a live one of SESSION's, says went wrong."
  (when (memq (plist-get event :type) '(done exit unstarted selection))
    (let ((delegated (plist-get (romacs-agent-options session) :parent))
          (type (plist-get event :type)))
      (cond
       ;; Raised by a launch, never journaled: never a replay's.
       ((eq type 'unstarted)
        (when (eq (romacs-agent-get (romacs-agent-id session)) session)
          (romacs-agent-issues-record 'resume-failed session :text (plist-get event :text))))
       ;; A turn end replayed answered its prompt: never judged again.
       ((not (romacs-agent--live-p session))
        (when (and delegated (memq type '(done exit)))
          (romacs-agent-issues--asked session)
          (remhash session romacs-agent-issues--told)))
       ((eq type 'done)
        (if delegated (romacs-agent-issues--end session event)
          (when (plist-get event :limit)
            (romacs-agent-issues-record 'limit session :seq (plist-get event :seq)
                                        :provider (romacs-agent-provider session) :limit (plist-get event :limit)))))
       ;; Its harness ended in the middle of a turn: the exit ended it, failed.
       ((eq type 'exit)
        (when (and delegated (romacs-agent-failed session)
                   (eql (romacs-agent-done-seq session) (plist-get event :seq)))
          (romacs-agent-issues--asked session)
          (remhash session romacs-agent-issues--told)
          (romacs-agent-issues-record 'no-result session :seq (plist-get event :seq) :how "exit"
                                      :status (plist-get event :status))))
       ((eq type 'selection)
        (let ((from (plist-get (plist-get event :from) :provider))
              (to (plist-get (plist-get event :to) :provider)))
          (unless (equal from to) (romacs-agent-issues-record 'moved session :from from :to to))))))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-issues--on-event)

;;; Tool calls

(defconst romacs-agent-issues--plain-arguments
  '("instance" "taskId" "mode" "provider" "model" "effort" "role" "kind" "wake" "isolation" "messageId" "unit")
  "The arguments an issue keeps as they are (when short): names and
choices, never what a caller wrote.")

(defun romacs-agent-issues--summary (name value)
  "VALUE of argument NAME as an issue keeps it: a plain argument's short
value; anything else only its type and size."
  (cond ((and (stringp value) (member name romacs-agent-issues--plain-arguments) (<= (length value) 80)) value)
        ((stringp value) (format "(%d characters)" (length value)))
        ((hash-table-p value) (format "(an object of %d)" (hash-table-count value)))
        ((vectorp value) (format "(%d items)" (length value)))
        ((or (numberp value) (memq value '(t :false :null))) value)
        (t "(a value)")))

(defun romacs-agent-issues--arguments (arguments)
  "ARGUMENTS of a tool call as an issue keeps them, each summarized
\(`romacs-agent-issues--summary')."
  (let ((kept (romacs-mcp--object)))
    (when (hash-table-p arguments)
      (maphash (lambda (name value) (puthash name (romacs-agent-issues--summary name value) kept)) arguments))
    kept))

(defun romacs-agent-issues--on-failure (peer name arguments message refused)
  "Log a tool call of the agents' (romacs_task_*, romacs_delegate_task and
the like) that failed: refused, or broken."
  (when (and (stringp name) (string-match-p "\\`romacs_\\(?:task\\|delegate\\|orchestrator\\)" name))
    (romacs-agent-issues-record (if refused 'tool-refused 'tool-error)
                                (and (processp peer) (process-get peer 'romacs-agent-session))
                                :tool name :error message :arguments (romacs-agent-issues--arguments arguments))))

(add-hook 'romacs-mcp-failure-functions #'romacs-agent-issues--on-failure)

;;; Reading it

(defun romacs-agent-issues-read (&optional since kind)
  "The issues logged since SINCE (a time, `float-time'), of KIND (a name)
when given, oldest first, each a table."
  (let ((file (romacs-agent-issues--file)) (issues nil))
    (when (file-readable-p file)
      (with-temp-buffer
        (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
        (goto-char (point-min))
        (while (not (eobp))
          (let ((issue (ignore-errors (json-parse-string (buffer-substring (point) (line-end-position))
                                                         :null-object :null :false-object :false))))
            (when (and (hash-table-p issue)
                       (or (not since) (>= (or (gethash "t" issue) 0) since))
                       (or (not kind) (equal (gethash "kind" issue) kind)))
              (push issue issues)))
          (forward-line 1))))
    (nreverse issues)))

(defun romacs-agent-issues-summary (&optional since kind examples)
  "The issues since SINCE of KIND (`romacs-agent-issues-read'): a table of
their total and their kinds, most first, each its count and its EXAMPLES
latest (default 3), latest first."
  (let ((counts (make-hash-table :test #'equal)) (latest (make-hash-table :test #'equal))
        (issues (romacs-agent-issues-read since kind)))
    (dolist (issue issues)
      (let ((kind (gethash "kind" issue)))
        (puthash kind (1+ (gethash kind counts 0)) counts)
        (push issue (gethash kind latest))))
    (romacs-mcp--object
     "total" (length issues)
     "kinds" (vconcat
              (mapcar (lambda (kind)
                        (romacs-mcp--object
                         "kind" kind "count" (gethash kind counts)
                         "recent" (vconcat (mapcar (lambda (issue)
                                                     (puthash "t" (format-time-string "%FT%T%z" (gethash "t" issue)) issue)
                                                     issue)
                                                   (seq-take (gethash kind latest) (or examples 3))))))
                      (sort (hash-table-keys counts) (lambda (a b) (> (gethash a counts) (gethash b counts))))))
     "log" (romacs-agent-issues--file))))

(defun romacs-agent-issues--tool (arguments)
  (romacs-mcp--instance arguments)
  (let ((hours (gethash "hours" arguments)) (examples (gethash "examples" arguments)))
    (when (and hours (not (and (numberp hours) (> hours 0)))) (user-error "hours must be a positive number"))
    (when (and examples (not (natnump examples))) (user-error "examples must be a natural number"))
    (romacs-agent-issues-summary (and hours (- (float-time) (* 3600 hours))) (gethash "kind" arguments) examples)))

(romacs-mcp-register-tool
 "romacs_orchestration_issues"
 "What went wrong between agent sessions, logged as it happened, counted by kind (most first) with the latest \
examples of each: tool-refused and tool-error (a romacs_task_* or delegate call refused or failing), crossed (a \
task's result made before messages its parent sent meanwhile), no-result, failed, result-shape (a result its \
role's shape does not fit, as a review with no verdict), limit, moved (to another instance), resume-failed, \
watch-unknown. hours: only the last N hours; kind: one kind; examples: how many of each (default 3)."
 (romacs-mcp--schema '(("instance" . "string") ("hours" . "number") ("kind" . "string") ("examples" . "integer")) nil)
 #'romacs-agent-issues--tool
 (romacs-mcp--object "readOnlyHint" t))

(provide 'romacs-agent-issues)
;;; romacs-agent-issues.el ends here

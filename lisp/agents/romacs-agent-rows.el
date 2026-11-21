;;; romacs-agent-rows.el --- How a chat draws a tool's call -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat (romacs-agent-log.el) draws each tool call as one row: what
;; the agent does, in the user's words, and once it ends what came of
;; it, never the payload.  A tool with a renderer here gets a designed
;; row; any other, its name, a brief of its input and a generic outcome
;; (`romacs-agent-rows-outcome': a status, a title or an error picked
;; out of a JSON answer, else how many lines).  Romacs's own tools all
;; read as what they did, behind Romacs's icon.  A new tool adds a
;; renderer, never a case in the chat.
;;
;; A renderer is a plist (`romacs-agent-rows-define'):
;;   :row FN       (INPUT): the row's text as the call starts.
;;   :ended FN     (INPUT OUTPUT): its text once the call ended well, or
;;                 nil for the row and the generic outcome.
;;   :body FN      (INPUT OUTPUT): the details RET on the row shows above
;;                 the raw answer (a table, a list), or nil.
;; A call that ended well is its row alone: what it answered is one key
;; away.
;; INPUT is the call's arguments, a hash table; OUTPUT its result's text.
;; The functions are pure: they read the session table at most, and the
;; chat draws what they return.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-icons)
(require 'romacs-lexical)

(defvar romacs-agent-rows (make-hash-table :test #'equal)
  "Each tool's renderer, by the tool's name (`romacs-agent-rows-define').")

(defun romacs-agent-rows-define (name &rest renderer)
  "Draw tool NAME's calls by RENDERER, a plist of :row :ended :body
\(this file's Commentary)."
  (puthash name renderer romacs-agent-rows))

(defun romacs-agent-rows-name (name)
  "A tool's NAME as a row shows it and its renderer is known by: an MCP
tool by its own name."
  (if (and (stringp name) (string-match "\\`mcp__[^_]+\\(?:_[^_]+\\)*__\\(.+\\)\\'" name))
      (match-string 1 name)
    (or name "tool")))

(defun romacs-agent-rows-get (name)
  "Tool NAME's renderer: its own, Romacs's generic one for any other
romacs_ tool, or nil, the chat's generic row.  A Romacs tool's answer
reads as words where its renderer says nothing of it, never as JSON."
  (let ((romacs (and (stringp name) (string-prefix-p "romacs_" name)))
        (renderer (gethash name romacs-agent-rows)))
    (cond ((not romacs) renderer)
          ((not renderer) (romacs-agent-rows--romacs-generic name))
          ;; Its body when it says something, else the answer as words.
          ((plist-get renderer :body)
           (let ((body (plist-get renderer :body)))
             (append (list :body (lambda (input output) (or (funcall body input output) (romacs-agent-rows-words output))))
                     renderer)))
          (t (append renderer (list :body (lambda (_input output) (romacs-agent-rows-words output))))))))

(defun romacs-agent-rows-words (output)
  "OUTPUT, a tool's answer, as words: a JSON object's keys and values a
line each (a text of several lines in full under its key, a list as its
items, or how many when they are records), any other text as it is."
  (let ((record (romacs-agent-rows--json output)))
    (if (not (hash-table-p record)) (or output "")
      (let ((lines nil))
        (maphash
         (lambda (key value)
           (let ((said (cond ((stringp value)
                              (if (string-search "\n" value)
                                  (concat "\n" (replace-regexp-in-string "^" "  " (string-trim-right value)))
                                value))
                             ((numberp value) (number-to-string value))
                             ((eq value t) "yes")
                             ((memq value '(:false :null nil)) nil)
                             ((and (vectorp value) (cl-every (lambda (v) (or (stringp v) (numberp v))) value))
                              (mapconcat (lambda (v) (format "%s" v)) value ", "))
                             ((vectorp value) (format "%d items" (length value)))
                             ((hash-table-p value) (format "%d fields" (hash-table-count value))))))
             (when (and said (not (string-empty-p said)))
               (push (concat (romacs-agent-rows--dim (concat key "  ")) said) lines))))
         record)
        (string-join (nreverse lines) "\n")))))

;;; What a call came to

(defun romacs-agent-rows--started (record)
  "What a delegation came to: \"started\" for a task now running, else its
state, dim after a dot; a history read later shows the call's moment."
  (let ((state (or (romacs-agent-rows--field record "state") (romacs-agent-rows--field record "status"))))
    (if (member state '("running" "starting")) (romacs-agent-rows--dim " · started")
      (romacs-agent-rows--state record))))

(defun romacs-agent-rows--said-state (state)
  "STATE, a task's, after a dot: dim, but a failure's in the error face
behind its mark, seen before it is read."
  (if (member state '("failed" "error" "cancelled" "unstarted"))
      (concat (romacs-agent-rows--dim " · ") (propertize (concat "✗ " state) 'face 'romacs-agent-error))
    (romacs-agent-rows--dim (concat " · " state))))

(defun romacs-agent-rows--state (record)
  "RECORD's state, or its status, after a dot (`romacs-agent-rows--said-state'),
or nothing."
  (if-let* ((state (or (romacs-agent-rows--field record "state") (romacs-agent-rows--field record "status"))))
      (romacs-agent-rows--said-state state)
    ""))

(defun romacs-agent-rows-outcome (output)
  "What OUTPUT, a call's answer, came to as a row says it after the call,
after a dim dot: a JSON object's error, in the error face; else its
status or state and its title, name or id, else how many fields it has;
a list's length; a text's one line, else how many lines; nothing for no
output."
  (if (not (romacs-agent-rows-readable-p output))
      (romacs-agent-rows--dim (concat " · " (romacs-agent-rows-size output)))
  (let* ((value (and (stringp output) (string-match-p "\\`[ \t\n]*[[{]" output)
                     (ignore-errors (json-parse-string output :null-object nil :false-object nil))))
         (error (and (hash-table-p value) (romacs-agent-rows--field value "error")))
         (said
          (cond (error nil)
                ((hash-table-p value)
                 (let ((state (or (romacs-agent-rows--field value "status") (romacs-agent-rows--field value "state")))
                       (what (seq-some (lambda (key) (romacs-agent-rows--field value key)) '("title" "name" "id"))))
                   (if (or state what)
                       (string-join (delq nil (list state (and what (truncate-string-to-width (romacs-agent-rows--line what) 48 nil nil "…"))))
                                    " · ")
                     (romacs-agent-rows--plural (hash-table-count value) "field"))))
                ((vectorp value) (romacs-agent-rows--plural (length value) "item"))
                ((or (not (stringp output)) (not (string-match-p "[^ \t\n\r]" output))) nil)
                ;; One line: its first clause, without the tool's own
                ;; name the row says already.
                (t (let ((count (romacs-agent-rows--lines output)))
                     (if (/= count 1) (romacs-agent-rows--plural count "line")
                       (let ((line (romacs-agent-rows--line output)))
                         (truncate-string-to-width (string-trim (car (split-string (substring line 0 (min (length line) 200)) " *[(;]")))
                                                   48 nil nil "…"))))))))
    (cond (error (concat (romacs-agent-rows--dim " · ") (propertize (romacs-agent-rows--line error) 'face 'romacs-agent-error)))
          ((or (null said) (string-empty-p said)) "")
          (t (romacs-agent-rows--dim (concat " · " said)))))))

(defun romacs-agent-rows-ended (name input row output)
  "The row of tool NAME's call with INPUT, first drawn as ROW, once it
ended well with OUTPUT: its renderer's :ended, else ROW and the generic
outcome (`romacs-agent-rows-outcome')."
  (cond ((not (romacs-agent-rows-readable-p output))
         (concat row (romacs-agent-rows--dim (concat " · " (romacs-agent-rows-size output)))))
        ((when-let* ((renderer (romacs-agent-rows-get (romacs-agent-rows-name name)))
                     (ended (plist-get renderer :ended)))
           (funcall ended (if (hash-table-p input) input (make-hash-table :test #'equal)) output)))
        (t (concat row (romacs-agent-rows-outcome output)))))

(defun romacs-agent-rows-details (name input output)
  "The details RET on tool NAME's row shows above its raw answer: its
renderer's :body for INPUT and OUTPUT, or nil."
  (when-let* ((renderer (romacs-agent-rows-get (romacs-agent-rows-name name)))
              (body (plist-get renderer :body)))
    (let ((details (funcall body (if (hash-table-p input) input (make-hash-table :test #'equal)) output)))
      (and (stringp details) (not (string-empty-p details)) details))))

;;; Pieces

(defface romacs-agent-romacs-tool '((t :inherit romacs-working-running))
  "The mark of a call of Romacs's own tools, which tells them from a
harness's: it keeps its colour as the row settles and dims.")

(defun romacs-agent-rows--romacs (text)
  "TEXT behind Romacs's mark: its icon where it can be drawn, else ◈."
  (concat (propertize (or (romacs-icon 'romacs) "◈") 'face 'romacs-agent-romacs-tool) " " text))

(defun romacs-agent-rows--field (table key)
  "TABLE's KEY when TABLE is a hash table and KEY holds a string."
  (and (hash-table-p table) (stringp (gethash key table)) (gethash key table)))

(defun romacs-agent-rows--get (table key)
  "TABLE's KEY when TABLE is a hash table, whatever it holds, else nil."
  (and (hash-table-p table) (gethash key table)))

(defun romacs-agent-rows--json (output)
  "OUTPUT read as a JSON object, or nil; unread past
`romacs-agent-rows-read-limit'."
  (let ((value (and (romacs-agent-rows-readable-p output)
                    (ignore-errors (json-parse-string output :null-object nil :false-object nil)))))
    (and (hash-table-p value) value)))

(defun romacs-agent-rows--line (text)
  "TEXT's first line that says something, trimmed: read from its start,
no further than its first 4096 characters."
  (let* ((head (let ((text (or text ""))) (substring text 0 (min (length text) 4096))))
         (from (if (string-match "\\`[ \t\n\r]*" head) (match-end 0) 0)))
    (string-trim (substring head from (string-search "\n" head from)))))

(defun romacs-agent-rows--dim (text)
  (propertize text 'face 'romacs-agent-dim))

(defun romacs-agent-rows--title (id)
  "Task ID's title, while the session is known."
  (or (and (stringp id) (and-let* ((session (romacs-agent-loaded id))) (romacs-agent-title session))) "a task"))

(defun romacs-agent-rows-quoted (text &optional face)
  "TEXT quoted, in FACE when given: a bar before each of its lines, which
is decoration (`romacs-agent-decor'), never part of the text one copies,
and before each line a long one wraps onto (`wrap-prefix')."
  (let ((bar (propertize "┃ " 'face 'romacs-agent-message-bar)))
    (mapconcat (lambda (line)
                 (concat (propertize bar 'romacs-agent-decor t 'wrap-prefix bar)
                         (propertize (if face (propertize line 'face face) line) 'wrap-prefix bar)))
               (split-string (string-trim-right (or text "")) "\n")
               (propertize "\n" 'wrap-prefix bar))))

(defun romacs-agent-rows--instance (model harness provider)
  "MODEL@PROVIDER, the model without its HARNESS's prefix; what of it is
known."
  (let ((model (and model harness (string-remove-prefix (concat harness "-") model))))
    (cond ((and model provider) (format "%s@%s" model provider)) (model) (provider))))

;;; Messages and delegations

(defun romacs-agent-rows--message-body (input _output)
  "A message this chat sent another thread, quoted and dim
\(`romacs-agent-ui-sent'): not words for the user."
  (romacs-agent-rows-quoted (or (romacs-agent-rows--field input "message") (romacs-agent-rows--field input "text"))
                            'romacs-agent-ui-sent))

(romacs-agent-rows-define
 "romacs_task_send"
 :row (lambda (input) (romacs-agent-rows--romacs
                       (concat "✉\uFE0E → " (propertize (romacs-agent-rows--title (romacs-agent-rows--field input "taskId"))
                                                   'face 'romacs-agent-message))))
 :ended (lambda (input output)
          (let ((record (romacs-agent-rows--json output)))
            (romacs-agent-rows--romacs
             (concat "✉\uFE0E → " (propertize (or (romacs-agent-rows--field record "title")
                                           (romacs-agent-rows--title (romacs-agent-rows--field input "taskId")))
                                       'face 'romacs-agent-message)
                     (romacs-agent-rows--dim " · sent")))))
 :body #'romacs-agent-rows--message-body)

(romacs-agent-rows-define
 "romacs_task_reply"
 ;; Whom it goes to is the answer's: the sender's task record.
 :row (lambda (_input) (romacs-agent-rows--romacs (concat "✉\uFE0E " (propertize "reply" 'face 'romacs-agent-message))))
 :ended (lambda (_input output)
          (and-let* ((title (romacs-agent-rows--field (romacs-agent-rows--json output) "title")))
            (romacs-agent-rows--romacs (concat "✉\uFE0E → " (propertize title 'face 'romacs-agent-message)))))
 :body #'romacs-agent-rows--message-body)

(defun romacs-agent-rows--delegated (title instance)
  (concat "⑂ " (propertize title 'face 'romacs-agent-delegate)
          (if instance (romacs-agent-rows--dim (concat " · " instance)) "")))

(romacs-agent-rows-define
 "romacs_delegate_task"
 :row (lambda (input)
        (let ((target (gethash "target" input)))
          (romacs-agent-rows--romacs
           (romacs-agent-rows--delegated (or (romacs-agent-rows--field input "title") "task")
                                         (romacs-agent-rows--instance (romacs-agent-rows--field target "model") nil
                                                                      (romacs-agent-rows--field target "provider"))))))
 ;; What the task runs on, as its record says, else as it was asked.
 :ended (lambda (input output)
          (when-let* ((record (romacs-agent-rows--json output)))
            (let ((target (gethash "target" input)))
              (romacs-agent-rows--romacs
               (concat (romacs-agent-rows--delegated
                        (or (romacs-agent-rows--field record "title") (romacs-agent-rows--field input "title") "task")
                        (romacs-agent-rows--instance
                         (or (romacs-agent-rows--field record "model") (romacs-agent-rows--field target "model"))
                         (romacs-agent-rows--field record "harness")
                         (or (romacs-agent-rows--field record "provider") (romacs-agent-rows--field target "provider"))))
                       (romacs-agent-rows--started record))))))
 ;; A `wait' answers with the task's result.
 :body (lambda (_input output) (romacs-agent-rows--field (romacs-agent-rows--json output) "result")))

;; A harness's own subagent: the same card, with no task of ours.
(defun romacs-agent-rows-report (output)
  "A subagent's OUTPUT as its report: without the frame Claude Code puts
around it for its model (the hand-back notice before it, the agent's id
and its usage after it).  RET still shows all of it."
  (let ((report (string-trim-right
                 (if (string-match "\\`\\[Subagent hand-back\\]\\(?:.\\|\n\\)*?The report follows:[ \t]*\n" output)
                     (substring output (match-end 0))
                   output))))
    ;; Its last usage block, then its id line, at its end only.
    (when (string-suffix-p "</usage>" report)
      ;; A usage block is a few lines: looked for in the report's end.
      (let ((open nil) (from (max 0 (- (length report) 2048))))
        (while (setq from (string-search "<usage>" report from))
          (setq open from from (1+ from)))
        ;; A block, closed by the report's last tag: none quoted inside it.
        (when (and open (= (string-search "</usage>" report open) (- (length report) (length "</usage>"))))
          (setq report (string-trim-right (substring report 0 open))))))
    (when (string-match "\n[ \t]*agentId: [^\n]*\\'" report)
      (setq report (substring report 0 (match-beginning 0))))
    (string-trim report)))


(dolist (name '("Agent" "Task"))
  (romacs-agent-rows-define
   name
   :row (lambda (input) (romacs-agent-rows--delegated (or (romacs-agent-rows--field input "description") "task")
                                                       (romacs-agent-rows--field input "subagent_type")))
   ;; Its report's first line: the whole report is RET away.
   :ended (lambda (input output)
            (concat (romacs-agent-rows--delegated (or (romacs-agent-rows--field input "description") "task")
                                                  (romacs-agent-rows--field input "subagent_type"))
                    (romacs-agent-rows--dim " · ")
                    (truncate-string-to-width (romacs-agent-rows--line (romacs-agent-rows-report (or output ""))) 60 nil nil "…")))))

;;; Tasks: one line each

(defconst romacs-agent-rows--status-glyphs
  '(("running" . "●") ("waiting" . "◆") ("completed" . "✓") ("failed" . "!") ("cancelled" . "×") ("idle" . "·"))
  "Each task status's glyph in a list of tasks.")

(romacs-agent-rows-define
 "romacs_task_status"
 :row (lambda (input) (romacs-agent-rows--romacs (concat "◷ " (romacs-agent-rows--title (romacs-agent-rows--field input "taskId")))))
 :ended (lambda (input output)
          (let ((record (romacs-agent-rows--json output)))
            (romacs-agent-rows--romacs
             (concat "◷ " (or (romacs-agent-rows--field record "title")
                              (romacs-agent-rows--title (romacs-agent-rows--field input "taskId")))
                     (if-let* ((status (romacs-agent-rows--field record "status")))
                         (romacs-agent-rows--said-state status)
                       "")))))
 :body (lambda (_input output) (romacs-agent-rows--field (romacs-agent-rows--json output) "result")))

(defun romacs-agent-rows--tasks (output)
  "The task records OUTPUT, a task list's answer, holds."
  (let ((tasks (gethash "tasks" (or (romacs-agent-rows--json output) (make-hash-table)))))
    (and (vectorp tasks) (cl-remove-if-not #'hash-table-p (append tasks nil)))))

(romacs-agent-rows-define
 "romacs_task_list"
 :row (lambda (input) (romacs-agent-rows--romacs (if (eq (gethash "all" input) t) "◷ every session's tasks" "◷ tasks")))
 :ended (lambda (_input output)
          (let* ((tasks (romacs-agent-rows--tasks output))
                 (count (lambda (status) (cl-count status tasks :key (lambda (task) (gethash "status" task)) :test #'equal))))
            (romacs-agent-rows--romacs
             (concat (format "◷ %d task%s" (length tasks) (if (= (length tasks) 1) "" "s"))
                     (romacs-agent-rows--dim
                      (mapconcat (lambda (status) (let ((n (funcall count status))) (if (> n 0) (format " · %d %s" n status) "")))
                                 '("running" "waiting" "failed") ""))))))
 :body (lambda (_input output)
         (mapconcat (lambda (task)
                      (format "%s %s" (or (cdr (assoc (gethash "status" task) romacs-agent-rows--status-glyphs)) "·")
                              (or (romacs-agent-rows--field task "title") (romacs-agent-rows--field task "taskId") "task")))
                    (romacs-agent-rows--tasks output) "\n")))

(defun romacs-agent-rows--watched (input)
  "What a watch call names: its unit, or its file and section."
  (or (romacs-agent-rows--field input "unit")
      (and-let* ((file (romacs-agent-rows--field input "file")))
        (if-let* ((section (romacs-agent-rows--field input "section"))) (format "%s § %s" file section) file))
      "its watches"))

(romacs-agent-rows-define
 "romacs_task_watch"
 :row (lambda (input)
        (romacs-agent-rows--romacs (concat (if (eq (gethash "stop" input) t) "◷ stop watching " "◷ watch ")
                                           (romacs-agent-rows--watched input))))
 :ended (lambda (input output)
          (let ((watching (gethash "watching" (or (romacs-agent-rows--json output) (make-hash-table)))))
            (romacs-agent-rows--romacs
             (concat (if (eq (gethash "stop" input) t) "◷ stopped watching " "◷ watching ") (romacs-agent-rows--watched input)
                     (if (and (vectorp watching) (> (length watching) 1))
                         (romacs-agent-rows--dim (format " · %d watches" (length watching)))
                       "")))))
 :body (lambda (_input output)
         (let ((watching (gethash "watching" (or (romacs-agent-rows--json output) (make-hash-table)))))
           (and (vectorp watching) (mapconcat (lambda (w) (format "%s" w)) watching "\n")))))

;;; Lisp

(defun romacs-agent-rows-lisp (code)
  "CODE coloured by its lexical classes, as Emacs Lisp (romacs-lexical.el)."
  (romacs-lexical-string 'emacs-lisp code))

(romacs-agent-rows-define
 "romacs_eval"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (concat "λ " (romacs-agent-rows-lisp
                       (replace-regexp-in-string "[ \t\n]+" " " (string-trim (or (romacs-agent-rows--field input "code") "")))))))
 ;; What it gave, after the form: the form whole, and the answer, RET.
 :ended (lambda (input output)
          (let ((code (replace-regexp-in-string "[ \t\n]+" " " (string-trim (or (romacs-agent-rows--field input "code") "")))))
            (romacs-agent-rows--romacs
             (concat "λ " (romacs-agent-rows-lisp (truncate-string-to-width code 60 nil nil "…"))
                     (romacs-agent-rows--dim " ⇒ ")
                     (truncate-string-to-width (romacs-agent-rows--line output) 60 nil nil "…")))))
 :body (lambda (input _output) (romacs-agent-rows-lisp (string-trim (or (romacs-agent-rows--field input "code") "")))))

;;; Tables

(defun romacs-agent-rows--table (rows)
  "ROWS, lists of cells, as lines whose columns line up; none, nothing."
  (when rows
    (let ((widths (apply #'cl-mapcar (lambda (&rest cells) (apply #'max (mapcar #'string-width cells)))
                         (mapcar (lambda (row) (mapcar (lambda (cell) (format "%s" cell)) row)) rows))))
      (mapconcat (lambda (row)
                   (string-trim-right
                    (cl-loop for cell in row for width in widths
                             concat (let ((cell (format "%s" cell)))
                                      (concat cell (make-string (- (+ 2 width) (string-width cell)) ?\s))))))
                 rows "\n"))))

(defun romacs-agent-rows--providers (output)
  (let ((providers (gethash "providers" (or (romacs-agent-rows--json output) (make-hash-table)))))
    (and (vectorp providers) (cl-remove-if-not #'hash-table-p (append providers nil)))))

(romacs-agent-rows-define
 "romacs_orchestrator_capabilities"
 :row (lambda (_input) (romacs-agent-rows--romacs "▤ instances"))
 :ended (lambda (_input output)
          (let* ((providers (romacs-agent-rows--providers output))
                 (enabled (cl-count-if (lambda (p) (eq (gethash "enabled" p) t)) providers))
                 (models (apply #'+ (mapcar (lambda (p) (let ((models (gethash "models" p))) (if (vectorp models) (length models) 0)))
                                            providers))))
            ;; How many instances and models; how many are on, when not all.
            (romacs-agent-rows--romacs
             (concat (format "▤ %s" (romacs-agent-rows--plural (length providers) "instance"))
                     (romacs-agent-rows--dim
                      (concat (format " · %s" (romacs-agent-rows--plural models "model"))
                              (if (= enabled (length providers)) "" (format " · %d enabled" enabled))))))))
 :body (lambda (_input output)
         (romacs-agent-rows--table
          (mapcar (lambda (p)
                    (let ((models (gethash "models" p)))
                      (list (if (eq (gethash "enabled" p) t) "✓" "·")
                            (or (romacs-agent-rows--field p "providerInstanceId") "?")
                            (or (romacs-agent-rows--field p "harness") "")
                            (format "%d models" (if (vectorp models) (length models) 0)))))
                  (romacs-agent-rows--providers output)))))

(defun romacs-agent-rows--harnesses (output)
  (let ((harnesses (gethash "harnesses" (or (romacs-agent-rows--json output) (make-hash-table)))))
    (and (vectorp harnesses) (cl-remove-if-not #'hash-table-p (append harnesses nil)))))

(defun romacs-agent-rows--windows (account)
  "ACCOUNT's limit windows as \"5h 58%\" cells, as far as they say."
  (let ((windows (gethash "windows" account)))
    (mapconcat (lambda (w)
                 (if (hash-table-p w)
                     (string-trim (format "%s %s" (or (gethash "window" w) "")
                                          (if (numberp (gethash "used" w)) (format "%d%%" (gethash "used" w)) "")))
                   ""))
               (if (vectorp windows) windows nil) "  ")))

(romacs-agent-rows-define
 "romacs_limits"
 :row (lambda (_input) (romacs-agent-rows--romacs "▤ limits"))
 :ended (lambda (_input output)
          (romacs-agent-rows--romacs
           (concat "▤ limits"
                   (if-let* ((suggestion (romacs-agent-rows--field (car (romacs-agent-rows--harnesses output)) "suggestion")))
                       (romacs-agent-rows--dim (concat " · " suggestion))
                     ""))))
 ;; No account, no table: nothing to say.
 :body (lambda (_input output)
         (or (romacs-agent-rows--table
          (cl-loop for harness in (romacs-agent-rows--harnesses output)
                   append (cl-loop for account across (or (gethash "accounts" harness) [])
                                   when (hash-table-p account)
                                   collect (list (if (eq (gethash "pick" account) t) "●" "·")
                                                 (or (romacs-agent-rows--field account "account") "?")
                                                 (romacs-agent-rows--windows account)
                                                 (format "%s sessions" (or (gethash "sessions" account) 0))))))
             "")))

;;; Romacs itself

(romacs-agent-rows-define
 "romacs_restart_request"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (if-let* ((target (romacs-agent-rows--field input "target")))
             (format "↻ restart into %s after this turn" (abbreviate-file-name target))
           "↻ restart after this turn")))
)

(romacs-agent-rows-define
 "romacs_dev_check"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (concat "⚑ dev-check" (if-let* ((tree (romacs-agent-rows--field input "worktree")))
                                   (romacs-agent-rows--dim (concat " · " (file-name-nondirectory (directory-file-name tree))))
                                 ""))))
 ;; The check runs on as a unit: the call answers its start, and the
 ;; task is told its end, or reads the report it names.
 :ended (lambda (_input output)
          (let ((answer (romacs-agent-rows--json output)))
            (romacs-agent-rows--romacs
             (concat "⚑ dev-check " (propertize "started" 'face 'romacs-agent-done)
                     (romacs-agent-rows--dim
                      (if (eq (romacs-agent-rows--get answer "watched") t) " · told at its end"
                        (concat " · report " (abbreviate-file-name (or (romacs-agent-rows--field answer "report") "?")))))))))
 :body (lambda (_input output) (romacs-agent-rows--field (romacs-agent-rows--json output) "next")))

;;; The harnesses' own tools
;;
;; One line each, a glyph and what it touched: a command, a file and its
;; place, a pattern and how much it found.  Codex's calls arrive under the
;; same names (romacs-agent-codex.el).

(defun romacs-agent-rows--directory (path)
  "PATH's directory as a row names it after the file, dim, or nothing."
  (if-let* (((stringp path)) (directory (file-name-directory path)))
      (romacs-agent-rows--dim (concat "  " (abbreviate-file-name (directory-file-name directory))))
    ""))

(defun romacs-agent-rows--path (path &optional span)
  "PATH as a row names it: the file, SPAN (its lines), then its directory."
  (if (not (stringp path)) ""
    (concat (file-name-nondirectory (directory-file-name path)) (or span "")
            (romacs-agent-rows--directory path))))

(defun romacs-agent-rows--count (text)
  "How many lines TEXT has that say something: none when it says it
found none."
  (if (or (not (stringp text)) (string-match-p "\\`[ \t\n]*No \\(?:matches\\|files\\) found" text)) 0
    (with-temp-buffer (insert text) (how-many "^[ \t]*[^ \t\n]" (point-min) (point-max)))))

(defun romacs-agent-rows--lines (text)
  "How many lines TEXT spans, a string's, 0 for none; past
`romacs-agent-rows-read-limit', how many its first part spans."
  (if (or (not (stringp text)) (string-empty-p text)) 0
    ;; One line more than its line breaks, a last one aside.
    (with-temp-buffer
      (insert (romacs-agent-rows-clip text))
      (when (eq (char-before) ?\n) (delete-char -1))
      (1+ (- (count-lines (point-min) (point-max)) (if (or (bobp) (eq (char-before) ?\n)) 0 1))))))

(defconst romacs-agent-rows-read-limit (* 1024 1024)
  "The most characters of a tool's output the chat reads on the main VM.
A longer output is read at its first or last this many only, and its
row says its size, not what its renderer would read in it: drawing a
call costs the same however much it answered.  RET shows all of it.")

(defun romacs-agent-rows-readable-p (text)
  "Whether TEXT is short enough for the chat to read whole
\(`romacs-agent-rows-read-limit')."
  (or (not (stringp text)) (<= (length text) romacs-agent-rows-read-limit)))

(defun romacs-agent-rows-clip (text)
  "TEXT as far as the chat reads it: whole, or its first
`romacs-agent-rows-read-limit' characters."
  (if (romacs-agent-rows-readable-p text) text (substring text 0 romacs-agent-rows-read-limit)))

(defun romacs-agent-rows-size (text)
  "How long TEXT is, as a row says it: \"9.7M characters\"."
  (format "%s characters" (file-size-human-readable (length text) 'si)))

(defun romacs-agent-rows-extent (text)
  "How much TEXT holds, as a row says it: its lines, or its size past
`romacs-agent-rows-read-limit'."
  (if (romacs-agent-rows-readable-p text)
      (romacs-agent-rows--plural (romacs-agent-rows--lines text) "line")
    (romacs-agent-rows-size text)))

(defun romacs-agent-rows-glimpse (text limit &optional width blanks)
  "TEXT's first LIMIT lines, and how many lines it
spans without its trailing BLANKS (characters, line breaks unless
given): (LINES . COUNT).  WIDTH, when given, cuts each line to as many
characters, read no further.  TEXT is read as `romacs-agent-rows-clip'
has it, walked by the buffer's own motion, never split whole in Lisp;
past the bound COUNT is nil."
  (with-temp-buffer
    (insert (romacs-agent-rows-clip text))
    (skip-chars-backward (or blanks "\n"))
    (delete-region (point) (point-max))
    (let ((count (if (bobp) 0 (count-lines (point-min) (point-max))))
          (cut (lambda (start end) (buffer-substring start (if width (min end (+ start width)) end)))))
      (goto-char (point-min))
      (cons (cl-loop repeat (min limit count)
                     collect (prog1 (funcall cut (line-beginning-position) (line-end-position))
                               (forward-line 1)))
            (and (romacs-agent-rows-readable-p text) count)))))

(defun romacs-agent-rows--plural (n word)
  (format "%d %s%s" n word (if (= n 1) "" "s")))

(defun romacs-agent-rows--command (command)
  "COMMAND as a row says it: a string's first line, or an argument list
\(Codex's) as words, a shell's `-c' script alone."
  (cond ((stringp command) (romacs-agent-rows--line command))
        ((and (vectorp command) (cl-every #'stringp command))
         (let ((words (append command nil)))
           (romacs-agent-rows--line
            (if (and (= (length words) 3) (member (nth 1 words) '("-c" "-lc")))
                (nth 2 words)
              (string-join words " ")))))
        (t "")))

(defun romacs-agent-rows--shell (input)
  "A shell call's row: its command, or, when that is longer than a row
holds, the description its caller gave (RET shows the command)."
  (let ((command (romacs-agent-rows--command (romacs-agent-rows--get input "command")))
        (described (romacs-agent-rows--field input "description")))
    (concat "$ " (if (and described (> (string-width command) 60))
                     (romacs-agent-rows--line described)
                   (romacs-lexical-string 'shell command)))))

(defcustom romacs-agent-rows-command-outcomes
  '(("\\_<git\\(?: -C [^ ]+\\)? commit\\_>" "^\\[[^] ]+ \\([0-9a-f]\\{7,\\}\\)\\] \\(.*\\)$" "✓ committed %s %s")
    ("\\_<git\\(?: -C [^ ]+\\)? merge\\_>" "^Merge made by\\|^Fast-forward" "✓ merged")
    ("\\_<forge submit\\_>" "^\\([0-9]\\{8\\}T[0-9]\\{6\\}-[^ \n]+\\)$" "⇡ queued %s")
    ("\\_<forge watch\\_>" "^landed +[^ ]+ +\\([0-9a-f]\\{7\\}\\)" "✓ landed %s"))
  "What a shell command's row says it came to: each (COMMAND OUTPUT
FORMAT), COMMAND a regexp its command matches, OUTPUT one its output
matches, FORMAT given OUTPUT's groups.  The first that matches says it."
  :type '(repeat (list regexp regexp string))
  :group 'romacs-agent)

(defun romacs-agent-rows--command-outcome (command output)
  "What COMMAND came to, its OUTPUT read by
`romacs-agent-rows-command-outcomes', or nil."
  (and (stringp output)
       (seq-some (pcase-lambda (`(,command-re ,output-re ,format))
                   (when (and (string-match-p command-re command) (string-match output-re output))
                     (let ((groups (cl-loop for n from 1 while (match-beginning n) collect (match-string n output))))
                       (propertize (apply #'format format groups) 'face 'romacs-agent-done))))
                 romacs-agent-rows-command-outcomes)))

(romacs-agent-rows-define
 "Bash"
 :row #'romacs-agent-rows--shell
 :ended (lambda (input output)
          (when-let* ((said (romacs-agent-rows--command-outcome (romacs-agent-rows--command (romacs-agent-rows--get input "command")) output)))
            (concat (romacs-agent-rows--shell input) (romacs-agent-rows--dim " · ") said))))

(defun romacs-agent-rows--read-span (input)
  "Which of a file's lines a Read call took, as words, or nil for all."
  (let ((offset (romacs-agent-rows--get input "offset")) (limit (romacs-agent-rows--get input "limit")))
    (cond ((and (numberp offset) (numberp limit)) (format ":%d-%d" offset (+ offset limit -1)))
          ((numberp offset) (format ":%d-" offset))
          ((numberp limit) (format ":1-%d" limit)))))

(romacs-agent-rows-define
 "Read"
 :row (lambda (input)
        (concat "◧ " (romacs-agent-rows--path (romacs-agent-rows--field input "file_path")
                                             (romacs-agent-rows--read-span input))))
 ;; How much it read, never what: RET shows that.
 :ended (lambda (input output)
          (concat "◧ " (romacs-agent-rows--path (romacs-agent-rows--field input "file_path")
                                               (romacs-agent-rows--read-span input))
                  (romacs-agent-rows--dim (concat " · " (romacs-agent-rows-extent output))))))

(defun romacs-agent-rows--change (edits)
  "EDITS, (OLD . NEW) pairs, as the lines they add and take: \"+A −R\"."
  (let ((added (apply #'+ (mapcar (lambda (edit) (romacs-agent-rows--lines (cdr edit))) edits)))
        (removed (apply #'+ (mapcar (lambda (edit) (romacs-agent-rows--lines (car edit))) edits))))
    (concat (propertize (format "+%d" added) 'face 'romacs-agent-done) " "
            (propertize (format "−%d" removed) 'face 'romacs-agent-error))))

(defun romacs-agent-rows--pair (edit)
  "An edit's (OLD . NEW)."
  (cons (romacs-agent-rows--field edit "old_string") (romacs-agent-rows--field edit "new_string")))

(defun romacs-agent-rows--edit (input)
  "An Edit, MultiEdit or Codex file change's row."
  (let ((changes (romacs-agent-rows--get input "changes"))
        (edits (romacs-agent-rows--get input "edits")))
    (cond
     ;; Codex: files, each with its diff.
     ((and (vectorp changes) (> (length changes) 0))
      (let* ((first (aref changes 0)) (path (and (hash-table-p first) (gethash "path" first))))
        (concat "✎ " (romacs-agent-rows--path path)
                (if (> (length changes) 1)
                    (romacs-agent-rows--dim (format "  +%d more" (1- (length changes)))) ""))))
     ((vectorp edits)
      (concat "✎ " (romacs-agent-rows--path (romacs-agent-rows--field input "file_path")) "  "
              (romacs-agent-rows--change (mapcar #'romacs-agent-rows--pair edits))))
     (t
      (concat "✎ " (romacs-agent-rows--path (romacs-agent-rows--field input "file_path")) "  "
              (romacs-agent-rows--change (list (romacs-agent-rows--pair input))))))))

(romacs-agent-rows-define "Edit" :row #'romacs-agent-rows--edit)
(romacs-agent-rows-define "MultiEdit" :row #'romacs-agent-rows--edit)

(romacs-agent-rows-define
 "Write"
 :row (lambda (input)
        (concat "✎ " (romacs-agent-rows--path (romacs-agent-rows--field input "file_path"))
                (romacs-agent-rows--dim
                 (format "  written · %s" (romacs-agent-rows-extent (or (romacs-agent-rows--field input "content") "")))))))

(defun romacs-agent-rows--where (input)
  "Where a search looked: its path or glob, dim, or nothing."
  (let ((where (or (romacs-agent-rows--field input "path") (romacs-agent-rows--field input "glob"))))
    (if (stringp where) (romacs-agent-rows--dim (concat "  in " (abbreviate-file-name where))) "")))

(defun romacs-agent-rows--found (unit)
  "A search's :ended: its row, then how many UNITs its output names."
  (lambda (input output)
    (concat (romacs-agent-rows--searched input)
            (romacs-agent-rows--dim (concat " · " (romacs-agent-rows--plural (romacs-agent-rows--count output) unit))))))

(defun romacs-agent-rows--searched (input)
  (concat "⌕ " (romacs-agent-rows--line (or (romacs-agent-rows--field input "pattern") ""))
          (romacs-agent-rows--where input)))

(romacs-agent-rows-define "Grep" :row #'romacs-agent-rows--searched :ended (romacs-agent-rows--found "line"))
(romacs-agent-rows-define "Glob" :row #'romacs-agent-rows--searched :ended (romacs-agent-rows--found "file"))

(romacs-agent-rows-define
 "WebFetch"
 :row (lambda (input)
        (let ((url (or (romacs-agent-rows--field input "url") "")))
          (concat "↗ " (replace-regexp-in-string "\\`https?://\\(www\\.\\)?" "" url)))))

(romacs-agent-rows-define
 "WebSearch"
 :row (lambda (input) (concat "◍ " (romacs-agent-rows--line (or (romacs-agent-rows--field input "query") "")))))

(romacs-agent-rows-define
 "TodoWrite"
 :row (lambda (input)
        (let* ((todos (romacs-agent-rows--get input "todos"))
               (todos (if (vectorp todos) (append todos nil) nil))
               (done (cl-count-if (lambda (todo) (equal (romacs-agent-rows--field todo "status") "completed")) todos))
               (now (cl-find-if (lambda (todo) (equal (romacs-agent-rows--field todo "status") "in_progress")) todos)))
          (concat (format "☐ %d/%d done" done (length todos))
                  (if now (romacs-agent-rows--dim (concat "  " (or (romacs-agent-rows--field now "activeForm")
                                                                   (romacs-agent-rows--field now "content") "")))
                    "")))))

(defun romacs-agent-rows--asked (input)
  "An AskUserQuestion's row: its first question."
  (let ((questions (romacs-agent-rows--get input "questions")))
    (concat "? " (if (and (vectorp questions) (> (length questions) 0))
                     (romacs-agent-rows--line (or (romacs-agent-rows--field (aref questions 0) "question") ""))
                   "a question"))))

(romacs-agent-rows-define
 "AskUserQuestion"
 :row #'romacs-agent-rows--asked
 ;; Its answers, as the harness took them: the card above asked.
 :ended (lambda (input output)
          (let ((answers (and (stringp output)
                              (let ((found nil) (at 0))
                                (while (string-match "=\"\\([^\"]*\\)\"" output at)
                                  (push (match-string 1 output) found)
                                  (setq at (match-end 0)))
                                (nreverse found)))))
            (and answers
                 (concat (romacs-agent-rows--asked input)
                         (romacs-agent-rows--dim (concat "  → " (string-join answers ", "))))))))

;;; Romacs's other tools

(defun romacs-agent-rows--brief (input)
  "INPUT's values but the instance, as words: what a call was given."
  (let ((values nil))
    (when (hash-table-p input)
      (maphash (lambda (key value)
                 (unless (equal key "instance")
                   (let ((text (cond ((stringp value) value) ((numberp value) (number-to-string value))
                                     ((eq value t) key))))
                     (when text (push (replace-regexp-in-string "[ \t\n]+" " " (string-trim text)) values)))))
               input))
    (string-join (nreverse values) " · ")))

(defun romacs-agent-rows--romacs-generic (name)
  "The renderer of a Romacs tool NAME with none of its own: its name, as
words, and what it was given."
  (list :row (lambda (input)
               (let ((brief (romacs-agent-rows--brief input)))
                 (romacs-agent-rows--romacs
                  (concat (string-replace "_" " " (string-remove-prefix "romacs_" name))
                          (if (string-empty-p brief) "" (romacs-agent-rows--dim (concat "  " brief)))))))
        :body (lambda (_input output) (romacs-agent-rows-words output))))

(defun romacs-agent-rows-texts (name input output)
  "What a call of tool NAME with INPUT (a table) shows on its row, as a
turn's row keeps it: a plist of :row, its text as it starts, and :ended,
its text once it ended with OUTPUT (nil when its renderer says none, or
OUTPUT is nil).  Pure: what a chat drawn from rows shows without the
call's whole input or output."
  (when-let* ((renderer (romacs-agent-rows-get (romacs-agent-rows-name name))))
    (let ((input (if (hash-table-p input) input (make-hash-table :test #'equal))))
      (list :row (funcall (plist-get renderer :row) input)
            :ended (and output (romacs-agent-rows-readable-p output) (plist-get renderer :ended)
                        (funcall (plist-get renderer :ended) input output))))))

;; A turn's row keeps its calls' texts as drawn (romacs-agent-turns.el).
(defvar romacs-agent-turns-tool-texts-function)
(setq romacs-agent-turns-tool-texts-function #'romacs-agent-rows-texts)

(provide 'romacs-agent-rows)
;;; romacs-agent-rows.el ends here

;;; romacs-agent-rows.el --- How a chat draws a tool's call -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat (romacs-agent-ui.el) draws each tool call as a row of its turn's
;; run.  A tool with a renderer here gets a designed row; any other, its
;; name and a brief of its input.  Romacs's own tools all read as what
;; they did, never as JSON, behind Romacs's icon.  A new tool adds a
;; renderer, never a case in the chat.
;;
;; A renderer is a plist (`romacs-agent-rows-define'):
;;   :row FN       (INPUT): the row's text as the call starts.
;;   :ended FN     (INPUT OUTPUT): its text once the call ended well, or
;;                 nil to keep the first.
;;   :body FN      (INPUT OUTPUT): what goes under the row (default OUTPUT).
;;   :shown        the body shows, as a message sent does; else it is folded.
;;   :card         the row shows while its run is folded; OUTPUT is a task
;;                 record, whose state and thread the chat adds to it.
;;   :answers FN   (INPUT): the id of the message the call answers.
;; INPUT is the call's arguments, a hash table; OUTPUT its result's text.
;; The functions are pure: they read the session table at most, and the
;; chat draws what they return.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-icons)

(defvar romacs-agent-rows (make-hash-table :test #'equal)
  "Each tool's renderer, by the tool's name (`romacs-agent-rows-define').")

(defun romacs-agent-rows-define (name &rest renderer)
  "Draw tool NAME's calls by RENDERER, a plist of :row :ended :body
:shown :card :answers (this file's Commentary)."
  (puthash name renderer romacs-agent-rows))

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

;;; Pieces

(defface romacs-agent-romacs-tool '((t :inherit font-lock-keyword-face))
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
  "OUTPUT read as a JSON object, or nil."
  (let ((value (ignore-errors (json-parse-string output :null-object nil :false-object nil))))
    (and (hash-table-p value) value)))

(defun romacs-agent-rows--line (text)
  "TEXT's first line, trimmed."
  (car (split-string (string-trim (or text "")) "\n")))

(defun romacs-agent-rows--dim (text)
  (propertize text 'face 'romacs-agent-dim))

(defun romacs-agent-rows--title (id)
  "Task ID's title, while the session is known."
  (or (and (stringp id) (and-let* ((session (romacs-agent-get id))) (romacs-agent-title session))) "a task"))

(defun romacs-agent-rows-quoted (text)
  "TEXT quoted: each of its lines behind a bar, which is decoration
\(`romacs-agent-decor'), never part of the text one copies."
  (mapconcat (lambda (line)
               (concat (propertize "┃ " 'face 'romacs-agent-message-bar 'romacs-agent-decor t) line))
             (split-string (string-trim-right (or text "")) "\n")
             "\n"))

(defun romacs-agent-rows--instance (model harness provider)
  "MODEL@PROVIDER, the model without its HARNESS's prefix; what of it is
known."
  (let ((model (and model harness (string-remove-prefix (concat harness "-") model))))
    (cond ((and model provider) (format "%s@%s" model provider)) (model) (provider))))

;;; Messages and delegations: cards

(defun romacs-agent-rows--message-body (input _output)
  (romacs-agent-rows-quoted (or (romacs-agent-rows--field input "message") (romacs-agent-rows--field input "text"))))

(romacs-agent-rows-define
 "romacs_task_send"
 :row (lambda (input) (romacs-agent-rows--romacs
                       (concat "✉ to " (propertize (romacs-agent-rows--title (romacs-agent-rows--field input "taskId"))
                                                   'face 'romacs-agent-message))))
 :body #'romacs-agent-rows--message-body :shown t :card t
 :answers (lambda (input) (romacs-agent-rows--field input "inReplyTo")))

(romacs-agent-rows-define
 "romacs_task_reply"
 ;; Whom it goes to is the answer's: the sender's task record.
 :row (lambda (_input) (romacs-agent-rows--romacs (concat "✉ " (propertize "reply" 'face 'romacs-agent-message))))
 :ended (lambda (_input output)
          (and-let* ((title (romacs-agent-rows--field (romacs-agent-rows--json output) "title")))
            (romacs-agent-rows--romacs (concat "✉ to " (propertize title 'face 'romacs-agent-message)))))
 :body #'romacs-agent-rows--message-body :shown t :card t
 :answers (lambda (input) (romacs-agent-rows--field input "messageId")))

(defun romacs-agent-rows--delegated (title instance)
  (concat "⇢ " (propertize title 'face 'romacs-agent-delegate)
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
               (romacs-agent-rows--delegated
                (or (romacs-agent-rows--field record "title") (romacs-agent-rows--field input "title") "task")
                (romacs-agent-rows--instance
                 (or (romacs-agent-rows--field record "model") (romacs-agent-rows--field target "model"))
                 (romacs-agent-rows--field record "harness")
                 (or (romacs-agent-rows--field record "provider") (romacs-agent-rows--field target "provider"))))))))
 ;; A `wait' answers with the task's result: the card holds it.
 :body (lambda (_input output) (or (romacs-agent-rows--field (romacs-agent-rows--json output) "result") ""))
 :card t)

;; A harness's own subagent: the same card, with no task of ours.
(dolist (name '("Agent" "Task"))
  (romacs-agent-rows-define
   name
   :row (lambda (input) (romacs-agent-rows--delegated (or (romacs-agent-rows--field input "description") "task")
                                                       (romacs-agent-rows--field input "subagent_type")))
   :card t))

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
                         (romacs-agent-rows--dim (concat " · " status))
                       "")))))
 :body (lambda (_input output) (or (romacs-agent-rows--field (romacs-agent-rows--json output) "result") "")))

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
           (if (vectorp watching) (mapconcat (lambda (w) (format "%s" w)) watching "\n")
             (romacs-agent-rows-words output)))))

;;; Lisp

(defvar romacs-agent-rows--lisp-buffer nil
  "The hidden buffer forms are coloured in: one, its mode made once.")

(defun romacs-agent-rows-lisp (code)
  "CODE coloured as `emacs-lisp-mode' colours it."
  (unless (buffer-live-p romacs-agent-rows--lisp-buffer)
    (setq romacs-agent-rows--lisp-buffer (generate-new-buffer " *romacs-agent-lisp*"))
    (with-current-buffer romacs-agent-rows--lisp-buffer (delay-mode-hooks (emacs-lisp-mode))))
  (with-current-buffer romacs-agent-rows--lisp-buffer
    (erase-buffer)
    (insert code)
    (font-lock-ensure)
    (buffer-string)))

(romacs-agent-rows-define
 "romacs_eval"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (concat "λ " (romacs-agent-rows-lisp
                       (replace-regexp-in-string "[ \t\n]+" " " (string-trim (or (romacs-agent-rows--field input "code") "")))))))
 ;; A form its row holds whole is said once, there: under it, what it
 ;; gave.  One of several lines, or long, is there whole too.
 :body (lambda (input output)
         (let ((code (string-trim (or (romacs-agent-rows--field input "code") ""))))
           (concat (if (or (string-search "\n" code) (> (string-width code) 60))
                       (concat (romacs-agent-rows-lisp code) "\n")
                     "")
                   (romacs-agent-rows--dim "⇒ ") output))))

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
          (let ((providers (romacs-agent-rows--providers output)))
            (romacs-agent-rows--romacs
             (concat (format "▤ %d instances" (length providers))
                     (romacs-agent-rows--dim (format " · %d enabled" (cl-count-if (lambda (p) (eq (gethash "enabled" p) t))
                                                                                  providers)))))))
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
 :body (lambda (_input output)
         (or (romacs-agent-rows--field (romacs-agent-rows--json output) "status") (romacs-agent-rows-words output))))

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
 :body (lambda (_input output) (or (romacs-agent-rows--field (romacs-agent-rows--json output) "next")
                                  (romacs-agent-rows-words output))))

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
    (cl-count-if (lambda (line) (string-match-p "[^ \t]" line)) (split-string text "\n"))))

(defun romacs-agent-rows--lines (text)
  "How many lines TEXT spans, a string's, 0 for none."
  (if (or (not (stringp text)) (string-empty-p text)) 0 (1+ (cl-count ?\n (string-trim-right text "\n")))))

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

(romacs-agent-rows-define
 "Bash"
 :row (lambda (input) (concat "$ " (romacs-agent-rows--command (romacs-agent-rows--get input "command")))))

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
                                             (romacs-agent-rows--read-span input)))))

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
                 (format "  written · %s" (romacs-agent-rows--plural
                                           (romacs-agent-rows--lines (romacs-agent-rows--field input "content")) "line"))))))

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

(provide 'romacs-agent-rows)
;;; romacs-agent-rows.el ends here

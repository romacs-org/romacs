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
romacs_ tool, or nil, the chat's generic row."
  (or (gethash name romacs-agent-rows)
      (and (stringp name) (string-prefix-p "romacs_" name) (romacs-agent-rows--romacs-generic name))))

;;; Pieces

(defun romacs-agent-rows--romacs (text)
  "TEXT behind Romacs's icon, where it can be drawn."
  (if-let* ((icon (romacs-icon 'romacs))) (concat icon " " text) text))

(defun romacs-agent-rows--field (table key)
  "TABLE's KEY when TABLE is a hash table and KEY holds a string."
  (and (hash-table-p table) (stringp (gethash key table)) (gethash key table)))

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
 :body (lambda (_input output) (or (romacs-agent-rows--field (romacs-agent-rows--json output) "result") output)))

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
           (if (vectorp watching) (mapconcat (lambda (w) (format "%s" w)) watching "\n") output))))

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
 :body (lambda (input output)
         (concat (romacs-agent-rows-lisp (string-trim (or (romacs-agent-rows--field input "code") "")))
                 "\n" (romacs-agent-rows--dim "⇒ ") output)))

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
 :body (lambda (_input output)
         (romacs-agent-rows--table
          (cl-loop for harness in (romacs-agent-rows--harnesses output)
                   append (cl-loop for account across (or (gethash "accounts" harness) [])
                                   when (hash-table-p account)
                                   collect (list (if (eq (gethash "pick" account) t) "●" "·")
                                                 (or (romacs-agent-rows--field account "account") "?")
                                                 (romacs-agent-rows--windows account)
                                                 (format "%s sessions" (or (gethash "sessions" account) 0))))))))

;;; Romacs itself

(romacs-agent-rows-define
 "romacs_restart_request"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (if-let* ((target (romacs-agent-rows--field input "target")))
             (format "↻ restart into %s after this turn" (abbreviate-file-name target))
           "↻ restart after this turn")))
 :body (lambda (_input output)
         (or (romacs-agent-rows--field (romacs-agent-rows--json output) "status") output)))

(defun romacs-agent-rows--stages (report)
  "REPORT's stages, (NAME . OK) for each line \"NAME: ok|...\" but its verdict."
  (cl-loop for line in (split-string (or report "") "\n")
           when (and (string-match "\\`\\([a-z][a-z-]*\\): \\([A-Za-z]+\\)" line)
                     (not (equal (match-string 1 line) "dev-check")))
           collect (cons (match-string 1 line) (equal (downcase (match-string 2 line)) "ok"))))

(romacs-agent-rows-define
 "romacs_dev_check"
 :row (lambda (input)
        (romacs-agent-rows--romacs
         (concat "⚑ dev-check" (if-let* ((tree (romacs-agent-rows--field input "worktree")))
                                   (romacs-agent-rows--dim (concat " · " (file-name-nondirectory (directory-file-name tree))))
                                 ""))))
 :ended (lambda (_input output)
          (let* ((answer (romacs-agent-rows--json output))
                 (stages (romacs-agent-rows--stages (romacs-agent-rows--field answer "report")))
                 (failed (cl-remove-if #'cdr stages))
                 (verdict (or (romacs-agent-rows--field answer "verdict") "?")))
            (romacs-agent-rows--romacs
             (concat "⚑ dev-check " (propertize verdict 'face (if (equal verdict "ok") 'romacs-agent-done 'romacs-agent-error))
                     (romacs-agent-rows--dim
                      (if failed (concat " · " (mapconcat #'car failed ", "))
                        (format " · %d/%d stages" (length stages) (length stages))))))))
 :body (lambda (_input output) (or (romacs-agent-rows--field (romacs-agent-rows--json output) "report") output)))

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
                          (if (string-empty-p brief) "" (romacs-agent-rows--dim (concat "  " brief)))))))))

(provide 'romacs-agent-rows)
;;; romacs-agent-rows.el ends here

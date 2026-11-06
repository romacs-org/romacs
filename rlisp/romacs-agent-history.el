;;; romacs-agent-history.el --- The harnesses' own sessions, read and resumed -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Claude Code and Codex keep every conversation they run in their
;; configuration directories: Claude Code a transcript per session under
;; projects/, Codex a rollout per thread under sessions/.  This lists
;; them per provider instance (and each harness's default directory),
;; renders one read-only in the chat's own look, and resumes one as a
;; Romacs session through the harness's own resume.
;;
;; Harness files are only ever read.  Listing is directory metadata; the
;; newest `romacs-agent-history-limit' transcripts of a directory are
;; described from a bounded head of each, cached by size and time, so a
;; redraw reads nothing new.  A view renders the end of a transcript, at
;; most `romacs-agent-history-view-bytes' of it.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-ui)

(defcustom romacs-agent-history-limit 50
  "How many of a directory's newest transcripts the home page lists."
  :type 'integer :group 'romacs-agent)

(defcustom romacs-agent-history-per-project 3
  "How many history rows a project shows until `h' shows them all."
  :type 'integer :group 'romacs-agent)

(defcustom romacs-agent-history-view-bytes (* 2 1024 1024)
  "How much of a transcript's end its view renders, in bytes."
  :type 'integer :group 'romacs-agent)

(defconst romacs-agent-history--heads '(131072 524288 1048576)
  "The heads read, in turn, until one names its transcript: Codex writes
its instructions before the first prompt.")

(defvar romacs-agent-history--descriptions (make-hash-table :test #'equal)
  "A transcript's path to (SIZE MTIME . DESCRIPTION), DESCRIPTION nil for
one with nothing to resume.")

(defvar romacs-agent-history--listing nil
  "The transcripts the last listing described, newest first per directory.")

;;; Reading

(defun romacs-agent-history--read (file beg end keep)
  "The JSON objects of FILE's complete lines between bytes BEG and END, in
order, that KEEP accepts: it is called with a line's first \"type\" and
the line's end, point after that type, so it may look further.  A line
BEG or END cuts is left out."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    ;; From the byte before BEG: a line starting at BEG is whole.
    (insert-file-contents-literally file nil (max 0 (1- beg)) end)
    (goto-char (point-min))
    ;; Cut at the start: a line's tail, or the newline before BEG.
    (when (> beg 0) (forward-line 1))
    (let ((objects nil))
      (while (search-forward "\n" nil t)
        (let ((line-end (1- (point))) (line-start (line-beginning-position 0)))
          (goto-char line-start)
          (when (and (re-search-forward "\"type\":\"\\([^\"]*\\)\"" line-end t)
                     (funcall keep (match-string 1) line-end))
            (let ((object (ignore-errors
                            (json-parse-string (decode-coding-string (buffer-substring-no-properties line-start line-end)
                                                                     'utf-8)
                                               :object-type 'hash-table :null-object :null :false-object :false))))
              (when (hash-table-p object) (push object objects))))
          (goto-char (1+ line-end))))
      (nreverse objects))))

(defun romacs-agent-history--payload-type (line-end)
  "The next \"type\" on the line, before LINE-END: a Codex payload's."
  (and (re-search-forward "\"type\":\"\\([^\"]*\\)\"" line-end t) (match-string 1)))

(defun romacs-agent-history--title (text)
  "TEXT's first line, as a title (`romacs-agent--task-line'); nil for
none, or for a harness's own injected context (a <tag>, AGENTS.md's
instructions)."
  (let ((text (and (stringp text) (string-trim text))))
    (unless (or (null text) (string-empty-p text) (string-prefix-p "<" text)
                (string-prefix-p "# AGENTS.md instructions" text))
      (truncate-string-to-width (romacs-agent--task-line text) 60 nil nil "…"))))

(defun romacs-agent-history--prompt (content)
  "The user's typed text in a Claude Code message CONTENT, or nil (a tool
result, an injected context)."
  (cond ((stringp content) (and (romacs-agent-history--title content) content))
        ((vectorp content)
         (unless (cl-some (lambda (block) (and (hash-table-p block) (equal (gethash "type" block) "tool_result"))) content)
           (let ((text (romacs-agent--text-of content)))
             (and (romacs-agent-history--title text) text))))))

(defun romacs-agent-history--time (stamp)
  (and (stringp stamp) (ignore-errors (float-time (date-to-time stamp)))))

;;; Claude Code

(defun romacs-agent-history--claude-keep (type _line-end)
  ;; An assistant line's first type is its message's.
  (member type '("user" "message" "assistant")))

(defun romacs-agent-history--claude-files (dir _limit)
  "Claude Code's transcripts under DIR: (PATH SIZE MTIME)."
  (let ((projects (expand-file-name "projects" dir)) (files nil))
    (when (file-directory-p projects)
      (dolist (project (directory-files projects t "\\`[^.]"))
        (when (file-directory-p project)
          (dolist (entry (directory-files-and-attributes project t "\\`[0-9a-f-]\\{36\\}\\.jsonl\\'" t))
            (push (list (car entry) (file-attribute-size (cdr entry))
                        (float-time (file-attribute-modification-time (cdr entry))))
                  files)))))
    files))

(defun romacs-agent-history--claude-describe (objects path)
  "A Claude Code transcript's description from its head's OBJECTS, or nil
while they name no prompt."
  (let ((cwd nil) (title nil))
    (cl-loop for line in objects
             when (and (equal (gethash "type" line) "user") (not (eq (gethash "isSidechain" line) t))) do
             (setq cwd (or cwd (romacs-agent--value (gethash "cwd" line))))
             (unless (or title (eq (gethash "isMeta" line) t))
               (setq title (romacs-agent-history--title
                            (romacs-agent-history--prompt (gethash "content" (gethash "message" line))))))
             until title)
    (when (and cwd title)
      (list :harness 'claude :id (file-name-base path) :cwd cwd :title title))))

(defun romacs-agent-history--claude-events (objects)
  "A Claude Code transcript's OBJECTS as chat events."
  (let ((scratch (romacs-agent--make :harness 'claude)) (events nil) (turn nil) (last nil) (model nil))
    (cl-flet ((close ()
                (when turn
                  (push (list :type 'done :duration (let ((end (romacs-agent-history--time last))
                                                          (start (romacs-agent-history--time turn)))
                                                      (and end start (max 0 (- end start)))))
                        events)
                  (setq turn nil))))
      (dolist (line objects)
        (unless (or (eq (gethash "isSidechain" line) t) (eq (gethash "isMeta" line) t)
                    (eq (gethash "isCompactSummary" line) t))
          (pcase (gethash "type" line)
            ("user"
             (let* ((content (gethash "content" (gethash "message" line)))
                    (prompt (romacs-agent-history--prompt content)))
               (cond (prompt
                      (close)
                      (setq turn (gethash "timestamp" line))
                      (push (list :type 'meta :permission-mode (gethash "permissionMode" line)) events)
                      (push (list :type 'input :text prompt) events))
                     ((vectorp content)
                      (dolist (event (romacs-agent--claude-events scratch line))
                        (when (eq (plist-get event :type) 'tool) (push event events)))))))
            ("assistant"
             (let ((name (gethash "model" (gethash "message" line))))
               (when (and (stringp name) (not (equal name model)) (not (string-prefix-p "<" name)))
                 (setq model name)
                 (push (list :type 'meta :model name) events)))
             (dolist (event (romacs-agent--claude-events scratch line)) (push event events))))
          (setq last (or (gethash "timestamp" line) last))))
      (close))
    (nreverse events)))

;;; Codex

(defun romacs-agent-history--codex-keep (type line-end)
  (pcase type
    ((or "session_meta" "turn_context") t)
    ("response_item"
     (member (romacs-agent-history--payload-type line-end)
             '("message" "function_call" "function_call_output" "custom_tool_call" "custom_tool_call_output"
               "local_shell_call" "web_search_call")))
    ("event_msg"
     (pcase (romacs-agent-history--payload-type line-end)
       ((or "user_message" "task_complete" "token_count") t)
       ("item_completed" (save-excursion (re-search-forward "\"item\":{\"type\":\"UserMessage\"" line-end t)))))))

(defun romacs-agent-history--codex-files (dir limit)
  "Codex's rollouts under DIR, newest days first, until LIMIT are found:
\(PATH SIZE MTIME)."
  (let ((files nil) (count 0))
    (cl-flet ((under (parent) (and (file-directory-p parent) (reverse (directory-files parent t "\\`[0-9]+\\'")))))
      (catch 'enough
        (dolist (year (under (expand-file-name "sessions" dir)))
          (dolist (month (under year))
            (dolist (day (under month))
              (dolist (entry (directory-files-and-attributes day t "\\`rollout-.*\\.jsonl\\'" t))
                (cl-incf count)
                (push (list (car entry) (file-attribute-size (cdr entry))
                            (float-time (file-attribute-modification-time (cdr entry))))
                      files))
              (when (>= count limit) (throw 'enough nil)))))))
    files))

(defun romacs-agent-history--codex-prompt (payload)
  "The user's typed text in a Codex event PAYLOAD, or nil."
  (pcase (gethash "type" payload)
    ("user_message" (romacs-agent--value (gethash "message" payload)))
    ("item_completed" (romacs-agent--text-of (gethash "content" (gethash "item" payload))))))

(defun romacs-agent-history--codex-describe (objects path)
  (let ((thread nil) (cwd nil) (title nil))
    (cl-loop for line in objects
             for payload = (gethash "payload" line)
             when (hash-table-p payload) do
             (pcase (gethash "type" line)
               ("session_meta" (setq thread (romacs-agent--value (gethash "id" payload))
                                     cwd (romacs-agent--value (gethash "cwd" payload))))
               ("event_msg" (setq title (or title (romacs-agent-history--title (romacs-agent-history--codex-prompt payload))))))
             until title)
    (setq thread (or thread (and (string-match "\\([0-9a-f]\\{8\\}-[0-9a-f-]\\{27\\}\\)\\.jsonl\\'" path)
                                 (match-string 1 path))))
    (when (and (romacs-agent--uuid-p thread) cwd title)
      (list :harness 'codex :id thread :cwd cwd :title title))))

(defun romacs-agent-history--codex-input (payload)
  (let ((arguments (gethash "arguments" payload)) (input (gethash "input" payload)))
    (cond ((stringp arguments)
           (let ((object (ignore-errors (json-parse-string arguments :object-type 'hash-table
                                                           :null-object :null :false-object :false))))
             (if (hash-table-p object)
                 (progn (when (and (gethash "cmd" object) (not (gethash "command" object)))
                          (puthash "command" (gethash "cmd" object) object))
                        object)
               (romacs-mcp--object "code" arguments))))
          ((stringp input) (romacs-mcp--object "code" input))
          (t nil))))

(defun romacs-agent-history--codex-output (output)
  (cond ((stringp output) output)
        ((vectorp output) (romacs-agent--text-of output))
        ((hash-table-p output) (or (gethash "output" output) (romacs-agent--text-of (or (gethash "content" output) ""))))
        (t "")))

(defun romacs-agent-history--codex-events (objects)
  "A Codex rollout's OBJECTS as chat events."
  (let ((events nil) (turn nil) (last nil) (model nil) (messages 0))
    (cl-flet ((close ()
                (when turn
                  (push (list :type 'done :duration (let ((end (romacs-agent-history--time last))
                                                          (start (romacs-agent-history--time turn)))
                                                      (and end start (max 0 (- end start)))))
                        events)
                  (setq turn nil))))
      (dolist (line objects)
        (let ((payload (gethash "payload" line)))
          (when (hash-table-p payload)
            (setq last (or (gethash "timestamp" line) last))
            (pcase (list (gethash "type" line) (gethash "type" payload))
              (`("turn_context" ,_)
               (let ((name (romacs-agent--value (gethash "model" payload))))
                 (when (and (stringp name) (not (equal name model)))
                   (setq model name)
                   (push (list :type 'meta :model name) events))))
              (`("event_msg" "task_complete") (close))
              (`("event_msg" "token_count")
               (let* ((info (gethash "info" payload))
                      (usage (and (hash-table-p info) (gethash "last_token_usage" info))))
                 (when (hash-table-p usage)
                   (push (list :type 'meta :context-used (gethash "input_tokens" usage)
                               :context-window (romacs-agent--value (gethash "model_context_window" info)))
                         events))))
              (`("event_msg" ,_)
               (when-let* ((prompt (romacs-agent-history--codex-prompt payload))
                           ((romacs-agent-history--title prompt)))
                 (close)
                 (setq turn (gethash "timestamp" line))
                 (push (list :type 'input :text prompt) events)))
              (`("response_item" "message")
               (when (equal (gethash "role" payload) "assistant")
                 (let ((id (format "history:%d" (cl-incf messages)))
                       (text (romacs-agent--text-of (gethash "content" payload))))
                   (unless (string-empty-p text)
                     (push (list :type 'delta :text (concat text "\n") :message id) events)
                     (push (list :type 'message-end :message id) events)))))
              (`("response_item" ,(or "function_call" "custom_tool_call"))
               (push (list :type 'tool :state 'started :id (gethash "call_id" payload)
                           ;; A shell command by the name `codex exec' gives it.
                           :name (if (member (gethash "name" payload) '("exec_command" "shell")) "Bash"
                                   (gethash "name" payload))
                           :input (romacs-agent-history--codex-input payload))
                     events))
              (`("response_item" ,(or "function_call_output" "custom_tool_call_output"))
               (push (list :type 'tool :state 'completed :id (gethash "call_id" payload)
                           :output (romacs-agent-history--codex-output (gethash "output" payload)))
                     events))
              (`("response_item" ,type)
               (let ((id (or (romacs-agent--value (gethash "call_id" payload)) (romacs-agent--value (gethash "id" payload)))))
                 (push (list :type 'tool :state 'started :id id :name type :input nil) events)
                 (push (list :type 'tool :state 'completed :id id :output "") events)))))))
      (close))
    (nreverse events)))

;;; Listing

(defun romacs-agent-history--get (harness key)
  "How HARNESS's history is read: its driver's :history KEY."
  (plist-get (plist-get (romacs-agent--driver-of harness) :history) key))

(defun romacs-agent-history--places ()
  "The directories holding history, each once: (DIR HARNESS PROVIDER
CONFIG-DIR), every instance's own and its harness's default (under the
first instance of that harness, run with the default directory)."
  (let ((places nil))
    (dolist (provider (romacs-agent-instances))
      (let* ((harness (plist-get provider :harness))
             (default (romacs-agent--home harness nil))
             (config-dir (romacs-agent--config-dir provider)))
        (when (and (plist-get provider :enabled) (plist-get (romacs-agent--driver-of harness) :history))
          (push (list (or config-dir default) harness (plist-get provider :name) config-dir) places)
          (push (list default harness (plist-get provider :name) nil) places))))
    (let ((seen (make-hash-table :test #'equal)))
      (cl-remove-if (lambda (place) (prog1 (gethash (car place) seen) (puthash (car place) t seen)))
                    (nreverse places)))))

(defun romacs-agent-history--describe (path size mtime harness)
  "PATH's description, read from its head once per SIZE and MTIME."
  (let ((cached (gethash path romacs-agent-history--descriptions)))
    (if (and cached (eql (car cached) size) (equal (cadr cached) mtime))
        (cddr cached)
      (let ((description nil) (heads romacs-agent-history--heads))
        (while heads
          (setq description (funcall (romacs-agent-history--get harness :describe)
                                     (romacs-agent-history--read path 0 (min size (car heads))
                                                                 (romacs-agent-history--get harness :keep))
                                     path))
          (setq heads (and (not description) (> size (car heads)) (cdr heads))))
        (puthash path (cons size (cons mtime description)) romacs-agent-history--descriptions)
        description))))

(declare-function romacs-agent-t3-thread-titles "romacs-agent-t3" (store))
(declare-function romacs-agent-t3-title-key "romacs-agent-t3" (instance native))

(defun romacs-agent-history-list ()
  "List the harnesses' transcripts anew: the newest of each directory,
described (from their heads, once per size and time).  A transcript of
an instance T3 Code configures takes the title T3 gave its thread, when
T3 ran it: its first prompt is then T3's preamble."
  (setq romacs-agent-history--listing
        (cl-loop with from-t3 = (let ((table (make-hash-table :test #'equal)))
                                  ;; Each T3 instance's id and thread store, by name.
                                  (dolist (i (romacs-agent-instances) table)
                                    (when (plist-get i :t3-store)
                                      (puthash (plist-get i :name) (cons (plist-get i :t3-id) (plist-get i :t3-store)) table))))
                 with stores = (make-hash-table :test #'equal)
                 for (dir harness provider config-dir) in (romacs-agent-history--places)
                 for t3 = (gethash provider from-t3)
                 ;; One read of each store a refresh, failed or not.
                 for titles = (and t3 (let ((kept (gethash (cdr t3) stores 'unread)))
                                        (if (not (eq kept 'unread)) kept
                                          (require 'romacs-agent-t3)
                                          (puthash (cdr t3) (romacs-agent-t3-thread-titles (cdr t3)) stores))))
                 append
                 (let ((files (sort (funcall (romacs-agent-history--get harness :files) dir romacs-agent-history-limit)
                                    (lambda (a b) (> (nth 2 a) (nth 2 b))))))
                   (cl-loop for (path size mtime) in (seq-take files romacs-agent-history-limit)
                            for description = (romacs-agent-history--describe path size mtime harness)
                            when description
                            collect (append (let ((title (and titles
                                                              (gethash (romacs-agent-t3-title-key (car t3) (plist-get description :id))
                                                                       titles))))
                                              ;; T3's title as T3 shows it, cut to fit.
                                              (and title (not (string-empty-p (string-trim title)))
                                                   (list :title (truncate-string-to-width (string-trim title) 60 nil nil "…"))))
                                            (list :path path :size size :mtime mtime
                                                  :provider provider :config-dir config-dir)
                                            description))))))

(defun romacs-agent-history-items ()
  "The listed transcripts no Romacs session continues, newest first."
  (let ((known (make-hash-table :test #'equal)))
    ;; A conversation by its identity: two configuration directories can
    ;; hold the same id.
    (maphash (lambda (_id session)
               (puthash (romacs-agent--native-key (romacs-agent-harness session) (romacs-agent-config-dir session)
                                                  (romacs-agent--native-id session))
                        t known)
               (when-let* ((imported (romacs-agent-imported session))) (puthash (plist-get imported :path) t known)))
             romacs-agent--sessions)
    (sort (cl-remove-if (lambda (item)
                          (or (gethash (romacs-agent--native-key (plist-get item :harness) (plist-get item :config-dir)
                                                                 (plist-get item :id))
                                       known)
                              (gethash (plist-get item :path) known)))
                        romacs-agent-history--listing)
          (lambda (a b) (> (plist-get a :mtime) (plist-get b :mtime))))))

;;; The view

(defconst romacs-agent-history--facts
  '((:model . romacs-agent-model) (:permission-mode . romacs-agent-permission-mode)
    (:context-used . romacs-agent-context-used) (:context-window . romacs-agent-context-window))
  "The header's facts a transcript tells, and the session's fields for them.")

(defun romacs-agent-history--keep-facts (session facts)
  "Give SESSION the transcript's last FACTS where it has none of its own:
an imported session's own turns know better."
  (pcase-dolist (`(,key . ,field) romacs-agent-history--facts)
    (when (and (plist-get facts key) (null (funcall field session)))
      (pcase key
        (:model (setf (romacs-agent-model session) (plist-get facts key)))
        (:permission-mode (setf (romacs-agent-permission-mode session) (plist-get facts key)))
        (:context-used (setf (romacs-agent-context-used session) (plist-get facts key)))
        (:context-window (setf (romacs-agent-context-window session) (plist-get facts key)))))))

(defun romacs-agent-history-draw (session imported)
  "Draw the transcript IMPORTED names (:path, and :size, how much of it
was the harness's before Romacs went on with it) into the current chat
buffer, for SESSION: its end, from a prompt on."
  (let* ((path (plist-get imported :path))
         (end (or (plist-get imported :size) (file-attribute-size (file-attributes path)) 0))
         (beg (max 0 (- end romacs-agent-history-view-bytes)))
         (harness (romacs-agent-harness session))
         (events (and (file-readable-p path)
                      (funcall (romacs-agent-history--get harness :events)
                               (romacs-agent-history--read path beg end (romacs-agent-history--get harness :keep)))))
         (started (make-hash-table :test #'equal))
         (facts nil))
    ;; Cut: from the first prompt on.
    (when (> beg 0)
      (while (and events (not (eq (plist-get (car events) :type) 'input))) (pop events))
      (romacs-agent-ui--insert (format "· earlier turns are in %s\n" (abbreviate-file-name path)) 'face 'romacs-agent-dim))
    (unless (file-readable-p path)
      (romacs-agent-ui--insert (format "! %s is gone\n" (abbreviate-file-name path)) 'face 'romacs-agent-error))
    (dolist (event events)
      (pcase-dolist (`(,key . ,_) romacs-agent-history--facts)
        (when (plist-get event key) (setq facts (plist-put facts key (plist-get event key)))))
      (when (eq (plist-get event :type) 'tool)
        (if (eq (plist-get event :state) 'started) (puthash (plist-get event :id) t started)
          ;; A result whose call the cut left out shows nothing.
          (unless (gethash (plist-get event :id) started) (setq event nil))))
      (when event (romacs-agent-ui--draw session event (current-buffer))))
    (romacs-agent-history--keep-facts session facts)))

(defvar-local romacs-agent-history--item nil "The transcript this view shows.")

(defvar romacs-agent-history-mode-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map special-mode-map)
    (define-key map (kbd "RET") #'romacs-agent-chat-toggle)
    (define-key map (kbd "TAB") #'romacs-agent-chat-toggle)
    (define-key map "r" #'romacs-agent-history-resume-this)
    (define-key map "n" #'romacs-agent-chat-next-turn)
    (define-key map "p" #'romacs-agent-chat-previous-turn)
    map)
  "Keys of a harness transcript's view.")

(define-derived-mode romacs-agent-history-mode romacs-agent-chat-mode "Agent history"
  "A harness's own transcript, read-only: \\[romacs-agent-history-resume-this] goes on with it.
\\{romacs-agent-history-mode-map}"
  (setq-local desktop-save-buffer nil)
  (setq buffer-read-only t))

;;;###autoload
(defun romacs-agent-history-open (item)
  "Show the transcript ITEM (one of `romacs-agent-history-items') read-only."
  (romacs-agent-ui--helpers)
  (let ((buffer (cl-find-if (lambda (buffer) (equal (plist-get (buffer-local-value 'romacs-agent-history--item buffer) :path)
                                                    (plist-get item :path)))
                            (buffer-list))))
    (unless buffer
      (setq buffer (generate-new-buffer (format "*history: %s*" (plist-get item :title))))
      (with-current-buffer buffer
        (romacs-agent-history-mode)
        (setq romacs-agent-history--item item
              romacs-agent-ui--session (romacs-agent--make
                                        :id (plist-get item :id) :harness (plist-get item :harness)
                                        :provider (plist-get item :provider) :config-dir (plist-get item :config-dir)
                                        :cwd (plist-get item :cwd) :title (plist-get item :title) :state 'history)
              romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
              romacs-agent-ui--messages (make-hash-table :test #'equal)
              romacs-agent-ui--end (point-marker)))
      ;; Shown first: its rules fit its window.
      (pop-to-buffer buffer)
      (with-current-buffer buffer
        (romacs-agent-history-draw romacs-agent-ui--session (list :path (plist-get item :path)))))
    (pop-to-buffer buffer)
    (goto-char (point-max))
    (recenter -1)
    buffer))

(defun romacs-agent-history-resume (item)
  "Go on with the transcript ITEM as a Romacs session, through its
harness's own resume, and show its chat: the session already going on
with it, when there is one, attached or resumed."
  (let* ((id (plist-get item :id))
         (known (romacs-agent--native-session (plist-get item :harness) (plist-get item :config-dir) id))
         (session (if known (romacs-agent--continue known)
                    (romacs-agent-import (plist-get item :harness) id (plist-get item :path)
                                         (plist-get item :cwd) (plist-get item :title)
                                         (plist-get item :provider) (plist-get item :config-dir)))))
    (romacs-agent-ui--dashboard-changed)
    (romacs-agent-show session)))

(defun romacs-agent-history-resume-this ()
  "Go on with this transcript as a Romacs session."
  (interactive)
  (let ((item romacs-agent-history--item) (view (current-buffer)))
    (romacs-agent-history-resume item)
    (kill-buffer view)))

(provide 'romacs-agent-history)
;;; romacs-agent-history.el ends here

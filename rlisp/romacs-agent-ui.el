;;; romacs-agent-ui.el --- Chat buffers and the agents home page -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's own interface to agent sessions (romacs-agent.el), in the look of
;; emacs-hermes's chat (rlisp/hermes, by the same author): its pure
;; formatting helpers (hermes-chat-format.el: the clock, token counts,
;; Markdown) are used as they are; the prompt face, the full-width rule, the
;; turn footer, the composer hint and the header's shape are adapted from
;; hermes-chat.el and hermes-chat-buffer.el to romacs-agent's sessions.
;;
;; - `romacs-agents': the home page, as T3 Code's sidebar: sessions grouped
;;   by project, delegated ones nested under the session that started them,
;;   each with its state, the time of its last activity and its model
;;   (and instance, when not the default).  RET opens a chat, c starts a session, i
;;   interrupts, k stops, r resumes, g refreshes; n and p move.
;; - A chat buffer per session: a header (harness, permission mode, model,
;;   context used of its window), the transcript (`> prompt', tool rows
;;   marked ● running, ✓ done or ! failed with their output folded, the
;;   reply in Markdown), a rule closing each turn with its time, a
;;   live rule while a turn runs, and the composer.  RET in the composer
;;   sends, C-c C-c interrupts, C-c C-k stops, C-c C-r resumes, M-p recalls;
;;   in the transcript TAB folds a tool's output and n and p move between
;;   prompts.  An approval the harness asks for is answered with y or N (or
;;   the actions menu).  C-c C-a attaches an image.  C-c C-b goes to another
;;   open chat, C-c C-t to any session, opening its chat.
;;
;; The chat is rebuilt from the journal, so a new server shows what the
;; session did while no server was there.

;;; Code:
(require 'romacs-truncate)
(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-key-hint)

;;;###autoload
(defvar romacs-agents-map
  (let ((map (make-sparse-keymap)))
    (define-key map "n" #'romacs-agent-start)
    (define-key map "s" #'romacs-agent-switch-chat)
    (define-key map "a" #'romacs-agents)
    map)
  "Global prefix map for starting and finding agent sessions.
\\{romacs-agents-map}")

;;;###autoload (define-key global-map (kbd "C-c C-;") romacs-agents-map)

(declare-function hermes-chat--clock "hermes-chat-format" (seconds &optional tenths))
(declare-function hermes-chat--abbrev-tokens "hermes-chat-format" (n))
(declare-function hermes-chat--fontify-markdown-string "hermes-chat-format" (text))

;;; Faces (hermes-chat's, for the same look)

(defface romacs-agent-prompt '((t :inherit highlight))
  "A user's prompt in a chat (hermes-chat-user-input).")
(defface romacs-agent-rule
  '((((background light)) :foreground "gray70")
    (t :foreground "gray30"))
  "The rule closing a turn and above the composer (hermes-chat-separator's colours).")
(defface romacs-agent-running '((t :inherit font-lock-keyword-face))
  "What is running: a tool's mark, the live rule (hermes-chat-turn-running).")
(defface romacs-agent-dim '((t :inherit shadow))
  "Settled rows, thinking, folded output and metadata.")
(defface romacs-agent-done '((t :inherit success))
  "A settled mark.")
(defface romacs-agent-approval '((t :inherit warning :weight bold))
  "An approval the harness waits for.")
(defface romacs-agent-error '((t :inherit error))
  "A failed tool call or turn.")
(defface romacs-agent-model '((t :inherit font-lock-type-face))
  "The model in a chat's header (hermes-chat-header-model).")
(defface romacs-agent-effort '((t :weight bold))
  "A reasoning effort; each level's face inherits it.")
(defface romacs-agent-effort-default '((t :inherit shadow))
  "No effort named: the harness's own.")
(defface romacs-agent-effort-low '((t :inherit (romacs-agent-effort success))) "Effort low.")
(defface romacs-agent-effort-medium '((t :inherit (romacs-agent-effort font-lock-constant-face))) "Effort medium.")
(defface romacs-agent-effort-high '((t :inherit (romacs-agent-effort font-lock-keyword-face))) "Effort high.")
(defface romacs-agent-effort-xhigh '((t :inherit (romacs-agent-effort warning))) "Effort xhigh.")
(defface romacs-agent-effort-max '((t :inherit (romacs-agent-effort error))) "Effort max.")
(defface romacs-agent-effort-ultra '((t :inherit (romacs-agent-effort error) :underline t)) "Effort ultra.")
(defface romacs-agent-project '((t :inherit font-lock-function-name-face))
  "A session's project, where a row names it.")
(defface romacs-agent-context '((t :inherit font-lock-number-face))
  "Context tokens in a chat's header (hermes-chat-header-context).")
(defface romacs-agent-mode '((t :inherit font-lock-builtin-face))
  "The harness's permission mode in a chat's header.")
(defface romacs-agent-section '((t :inherit font-lock-function-name-face :weight bold))
  "A project heading on the home page.")
(defface romacs-agent-delegate '((t :inherit font-lock-function-name-face :weight bold))
  "Face of a delegated task's title.")
(defface romacs-agent-link '((t :inherit link))
  "Face of a link to a delegated task's thread.")
(defface romacs-agent-unread '((t :weight bold))
  "A session whose last turn ended since its chat was shown.")

(defvar-local romacs-agent-ui--session nil)
(defvar-local romacs-agent-ui--end nil
  "Marker at the transcript's end; the composer follows it.")
(defvar-local romacs-agent-ui--tool-lines nil
  "Tool use id to the marker at its row's mark.")
(defvar-local romacs-agent-ui--draft-images nil
  "The image files of the message being written, in order.")
(defvar-local romacs-agent-ui--tasks nil
  "A delegated task's id to the marker of its card's state chip.")
(defvar-local romacs-agent-ui--messages nil
  "An open message's id to (START . LENGTH): where its text starts and how
much of it its deltas wrote.")
(defvar-local romacs-agent-ui--shown 0
  "The last journal record this buffer shows whole.")
(defvar-local romacs-agent-ui--open nil
  "The kind of block the transcript ends in: `text', `thinking' or nil.")
(defvar-local romacs-agent-ui--history nil)
(defvar-local romacs-agent-ui--composer nil
  "The overlay at the composer's start: the live rule and the empty hint.")
(defvar-local romacs-agent-ui--fills nil
  "A marker at the start of each turn rule's fill, as they are drawn.")
(defvar-local romacs-agent-ui--fitted nil
  "The width the rules' fills last fitted.")
(defvar-local romacs-agent-ui--tool nil
  "The running tool's name, for the live rule.")

(defun romacs-agent-ui--helpers ()
  "Hermes's formatting helpers, loaded when a view first needs them."
  (require 'hermes-chat-format))

;;; The transcript

(defun romacs-agent-ui--depth (session parent)
  "How deep a block under tool call PARENT nests."
  (let ((depth 0) (tools (romacs-agent-tools session)))
    (while (and parent (stringp parent) (< depth 8))
      (setq depth (1+ depth) parent (gethash parent tools)))
    depth))

(defun romacs-agent-ui--sealed (text)
  "TEXT as the transcript holds it: read-only, and passing none of its
properties to text typed after it (the composer), so no fold, face or
mark of the transcript's end hides or styles what is typed there."
  (let ((text (copy-sequence text)))
    (add-text-properties 0 (length text) '(read-only t front-sticky (read-only) rear-nonsticky t) text)
    text))

(defun romacs-agent-ui--insert-at (position text)
  "Insert TEXT, sealed, at POSITION in the transcript.  At its end, before
the end marker and every marker there (the composer, a window's point at
the composer, follow the transcript), but for a window's start, which
would leave the transcript above the window; a row's mark or a message's
start is always taken after its own text is in."
  (save-excursion
    ;; A number: the end marker moves past what is inserted at it.
    (let ((inhibit-read-only t) (text (romacs-agent-ui--sealed text)) (start (+ 0 position)))
      (goto-char start)
      (if (/= start romacs-agent-ui--end) (insert text)
        (let ((windows (seq-filter (lambda (window) (= (window-start window) start))
                                   (get-buffer-window-list nil nil t))))
          (insert-before-markers text)
          (dolist (window windows) (set-window-start window start t))))
      ;; A rule's fill, remembered: a resize refits the rules alone.
      (when-let* ((offset (text-property-any 0 (length text) 'romacs-agent-rule-fill t text)))
        (push (copy-marker (+ start offset)) romacs-agent-ui--fills)
        ;; Drawn at the width of its day: the next refit visits every rule.
        (setq romacs-agent-ui--fitted nil)))))

(defun romacs-agent-ui--insert (text &rest properties)
  "Insert TEXT at the transcript's end with PROPERTIES, sealed."
  (romacs-agent-ui--insert-at romacs-agent-ui--end (apply #'propertize text properties)))

(defun romacs-agent-ui--line (session event text &rest properties)
  "Start a new row for EVENT, indented by its nesting."
  (let ((indent (make-string (* 2 (romacs-agent-ui--depth session (plist-get event :parent))) ?\s)))
    (setq romacs-agent-ui--open nil)
    (apply #'romacs-agent-ui--insert
           (concat (if (save-excursion (goto-char romacs-agent-ui--end) (bolp)) "" "\n") indent text "\n")
           properties)))

(defun romacs-agent-ui--brief (input)
  "One line of a tool's INPUT."
  (let ((text (cond ((not (hash-table-p input)) "")
                    ((gethash "command" input))
                    ((gethash "file_path" input))
                    ((gethash "pattern" input))
                    ((gethash "query" input))
                    ((gethash "description" input))
                    ((gethash "code" input))
                    ((zerop (hash-table-count input)) "")
                    (t (romacs-agent--json input)))))
    (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " (format "%s" text)) 100 nil nil t)))

(defun romacs-agent-ui--tool-name (name)
  "A tool's NAME as a row shows it: an MCP tool by its own name."
  (if (and (stringp name) (string-match "\\`mcp__[^_]+\\(?:_[^_]+\\)*__\\(.+\\)\\'" name))
      (match-string 1 name)
    (or name "tool")))

(defun romacs-agent-ui--png-size (file)
  "FILE's width and height, (W . H), from its header, or nil unless it is a
whole PNG by what can be read cheaply: signature, an IHDR of sizes above
zero, and an IEND closing it.  Where `image-size' decodes the image, this
reads 33 bytes and the last 12."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (let ((length (file-attribute-size (file-attributes file))))
      (when (and length (>= length 45))
        (insert-file-contents-literally file nil 0 24)
        (let ((bytes (lambda (from) (mapcar #'char-after (number-sequence from (+ from 7)))))
              (word (lambda (at) (+ (ash (char-after at) 24) (ash (char-after (+ at 1)) 16)
                                    (ash (char-after (+ at 2)) 8) (char-after (+ at 3))))))
          (when (and (= (buffer-size) 24)
                     (equal (funcall bytes 1) '(137 80 78 71 13 10 26 10))
                     (equal (funcall bytes 9) '(0 0 0 13 73 72 68 82)))
            (let ((width (funcall word 17)) (height (funcall word 21)))
              (erase-buffer)
              (insert-file-contents-literally file nil (- length 12) length)
              (when (and (> width 0) (> height 0) (= (buffer-size) 12)
                         (equal (mapcar #'char-after (number-sequence 5 8)) '(73 69 78 68)))
                (cons width height)))))))))

(defun romacs-agent-ui--fitted-size (size width height)
  "SIZE, (W . H), as :max-width WIDTH and :max-height HEIGHT shrink it."
  (let ((scale (min 1.0 (/ width (float (car size))) (/ height (float (cdr size))))))
    ;; Up, as the display rounds a scaled image.
    (cons (ceiling (* scale (car size))) (ceiling (* scale (cdr size))))))

(defun romacs-agent-ui--image-strips (file)
  "FILE as rows of a sliced image, a string, or nil when it will not load:
moving and scrolling step through it, as through text.  Each strip but the
last is one line tall, in pixels, so it fills its row: the display has no
`line-height' to let a shorter row through, and a shorter strip would leave
a seam.  Centred on the line, a strip's row measures as it draws, one line
tall.  A PNG is laid out from its header, so a transcript's images are
decoded as they come into view, not when it is replayed."
  (ignore-errors
    (let* ((image (create-image file nil nil :max-width 480 :max-height 270 :ascent 'center :scale 1))
           (png (romacs-agent-ui--png-size file))
           (size (if png (romacs-agent-ui--fitted-size png 480 270) (image-size image t)))
           (line (default-line-height))
           (strips nil))
      (dotimes (row (ceiling (cdr size) line))
        (let ((y (* row line)))
          (push (propertize (concat (propertize " " 'display `((slice 0 ,y ,(car size) ,(min line (- (cdr size) y))) ,image)
                                                'rear-nonsticky '(display))
                                    "\n")
                            'romacs-agent-strip t)
                strips)))
      (apply #'concat (nreverse strips)))))

(defun romacs-agent-ui--image-rows (file)
  "What the transcript shows for attached image FILE: its rows and its name,
or the name and that it cannot be shown.  The name is always there: rows are
laid out before the image is decoded, so a corrupt one can still draw blank."
  (let ((rows (and (display-images-p) (romacs-agent-image-file-p file) (romacs-agent-ui--image-strips file))))
    (concat rows (propertize (format "  ▣ %s%s\n" (file-name-nondirectory file) (if rows "" " (cannot show)"))
                             'face 'romacs-agent-dim 'romacs-agent-strip t))))

(defun romacs-agent-ui--delegation (name input)
  "What a card shows for tool NAME's INPUT, (TITLE DETAIL), or nil when
the tool delegates nothing.  One card serves a harness's own subagent
tool and `romacs_delegate_task'."
  (when (hash-table-p input)
    (pcase name
      ("romacs_delegate_task"
       (let ((target (gethash "target" input)))
         (list (or (gethash "title" input) "task")
               (and (hash-table-p target) (gethash "provider" target)))))
      ((or "Agent" "Task")
       (list (or (gethash "description" input) "task") (gethash "subagent_type" input))))))

(defun romacs-agent-ui--tool-text (name input)
  "A tool row's text after its mark: a delegation's card head, or NAME and
a brief of INPUT."
  (let ((delegation (romacs-agent-ui--delegation name input)))
    (if delegation
        (concat " ⇢ " (propertize (car delegation) 'face 'romacs-agent-delegate 'romacs-agent-card t)
                (if (cadr delegation) (propertize (concat "  " (cadr delegation)) 'face 'romacs-agent-dim) ""))
      (let ((brief (romacs-agent-ui--brief input)))
        (concat " " name (if (string-empty-p brief) "" (concat "  " brief)))))))

(defun romacs-agent-ui--chip (state)
  "A task's STATE as its card shows it."
  (propertize state 'romacs-agent-chip t
              'face (pcase state
                      ("completed" 'romacs-agent-done)
                      ("waiting" 'warning)
                      ((or "failed" "cancelled") 'romacs-agent-error)
                      (_ 'romacs-agent-running))))

(defvar romacs-agent-ui--link-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agent-chat-open-task)
    (define-key map [mouse-1] #'romacs-agent-chat-open-task)
    map)
  "Keys of a link to a delegated task's thread.")

(defun romacs-agent-ui--thread-link (id)
  "A link to task ID's thread."
  (propertize "↗ thread" 'face 'romacs-agent-link 'mouse-face 'highlight 'help-echo "Open this task's thread"
              'keymap romacs-agent-ui--link-map 'romacs-agent-task id))

(defvar romacs-agent-ui--compaction-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agent-chat-show-compaction)
    (define-key map [mouse-1] #'romacs-agent-chat-show-compaction)
    map)
  "Keys of a compaction's row.")

(defun romacs-agent-ui--task-record (output)
  "The task record a delegate call's OUTPUT holds, or nil."
  (let ((record (ignore-errors (json-parse-string output))))
    (and (hash-table-p record) (stringp (gethash "taskId" record)) record)))

(defun romacs-agent-ui--card-task (record tool)
  "Give the card of tool call TOOL on this line RECORD's state and a link
to its thread, which fold its result as the rest of the row does."
  (let ((id (gethash "taskId" record)))
    (save-excursion
      (end-of-line)
      (let ((start (point)))
        (romacs-agent-ui--insert-at
         start (propertize (concat "  " (romacs-agent-ui--chip (gethash "status" record "running"))
                                   "  " (romacs-agent-ui--thread-link id))
                           'romacs-agent-tool tool))
        (puthash id (copy-marker (+ start 2)) romacs-agent-ui--tasks)))))

(defun romacs-agent-ui--settle-task (id state)
  "Show STATE on task ID's card, when it has one here."
  (when-let* ((mark (gethash id romacs-agent-ui--tasks)))
    (save-excursion
      (let* ((inhibit-read-only t) (start (marker-position mark))
             (tool (get-text-property start 'romacs-agent-tool)))
        (delete-region start (next-single-property-change start 'romacs-agent-chip))
        (romacs-agent-ui--insert-at start (propertize (romacs-agent-ui--chip state) 'romacs-agent-tool tool))
        (set-marker mark start)))))

(defun romacs-agent-ui--wake (session text)
  "TEXT, a message to SESSION, as the wake of a task it delegated, or nil:
what the user wrote stays theirs, whatever it looks like."
  (when-let* ((wake (romacs-agent-tools--wake-parse text))
              (child (romacs-agent-get (plist-get wake :id)))
              ((equal (plist-get (romacs-agent-options child) :parent) (romacs-agent-id session))))
    wake))

(defun romacs-agent-ui--arrival (session event wake)
  "Show WAKE, a task's result reaching SESSION at EVENT: a row of its own,
never a prompt, the result folded below it, and its card settled."
  (let* ((id (plist-get wake :id)) (state (plist-get wake :state))
         (result (string-trim (plist-get wake :result)))
         (key (format "%s:%s" id (plist-get event :seq)))
         (preview (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " result) 72 nil nil t)))
    (romacs-agent-ui--settle-task id state)
    (romacs-agent-ui--line
     session event
     (concat "  ⇠ " (propertize (plist-get wake :title) 'face 'romacs-agent-delegate 'romacs-agent-card t)
             "  " (romacs-agent-ui--chip state) "  " (romacs-agent-ui--thread-link id)
             (if (string-empty-p preview) "" (propertize (concat "  " preview) 'face 'romacs-agent-dim)))
     'romacs-agent-tool key)
    (unless (string-empty-p result)
      (romacs-agent-ui--insert (concat result "\n") 'face 'romacs-agent-dim 'romacs-agent-output key
                               'invisible 'romacs-agent-output))))

(defun romacs-agent-ui--tool (session event)
  "A tool row: ● while running, then ✓ or !, its output folded below.  A
delegation's row is a card (`romacs-agent-ui--tool-text')."
  (let ((id (plist-get event :id)))
    (pcase (plist-get event :state)
      ('started
       (let ((name (romacs-agent-ui--tool-name (plist-get event :name))))
         (setq romacs-agent-ui--tool name romacs-agent-ui--open nil)
         (let ((indent (make-string (* 2 (1+ (romacs-agent-ui--depth session (plist-get event :parent)))) ?\s)))
           (romacs-agent-ui--insert (concat (if (save-excursion (goto-char romacs-agent-ui--end) (bolp)) "" "\n") indent))
           (romacs-agent-ui--insert "●" 'face 'romacs-agent-running)
           (puthash id (copy-marker (1- romacs-agent-ui--end)) romacs-agent-ui--tool-lines)
           (romacs-agent-ui--insert (concat (romacs-agent-ui--tool-text name (plist-get event :input)) "\n")
                                    'romacs-agent-tool id))))
      (state
       (let* ((output (or (plist-get event :output) ""))
              (failed (eq state 'failed))
              (mark (gethash id romacs-agent-ui--tool-lines))
              (card (and mark (save-excursion
                                (goto-char mark)
                                (text-property-any (point) (line-end-position) 'romacs-agent-card t))))
              (record (and card (romacs-agent-ui--task-record output)))
              ;; A task's record is the card; a `wait' brings its result
              ;; here, and no wake follows it.
              (body (if record (let ((result (gethash "result" record))) (if (stringp result) result "")) output)))
         (setq romacs-agent-ui--tool nil)
         (save-excursion
           (let ((inhibit-read-only t))
             (if (not mark)
                 (goto-char romacs-agent-ui--end)
               ;; The mark settles; the row recedes (hermes-chat--entry-head-face).
               (goto-char mark)
               (delete-char 1)
               (insert (romacs-agent-ui--sealed
                        (propertize (if failed "!" "✓") 'face (if failed 'romacs-agent-error 'romacs-agent-done))))
               ;; A card stays readable: it is the task, not a settled call.
               (when (or failed (not card))
                 (add-face-text-property (point) (line-end-position) (if failed 'romacs-agent-error 'romacs-agent-dim)))
               (when record (romacs-agent-ui--card-task record id))
               (forward-line 1))
             (unless (string-empty-p (string-trim body))
               (romacs-agent-ui--insert-at
                (point) (propertize (concat (string-trim-right body) "\n")
                                    'face 'romacs-agent-dim 'romacs-agent-output id
                                    'invisible 'romacs-agent-output))))))))))

(defun romacs-agent-ui--table-width ()
  "Columns a table may take, as hermes-chat--table-window-width counts them:
the window's pixels in the fixed-pitch font's width, less two for the line's
end.  Hidden, the rule's width."
  (let ((window (get-buffer-window (current-buffer) t)))
    (max 20 (- (if window
                   (/ (window-body-width window t) (max 1 (window-font-width window 'fixed-pitch)))
                 (romacs-agent-ui--rule-width))
               2))))

(defun romacs-agent-ui--markdown (body width)
  "BODY as the transcript shows it: fontified, each pipe table a grid within
WIDTH that keeps its source in `romacs-agent-table', for a later width."
  (let* ((text (hermes-chat--fontify-markdown-string body)) (end (length text)) (start 0) (parts nil))
    (while (< start end)
      (let ((next (next-single-property-change start 'hermes-chat-table text end))
            (table (get-text-property start 'hermes-chat-table text)))
        (push (if table
                  (propertize (hermes-chat--format-table table width) 'romacs-agent-table table)
                (substring text start next))
              parts)
        (setq start next)))
    (apply #'concat (nreverse parts))))

(defun romacs-agent-ui--refit-tables ()
  "Draw each rendered table again at the window's width.  They are found by
their source property: a resize is rare, and no record of them can go stale."
  (let ((width (romacs-agent-ui--table-width)) (at (point-min)))
    (while (< at romacs-agent-ui--end)
      (let ((source (get-text-property at 'romacs-agent-table))
            (end (next-single-property-change at 'romacs-agent-table nil (+ 0 romacs-agent-ui--end))))
        (if (not source) (setq at end)
          (let ((grid (propertize (hermes-chat--format-table source width) 'romacs-agent-table source)))
            (delete-region at end)
            (romacs-agent-ui--insert-at at grid)
            (setq at (+ at (length grid)))))))))

(defun romacs-agent-ui--message-end (id)
  "Render message ID's text, whole now, as Markdown, in place: exactly the
range its deltas wrote."
  (when-let* ((range (gethash id romacs-agent-ui--messages)))
    (remhash id romacs-agent-ui--messages)
    (let* ((start (marker-position (car range))) (end (min (+ start (cdr range)) romacs-agent-ui--end)))
      (when (< start end)
        (romacs-agent-ui--helpers)
        (let* ((inhibit-read-only t)
               (body (string-trim-right (buffer-substring-no-properties start end)))
               (fontified (romacs-agent-ui--markdown body (romacs-agent-ui--table-width))))
          (save-excursion
            (goto-char start)
            (delete-region start (+ start (length body)))
            (romacs-agent-ui--insert-at
             start fontified)))))
    (set-marker (car range) nil)))

(defun romacs-agent-ui--footer (session event)
  "The rule closing a turn: ✓ or !, and how long it took."
  (romacs-agent-ui--helpers)
  (maphash (lambda (id _) (romacs-agent-ui--message-end id)) (copy-hash-table romacs-agent-ui--messages))
  (let* ((failed (plist-get event :error))
         (duration (or (plist-get event :duration)
                       (and (numberp (plist-get event :time)) (numberp (romacs-agent-turn-start session))
                            (- (plist-get event :time) (romacs-agent-turn-start session)))))
         (text (concat (propertize (if failed "!" "✓") 'face (if failed 'romacs-agent-error 'romacs-agent-dim))
                       (if duration (propertize (concat " " (hermes-chat--clock duration)) 'face 'romacs-agent-dim) ""))))
    (romacs-agent-ui--line session event (romacs-agent-ui--rule text))))

(defun romacs-agent-ui--rule-width ()
  "The chat window's width, else the frame's, for a rule to fill."
  (let ((window (get-buffer-window (current-buffer) 'visible)))
    (max 20 (1- (if window (window-body-width window) (frame-width))))))

(defun romacs-agent-ui--rule (&optional text)
  "A rule across the chat, carrying TEXT after a short lead
\(hermes-chat--status-rule-line's shape).  Drawn with box-drawing
characters to the window's width: Romacs's display does not yet paint
a face's strike-through on hermes's stretch glyphs."
  (let* ((lead (propertize "──" 'face 'romacs-agent-rule))
         (head (if text (concat lead " " text " ") lead)))
    (concat head (propertize (make-string (max 1 (- (romacs-agent-ui--rule-width) (string-width head))) ?─)
                             'face 'romacs-agent-rule 'romacs-agent-rule-fill t))))

(defun romacs-agent-ui--refit-rules (&rest _)
  "The window's width changed: each turn's rule fills it again.  Only the
rules are visited (their fills' markers), never the transcript, and a fill
already that wide is left alone."
  (when (and (derived-mode-p 'romacs-agent-chat-mode) (markerp romacs-agent-ui--end))
    (let ((width (romacs-agent-ui--rule-width)))
      (unless (eql width romacs-agent-ui--fitted)
        (setq romacs-agent-ui--fitted width)
        (let ((inhibit-read-only t))
          (save-excursion
            (dolist (marker romacs-agent-ui--fills)
              (let* ((start (marker-position marker))
                     (end (next-single-property-change start 'romacs-agent-rule-fill nil (point-max)))
                     (fill (progn (goto-char start)
                                  (max 1 (- width (string-width (buffer-substring (line-beginning-position) start)))))))
                ;; Its tail alone changes: the first character, where its
                ;; marker is, stays (an edit there costs every marker's
                ;; record in the undo list).
                (cond ((> fill (- end start))
                       (goto-char end)
                       (insert (romacs-agent-ui--sealed
                                (propertize (make-string (- fill (- end start)) ?─)
                                            'face 'romacs-agent-rule 'romacs-agent-rule-fill t))))
                      ((< fill (- end start))
                       (delete-region (+ start fill) end)))))
            (romacs-agent-ui--refit-tables)))
        (romacs-agent-ui--composer-update)))))

(defun romacs-agent-ui--render (session event)
  "Show SESSION's EVENT in its chat buffer, when it has one and has not
shown it: the journal's replay and the live records meet without a gap
or a repeat."
  (when-let* ((buffer (romacs-agent-buffer session)) ((buffer-live-p buffer))
              ((> (or (plist-get event :seq) 0) (buffer-local-value 'romacs-agent-ui--shown buffer))))
    (with-current-buffer buffer
      (when (plist-get event :last) (setq romacs-agent-ui--shown (plist-get event :seq)))
      (romacs-agent-ui--draw session event buffer))))

(defun romacs-agent-ui--selection-label (selection)
  "SELECTION, a model and effort, for a person: unnamed, the harness's own."
  (mapconcat #'identity (or (delq nil (list (plist-get selection :model) (plist-get selection :effort)))
                            '("harness's own"))
             " · "))

(defun romacs-agent-ui--draw (session event buffer)
  "Draw SESSION's EVENT in its chat BUFFER, the current buffer."
      (pcase (plist-get event :type)
        ('input
         (let ((wake (romacs-agent-ui--wake session (plist-get event :text))))
           (if wake (romacs-agent-ui--arrival session event wake)
             (romacs-agent-ui--line session event "")
             (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
             (romacs-agent-ui--insert "> " 'romacs-agent-turn t)
             (let ((images (plist-get event :images)))
               (romacs-agent-ui--insert (concat (plist-get event :text)) 'face 'romacs-agent-prompt 'romacs-agent-turn t
                                        'romacs-agent-images images)
               (romacs-agent-ui--insert "\n")
               (dolist (file images)
                 (romacs-agent-ui--insert-at romacs-agent-ui--end (romacs-agent-ui--image-rows file))))))
         ;; The first prompt names the session: its buffer follows.
         (let ((name (format "*agent: %s*" (romacs-agent-title session))))
           (unless (or (not (overlayp romacs-agent-ui--composer)) (string-prefix-p (substring name 0 -1) (buffer-name)))
             (rename-buffer name t))))
        ('delta
         (let ((text (plist-get event :text)) (id (plist-get event :message)))
           (unless (eq romacs-agent-ui--open 'text)
             (romacs-agent-ui--line session event "")
             (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
             (setq romacs-agent-ui--open 'text))
           (romacs-agent-ui--insert text)
           (when id
             (let ((range (gethash id romacs-agent-ui--messages)))
               (if range (setcdr range (+ (cdr range) (length text)))
                 ;; Advancing: a tool's output written where the reply
                 ;; starts goes before it.
                 (puthash id (cons (copy-marker (- romacs-agent-ui--end (length text)) t) (length text))
                          romacs-agent-ui--messages))))))
        ('message-end (romacs-agent-ui--message-end (plist-get event :message)))
        ('thinking
         ;; No row until there is thinking to show.
         (let ((text (plist-get event :text)))
           ;; Opened by the first delta with something to show; then every
           ;; delta, spaces and newlines too.
           (when (and text (or (eq romacs-agent-ui--open 'thinking) (not (string-empty-p (string-trim text)))))
             (unless (eq romacs-agent-ui--open 'thinking)
               (romacs-agent-ui--line session event "∴ " 'face 'romacs-agent-dim)
               (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
               (setq romacs-agent-ui--open 'thinking))
             (romacs-agent-ui--insert text 'face 'romacs-agent-dim))))
        ('tool (romacs-agent-ui--tool session event))
        ('compacted
         (romacs-agent-ui--line session event
                                (concat (propertize "  · " 'face 'romacs-agent-dim)
                                        (propertize "Session compacted" 'face 'romacs-agent-link 'mouse-face 'highlight))
                                'keymap romacs-agent-ui--compaction-map 'help-echo "Show what the session kept"
                                'romacs-agent-compaction (plist-get event :text)))
        ('approval
         (romacs-agent-ui--line session event
                                (format "◆ %s asks: %s  %s   [y] allow  [N] deny"
                                        (plist-get event :tool) (romacs-agent-ui--brief (plist-get event :input))
                                        (or (plist-get event :description) ""))
                                'face 'romacs-agent-approval 'romacs-agent-approval (plist-get event :request-id)))
        ('answered
         (romacs-agent-ui--line session event (format "  %s" (pcase (plist-get event :behavior)
                                                               ("allow" "✓ allowed") ("deny" "! denied") (_ "answered")))
                                'face 'romacs-agent-dim))
        ('done (when-let* ((reason (plist-get event :reason)))
                 (romacs-agent-ui--line session event (format "! %s" reason) 'face 'romacs-agent-error))
               (romacs-agent-ui--footer session event)
               (when (get-buffer-window buffer 'visible)
                 (setf (romacs-agent-seen-seq session) (romacs-agent-done-seq session))))
        ('status
         (unless (member (plist-get event :text) '("requesting" "compacting" "detached" "selection armed"))
           (romacs-agent-ui--line session event (format "  · %s" (plist-get event :text)) 'face 'romacs-agent-dim)))
        ('error (romacs-agent-ui--line session event (format "! %s" (plist-get event :text)) 'face 'romacs-agent-error))
        ('selection
         (romacs-agent-ui--line session event
                                (format "── %s → %s ──" (romacs-agent-ui--selection-label (plist-get event :from))
                                        (romacs-agent-ui--selection-label (plist-get event :to)))
                                'face 'romacs-agent-dim))
        ('exit
         ;; A per-turn harness ends every turn: only a failed run is news.
         (unless (and (romacs-agent--per-turn-p session) (eql (plist-get event :status) 0))
           (romacs-agent-ui--line session event (format "  · harness exited (%s)" (plist-get event :status))
                                  'face 'romacs-agent-dim)))
        (_ nil))
      (romacs-agent-ui--composer-update)
      (when (and (eq (plist-get event :type) 'input) (overlayp romacs-agent-ui--composer)) (romacs-agent-ui--tick-start)))

(defun romacs-agent-ui--on-event (session event)
  (romacs-agent-ui--track-children session event)
  (romacs-agent-ui--render session event)
  (romacs-agent-ui--dashboard-changed))

(add-hook 'romacs-agent-event-functions #'romacs-agent-ui--on-event)

;;; What runs under a session
;;
;; A session's delegated tasks and its harness's own subagents, as rows for
;; the actions menu and a line for the header.  Both are made here, once per
;; batch of the events that change them: drawing only reads them.

(defconst romacs-agent-ui--subagent-tools '("Agent" "Task")
  "The tools a harness runs a subagent of its own with: no session of ours.")

(defvar romacs-agent-ui--children-changed (make-hash-table :test #'eq)
  "The sessions whose rows are to be made again.")
(defvar romacs-agent-ui--children-timer nil)

(defun romacs-agent-ui--children-changed (session)
  "Make SESSION's rows again at the end of this batch of events."
  (puthash session t romacs-agent-ui--children-changed)
  (unless romacs-agent-ui--children-timer
    (setq romacs-agent-ui--children-timer (run-at-time 0 nil #'romacs-agent-ui--children-flush))))

(defun romacs-agent-ui--children-flush ()
  (setq romacs-agent-ui--children-timer nil)
  (let ((sessions (hash-table-keys romacs-agent-ui--children-changed)))
    (clrhash romacs-agent-ui--children-changed)
    (dolist (session sessions)
      (romacs-agent-ui--summarize-children session)
      (when-let* ((buffer (romacs-agent-buffer session)) ((buffer-live-p buffer)))
        (with-current-buffer buffer (force-mode-line-update))
        ;; An open actions menu follows.
        (when (and (buffer-live-p keymap-popup--buffer)
                   (eq buffer (plist-get (keymap-popup--session-state keymap-popup--buffer) :source)))
          (keymap-popup--refresh keymap-popup--buffer))))))

(defun romacs-agent-ui--track-subagent (session event)
  "Keep what tool EVENT says of SESSION's harness's own subagents."
  (let ((id (plist-get event :id)) (table (romacs-agent-subagents session)))
    (if (eq (plist-get event :state) 'started)
        (when (and (member (plist-get event :name) romacs-agent-ui--subagent-tools)
                   (not (plist-get event :parent)))
          (unless table (setq table (setf (romacs-agent-subagents session) (make-hash-table :test #'equal))))
          (puthash id (list :tool id :state "running" :time (plist-get event :time)
                            :title (car (romacs-agent-ui--delegation (plist-get event :name) (plist-get event :input))))
                   table)
          (romacs-agent-ui--children-changed session))
      ;; A task the harness runs on, in the background, ends by its own
      ;; notice, not by its tool call returning.
      (when-let* ((row (and table (gethash id table)))
                  ((not (plist-get row :task))))
        (let ((failed (eq (plist-get event :state) 'failed)))
          (puthash id (plist-put (plist-put (plist-put row :state (if failed "failed" "completed")) :unseen failed)
                                 :time (or (plist-get event :time) (plist-get row :time)))
                   table))
        (romacs-agent-ui--children-changed session)))))

(defun romacs-agent-ui--task-state (state)
  "A harness task's end STATE in a card's words."
  (pcase state ('completed "completed") ('failed "failed") (_ "cancelled")))

(defun romacs-agent-ui--task-tool (table task)
  "The tool id of TABLE's row running harness task TASK, or nil."
  (cl-loop for id being the hash-keys of table using (hash-values row)
           when (equal (plist-get row :task) task) return id))

(defun romacs-agent-ui--track-task (session event)
  "Keep what task EVENT says of the work SESSION's harness runs under a tool
call: its subagents, and commands.  The task's row is its tool's, made
here for a tool that is not a subagent's: hidden while it runs in the
foreground, as the tool's own card shows it."
  (let* ((table (or (romacs-agent-subagents session)
                    (setf (romacs-agent-subagents session) (make-hash-table :test #'equal))))
         (id (or (plist-get event :id) (romacs-agent-ui--task-tool table (plist-get event :task))))
         (row (and id (or (gethash id table)
                          (list :tool id :state "running" :time (plist-get event :time) :title (plist-get event :title)
                                :hidden (not (plist-get event :background)))))))
    (when row
     (puthash id
             (pcase (plist-get event :state)
               ('started (plist-put row :task (plist-get event :task)))
               ('backgrounded (plist-put row :hidden nil))
               ('progress (plist-put (plist-put row :activity (plist-get event :activity))
                                     :time (or (plist-get event :time) (plist-get row :time))))
               (end (let ((failed (eq end 'failed)))
                      (plist-put (plist-put (plist-put (plist-put row :task nil) :state (romacs-agent-ui--task-state end))
                                            :unseen failed)
                                 :time (or (plist-get event :time) (plist-get row :time))))))
             table)
     (romacs-agent-ui--children-changed session))))

(defun romacs-agent-ui--end-subagents (session next gone time)
  "SESSION's turn ended at TIME: a subagent still running was cut, unless
it runs on as a task, which only its harness's going (GONE) ends.  NEXT: a
new turn starts, and last turn's failures have been seen."
  (when-let* ((table (romacs-agent-subagents session)))
    (dolist (id (hash-table-keys table))
      (let ((row (gethash id table)))
        (cond (next (puthash id (plist-put row :unseen nil) table))
              ((and (equal (plist-get row :state) "running") (or gone (not (plist-get row :task))))
               (puthash id (plist-put (plist-put (plist-put row :state "cancelled") :task nil)
                                      :time (or time (plist-get row :time)))
                        table)))))
    (romacs-agent-ui--children-changed session)))

(defconst romacs-agent-ui--child-statuses '("detached" "supervisor not reachable" "cancelled")
  "The `status' events that change what a task's state reads.")

(defun romacs-agent-ui--track-children (session event)
  "Note what EVENT changes of what runs under SESSION, or above it."
  (let ((type (plist-get event :type)))
    (pcase type
      ('tool (romacs-agent-ui--track-subagent session event))
      ('task (romacs-agent-ui--track-task session event))
      ('input (romacs-agent-ui--end-subagents session t nil nil))
      ((or 'done 'exit) (romacs-agent-ui--end-subagents session nil (eq type 'exit) (plist-get event :time))))
    (when (or (memq type '(run session input approval answered done exit))
              (and (eq type 'status) (member (plist-get event :text) romacs-agent-ui--child-statuses)))
      (when-let* ((parent (plist-get (romacs-agent-options session) :parent))
                  (known (romacs-agent-get parent)))
        (romacs-agent-ui--children-changed known)))))

(defun romacs-agent-ui--child-row (child)
  "Delegated session CHILD as a row: its state is its card's."
  (list :session child :state (romacs-agent-tools--status child) :title (romacs-agent-title child)
        :time (romacs-agent-last-time child)
        ;; Seen, a failure has been dealt with: it stays in the list only.
        :unseen (> (romacs-agent-done-seq child) (romacs-agent-seen-seq child))))

(defun romacs-agent-ui--child-rank (row)
  "ROW's place: what wants the user, then what runs, then the rest."
  (pcase (plist-get row :state)
    ("waiting" 0)
    ("failed" (if (plist-get row :unseen) 1 4))
    ((or "running" "starting") 2)
    (_ 3)))

(defun romacs-agent-ui--summarize-children (session)
  "Make SESSION's rows and its summary."
  (let* ((rows (append (mapcar #'romacs-agent-ui--child-row (romacs-agent-children session))
                       (and (romacs-agent-subagents session)
                            (cl-remove-if (lambda (row) (plist-get row :hidden))
                                          (hash-table-values (romacs-agent-subagents session))))))
         (counts (make-vector 3 0)))
    (dolist (row rows)
      (pcase (romacs-agent-ui--child-rank row)
        (0 (cl-incf (aref counts 1)))
        (1 (cl-incf (aref counts 2)))
        (2 (cl-incf (aref counts 0)))))
    (setf (romacs-agent-children-rows session)
          (sort rows (lambda (a b)
                       (let ((x (romacs-agent-ui--child-rank a)) (y (romacs-agent-ui--child-rank b)))
                         (if (= x y) (> (or (plist-get a :time) 0) (or (plist-get b :time) 0)) (< x y)))))
          (romacs-agent-children-summary session)
          (let ((parts (delq nil (list (and (> (aref counts 0) 0)
                                            (propertize (format "%d running" (aref counts 0)) 'face 'romacs-agent-running))
                                       (and (> (aref counts 1) 0)
                                            (propertize (format "%d waiting" (aref counts 1)) 'face 'warning))
                                       (and (> (aref counts 2) 0)
                                            (propertize (format "%d failed" (aref counts 2)) 'face 'romacs-agent-error))))))
            (and parts (mapconcat #'identity parts (propertize " · " 'face 'romacs-agent-dim)))))))

;;; The header (hermes-chat--header-line's shape)

(defun romacs-agent-ui--harness-icon (harness)
  (let ((icon (or (plist-get (cdr (gethash harness romacs-agent--driver-table)) :icon) "●")))
    (if (char-displayable-p (aref icon 0)) icon (symbol-name harness))))

(defun romacs-agent-ui--state-label (session)
  "SESSION's state, when the header should name it."
  (pcase (romacs-agent-state session)
    ('waiting-approval (propertize "Approval" 'face 'warning))
    ((or 'starting 'running) (propertize "Running" 'face 'romacs-agent-running))
    ('detached (propertize "Detached" 'face 'warning))
    ('ended (propertize "Ended" 'face 'romacs-agent-dim))
    ('history (propertize "History" 'face 'romacs-agent-dim))
    (_ (and (romacs-agent-failed session) (propertize "Failed" 'face 'romacs-agent-error)))))

(defun romacs-agent-ui--effort (effort)
  "EFFORT for a person, in its level's face: one Romacs does not know in
`romacs-agent-effort', none as \"default\"."
  (if effort
      (let ((face (intern (concat "romacs-agent-effort-" effort))))
        (propertize effort 'face (if (facep face) face 'romacs-agent-effort)))
    (propertize "default" 'face 'romacs-agent-effort-default)))

(defun romacs-agent-ui--header-line ()
  "The chat's header: harness | permission mode | model | context used/window
| state, fitted to the window (the instance named when not the default)."
  (let* ((session romacs-agent-ui--session)
         (separator (propertize " | " 'face 'romacs-agent-dim))
         (context (and (numberp (romacs-agent-context-used session))
                       (concat (hermes-chat--abbrev-tokens (romacs-agent-context-used session))
                               (if (numberp (romacs-agent-context-window session))
                                   (concat "/" (hermes-chat--abbrev-tokens (romacs-agent-context-window session)))
                                 ""))))
         (mode (romacs-agent-permission-mode session))
         (segments
          (delq nil
                (list (romacs-agent-ui--harness-icon (romacs-agent-harness session))
                      (and (not (equal (romacs-agent-provider session) (ignore-errors (romacs-agent--default-provider))))
                           (propertize (romacs-agent-provider session) 'face 'romacs-agent-dim))
                      ;; Hermes's words for the risky mode: YOLO, in the warning face.
                      (and mode (if (equal mode "bypassPermissions")
                                    (propertize "YOLO" 'face 'warning 'help-echo mode)
                                  (propertize mode 'face 'romacs-agent-mode)))
                      (let ((model (propertize (or (romacs-agent-model session) "...") 'face 'romacs-agent-model))
                            (effort (plist-get (romacs-agent-options session) :effort)))
                        (if effort (concat model (propertize " · " 'face 'romacs-agent-model)
                                           (romacs-agent-ui--effort effort))
                          model))
                      (when-let* ((switch (romacs-agent-pending-selection session)))
                        (propertize (format "→ %s next turn" (romacs-agent-ui--selection-label switch)) 'face 'warning))
                      (when-let* ((children (romacs-agent-children-summary session)))
                        (concat (propertize "⇢ " 'face 'romacs-agent-delegate) children))
                      (and context (propertize context 'face 'romacs-agent-context))
                      (romacs-agent-ui--state-label session))))
         (text (mapconcat #'identity segments separator)))
    (string-replace "%" "%%" (romacs-truncate text (max 1 (window-body-width))))))

;;; The composer

(defun romacs-agent-ui--key (command)
  "COMMAND's key in a chat, as hermes-chat--command-key-hint shows it."
  (substitute-command-keys (format "\\<romacs-agent-chat-mode-map>\\[%s]" command)))

(defvar romacs-agent-ui--hint nil "The composer's hint, made once: looking up keys is slow.")

(defun romacs-agent-ui--hint ()
  "The empty composer's hint (hermes-chat--placeholder-string)."
  (or romacs-agent-ui--hint (setq romacs-agent-ui--hint (romacs-agent-ui--make-hint))))

(defun romacs-agent-ui--make-hint ()
  (let ((hint (concat "Message... · " (romacs-agent-ui--key #'romacs-agent-chat-return) " send · "
                      (romacs-agent-ui--key #'romacs-agent-chat-interrupt) " interrupt · "
                      (romacs-agent-ui--key #'romacs-agent-chat-history-previous) " history")))
    (add-face-text-property 0 (length hint) 'romacs-agent-dim t hint)
    ;; The cursor before the hint, not after it.
    (put-text-property 0 1 'cursor t hint)
    hint))

(defun romacs-agent-ui--live-rule ()
  "The rule above the composer while a turn runs: ● Running, its clock, the tool."
  (let ((session romacs-agent-ui--session))
    (when (memq (romacs-agent-state session) '(starting running waiting-approval))
      (romacs-agent-ui--helpers)
      (let* ((approval (eq (romacs-agent-state session) 'waiting-approval))
             (start (romacs-agent-turn-start session))
             (clock (and (numberp start) (hermes-chat--clock (- (float-time) start)))))
        (concat (if (save-excursion (goto-char romacs-agent-ui--end) (bolp)) "" "\n")
                (romacs-agent-ui--rule
                 (concat (propertize (if approval "◆ Waiting for you" "● Running")
                                     'face (if approval 'romacs-agent-approval 'romacs-agent-running))
                         (if clock (propertize (concat " " clock) 'face 'romacs-agent-dim) "")
                         (if romacs-agent-ui--tool (propertize (concat " · " romacs-agent-ui--tool) 'face 'romacs-agent-dim) "")
                         (if (> (romacs-agent-queue-length session) 0)
                             (propertize (format " · %d queued" (romacs-agent-queue-length session)) 'face 'romacs-agent-dim)
                           "")))
                "\n")))))

(defun romacs-agent-ui--composer-empty-p ()
  (= (marker-position romacs-agent-ui--end) (point-max)))

(defun romacs-agent-ui--composer-update ()
  "The live rule and, in an empty composer, the hint."
  (when (overlayp romacs-agent-ui--composer)
    (move-overlay romacs-agent-ui--composer romacs-agent-ui--end romacs-agent-ui--end)
    (overlay-put romacs-agent-ui--composer 'before-string (romacs-agent-ui--live-rule))
    (overlay-put romacs-agent-ui--composer 'after-string
                 (and (romacs-agent-ui--composer-empty-p) (romacs-agent-ui--hint)))))

;; One timer ticks every shown chat whose turn runs; it ends when none does.
(defvar romacs-agent-ui--ticker nil)

(defun romacs-agent-ui--tick ()
  (let ((running nil))
    (walk-windows
     (lambda (window)
       (with-current-buffer (window-buffer window)
         (when (and (derived-mode-p 'romacs-agent-chat-mode) romacs-agent-ui--session
                    (memq (romacs-agent-state romacs-agent-ui--session) '(starting running waiting-approval)))
           (setq running t)
           (romacs-agent-ui--composer-update))))
     nil 'visible)
    (unless running
      (cancel-timer romacs-agent-ui--ticker)
      (setq romacs-agent-ui--ticker nil))))

(defun romacs-agent-ui--tick-start ()
  (unless (timerp romacs-agent-ui--ticker)
    (setq romacs-agent-ui--ticker (run-at-time 1 1 #'romacs-agent-ui--tick))))

;;; The chat buffer

(defun romacs-agent-ui--in-composer-p ()
  (and romacs-agent-ui--end (>= (point) romacs-agent-ui--end)))

(defun romacs-agent-ui--transcript-key (command)
  "COMMAND in the transcript, the key's own insertion in the composer."
  `(menu-item "" ,command :filter ,(lambda (command) (if (romacs-agent-ui--in-composer-p) #'self-insert-command command))))

(defvar romacs-agent-chat-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agent-chat-return)
    (define-key map (kbd "TAB") #'romacs-agent-chat-toggle)
    (define-key map (kbd "DEL") #'romacs-agent-chat-delete-backward)
    (define-key map [remap self-insert-command] #'romacs-agent-chat-self-insert)
    (define-key map [remap backward-kill-word] #'romacs-agent-chat-backward-kill-word)
    ;; emacs-hermes's chat keys (hermes-chat-mode-map), where a session
    ;; has the same action; stopping and resuming are in the actions menu.
    (define-key map (kbd "C-j") #'romacs-agent-chat-newline)
    (define-key map (kbd "S-<return>") #'romacs-agent-chat-newline)
    (define-key map (kbd "C-c C-c") #'romacs-agent-chat-interrupt)
    (define-key map (kbd "C-c C-i") #'romacs-agent-chat-interrupt)
    (define-key map (kbd "C-c C-k") #'romacs-agent-chat-interrupt-and-send)
    (define-key map (kbd "C-c C-q") #'romacs-agent-chat-queue)
    (define-key map (kbd "C-c C-s") #'romacs-agent-chat-steer)
    (define-key map (kbd "C-c C-a") #'romacs-agent-chat-attach)
    (define-key map (kbd "C-c C-w") #'romacs-agent-chat-remove-image)
    (define-key map (kbd "C-c C-d") #'romacs-agent-chat-deny)
    (define-key map (kbd "C-c C-o") #'romacs-agent-chat-actions-map-popup)
    (define-key map (kbd "C-c C-/") #'romacs-agent-chat-show-commands)
    (define-key map (kbd "C-c C-n") #'romacs-agent-chat-new)
    (define-key map (kbd "C-c C-r") #'romacs-agent-chat-rename)
    (define-key map (kbd "C-c C-e") #'romacs-agent-chat-effort)
    (define-key map (kbd "C-c C-b") #'romacs-agent-switch-chat)
    (define-key map (kbd "C-c C-t") #'romacs-agent-switch)
    (define-key map (kbd "C-c C-j") #'romacs-agent-chat-composer)
    (define-key map (kbd "M-p") #'romacs-agent-chat-history-previous)
    (define-key map (kbd "M-n") #'romacs-agent-chat-history-next)
    (define-key map "?" (romacs-agent-ui--transcript-key #'romacs-agent-chat-actions-map-popup))
    (define-key map "n" (romacs-agent-ui--transcript-key #'romacs-agent-chat-next-turn))
    (define-key map "p" (romacs-agent-ui--transcript-key #'romacs-agent-chat-previous-turn))
    (define-key map "o" (romacs-agent-ui--transcript-key #'romacs-agent-chat-open-task))
    (define-key map "w" (romacs-agent-ui--transcript-key #'romacs-agent-chat-copy))
    (define-key map "e" (romacs-agent-ui--transcript-key #'romacs-agent-chat-edit-turn))
    (define-key map "r" (romacs-agent-ui--transcript-key #'romacs-agent-chat-quote-region))
    (define-key map "y" (romacs-agent-ui--transcript-key #'romacs-agent-chat-allow))
    (define-key map "N" (romacs-agent-ui--transcript-key #'romacs-agent-chat-deny))
    (define-key map " " (romacs-agent-ui--transcript-key #'scroll-up-command))
    (define-key map (kbd "S-SPC") (romacs-agent-ui--transcript-key #'scroll-down-command))
    map)
  "Keys of an agent chat buffer.")

(define-derived-mode romacs-agent-chat-mode fundamental-mode "Agent"
  "An agent session's transcript and composer."
  :interactive nil
  (setq-local desktop-save-buffer #'romacs-agent-ui--desktop-save)
  (setq-local header-line-format '(:eval (romacs-agent-ui--header-line)))
  (setq-local buffer-invisibility-spec (list 'romacs-agent-output))
  ;; Wrapped at words, C-k still killing to the line's end.
  (setq-local truncate-lines nil word-wrap t)
  ;; New turns scroll the chat just enough, the composer at the bottom.
  (setq-local scroll-conservatively 101)
  ;; The hint shows exactly while the composer is empty.
  (add-hook 'post-command-hook #'romacs-agent-ui--composer-update nil t)
  (add-hook 'after-change-functions #'romacs-agent-ui--composer-plain nil t)
  (add-hook 'window-size-change-functions #'romacs-agent-ui--refit-rules nil t)
  ;; The composer moves as text arrives: the tray goes with it.
  (add-hook 'post-command-hook #'romacs-agent-ui--tray nil t)
  ;; Shown again while its turn runs: the live rule ticks again.
  (add-hook 'window-buffer-change-functions #'romacs-agent-ui--shown-again nil t))

(defun romacs-agent-ui--shown-again (&optional window)
  "WINDOW shows this chat again: rules drawn while it was hidden fit it,
and a running turn's live rule ticks."
  (with-current-buffer (if (window-live-p window) (window-buffer window) (current-buffer))
    (romacs-agent-ui--refit-rules)
    (when (and romacs-agent-ui--session
               (memq (romacs-agent-state romacs-agent-ui--session) '(starting running waiting-approval)))
      (romacs-agent-ui--composer-update)
      (romacs-agent-ui--tick-start))))

(defun romacs-agent-show (session)
  "Show SESSION's chat buffer, made from its journal when new."
  (romacs-agent-ui--helpers)
  (let ((buffer (romacs-agent-buffer session)))
    (unless (buffer-live-p buffer)
      ;; The session's own buffer: never found by its title, which another
      ;; session may share.
      (setq buffer (generate-new-buffer (format "*agent: %s*" (romacs-agent-title session))))
      (setf (romacs-agent-buffer session) buffer)
      (with-current-buffer buffer
        (romacs-agent-chat-mode)
        (setq romacs-agent-ui--session session
              romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
              romacs-agent-ui--tasks (make-hash-table :test #'equal)
              romacs-agent-ui--messages (make-hash-table :test #'equal))
        (let ((inhibit-read-only t)) (erase-buffer))
        (setq romacs-agent-ui--end (point-marker))
        (set-marker-insertion-type romacs-agent-ui--end nil)
        (setq romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil)))
      ;; Shown first: its rules fit its window.
      (pop-to-buffer buffer)
      (with-current-buffer buffer
        (romacs-agent-ui--replay session)
        (romacs-agent-ui--composer-update)
        (romacs-agent-ui--summarize-children session)))
    (pop-to-buffer buffer)
    (goto-char (point-max))
    ;; The replay carried the window's start along with the transcript's
    ;; end: the composer goes to the window's bottom, the turns above it.
    (recenter -1)
    (setf (romacs-agent-seen-seq session) (romacs-agent-done-seq session))
    ;; Its parent's failed count may have just lost it.
    (when-let* ((parent (romacs-agent-get (plist-get (romacs-agent-options session) :parent))))
      (romacs-agent-ui--children-changed parent))
    (romacs-agent-ui--dashboard-changed)
    (when (memq (romacs-agent-state session) '(starting running waiting-approval)) (romacs-agent-ui--tick-start))
    buffer))

(defvar desktop-buffer-mode-handlers)

;; A session outlives the editor in its supervisor, so a restored chat
;; (desktop.el, warm-restart.el) carries only which session it shows and
;; the composer's unsent text; the transcript replays from the journal.
(defun romacs-agent-ui--desktop-save (_dirname)
  "The session this chat shows and its unsent message."
  (list :session (romacs-agent-id romacs-agent-ui--session)
        :draft (and (markerp romacs-agent-ui--end) (buffer-substring-no-properties romacs-agent-ui--end (point-max)))
        :images romacs-agent-ui--draft-images))

(defun romacs-agent-ui-desktop-restore (_file _name misc)
  "The chat of MISC's session, made anew from its journal, with its draft."
  (let ((id (plist-get misc :session)))
    (unless (romacs-agent-get id) (romacs-agent-restore))
    (when-let* ((session (romacs-agent-get id))
                (buffer (save-window-excursion (romacs-agent-show session))))
      (with-current-buffer buffer
        (romacs-agent-ui--set-draft-parts (or (plist-get misc :draft) "") (plist-get misc :images)))
      buffer)))

(add-to-list 'desktop-buffer-mode-handlers '(romacs-agent-chat-mode . romacs-agent-ui-desktop-restore))

(declare-function romacs-agent-history-draw "romacs-agent-history" (session file))

(defun romacs-agent-ui--replay (session)
  "Render SESSION's journal into the current chat buffer, after the
harness's own transcript it was imported from, when it was."
  (when (romacs-agent-imported session)
    (require 'romacs-agent-history)
    (romacs-agent-history-draw session (romacs-agent-imported session)))
  ;; A copy parses, so the live session's own state is not replayed over.
  (let ((scratch (copy-romacs-agent session)))
    (setf (romacs-agent-seq scratch) 0 (romacs-agent-turns scratch) 0
          (romacs-agent-title-auto scratch) nil
          (romacs-agent-streamed scratch) (make-hash-table :test #'equal)
          (romacs-agent-tools scratch) (make-hash-table :test #'equal)
          (romacs-agent-approvals scratch) (make-hash-table :test #'equal)
          ;; Its replayed turn ends never send what waits for the live ones,
          ;; nor does its replayed startup write to the harness.
          (romacs-agent-queue scratch) nil (romacs-agent-queue-tail scratch) nil
          (romacs-agent-queue-length scratch) 0
          (romacs-agent-connection scratch) nil (romacs-agent-caught-up scratch) nil
          ;; The live session's: its rows come from its own events.
          (romacs-agent-subagents scratch) nil
          ;; The live session's: a confirmation replayed here is not this copy's.
          (romacs-agent-pending-selection scratch) nil (romacs-agent-models-request scratch) nil
          (romacs-agent-driver-state scratch) nil (romacs-agent-startup-pending scratch) nil)
    (let ((romacs-agent-event-functions
           (list (lambda (_ event)
                   ;; The scratch copy's turn data, for the footers it draws.
                   (setf (romacs-agent-turn-start session) (or (romacs-agent-turn-start scratch) (romacs-agent-turn-start session)))
                   ;; With no supervisor, no event reached the session: its
                   ;; harness's subagents are known from here.
                   (romacs-agent-ui--track-children session event)
                   (romacs-agent-ui--render session event)))))
      (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record))))))

(defun romacs-agent-ui--composer-text ()
  (string-trim (buffer-substring-no-properties romacs-agent-ui--end (point-max))))

(defun romacs-agent-chat-return ()
  "In the composer, send its text; on a tool row, show its output."
  (interactive nil romacs-agent-chat-mode)
  (if (not (romacs-agent-ui--in-composer-p))
      (romacs-agent-chat-toggle)
    (romacs-agent-ui--deliver (lambda (text images) (romacs-agent-send romacs-agent-ui--session text nil nil images)))))

(defun romacs-agent-ui--composer-plain (beg end _length)
  "Text typed or yanked into the composer is plain: whatever it inherited
or brought, nothing hides or styles it."
  (when (and (markerp romacs-agent-ui--end) (< beg end) (>= beg romacs-agent-ui--end))
    ;; Yanked transcript text arrives read-only; yank drops that only after this.
    (let ((inhibit-modification-hooks t) (inhibit-read-only t)) (set-text-properties beg end nil))))

(defun romacs-agent-chat-self-insert (n)
  "Insert the typed character N times in the composer: typed in the
transcript, it goes to the composer's end."
  (interactive "p" romacs-agent-chat-mode)
  (unless (romacs-agent-ui--in-composer-p) (goto-char (point-max)))
  (self-insert-command n))

;; A selection in the composer is replaced by what is typed, as with
;; `self-insert-command'.
(put 'romacs-agent-chat-self-insert 'delete-selection 'delete-selection-uses-region-p)

(defun romacs-agent-chat-backward-kill-word (n)
  "Kill N words backward, in the composer no further than its start: the
transcript above it is not the composer's to kill."
  (interactive "p" romacs-agent-chat-mode)
  (if (not (romacs-agent-ui--in-composer-p))
      (backward-kill-word n)
    ;; Point first: consecutive kills join in the buffer's order.
    (kill-region (point) (max romacs-agent-ui--end (save-excursion (backward-word n) (point))))))

(defun romacs-agent-chat-delete-backward (n)
  "Delete N characters backward in the composer; at its start, nothing:
the transcript above it is not the composer's to delete.  In the
transcript, scroll back, as hermes's chat does."
  (interactive "p" romacs-agent-chat-mode)
  (cond ((not (romacs-agent-ui--in-composer-p)) (scroll-down-command))
        ((> (point) romacs-agent-ui--end) (delete-char (- (min n (- (point) romacs-agent-ui--end)))))))

(defun romacs-agent-chat-toggle ()
  "Show or hide the output of the tool call on this row."
  (interactive nil romacs-agent-chat-mode)
  (let ((id (or (get-text-property (point) 'romacs-agent-tool)
                (get-text-property (line-beginning-position) 'romacs-agent-tool)
                (get-text-property (max (point-min) (1- (line-end-position))) 'romacs-agent-tool))))
    (when id
      (save-excursion
        (forward-line 1)
        (let ((inhibit-read-only t) (start (point)))
          (while (equal (get-text-property (point) 'romacs-agent-output) id) (forward-char 1))
          (when (> (point) start)
            (put-text-property start (point) 'invisible
                               (if (get-text-property start 'invisible) nil 'romacs-agent-output))))))))

(defun romacs-agent-chat-open-task ()
  "Open the thread of the delegated task on this row."
  (interactive nil romacs-agent-chat-mode)
  (let* ((start (line-beginning-position))
         (at (if (get-text-property start 'romacs-agent-task) start
               (next-single-property-change start 'romacs-agent-task nil (line-end-position))))
         (id (get-text-property at 'romacs-agent-task))
         (session (and id (romacs-agent-get id))))
    (cond (session (romacs-agent-show session))
          (id (user-error "Task %s is gone" id))
          (t (user-error "No delegated task on this row")))))

(define-derived-mode romacs-agent-compaction-mode special-mode "Compaction"
  "A compaction's summary."
  (visual-line-mode))

(defun romacs-agent-ui--compaction-buffer ()
  "The buffer to show a summary in: never one that holds something else."
  (let* ((name "*agent compaction*") (buffer (get-buffer name)))
    (if (and buffer (eq (buffer-local-value 'major-mode buffer) 'romacs-agent-compaction-mode))
        buffer
      (generate-new-buffer name))))

(defun romacs-agent-chat-show-compaction (&optional event)
  "Show the summary of the compaction on this row, or at mouse EVENT."
  (interactive (list last-nonmenu-event) romacs-agent-chat-mode)
  (let* ((posn (and (consp event) (event-end event)))
         (summary (if posn
                      (get-text-property (posn-point posn) 'romacs-agent-compaction (window-buffer (posn-window posn)))
                    (get-text-property (point) 'romacs-agent-compaction)))
         (width (romacs-agent-ui--table-width)))
    (unless summary (user-error "No compaction on this row"))
    (romacs-agent-ui--helpers)
    (let ((buffer (romacs-agent-ui--compaction-buffer)))
      (with-current-buffer buffer
        (romacs-agent-compaction-mode)
        (let ((inhibit-read-only t))
          (erase-buffer)
          (insert (romacs-agent-ui--markdown summary width)))
        (goto-char (point-min)))
      (pop-to-buffer buffer))))

(defun romacs-agent-chat-composer ()
  "Go to the composer."
  (interactive nil romacs-agent-chat-mode)
  (goto-char (point-max)))

(defun romacs-agent-ui--run-start (pos property)
  "The start of the run of PROPERTY the character before POS is in."
  (or (previous-single-property-change pos property) (point-min)))

(defun romacs-agent-ui--move-to (property direction)
  "Point to the start of the next (DIRECTION 1) or previous (-1) run of
PROPERTY: the previous one is the run point is in, past its start."
  (let ((pos (point)) (target nil))
    (if (> direction 0)
        (let ((after (if (get-text-property pos property) (next-single-property-change pos property) pos)))
          (setq target (and after (text-property-not-all after (point-max) property nil))))
      (if (and (> pos (point-min)) (get-text-property (1- pos) property))
          (setq target (romacs-agent-ui--run-start pos property))
        (let ((end (and (> pos (point-min)) (previous-single-property-change pos property))))
          (when (and end (> end (point-min)) (get-text-property (1- end) property))
            (setq target (romacs-agent-ui--run-start end property))))))
    (when target (goto-char target))))

(defun romacs-agent-chat-next-turn () "Next prompt." (interactive nil romacs-agent-chat-mode) (romacs-agent-ui--move-to 'romacs-agent-turn 1))
(defun romacs-agent-chat-previous-turn () "Previous prompt." (interactive nil romacs-agent-chat-mode) (romacs-agent-ui--move-to 'romacs-agent-turn -1))

(defun romacs-agent-chat-interrupt () "Interrupt the turn." (interactive nil romacs-agent-chat-mode) (romacs-agent-interrupt romacs-agent-ui--session))
(defun romacs-agent-chat-stop ()
  "Stop the session."
  (interactive nil romacs-agent-chat-mode)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop romacs-agent-ui--session)))
(defun romacs-agent-chat-resume () "Resume the session." (interactive nil romacs-agent-chat-mode) (romacs-agent-resume romacs-agent-ui--session))

(defun romacs-agent-ui--approval-here ()
  "The approval request on this row, else the session's only one."
  (or (get-text-property (line-beginning-position) 'romacs-agent-approval)
      (let ((ids (hash-table-keys (romacs-agent-approvals romacs-agent-ui--session))))
        (and (= (length ids) 1) (car ids)))))

(defun romacs-agent-chat-allow ()
  "Allow the tool call the harness asks about."
  (interactive nil romacs-agent-chat-mode)
  (let ((id (romacs-agent-ui--approval-here)))
    (unless id (user-error "No approval here"))
    (romacs-agent-respond romacs-agent-ui--session id t)))

(defun romacs-agent-chat-deny ()
  "Deny the tool call the harness asks about."
  (interactive nil romacs-agent-chat-mode)
  (let ((id (romacs-agent-ui--approval-here)))
    (unless id (user-error "No approval here"))
    (romacs-agent-respond romacs-agent-ui--session id nil)))

;;; emacs-hermes's chat actions

(defun romacs-agent-ui--running-p ()
  (memq (romacs-agent-state romacs-agent-ui--session) '(starting running waiting-approval)))

(defun romacs-agent-ui--draft-images-checked ()
  "The draft's images, or a `user-error' naming one that may not be sent:
the draft stays, for the tray to take it off."
  (dolist (file romacs-agent-ui--draft-images romacs-agent-ui--draft-images)
    (unless (romacs-agent-image-file-p file)
      (user-error "%s is gone or too large: %s removes it" (file-name-nondirectory file)
                  (substitute-command-keys "\\<romacs-agent-chat-mode-map>\\[romacs-agent-chat-remove-image]")))))

(defun romacs-agent-ui--deliver (send)
  "Call SEND with the composer's text and attached images, then empty it into its
history; when SEND fails, the draft stays.  Nothing when it is empty."
  (let* ((images (romacs-agent-ui--draft-images-checked))
         (written (romacs-agent-ui--composer-text))
         ;; A harness wants words: an image alone gets these.
         (text (if (and images (string-empty-p written)) "See the attached image." written)))
    (unless (string-empty-p text)
      (funcall send text images)
      (push (cons text images) romacs-agent-ui--history)
      (romacs-agent-ui--history-reset)
      (delete-region romacs-agent-ui--end (point-max))
      (setq romacs-agent-ui--draft-images nil)
      (romacs-agent-ui--tray)
      (romacs-agent-ui--composer-update))))

(defun romacs-agent-chat-newline ()
  "A new line in the composer."
  (interactive nil romacs-agent-chat-mode)
  (unless (romacs-agent-ui--in-composer-p) (goto-char (point-max)))
  (insert "\n"))

(defun romacs-agent-chat-queue ()
  "Send the composer's text when the running turn ends; now, when none runs."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--deliver (lambda (text images) (romacs-agent-send-later romacs-agent-ui--session text nil images))))

(defun romacs-agent-chat-steer ()
  "Send the composer's text now, into the running turn."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--deliver (lambda (text images) (romacs-agent-send romacs-agent-ui--session text nil nil images))))

(defun romacs-agent-chat-interrupt-and-send ()
  "Interrupt the running turn; the composer's text is sent once it ends."
  (interactive nil romacs-agent-chat-mode)
  (let ((session romacs-agent-ui--session))
    (if (not (romacs-agent-ui--running-p))
        (romacs-agent-ui--deliver (lambda (text images) (romacs-agent-send session text nil nil images)))
      ;; Interrupted first: a failure keeps the draft.
      (romacs-agent-interrupt session)
      (romacs-agent-ui--deliver (lambda (text images) (romacs-agent-send-later session text t images))))))

(defvar-local romacs-agent-ui--history-at nil
  "The history's cell whose prompt the composer shows, or nil for the draft.")
(defvar-local romacs-agent-ui--history-back nil
  "The cells M-p left, newest first: where M-n goes back to.")
(defvar-local romacs-agent-ui--history-draft nil
  "The composer's text, as it was, when M-p first left it.")

(defun romacs-agent-ui--history-reset ()
  (setq romacs-agent-ui--history-at nil romacs-agent-ui--history-back nil))


(defun romacs-agent-chat-history-previous ()
  "Put the previous prompt in the composer; the draft is kept for M-n."
  (interactive nil romacs-agent-chat-mode)
  (let ((next (if romacs-agent-ui--history-at (cdr romacs-agent-ui--history-at) romacs-agent-ui--history)))
    (when next
      (unless romacs-agent-ui--history-at
        (setq romacs-agent-ui--history-draft
              (cons (buffer-substring-no-properties romacs-agent-ui--end (point-max)) romacs-agent-ui--draft-images)))
      (push romacs-agent-ui--history-at romacs-agent-ui--history-back)
      (setq romacs-agent-ui--history-at next)
      (romacs-agent-ui--set-draft-parts (caar next) (cdar next)))))

(defun romacs-agent-chat-history-next ()
  "Put the next prompt in the composer; past the newest, the draft."
  (interactive nil romacs-agent-chat-mode)
  (when romacs-agent-ui--history-at
    (setq romacs-agent-ui--history-at (pop romacs-agent-ui--history-back))
    (if romacs-agent-ui--history-at
        (romacs-agent-ui--set-draft-parts (caar romacs-agent-ui--history-at) (cdar romacs-agent-ui--history-at))
      (romacs-agent-ui--set-draft-parts (or (car romacs-agent-ui--history-draft) "")
                                        (cdr romacs-agent-ui--history-draft)))))

(defun romacs-agent-ui--turn-bounds ()
  "The turn at point: (START . END), from its prompt to the next one's.
A buffer of this mode no session shows has no turns."
  (unless (markerp romacs-agent-ui--end) (user-error "No session here"))
  (let* ((here (point))
         (start (save-excursion (if (get-text-property here 'romacs-agent-turn) (forward-char 1))
                                (if (romacs-agent-ui--move-to 'romacs-agent-turn -1) (point) (point-min))))
         (end (save-excursion (goto-char (max here start))
                              (if (romacs-agent-ui--move-to 'romacs-agent-turn 1) (point) romacs-agent-ui--end))))
    (cons start (min end romacs-agent-ui--end))))

(defun romacs-agent-ui--text-of (start end)
  "The text between START and END, without the rows that only draw images."
  (let ((parts nil) (at start))
    (while (< at end)
      (let ((next (next-single-property-change at 'romacs-agent-strip nil end)))
        (unless (get-text-property at 'romacs-agent-strip)
          (push (buffer-substring-no-properties at next) parts))
        (setq at next)))
    (apply #'concat (nreverse parts))))

(defun romacs-agent-chat-copy ()
  "Copy the region, else the turn at point (its prompt and reply)."
  (interactive nil romacs-agent-chat-mode)
  (pcase-let ((`(,start . ,end) (if (use-region-p) (cons (region-beginning) (region-end)) (romacs-agent-ui--turn-bounds))))
    (kill-new (string-trim (romacs-agent-ui--text-of start end)))
    (message "Copied")))

(defun romacs-agent-chat-edit-turn ()
  "Put the prompt of the turn at point in the composer, to edit and send."
  (interactive nil romacs-agent-chat-mode)
  (let* ((start (let ((start (car (romacs-agent-ui--turn-bounds))))
                  ;; Before the first prompt: nothing to edit, and the draft stays.
                  (unless (get-text-property start 'romacs-agent-turn) (user-error "No turn here"))
                  start))
         (images (get-text-property (+ start 2) 'romacs-agent-images))
         (end (or (next-single-property-change start 'romacs-agent-turn nil romacs-agent-ui--end) romacs-agent-ui--end))
         (prompt (string-remove-prefix "> " (string-trim-right (buffer-substring-no-properties start end)))))
    (romacs-agent-ui--set-draft-parts prompt images)))

(defcustom romacs-agent-image-directory "~/Pictures/Screenshots/"
  "Where `romacs-agent-chat-attach' offers images from."
  :type 'directory :group 'romacs-agent)

(defun romacs-agent-ui--clipboard-image-type ()
  "The clipboard's image MIME type, or nil."
  (seq-find (lambda (type) (string-prefix-p "image/" type))
            (ignore-errors (process-lines "wl-paste" "--list-types"))))

(defun romacs-agent-ui--save-clipboard-image (session type)
  "Save the clipboard's image of MIME TYPE with SESSION; return its file."
  (let* ((dir (expand-file-name "attachments" (romacs-agent-dir session)))
         (file (progn (make-directory dir t)
                      (make-temp-file (expand-file-name "clipboard-" dir) nil
                                      (concat "." (substring type (length "image/"))))))
         (saved nil))
    (unwind-protect
        (progn
          (unless (eql 0 (call-process "wl-paste" nil (list :file file) nil "--type" type))
            (user-error "Could not read the clipboard's image"))
          (setq saved t)
          file)
      (unless saved (delete-file file)))))

(defun romacs-agent-ui--read-image-file ()
  "An image file, chosen by sight in `romacs-agent-image-directory', or
named when it holds none or the user asks to browse."
  (require 'romacs-agent-images)
  (let* ((dir (file-name-as-directory (expand-file-name romacs-agent-image-directory)))
         (dir (if (file-directory-p dir) dir default-directory))
         (choice (if (romacs-agent-images--files dir) (romacs-agent-images-pick dir) 'browse)))
    (pcase choice
      ('cancel (keyboard-quit))
      ('browse (expand-file-name (read-file-name "Attach image: " dir nil t)))
      (_ choice))))

(defun romacs-agent-chat-attach (file)
  "Add image FILE to the message being written; the tray above the
composer shows what it carries.  Interactively the clipboard's
image, saved with the session; with a prefix argument, or when the
clipboard holds none, one picked by sight (`romacs-agent-images-pick')."
  (interactive
   (let ((type (and (not current-prefix-arg) (romacs-agent-ui--clipboard-image-type))))
     (list (if type (romacs-agent-ui--save-clipboard-image romacs-agent-ui--session type)
             (romacs-agent-ui--read-image-file))))
   romacs-agent-chat-mode)
  ;; Codex's --image splits its value at commas.
  (when (string-match-p "," file)
    (user-error "An image's file name cannot hold a comma: %s" file))
  (let ((size (file-attribute-size (file-attributes (file-truename file)))))
    (when (and size (> size romacs-agent-image-max-bytes))
      (user-error "%s is %s, over `romacs-agent-image-max-bytes' (%s)" (file-name-nondirectory file)
                  (file-size-human-readable size) (file-size-human-readable romacs-agent-image-max-bytes))))
  (require 'romacs-agent-images)
  (setq romacs-agent-ui--draft-images (append romacs-agent-ui--draft-images (list file)))
  (romacs-agent-ui--tray))

(defun romacs-agent-chat-remove-image (n)
  "Take the Nth image off the message being written; the last, by default."
  (interactive "P" romacs-agent-chat-mode)
  (let* ((images romacs-agent-ui--draft-images)
         (index (if n (1- (prefix-numeric-value n)) (1- (length images)))))
    (unless (and images (>= index 0) (< index (length images)))
      (user-error "No such image in this message"))
    (setq romacs-agent-ui--draft-images (append (seq-take images index) (nthcdr (1+ index) images)))
    (romacs-agent-ui--tray)))

(defun romacs-agent-ui--set-draft-parts (text images)
  "Make TEXT, as it is, the composer's and IMAGES the tray's, in place of
what was there."
  (delete-region romacs-agent-ui--end (point-max))
  (goto-char (point-max))
  (insert text)
  (setq romacs-agent-ui--draft-images images)
  (when images (require 'romacs-agent-images))
  (romacs-agent-ui--tray))

(defun romacs-agent-ui--tray (&rest _)
  "Bring the tray up to date."
  (when (featurep 'romacs-agent-images) (romacs-agent-images-tray-sync)))

(defun romacs-agent-chat-quote-region ()
  "Quote the region in the composer, each line after \"> \"."
  (interactive nil romacs-agent-chat-mode)
  (unless (use-region-p) (user-error "No region to quote"))
  (let ((text (buffer-substring-no-properties (region-beginning) (region-end))))
    (deactivate-mark)
    (goto-char (point-max))
    (unless (bolp) (insert "\n"))
    (insert (mapconcat (lambda (line) (concat "> " line)) (split-string (string-trim-right text) "\n") "\n") "\n")))

(defun romacs-agent-ui--set-outputs (invisible)
  (let ((inhibit-read-only t) (position (point-min)))
    (while (setq position (text-property-not-all position romacs-agent-ui--end 'romacs-agent-output nil))
      (let ((end (or (next-single-property-change position 'romacs-agent-output nil romacs-agent-ui--end)
                     romacs-agent-ui--end)))
        (put-text-property position end 'invisible (and invisible 'romacs-agent-output))
        (setq position end)))))

(defun romacs-agent-chat-expand-all ()
  "Show every tool call's output."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--set-outputs nil))

(defun romacs-agent-chat-collapse-all ()
  "Fold every tool call's output."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--set-outputs t))

(defun romacs-agent-chat-rename (title)
  "Name this session TITLE."
  (interactive (list (read-string "Title: " (romacs-agent-title romacs-agent-ui--session)))
               romacs-agent-chat-mode)
  (let ((session romacs-agent-ui--session))
    (setf (romacs-agent-title session) title (romacs-agent-title-auto session) nil)
    (romacs-agent--update-manifest session)
    (rename-buffer (format "*agent: %s*" title) t)
    (romacs-agent-ui--dashboard-changed)))

(defun romacs-agent-chat-effort (effort)
  "Run this session at reasoning EFFORT from its next turn on, its model
kept; nil leaves it to the harness."
  (interactive
   (let* ((session romacs-agent-ui--session)
          (efforts (or (romacs-agent--efforts session) (user-error "This model offers no effort"))))
     (list (romacs-agent--read-effort efforts (plist-get (romacs-agent--wanted session) :effort))))
   romacs-agent-chat-mode)
  (let ((session romacs-agent-ui--session))
    (unless (romacs-agent-select session (plist-get (romacs-agent--wanted session) :model) effort)
      (message "Effort is already %s" (or effort "the harness's")))))

(defun romacs-agent-chat-model (model effort)
  "Run this session on MODEL, of its own instance, from its next turn on,
at EFFORT: the effort it runs at when MODEL takes it, else MODEL's default;
with a prefix argument, asked for."
  (interactive
   (let* ((session romacs-agent-ui--session)
          (instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session)))
          (fields (cdr (romacs-agent--pick-row
                        (romacs-agent--instance-rows instance (plist-get instance :display) nil))))
          (model (plist-get fields :model))
          (efforts (plist-get fields :efforts))
          (now (plist-get (romacs-agent--wanted session) :effort))
          (fitting (if (member now efforts) now (plist-get fields :default-effort))))
     (list model (if (and current-prefix-arg efforts) (romacs-agent--read-effort efforts fitting) fitting)))
   romacs-agent-chat-mode)
  (unless (romacs-agent-select romacs-agent-ui--session model effort)
    (message "Already on %s" (romacs-agent-ui--selection-label (list :model model :effort effort)))))

(defun romacs-agent-ui--model-label ()
  "The model entry: the model running, and the one to come if it differs."
  (let* ((session romacs-agent-ui--session)
         (harness (and session (romacs-agent-harness session)))
         (shown (lambda (model)
                  (propertize (or (and model (romacs-agent-ui--instance nil harness model nil)) "default")
                              'face (if model 'romacs-agent-model 'romacs-agent-effort-default))))
         (now (and session (or (plist-get (romacs-agent--selection session) :model) (romacs-agent-model session))))
         (next (and session (plist-get (romacs-agent--wanted session) :model))))
    (concat "Model: " (funcall shown now)
            (if (or (null (romacs-agent-pending-selection session)) (equal now next)) ""
              (concat " → " (funcall shown next))))))

(defun romacs-agent-chat-new ()
  "Start a new session, in this one's directory."
  (interactive nil romacs-agent-chat-mode)
  (apply #'funcall-interactively #'romacs-agent-start
         (romacs-agent--start-arguments (file-name-as-directory (romacs-agent-cwd romacs-agent-ui--session)))))

(defun romacs-agent-ui--attention (session)
  "How much SESSION wants the user, least first: its place in the switcher."
  (cond ((eq (romacs-agent-state session) 'waiting-approval) 0)
        ;; Its result reached its parent: unread here says nothing.
        ((romacs-agent-ui--finished-child-p session) 4)
        ((> (romacs-agent-done-seq session) (romacs-agent-seen-seq session)) 1)
        ((memq (romacs-agent-state session) '(starting running)) 2)
        (t 3)))

(defun romacs-agent-ui--by-attention (sessions)
  "SESSIONS, those wanting the user first, then the latest active."
  (sort (copy-sequence sessions)
        (lambda (a b)
          (let ((x (romacs-agent-ui--attention a)) (y (romacs-agent-ui--attention b)))
            (if (= x y)
                (> (or (romacs-agent-last-time a) 0) (or (romacs-agent-last-time b) 0))
              (< x y))))))

(defun romacs-agent-ui--context-share (session)
  "How full SESSION's context is, as \"NN%\", or nil when it is not known."
  (let ((used (romacs-agent-context-used session)) (window (romacs-agent-context-window session)))
    (and (numberp used) (numberp window) (> window 0)
         (format "%d%%" (round (* 100.0 (/ used (float window))))))))

(defun romacs-agent-ui--one-line (text width)
  "TEXT on one line, shortened to WIDTH, reading no more of it than that
can show: a reply can be long."
  (let ((text (or text "")))
    (romacs-truncate
     (string-trim (replace-regexp-in-string "[\n\t ]+" " " (substring text 0 (min (length text) (* 4 (max 0 width))))))
     width)))

(defcustom romacs-agent-switch-context-share 70
  "The context share, in percent, from which the switcher shows a
session's: below, it is nothing to act on."
  :type 'natnum :group 'romacs-agent)

(defun romacs-agent-ui--switch-facts (session default)
  "What is worth acting on in SESSION at a glance, each in its face: its
project, its context when nearly full, what waits in its queue, and its
instance when not DEFAULT."
  (let ((queued (romacs-agent-queue-length session))
        (used (romacs-agent-context-used session)) (window (romacs-agent-context-window session))
        (provider (romacs-agent-provider session)))
    (delq nil (list (and (romacs-agent-cwd session)
                         (propertize (file-name-nondirectory (directory-file-name (romacs-agent-cwd session)))
                                     'face 'romacs-agent-project))
                    (and (numberp used) (numberp window) (> window 0)
                         (>= (* 100.0 (/ used (float window))) romacs-agent-switch-context-share)
                         (propertize (romacs-agent-ui--context-share session) 'face 'warning))
                    (and (> queued 0) (propertize (format "%d queued" queued) 'face 'romacs-agent-running))
                    (and provider (not (equal provider default)) (propertize provider 'face 'romacs-agent-dim))))))

(defun romacs-agent-ui--plain (text)
  "TEXT, a reply's Markdown, as plain words for one line: links as their
text, code and emphasis marks, headings' and lists' marks gone."
  (thread-last (or text "")
               (replace-regexp-in-string "```[^\n]*" "")
               (replace-regexp-in-string "\\[\\([^]]*\\)\\](\\([^)]*\\))" "\\1")
               (replace-regexp-in-string "\\*\\*\\|__\\|`" "")
               (replace-regexp-in-string "^[ \t]*\\(?:#+\\|[-*+]\\|[0-9]+\\.\\)[ \t]+" "")
               (romacs-agent-ui--unemphasize)))

(defun romacs-agent-ui--unemphasize (text)
  "TEXT without single-marker emphasis, *word* or _word_, an underscore
within a word kept.  Again until none is left: a match takes the boundary
an adjacent span needs."
  (let ((next (replace-regexp-in-string
               "\\(^\\|[^[:alnum:]]\\)\\([*_]\\)\\([^*_ \n][^*_\n]*\\)\\2\\([^[:alnum:]]\\|$\\)"
               "\\1\\3\\4" text)))
    (if (equal next text) text (romacs-agent-ui--unemphasize next))))

(defconst romacs-agent-ui--switch-words '("working" "needs you" "failed")
  "The states the switcher names: the rest its mark says.")

(defun romacs-agent-ui--switch-suffix (session width default)
  "What the switcher says of SESSION after its title, in at most WIDTH
columns: a state that wants attention, then facts, then what room leaves
of its reply.  DEFAULT is the provider not worth naming."
  (pcase-let* ((`(,_mark ,face ,word) (romacs-agent-ui--status session))
               (unread (> (romacs-agent-done-seq session) (romacs-agent-seen-seq session)))
               (head (concat (propertize (format "%-9s" (if (member word romacs-agent-ui--switch-words) word ""))
                                         'face face)
                             (propertize (format "%5s  " (or (romacs-agent-ui--ago (romacs-agent-last-time session)) ""))
                                         'face 'romacs-agent-dim)
                             (if unread (propertize "● " 'face 'romacs-agent-unread) "  ")))
               (space (- width (string-width head)))
               (facts (romacs-truncate
                       (mapconcat #'identity (romacs-agent-ui--switch-facts session default) " · ") space))
               (room (- space (string-width facts) 2)))
    (truncate-string-to-width
     (concat head facts
             (if (> room 8)
                 (propertize (concat "  " (romacs-agent-ui--one-line (romacs-agent-ui--plain (romacs-agent-reply session)) room))
                             'face 'romacs-agent-dim)
               ""))
     (max 0 width))))

(defun romacs-agent-ui--session-names (sessions)
  "A table of a unique name to each of SESSIONS, and the names in order, as
\(TABLE . NAMES).  A title in use takes its id, then as much of it as it
needs, then a count: every name stands for one session."
  (let ((table (make-hash-table :test #'equal)) (names nil))
    (dolist (session sessions)
      (let* ((title (or (romacs-agent-title session) "session"))
             (id (romacs-agent-id session))
             (name title) (n 1))
        (while (gethash name table)
          (setq name (if (= n 1) (format "%s <%s>" title (substring id 0 (min 8 (length id))))
                       (format "%s <%s %d>" title (substring id 0 (min 8 (length id))) n))
                n (1+ n)))
        (puthash name session table)
        (push name names)))
    (cons table (nreverse names))))

(defun romacs-agent-ui--switch-row (name session column width default)
  "NAME's row for SESSION: the affixation of a candidate, whose value stays
NAME and whose shown form fits COLUMN.  DEFAULT is the provider not worth
naming."
  (let ((shown (romacs-truncate name column)))
    (list (if (equal shown name) name (propertize name 'display shown))
          (concat (romacs-agent-ui--status-mark session) " ")
          (concat (make-string (max 1 (- (+ column 2) (string-width shown))) ?\s)
                  (romacs-agent-ui--switch-suffix session (- width column 4) default)))))

(defun romacs-agent-ui--session-table (table names width)
  "A completion table of NAMES in their order, each TABLE's session, each
row its status within WIDTH columns."
  (let* ((column (min 44 (apply #'max 1 (mapcar #'string-width names))))
         (default (romacs-agent-ui--default-provider))
         (rows (lambda (candidates)
                 (mapcar (lambda (name)
                           (romacs-agent-ui--switch-row name (gethash name table) column width default))
                         candidates))))
    (lambda (string predicate action)
      (if (eq action 'metadata)
          `(metadata (category . romacs-agent) (display-sort-function . identity)
                     (cycle-sort-function . identity) (affixation-function . ,rows))
        (complete-with-action action names string predicate)))))

(defun romacs-agent-ui--status-mark (session)
  "SESSION's status mark, in its face."
  (pcase-let ((`(,mark ,face ,_word) (romacs-agent-ui--status session)))
    (propertize (or mark " ") 'face face)))

(defvar romacs-complete-center-width)

(defcustom romacs-agent-switch-width 0.8
  "How much of the frame's width `romacs-agent-switch' takes: a row says more
than most completions."
  :type 'float :group 'romacs-agent)

(defun romacs-agent-ui--finished-child-p (session)
  "Whether SESSION is a delegated task with nothing more for anyone to do."
  (and (plist-get (romacs-agent-options session) :parent)
       (not (memq (romacs-agent-state session) '(starting running waiting-approval)))))

(defun romacs-agent-ui--switch-sessions ()
  "Every session but this chat's, those wanting the user first."
  (let ((here (and (derived-mode-p 'romacs-agent-chat-mode) romacs-agent-ui--session)))
    (romacs-agent-ui--by-attention (remq here (romacs-agent-sessions)))))

(defun romacs-agent-ui--open-sessions ()
  "The sessions of `romacs-agent-ui--switch-sessions' with a live chat buffer."
  (seq-filter (lambda (session) (buffer-live-p (romacs-agent-buffer session)))
              (romacs-agent-ui--switch-sessions)))

(defun romacs-agent-ui--resolve-session (table names answer)
  "The session of ANSWER, as a prompt accepted it: its case may differ."
  (or (gethash answer table)
      (let ((name (seq-find (lambda (name) (eq t (compare-strings name nil nil answer nil nil t))) names)))
        (and name (gethash name table)))
      (user-error "No agent session %s" answer)))

(defun romacs-agent-ui--read-session (prompt sessions none)
  "Read one of SESSIONS by its status with PROMPT; with none, say NONE."
  (pcase-let ((`(,table . ,names) (romacs-agent-ui--session-names sessions)))
    (unless names (user-error "%s" none))
    (let ((romacs-complete-center-width romacs-agent-switch-width))
      (romacs-agent-ui--resolve-session
       table names
       (completing-read prompt
                        (romacs-agent-ui--session-table
                         table names (- (floor (* (frame-width) romacs-agent-switch-width)) 6))
                        nil t)))))

;;;###autoload
(defun romacs-agent-switch (session)
  "Switch to agent SESSION, in this window, chosen by what each is doing:
those that want you first, finished delegated tasks last, each with its
state, last activity, context, model, project and the start of its last
reply.  A session with no chat yet gets one."
  (interactive (list (romacs-agent-ui--read-session
                      "Agent: " (romacs-agent-ui--switch-sessions) "No other agent sessions")))
  (let ((display-buffer-overriding-action '(display-buffer-same-window)))
    (romacs-agent-show session)))

;;;###autoload
(defun romacs-agent-switch-chat (session)
  "Switch to agent SESSION as `romacs-agent-switch' does, the prompt offering
only sessions whose chat is open."
  (interactive (list (romacs-agent-ui--read-session
                      "Chat: " (romacs-agent-ui--open-sessions) "No other agent chats are open")))
  (romacs-agent-switch session))

(defalias 'romacs-agent-chat-switch #'romacs-agent-switch-chat)

(defun romacs-agent-chat-show-commands ()
  "List this chat's keys."
  (interactive nil romacs-agent-chat-mode)
  (describe-keymap 'romacs-agent-chat-mode-map))

(defun romacs-agent-chat-home () "The agents home page." (interactive nil romacs-agent-chat-mode) (romacs-agents))

(defun romacs-agent-ui--queue-label ()
  (let ((waiting (and romacs-agent-ui--session (romacs-agent-queue-length romacs-agent-ui--session))))
    (if (and waiting (> waiting 0)) (format "Queue (%d waiting)" waiting) "Queue")))

(defun romacs-agent-ui--effort-label ()
  "The effort entry: the effort running, and the one to come if it differs."
  (let* ((session romacs-agent-ui--session)
         (now (and session (plist-get (romacs-agent--selection session) :effort)))
         (next (and session (plist-get (romacs-agent--wanted session) :effort))))
    (concat "Effort: " (romacs-agent-ui--effort now)
            (if (equal now next) "" (concat " → " (romacs-agent-ui--effort next))))))

(require 'keymap-popup)

(defconst romacs-agent-ui--finished-workers 3
  "How many finished workers the actions menu keeps after the active ones.")

(defun romacs-agent-ui--workers (rows)
  "ROWS (`romacs-agent-children-rows', ranked) as the menu lists them:
those running or wanting the user, then the few that finished last,
however they ended."
  (let* ((active (seq-filter (lambda (row) (< (romacs-agent-ui--child-rank row) 3)) rows))
         (finished (sort (seq-difference rows active #'eq)
                         (lambda (a b) (> (or (plist-get a :time) 0) (or (plist-get b :time) 0))))))
    (append active (seq-take finished romacs-agent-ui--finished-workers))))

(defun romacs-agent-ui--children-here ()
  (and romacs-agent-ui--session (romacs-agent-ui--workers (romacs-agent-children-rows romacs-agent-ui--session))))

(defun romacs-agent-ui--children-title ()
  (let ((summary (and romacs-agent-ui--session (romacs-agent-children-summary romacs-agent-ui--session))))
    ;; A menu heading fits 40 columns.
    (if summary (concat "Workers " summary) "Workers")))

(defun romacs-agent-ui--child-label (index)
  "The menu's row of the child at INDEX, or nil to show none: its state,
what it is doing and when it last did something, its title in the room a
menu entry leaves (`keymap-popup--max-description-width')."
  (when-let* ((row (nth index (romacs-agent-ui--children-here))))
    (let* ((state (plist-get row :state))
           (activity (and (equal state "running") (plist-get row :activity)
                          (truncate-string-to-width
                           (replace-regexp-in-string "\\`mcp__[^_]+\\(?:_[^_]+\\)*__" "" (plist-get row :activity))
                           14 nil nil "…")))
           (ago (romacs-agent-ui--ago (plist-get row :time)))
           (tail (mapconcat #'identity
                            (delq nil (list (romacs-agent-ui--chip state)
                                            (and activity (propertize activity 'face 'romacs-agent-running))
                                            (and (not (string-empty-p ago)) (propertize ago 'face 'romacs-agent-dim))))
                            " ")))
      (concat (truncate-string-to-width (or (plist-get row :title) "task")
                                        (max 8 (- keymap-popup--max-description-width (string-width tail) 1))
                                        nil nil "…")
              " " tail))))

(defun romacs-agent-chat-open-child ()
  "Open the child on the menu row of the digit just pressed: a delegated
task's chat, or a harness's own subagent's card in this one."
  (interactive nil romacs-agent-chat-mode)
  (let ((row (nth (- last-command-event ?1) (romacs-agent-ui--children-here))))
    (cond ((null row) (user-error "No such subagent"))
          ((plist-get row :session) (romacs-agent-switch (plist-get row :session)))
          (t (goto-char (or (gethash (plist-get row :tool) romacs-agent-ui--tool-lines)
                            (user-error "Its card is not in this chat")))
             (recenter)))))

(defun romacs-agent-chat-children (session)
  "Switch to SESSION, chosen among those this one delegated."
  (interactive (list (romacs-agent-ui--read-session
                      "Subagent: " (romacs-agent-ui--by-attention (romacs-agent-children romacs-agent-ui--session))
                      "No delegated tasks"))
               romacs-agent-chat-mode)
  (romacs-agent-switch session))

(keymap-popup-define romacs-agent-chat-actions-map
  "Actions of an agent chat: an approval waiting first; then the turn, what
it runs on, the session and the workers under it; then composing and the
view."
  :popup-key "?"
  :exit-key "C-g"
  :group ("Approval" :if (lambda () (and romacs-agent-ui--session
                                         (> (hash-table-count (romacs-agent-approvals romacs-agent-ui--session)) 0))))
  "a" ("Allow" romacs-agent-chat-allow)
  "d" ("Deny" romacs-agent-chat-deny)
  :row
  :group "Turn"
  "s" ("Steer (send now)" romacs-agent-chat-steer :inapt-if (lambda () (not (romacs-agent-ui--running-p))))
  "i" ("Interrupt" romacs-agent-chat-interrupt :inapt-if (lambda () (not (romacs-agent-ui--running-p))))
  "k" ("Interrupt + send" romacs-agent-chat-interrupt-and-send :inapt-if (lambda () (not (romacs-agent-ui--running-p))))
  "q" (#'romacs-agent-ui--queue-label romacs-agent-chat-queue)
  :group "Runs on"
  "m" (#'romacs-agent-ui--model-label romacs-agent-chat-model)
  "E" (#'romacs-agent-ui--effort-label romacs-agent-chat-effort)
  :group "Session"
  "b" ("Switch chat" romacs-agent-switch-chat)
  "t" ("Switch thread" romacs-agent-switch)
  "n" ("New session" romacs-agent-chat-new)
  "R" ("Rename" romacs-agent-chat-rename)
  "r" ("Resume" romacs-agent-chat-resume :inapt-if #'romacs-agent-ui--running-p)
  "x" ("Stop" romacs-agent-chat-stop :inapt-if (lambda () (eq (romacs-agent-state romacs-agent-ui--session) 'ended)))
  "h" ("Home page" romacs-agent-chat-home)
  :group (#'romacs-agent-ui--children-title :if #'romacs-agent-ui--children-here)
  "1" ((lambda () (romacs-agent-ui--child-label 0)) romacs-agent-chat-open-child)
  "2" ((lambda () (romacs-agent-ui--child-label 1)) romacs-agent-chat-open-child)
  "3" ((lambda () (romacs-agent-ui--child-label 2)) romacs-agent-chat-open-child)
  "4" ((lambda () (romacs-agent-ui--child-label 3)) romacs-agent-chat-open-child)
  "5" ((lambda () (romacs-agent-ui--child-label 4)) romacs-agent-chat-open-child)
  "6" ((lambda () (romacs-agent-ui--child-label 5)) romacs-agent-chat-open-child)
  "7" ((lambda () (romacs-agent-ui--child-label 6)) romacs-agent-chat-open-child)
  "8" ((lambda () (romacs-agent-ui--child-label 7)) romacs-agent-chat-open-child)
  "9" ((lambda () (romacs-agent-ui--child-label 8)) romacs-agent-chat-open-child)
  "o" ("All delegated tasks" romacs-agent-chat-children
       :if (lambda () (romacs-agent-children romacs-agent-ui--session)))
  :row
  :group "Compose"
  "A" ("Attach image" romacs-agent-chat-attach)
  "W" ("Remove image" romacs-agent-chat-remove-image :inapt-if (lambda () (not romacs-agent-ui--draft-images)))
  "Q" ("Quote region" romacs-agent-chat-quote-region :inapt-if (lambda () (not (use-region-p))))
  "e" ("Edit turn at point" romacs-agent-chat-edit-turn)
  "w" ("Copy turn or region" romacs-agent-chat-copy)
  :group "View"
  "+" ("Expand all outputs" romacs-agent-chat-expand-all)
  "-" ("Collapse all outputs" romacs-agent-chat-collapse-all)
  "c" ("Show commands" romacs-agent-chat-show-commands))

(put 'romacs-agent-chat-actions-map-popup 'command-modes '(romacs-agent-chat-mode))

;;; The home page

(defun romacs-agent-ui--ago (time)
  "How long ago TIME was, compactly."
  (if (not (numberp time)) ""
    (let ((seconds (max 0 (- (float-time) time))))
      (cond ((< seconds 60) "now")
            ((< seconds 3600) (format "%dm" (/ seconds 60)))
            ((< seconds 86400) (format "%dh" (/ seconds 3600)))
            (t (format "%dd" (/ seconds 86400)))))))

(defun romacs-agent-ui--status (session)
  "SESSION's (MARK FACE WORD) on the home page."
  (let ((state (romacs-agent-state session)))
    (cond ((memq state '(starting running)) '("●" romacs-agent-running "working"))
          ((eq state 'waiting-approval) '("◆" romacs-agent-approval "needs you"))
          ((romacs-agent-cancelled session) '("○" romacs-agent-dim "cancelled"))
          ((romacs-agent-failed session) '("!" romacs-agent-error "failed"))
          ((eq state 'detached) '("○" romacs-agent-dim "detached"))
          ((eq state 'ended) (if (> (romacs-agent-turns session) 0) '("✓" romacs-agent-done "done") '("○" romacs-agent-dim "ended")))
          ((> (romacs-agent-turns session) 0) '("✓" romacs-agent-done "done"))
          (t '("○" romacs-agent-dim "idle")))))

(defun romacs-agent-ui--column (text width)
  (romacs-truncate (or text "") width ?\s))

(defconst romacs-agent-ui--word-width 10 "The status column: its longest word and a space.")
(defconst romacs-agent-ui--instance-width 20 "The model column, shown on a page 72 columns wide or more.")

(defun romacs-agent-ui--default-provider ()
  "The default provider's name, or nil when there is none: a row still shows."
  (ignore-errors (romacs-agent--default-provider)))

(defun romacs-agent-ui--instance (provider harness model default)
  "The model column: PROVIDER when not DEFAULT, then MODEL without its
HARNESS's prefix (claude-opus-5-5 is opus-5-5)."
  (let ((provider (and provider (not (equal provider default)) provider))
        (model (if (and model harness) (string-remove-prefix (concat (symbol-name harness) "-") model) model)))
    (mapconcat #'identity (delq nil (list provider model)) " · ")))

(defun romacs-agent-ui--fit (row width)
  "ROW, a plist (:indent :mark :face :title :title-face :word :ago :instance),
as one home-page line WIDTH columns wide: the title takes what the status,
age and model columns leave; a narrow page drops the model."
  (let* ((show-instance (>= width 72))
         (indent (plist-get row :indent))
         (face (plist-get row :face))
         (tail (+ romacs-agent-ui--word-width 5 (if show-instance romacs-agent-ui--instance-width 0)))
         (title-width (max 12 (- width (string-width indent) 2 1 tail))))
    (concat indent (propertize (plist-get row :mark) 'face face) " "
            (propertize (romacs-agent-ui--column (plist-get row :title) title-width) 'face (plist-get row :title-face))
            " " (propertize (romacs-agent-ui--column (plist-get row :word) romacs-agent-ui--word-width) 'face face)
            (propertize (romacs-agent-ui--column (plist-get row :ago) 5) 'face 'romacs-agent-dim)
            (if show-instance
                (propertize (truncate-string-to-width (plist-get row :instance) romacs-agent-ui--instance-width nil nil t)
                            'face 'romacs-agent-dim)
              "")
            "\n")))

(defun romacs-agent-ui--row (session depth root-cwd width)
  "SESSION's row on the home page, nested DEPTH under its root, fitted to WIDTH."
  (pcase-let ((`(,mark ,face ,word) (romacs-agent-ui--status session))
              (cwd (romacs-agent-cwd session)))
    (propertize
     (romacs-agent-ui--fit
      (list :indent (if (> depth 0) (concat (make-string (* 2 (1- depth)) ?\s) "└ ") "")
            :mark mark :face face :word word
            :title (concat (romacs-agent-title session)
                           (if (and (> depth 0) root-cwd (not (equal cwd root-cwd)))
                               (propertize (concat "  " (abbreviate-file-name cwd)) 'face 'romacs-agent-dim) ""))
            :title-face (if (> (romacs-agent-done-seq session) (romacs-agent-seen-seq session)) 'romacs-agent-unread 'default)
            :ago (romacs-agent-ui--ago (romacs-agent-last-time session))
            :instance (romacs-agent-ui--instance (romacs-agent-provider session) (romacs-agent-harness session)
                                                 (or (romacs-agent-model session)
                                                     (plist-get (romacs-agent-options session) :model))
                                                 (romacs-agent-ui--default-provider)))
      width)
     'romacs-agent-session (romacs-agent-id session))))

(defun romacs-agent-ui--tree ()
  "Every session as (ROOT . DESCENDANTS-BY-PARENT): the roots, newest
activity first, and a table from a session's id to its children."
  (let ((children (make-hash-table :test #'equal)) (roots nil))
    (dolist (session (romacs-agent-sessions))
      (let ((parent (plist-get (romacs-agent-options session) :parent)))
        (if (and parent (romacs-agent-get parent))
            (push session (gethash parent children))
          (push session roots))))
    (cons (sort roots (lambda (a b) (> (or (romacs-agent-last-time a) 0) (or (romacs-agent-last-time b) 0))))
          children)))

(defun romacs-agent-ui--newer (a b)
  "Whichever of sessions A and B (either nil) was active last."
  (if (and b (or (null a) (> (or (romacs-agent-last-time b) 0) (or (romacs-agent-last-time a) 0)))) b a))

(defun romacs-agent-ui--insert-subtree (session children depth root-cwd width)
  "Insert SESSION's row and its descendants'; return the one active last."
  (insert (romacs-agent-ui--row session depth root-cwd width))
  (let ((newest session))
    (dolist (child (sort (copy-sequence (gethash (romacs-agent-id session) children))
                         (lambda (a b) (< (or (romacs-agent-last-time a) 0) (or (romacs-agent-last-time b) 0)))))
      (setq newest (romacs-agent-ui--newer newest (romacs-agent-ui--insert-subtree child children (1+ depth) root-cwd width))))
    newest))

(defun romacs-agent-ui--history-row (item width)
  "A harness transcript ITEM's row, dimmed, in the session rows' columns."
  (propertize
   (romacs-agent-ui--fit
    (list :indent "" :mark "◇" :face 'romacs-agent-dim :word ""
          :title (plist-get item :title) :title-face 'romacs-agent-dim
          :ago (romacs-agent-ui--ago (plist-get item :mtime))
          :instance (romacs-agent-ui--instance (plist-get item :provider) nil nil (romacs-agent-ui--default-provider)))
    width)
   'romacs-agent-history item))

(defvar-local romacs-agents--all-history nil "Every project's history shows, not its newest few.")

(declare-function romacs-agent-history-items "romacs-agent-history" ())
(declare-function romacs-agent-history-list "romacs-agent-history" ())
(declare-function romacs-agent-history-open "romacs-agent-history" (item))
(declare-function romacs-agent-history-resume "romacs-agent-history" (item))

(defun romacs-agent-ui--home ()
  "Render the home page: a section per project, newest activity first,
its sessions nested and then the harnesses' own transcripts of it; the
row point was on stays under point."
  (require 'romacs-agent-history)
  (pcase-let* ((`(,roots . ,children) (romacs-agent-ui--tree))
               (sessions (make-hash-table :test #'equal))
               (history (make-hash-table :test #'equal))
               (recency (make-hash-table :test #'equal))
               (selected (or (get-text-property (line-beginning-position) 'romacs-agent-session)
                             (plist-get (get-text-property (line-beginning-position) 'romacs-agent-history) :path)))
               (now (float-time)))
    (cl-flet ((note (cwd time) (puthash cwd (max (or time now) (gethash cwd recency 0)) recency)))
      (dolist (root roots)
        (let ((cwd (or (romacs-agent-cwd root) "")))
          (note cwd (romacs-agent-last-time root))
          (push root (gethash cwd sessions))))
      (dolist (item (romacs-agent-history-items))
        (note (plist-get item :cwd) (plist-get item :mtime))
        (push item (gethash (plist-get item :cwd) history))))
    (let ((projects (sort (let ((keys nil)) (maphash (lambda (key _) (push key keys)) recency) keys) (lambda (a b) (> (gethash a recency) (gethash b recency))))))
    (setq romacs-agents--running
          (cl-count-if (lambda (s) (memq (romacs-agent-state s) '(starting running waiting-approval)))
                       (hash-table-values romacs-agent--sessions)))
    (let ((inhibit-read-only t) (line (line-number-at-pos))
          (width (let ((window (get-buffer-window (current-buffer) 'visible)))
                   (1- (if window (window-body-width window) (frame-width))))))
      (erase-buffer)
      (if (null projects)
          (insert (propertize "No agent sessions yet: c starts one.\n" 'face 'romacs-agent-dim))
        (dolist (cwd projects)
          (let* ((roots (nreverse (gethash cwd sessions)))
                 (items (nreverse (gethash cwd history)))
                 (shown (if romacs-agents--all-history items (seq-take items romacs-agent-history-per-project)))
                 (header (point)) (newest nil))
            (insert (propertize (file-name-nondirectory (directory-file-name cwd)) 'face 'romacs-agent-section)
                    "  " (propertize (abbreviate-file-name cwd) 'face 'romacs-agent-dim) "\n")
            (dolist (root roots)
              (setq newest (romacs-agent-ui--newer newest (romacs-agent-ui--insert-subtree root children 0 cwd width))))
            ;; RET on the project opens its newest: a session (a delegated
            ;; one too), else its newest transcript.
            (when-let* ((target (if newest (romacs-agent-id newest) (car items))))
              (put-text-property header (save-excursion (goto-char header) (line-beginning-position 2))
                                 'romacs-agents-newest target))
            (dolist (item shown) (insert (romacs-agent-ui--history-row item width)))
            (when (> (length items) (length shown))
              (insert (propertize (concat (propertize (format "  ◇ %d more  " (- (length items) (length shown)))
                                                      'face 'romacs-agent-dim)
                                          (romacs-key-hint "(\\<romacs-agents-mode-map>\\[romacs-agents-toggle-history] shows all)\n"
                                                           'romacs-agent-dim))
                                  'romacs-agents-more t)))
            ;; `c' anywhere in it starts a session here.
            (put-text-property header (point) 'romacs-agents-project cwd))
          (insert "\n")))
      (insert (romacs-key-hint "\\<romacs-agents-mode-map>\\[romacs-agents-open] open  \\[romacs-agents-new] new  \\[romacs-agents-interrupt] interrupt  \\[romacs-agents-stop] stop  \\[romacs-agents-resume] resume  \\[romacs-agents-toggle-history] history  \\[romacs-agents-refresh] refresh  \\[romacs-agents-next]/\\[romacs-agents-previous] move\n"
                               'romacs-agent-dim))
      (add-hook 'window-size-change-functions #'romacs-agents--resized nil t)
      (goto-char (point-min))
      (let ((row (and selected (or (text-property-any (point-min) (point-max) 'romacs-agent-session selected)
                                   (romacs-agent-ui--history-row-of selected)))))
        (if row (goto-char row) (forward-line (1- line))))))))

(defun romacs-agent-ui--history-row-of (path)
  "The start of PATH's history row, or nil."
  (let ((position (point-min)) (found nil))
    (while (and (not found) (setq position (next-single-property-change position 'romacs-agent-history)))
      (when (equal (plist-get (get-text-property position 'romacs-agent-history) :path) path) (setq found position)))
    found))

(defvar-local romacs-agents--running 0 "Sessions running at the last render.")

(defun romacs-agents--header-line ()
  (let ((running romacs-agents--running))
    (concat (propertize "Agents" 'face 'romacs-agent-section)
            (propertize " | " 'face 'romacs-agent-dim)
            (format "%d running" running)
            (propertize " | " 'face 'romacs-agent-dim)
            (romacs-key-hint "\\<romacs-agents-mode-map>\\[romacs-agents-new] new  \\[romacs-agents-refresh] refresh  \\[describe-mode] help"
                             'romacs-agent-dim))))

(defvar romacs-agents-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agents-open)
    (define-key map "c" #'romacs-agents-new)
    (define-key map "+" #'romacs-agents-new)
    (define-key map "i" #'romacs-agents-interrupt)
    (define-key map "k" #'romacs-agents-stop)
    (define-key map "r" #'romacs-agents-resume)
    (define-key map "g" #'romacs-agents-refresh)
    (define-key map "h" #'romacs-agents-toggle-history)
    (define-key map "n" #'romacs-agents-next)
    (define-key map "p" #'romacs-agents-previous)
    (define-key map "?" #'describe-mode)
    map)
  "Keys of the agents home page.")

;; The key the page names, of the two that start a session.
(put 'romacs-agents-new :advertised-binding [?c])

(define-derived-mode romacs-agents-mode special-mode "Agents"
  "Every agent session this server knows, by project.
\\{romacs-agents-mode-map}"
  :interactive nil
  (setq-local header-line-format '(:eval (romacs-agents--header-line)))
  (setq-local truncate-lines t))

(defvar romacs-agents--refresh nil)
(defvar romacs-agents--stale nil "The home page changed while not shown.")

(defun romacs-agent-ui--dashboard-changed ()
  "Refresh the home page soon, at most once a second, and only while it is
shown; hidden, it is rebuilt when shown."
  (if (not (and (get-buffer "*romacs-agents*") (get-buffer-window "*romacs-agents*" 'visible)))
      (setq romacs-agents--stale t)
    (unless romacs-agents--refresh
      (setq romacs-agents--refresh
            (run-at-time 1 nil (lambda ()
                                 (setq romacs-agents--refresh nil)
                                 (when-let* ((buffer (get-buffer "*romacs-agents*")))
                                   (with-current-buffer buffer (romacs-agent-ui--home)))))))))

;;;###autoload
(defun romacs-agents ()
  "Show every agent session, by project."
  (interactive)
  (romacs-agent-restore)
  (require 'romacs-agent-history)
  (romacs-agent-history-list)
  (with-current-buffer (get-buffer-create "*romacs-agents*")
    (unless (derived-mode-p 'romacs-agents-mode) (romacs-agents-mode))
    (pop-to-buffer (current-buffer))
    ;; Shown first: the rows fit its window.
    (romacs-agent-ui--home)
    (setq romacs-agents--stale nil)))

(defun romacs-agents-refresh ()
  "Rebuild the home page, the harnesses' transcripts listed anew."
  (interactive nil romacs-agents-mode)
  (require 'romacs-agent-history)
  (romacs-agent-history-list)
  (romacs-agent-ui--home))

(defun romacs-agents-toggle-history ()
  "Show every project's history, or its newest few."
  (interactive nil romacs-agents-mode)
  (setq romacs-agents--all-history (not romacs-agents--all-history))
  (romacs-agent-ui--home))

(defun romacs-agents--resized (_frame)
  "A new width: the rows fit it again (once per resize, not per event)."
  (when-let* ((buffer (get-buffer "*romacs-agents*")))
    (with-current-buffer buffer (romacs-agent-ui--home))))

(defun romacs-agents--at-point ()
  (or (romacs-agent-get (get-text-property (line-beginning-position) 'romacs-agent-session))
      (user-error "%s" (substitute-command-keys
                        "No session on this line: \\<romacs-agents-mode-map>\\[romacs-agents-next] and \\[romacs-agents-previous] move to one"))))

(defun romacs-agents--history-at-point ()
  (get-text-property (line-beginning-position) 'romacs-agent-history))

(defun romacs-agents--row-p ()
  (or (get-text-property (point) 'romacs-agent-session) (get-text-property (point) 'romacs-agent-history)))

(defun romacs-agents-next ()
  "The next row."
  (interactive nil romacs-agents-mode)
  (forward-line 1)
  (while (and (not (eobp)) (not (romacs-agents--row-p))) (forward-line 1)))

(defun romacs-agents-previous ()
  "The previous row."
  (interactive nil romacs-agents-mode)
  (forward-line -1)
  (while (and (not (bobp)) (not (romacs-agents--row-p))) (forward-line -1)))

(defun romacs-agents-open ()
  "Open this session's chat, or this transcript read-only; on a project,
its newest; on \"more\", every transcript."
  (interactive nil romacs-agents-mode)
  (let ((start (line-beginning-position)))
    (cond ((romacs-agents--history-at-point) (romacs-agent-history-open (romacs-agents--history-at-point)))
          ((get-text-property start 'romacs-agent-session) (romacs-agent-show (romacs-agents--at-point)))
          ((get-text-property start 'romacs-agents-more) (romacs-agents-toggle-history))
          ((get-text-property start 'romacs-agents-newest)
           (let ((newest (get-text-property start 'romacs-agents-newest)))
             (cond ((not (stringp newest)) (romacs-agent-history-open newest))
                   ((romacs-agent-get newest) (romacs-agent-show (romacs-agent-get newest)))
                   (t (romacs-agent-ui--home) (user-error "That session is gone; the page is redrawn")))))
          (t (romacs-agents--at-point)))))
(defun romacs-agents-new ()
  "Start a new session, by default in the project at point."
  (interactive nil romacs-agents-mode)
  (let ((project (get-text-property (line-beginning-position) 'romacs-agents-project)))
    (apply #'funcall-interactively #'romacs-agent-start
           (romacs-agent--start-arguments (if project (file-name-as-directory project)
                                            (romacs-agent--project-default))))))

(defun romacs-agents-interrupt () "Interrupt this session." (interactive nil romacs-agents-mode) (romacs-agent-interrupt (romacs-agents--at-point)))
(defun romacs-agents-stop ()
  "Stop this session."
  (interactive nil romacs-agents-mode)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop (romacs-agents--at-point))))
(defun romacs-agents-resume ()
  "Resume this session, or go on with this transcript as a session."
  (interactive nil romacs-agents-mode)
  (if-let* ((item (romacs-agents--history-at-point)))
      (romacs-agent-history-resume item)
    (romacs-agent-resume (romacs-agents--at-point))))

(provide 'romacs-agent-ui)
;;; romacs-agent-ui.el ends here

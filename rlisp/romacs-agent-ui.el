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
;;   C-c C-a).
;;
;; The chat is rebuilt from the journal, so a new server shows what the
;; session did while no server was there.

;;; Code:
(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-key-hint)

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
(defface romacs-agent-context '((t :inherit font-lock-number-face))
  "Context tokens in a chat's header (hermes-chat-header-context).")
(defface romacs-agent-mode '((t :inherit font-lock-builtin-face))
  "The harness's permission mode in a chat's header.")
(defface romacs-agent-section '((t :inherit font-lock-function-name-face :weight bold))
  "A project heading on the home page.")
(defface romacs-agent-unread '((t :weight bold))
  "A session whose last turn ended since its chat was shown.")

(defvar-local romacs-agent-ui--session nil)
(defvar-local romacs-agent-ui--end nil
  "Marker at the transcript's end; the composer follows it.")
(defvar-local romacs-agent-ui--tool-lines nil
  "Tool use id to the marker at its row's mark.")
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
the composer, follow the transcript); a row's mark or a message's start
is always taken after its own text is in."
  (save-excursion
    ;; A number: the end marker moves past what is inserted at it.
    (let ((inhibit-read-only t) (text (romacs-agent-ui--sealed text)) (start (+ 0 position)))
      (goto-char start)
      (if (= start romacs-agent-ui--end) (insert-before-markers text) (insert text))
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
    (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " (format "%s" text)) 100 nil nil "…")))

(defun romacs-agent-ui--tool-name (name)
  "A tool's NAME as a row shows it: an MCP tool by its own name."
  (if (and (stringp name) (string-match "\\`mcp__[^_]+\\(?:_[^_]+\\)*__\\(.+\\)\\'" name))
      (match-string 1 name)
    (or name "tool")))

(defun romacs-agent-ui--tool (session event)
  "A tool row: ● while running, then ✓ or !, its output folded below."
  (let ((id (plist-get event :id)))
    (pcase (plist-get event :state)
      ('started
       (let ((name (romacs-agent-ui--tool-name (plist-get event :name))))
         (setq romacs-agent-ui--tool name romacs-agent-ui--open nil)
         (let ((indent (make-string (* 2 (1+ (romacs-agent-ui--depth session (plist-get event :parent)))) ?\s)))
           (romacs-agent-ui--insert (concat (if (save-excursion (goto-char romacs-agent-ui--end) (bolp)) "" "\n") indent))
           (romacs-agent-ui--insert "●" 'face 'romacs-agent-running)
           (puthash id (copy-marker (1- romacs-agent-ui--end)) romacs-agent-ui--tool-lines)
           (romacs-agent-ui--insert (concat " " name
                                            (let ((brief (romacs-agent-ui--brief (plist-get event :input))))
                                              (if (string-empty-p brief) "" (concat "  " brief)))
                                            "\n")
                                    'romacs-agent-tool id))))
      (state
       (let* ((output (or (plist-get event :output) ""))
              (failed (eq state 'failed))
              (mark (gethash id romacs-agent-ui--tool-lines)))
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
               (add-face-text-property (point) (line-end-position) (if failed 'romacs-agent-error 'romacs-agent-dim))
               (forward-line 1))
             (unless (string-empty-p (string-trim output))
               (romacs-agent-ui--insert-at
                (point) (propertize (concat (string-trim-right output) "\n")
                                    'face 'romacs-agent-dim 'romacs-agent-output id
                                    'invisible 'romacs-agent-output))))))))))

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
               (fontified (hermes-chat--fontify-markdown-string body)))
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
                       (delete-region (+ start fill) end)))))))
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

(defun romacs-agent-ui--draw (session event buffer)
  "Draw SESSION's EVENT in its chat BUFFER, the current buffer."
      (pcase (plist-get event :type)
        ('input
         (romacs-agent-ui--line session event "")
         (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
         (romacs-agent-ui--insert "> " 'romacs-agent-turn t)
         (romacs-agent-ui--insert (concat (plist-get event :text)) 'face 'romacs-agent-prompt 'romacs-agent-turn t)
         (romacs-agent-ui--insert "\n")
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
        ('done (romacs-agent-ui--footer session event)
               (when (get-buffer-window buffer 'visible)
                 (setf (romacs-agent-seen-seq session) (romacs-agent-done-seq session))))
        ('status
         (unless (member (plist-get event :text) '("requesting" "compacting" "detached"))
           (romacs-agent-ui--line session event (format "  · %s" (plist-get event :text)) 'face 'romacs-agent-dim)))
        ('error (romacs-agent-ui--line session event (format "! %s" (plist-get event :text)) 'face 'romacs-agent-error))
        ('exit
         ;; A per-turn harness ends every turn: only a failed run is news.
         (unless (and (romacs-agent--per-turn-p session) (eql (plist-get event :status) 0))
           (romacs-agent-ui--line session event (format "  · harness exited (%s)" (plist-get event :status))
                                  'face 'romacs-agent-dim)))
        (_ nil))
      (romacs-agent-ui--composer-update)
      (when (and (eq (plist-get event :type) 'input) (overlayp romacs-agent-ui--composer)) (romacs-agent-ui--tick-start)))

(defun romacs-agent-ui--on-event (session event)
  (romacs-agent-ui--render session event)
  (romacs-agent-ui--dashboard-changed))

(add-hook 'romacs-agent-event-functions #'romacs-agent-ui--on-event)

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
                      (propertize (or (romacs-agent-model session) "…") 'face 'romacs-agent-model)
                      (and context (propertize context 'face 'romacs-agent-context))
                      (romacs-agent-ui--state-label session))))
         (text (mapconcat #'identity segments separator)))
    (string-replace "%" "%%" (truncate-string-to-width text (max 1 (window-body-width)) nil nil "…"))))

;;; The composer

(defun romacs-agent-ui--key (command)
  "COMMAND's key in a chat, as hermes-chat--command-key-hint shows it."
  (substitute-command-keys (format "\\<romacs-agent-chat-mode-map>\\[%s]" command)))

(defvar romacs-agent-ui--hint nil "The composer's hint, made once: looking up keys is slow.")

(defun romacs-agent-ui--hint ()
  "The empty composer's hint (hermes-chat--placeholder-string)."
  (or romacs-agent-ui--hint (setq romacs-agent-ui--hint (romacs-agent-ui--make-hint))))

(defun romacs-agent-ui--make-hint ()
  (let ((hint (concat "Message… · " (romacs-agent-ui--key #'romacs-agent-chat-return) " send · "
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
    (define-key map (kbd "C-c C-a") #'romacs-agent-chat-respond)
    (define-key map (kbd "C-c C-d") #'romacs-agent-chat-deny)
    (define-key map (kbd "C-c C-o") #'romacs-agent-chat-actions-map-popup)
    (define-key map (kbd "C-c C-/") #'romacs-agent-chat-show-commands)
    (define-key map (kbd "C-c C-n") #'romacs-agent-chat-new)
    (define-key map (kbd "C-c C-r") #'romacs-agent-chat-rename)
    (define-key map (kbd "C-c C-b") #'romacs-agent-chat-switch)
    (define-key map (kbd "C-c C-j") #'romacs-agent-chat-composer)
    (define-key map (kbd "M-p") #'romacs-agent-chat-history-previous)
    (define-key map (kbd "M-n") #'romacs-agent-chat-history-next)
    (define-key map "?" (romacs-agent-ui--transcript-key #'romacs-agent-chat-actions-map-popup))
    (define-key map "n" (romacs-agent-ui--transcript-key #'romacs-agent-chat-next-turn))
    (define-key map "p" (romacs-agent-ui--transcript-key #'romacs-agent-chat-previous-turn))
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
              romacs-agent-ui--messages (make-hash-table :test #'equal))
        (let ((inhibit-read-only t)) (erase-buffer))
        (setq romacs-agent-ui--end (point-marker))
        (set-marker-insertion-type romacs-agent-ui--end nil)
        (setq romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil)))
      ;; Shown first: its rules fit its window.
      (pop-to-buffer buffer)
      (with-current-buffer buffer
        (romacs-agent-ui--replay session)
        (romacs-agent-ui--composer-update)))
    (pop-to-buffer buffer)
    (goto-char (point-max))
    ;; The replay carried the window's start along with the transcript's
    ;; end: the composer goes to the window's bottom, the turns above it.
    (recenter -1)
    (setf (romacs-agent-seen-seq session) (romacs-agent-done-seq session))
    (romacs-agent-ui--dashboard-changed)
    (when (memq (romacs-agent-state session) '(starting running waiting-approval)) (romacs-agent-ui--tick-start))
    buffer))

(defvar desktop-buffer-mode-handlers)

;; A session outlives the editor in its supervisor, so a restored chat
;; (desktop.el, warm-restart.el) carries only which session it shows and
;; the composer's unsent text; the transcript replays from the journal.
(defun romacs-agent-ui--desktop-save (_dirname)
  "The session this chat shows and its composer's unsent text."
  (list :session (romacs-agent-id romacs-agent-ui--session)
        :draft (and (markerp romacs-agent-ui--end) (romacs-agent-ui--composer-text))))

(defun romacs-agent-ui-desktop-restore (_file _name misc)
  "The chat of MISC's session, made anew from its journal, with its draft."
  (let ((id (plist-get misc :session)))
    (unless (romacs-agent-get id) (romacs-agent-restore))
    (when-let* ((session (romacs-agent-get id))
                (buffer (save-window-excursion (romacs-agent-show session))))
      (with-current-buffer buffer
        (when-let* ((draft (plist-get misc :draft)))
          (unless (string-empty-p draft)
            (goto-char (point-max))
            (insert draft))))
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
          (romacs-agent-driver-state scratch) nil (romacs-agent-startup-pending scratch) nil)
    (let ((romacs-agent-event-functions
           (list (lambda (_ event)
                   ;; The scratch copy's turn data, for the footers it draws.
                   (setf (romacs-agent-turn-start session) (or (romacs-agent-turn-start scratch) (romacs-agent-turn-start session)))
                   (romacs-agent-ui--render session event)))))
      (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record))))))

(defun romacs-agent-ui--composer-text ()
  (string-trim (buffer-substring-no-properties romacs-agent-ui--end (point-max))))

(defun romacs-agent-chat-return ()
  "In the composer, send its text; on a tool row, show its output."
  (interactive nil romacs-agent-chat-mode)
  (if (not (romacs-agent-ui--in-composer-p))
      (romacs-agent-chat-toggle)
    (let ((text (romacs-agent-ui--composer-text)))
      (unless (string-empty-p text)
        (romacs-agent-send romacs-agent-ui--session text)
        (push text romacs-agent-ui--history)
        (romacs-agent-ui--history-reset)
        (delete-region romacs-agent-ui--end (point-max))))))

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

(defun romacs-agent-chat-respond ()
  "Answer the approval the harness waits for."
  (interactive nil romacs-agent-chat-mode)
  (pcase (read-char-choice "Allow this tool call? (y/n) " '(?y ?n))
    (?y (romacs-agent-chat-allow))
    (?n (romacs-agent-chat-deny))))

;;; emacs-hermes's chat actions

(defun romacs-agent-ui--running-p ()
  (memq (romacs-agent-state romacs-agent-ui--session) '(starting running waiting-approval)))

(defun romacs-agent-ui--deliver (send)
  "Call SEND with the composer's text, then empty the composer into its
history; when SEND fails, the draft stays.  Nothing when it is empty."
  (let ((text (romacs-agent-ui--composer-text)))
    (unless (string-empty-p text)
      (funcall send text)
      (push text romacs-agent-ui--history)
      (romacs-agent-ui--history-reset)
      (delete-region romacs-agent-ui--end (point-max))
      (romacs-agent-ui--composer-update))))

(defun romacs-agent-chat-newline ()
  "A new line in the composer."
  (interactive nil romacs-agent-chat-mode)
  (unless (romacs-agent-ui--in-composer-p) (goto-char (point-max)))
  (insert "\n"))

(defun romacs-agent-chat-queue ()
  "Send the composer's text when the running turn ends; now, when none runs."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--deliver (lambda (text) (romacs-agent-send-later romacs-agent-ui--session text))))

(defun romacs-agent-chat-steer ()
  "Send the composer's text now, into the running turn."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ui--deliver (lambda (text) (romacs-agent-send romacs-agent-ui--session text))))

(defun romacs-agent-chat-interrupt-and-send ()
  "Interrupt the running turn; the composer's text is sent once it ends."
  (interactive nil romacs-agent-chat-mode)
  (let ((session romacs-agent-ui--session))
    (if (not (romacs-agent-ui--running-p))
        (romacs-agent-ui--deliver (lambda (text) (romacs-agent-send session text)))
      ;; Interrupted first: a failure keeps the draft.
      (romacs-agent-interrupt session)
      (romacs-agent-ui--deliver (lambda (text) (romacs-agent-send-later session text t))))))

(defvar-local romacs-agent-ui--history-at nil
  "The history's cell whose prompt the composer shows, or nil for the draft.")
(defvar-local romacs-agent-ui--history-back nil
  "The cells M-p left, newest first: where M-n goes back to.")
(defvar-local romacs-agent-ui--history-draft nil
  "The composer's text, as it was, when M-p first left it.")

(defun romacs-agent-ui--history-reset ()
  (setq romacs-agent-ui--history-at nil romacs-agent-ui--history-back nil))

(defun romacs-agent-ui--show-history (text)
  (delete-region romacs-agent-ui--end (point-max))
  (goto-char (point-max))
  (insert text))

(defun romacs-agent-chat-history-previous ()
  "Put the previous prompt in the composer; the draft is kept for M-n."
  (interactive nil romacs-agent-chat-mode)
  (let ((next (if romacs-agent-ui--history-at (cdr romacs-agent-ui--history-at) romacs-agent-ui--history)))
    (when next
      (unless romacs-agent-ui--history-at
        (setq romacs-agent-ui--history-draft (buffer-substring-no-properties romacs-agent-ui--end (point-max))))
      (push romacs-agent-ui--history-at romacs-agent-ui--history-back)
      (setq romacs-agent-ui--history-at next)
      (romacs-agent-ui--show-history (car next)))))

(defun romacs-agent-chat-history-next ()
  "Put the next prompt in the composer; past the newest, the draft."
  (interactive nil romacs-agent-chat-mode)
  (when romacs-agent-ui--history-at
    (setq romacs-agent-ui--history-at (pop romacs-agent-ui--history-back))
    (romacs-agent-ui--show-history (if romacs-agent-ui--history-at (car romacs-agent-ui--history-at)
                                     (or romacs-agent-ui--history-draft "")))))

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

(defun romacs-agent-chat-copy ()
  "Copy the region, else the turn at point (its prompt and reply)."
  (interactive nil romacs-agent-chat-mode)
  (pcase-let ((`(,start . ,end) (if (use-region-p) (cons (region-beginning) (region-end)) (romacs-agent-ui--turn-bounds))))
    (kill-new (string-trim (buffer-substring-no-properties start end)))
    (message "Copied")))

(defun romacs-agent-chat-edit-turn ()
  "Put the prompt of the turn at point in the composer, to edit and send."
  (interactive nil romacs-agent-chat-mode)
  (let* ((start (let ((start (car (romacs-agent-ui--turn-bounds))))
                  ;; Before the first prompt: nothing to edit, and the draft stays.
                  (unless (get-text-property start 'romacs-agent-turn) (user-error "No turn here"))
                  start))
         (end (or (next-single-property-change start 'romacs-agent-turn nil romacs-agent-ui--end) romacs-agent-ui--end))
         (prompt (string-remove-prefix "> " (string-trim-right (buffer-substring-no-properties start end)))))
    (delete-region romacs-agent-ui--end (point-max))
    (goto-char (point-max))
    (insert prompt)))

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

(defun romacs-agent-chat-new ()
  "Start a new session, in this one's directory."
  (interactive nil romacs-agent-chat-mode)
  (apply #'funcall-interactively #'romacs-agent-start
         (romacs-agent--start-arguments (file-name-as-directory (romacs-agent-cwd romacs-agent-ui--session)))))

(defun romacs-agent-chat-switch (buffer)
  "Switch to another session's chat BUFFER."
  (interactive
   (list (read-buffer "Chat: " nil t
                      (lambda (entry)
                        (let ((buffer (get-buffer (if (consp entry) (car entry) entry))))
                          (and buffer (not (eq buffer (current-buffer)))
                               (eq (buffer-local-value 'major-mode buffer) 'romacs-agent-chat-mode))))))
   romacs-agent-chat-mode)
  (switch-to-buffer buffer))

(defun romacs-agent-chat-show-commands ()
  "List this chat's keys."
  (interactive nil romacs-agent-chat-mode)
  (describe-keymap 'romacs-agent-chat-mode-map))

(defun romacs-agent-chat-home () "The agents home page." (interactive nil romacs-agent-chat-mode) (romacs-agents))

(defun romacs-agent-ui--queue-label ()
  (let ((waiting (and romacs-agent-ui--session (romacs-agent-queue-length romacs-agent-ui--session))))
    (if (and waiting (> waiting 0)) (format "Queue (%d waiting)" waiting) "Queue")))

(require 'keymap-popup)

(keymap-popup-define romacs-agent-chat-actions-map
  "Actions of an agent chat, as emacs-hermes's chat has them."
  :popup-key "?"
  :exit-key "C-g"
  :group "Turn"
  "s" ("Steer (send now)" romacs-agent-chat-steer)
  "i" ("Interrupt" romacs-agent-chat-interrupt :inapt-if (lambda () (not (romacs-agent-ui--running-p))))
  "k" ("Interrupt + send" romacs-agent-chat-interrupt-and-send)
  "q" (#'romacs-agent-ui--queue-label romacs-agent-chat-queue)
  :group "Compose"
  "RET" ("Send" romacs-agent-chat-return)
  "Q" ("Quote region" romacs-agent-chat-quote-region :inapt-if (lambda () (not (use-region-p))))
  "e" ("Edit turn at point" romacs-agent-chat-edit-turn)
  "w" ("Copy turn or region" romacs-agent-chat-copy)
  :row
  :group "Session"
  "R" ("Rename" romacs-agent-chat-rename)
  "n" ("New session" romacs-agent-chat-new)
  "b" ("Switch chat" romacs-agent-chat-switch)
  "r" ("Resume" romacs-agent-chat-resume :inapt-if #'romacs-agent-ui--running-p)
  "x" ("Stop" romacs-agent-chat-stop)
  "h" ("Home page" romacs-agent-chat-home)
  :group "Show"
  "+" ("Expand all outputs" romacs-agent-chat-expand-all)
  "-" ("Collapse all outputs" romacs-agent-chat-collapse-all)
  "c" ("Show commands" romacs-agent-chat-show-commands)
  :group ("Approval" :if (lambda () (and romacs-agent-ui--session
                                         (> (hash-table-count (romacs-agent-approvals romacs-agent-ui--session)) 0))))
  "a" ("Allow" romacs-agent-chat-allow)
  "d" ("Deny" romacs-agent-chat-deny))

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
  (truncate-string-to-width (or text "") width nil ?\s "…"))

(defconst romacs-agent-ui--word-width 10 "The status column: its longest word and a space.")
(defconst romacs-agent-ui--instance-width 20 "The model column, shown on a page 72 columns wide or more.")

(defun romacs-agent-ui--instance (provider harness model)
  "The model column: PROVIDER when not the default, then MODEL without its
HARNESS's prefix (claude-opus-5-5 is opus-5-5)."
  (let ((provider (and provider (not (equal provider (ignore-errors (romacs-agent--default-provider)))) provider))
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
                (propertize (truncate-string-to-width (plist-get row :instance) romacs-agent-ui--instance-width nil nil "…")
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
                                                     (plist-get (romacs-agent-options session) :model))))
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
          :instance (romacs-agent-ui--instance (plist-get item :provider) nil nil))
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

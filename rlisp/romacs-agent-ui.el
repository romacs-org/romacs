;;; romacs-agent-ui.el --- Chat buffers and the agents dashboard -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's own interface to agent sessions (romacs-agent.el):
;;
;; - `romacs-agents': the dashboard, one line per session.  RET opens a
;;   session's chat, + starts one, i interrupts, k stops, r resumes, g
;;   refreshes; n and p move.
;; - A chat buffer per session: the transcript, streamed as the harness
;;   writes, above a composer.  Tool calls are one line each, their output
;;   folded (TAB shows it); a subagent's work is indented under the tool
;;   call that started it; an approval the harness asks for is a block
;;   answered with y or n (or C-c C-a).  In the transcript n and p move
;;   between turns; RET in the composer sends, C-c C-c interrupts the turn,
;;   C-c C-k stops the session, C-c C-r resumes it.
;;
;; The chat is rebuilt from the journal, so a new server shows what the
;; session did while no server was there.

;;; Code:
(require 'cl-lib)
(require 'tabulated-list)
(require 'romacs-agent)

(defface romacs-agent-user '((t :inherit font-lock-keyword-face :weight bold))
  "A user's prompt in a chat.")
(defface romacs-agent-tool '((t :inherit font-lock-function-name-face))
  "A tool call's line.")
(defface romacs-agent-dim '((t :inherit shadow))
  "Status lines, thinking and folded output.")
(defface romacs-agent-approval '((t :inherit warning :weight bold))
  "An approval the harness waits for.")
(defface romacs-agent-error '((t :inherit error))
  "A failed tool call or turn.")

(defvar-local romacs-agent-ui--session nil)
(defvar-local romacs-agent-ui--end nil
  "Marker at the transcript's end; the composer follows it.")
(defvar-local romacs-agent-ui--tool-lines nil
  "Tool use id to the marker at the end of its call line.")
(defvar-local romacs-agent-ui--shown 0
  "The last journal record this buffer shows whole.")
(defvar-local romacs-agent-ui--open nil
  "The kind of block the transcript ends in: `text', `thinking' or nil.")
(defvar-local romacs-agent-ui--history nil)

;;; Rendering

(defun romacs-agent-ui--depth (session parent)
  "How deep a block under tool call PARENT nests."
  (let ((depth 0) (tools (romacs-agent-tools session)))
    (while (and parent (stringp parent) (< depth 8))
      (setq depth (1+ depth) parent (gethash parent tools)))
    depth))

(defun romacs-agent-ui--insert (text &rest properties)
  "Insert TEXT at the transcript's end with PROPERTIES, read-only."
  (save-excursion
    (let ((inhibit-read-only t))
      (goto-char romacs-agent-ui--end)
      ;; Before the end marker, which then follows the transcript: what the
      ;; user types in the composer stays after it.
      (insert-before-markers (apply #'propertize text 'read-only t 'front-sticky '(read-only) 'rear-nonsticky '(read-only)
                     properties)))))

(defun romacs-agent-ui--line (session event text &rest properties)
  "Start a new block line for EVENT, indented by its nesting."
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
                    ((gethash "description" input))
                    ((gethash "code" input))
                    (t (json-serialize input :false-object :false :null-object :null)))))
    (truncate-string-to-width (replace-regexp-in-string "\n" " " (format "%s" text)) 100 nil nil "…")))

(defun romacs-agent-ui--tool (session event)
  (let ((id (plist-get event :id)))
    (pcase (plist-get event :state)
      ('started
       (romacs-agent-ui--line session event
                              (format "▸ %s  %s" (plist-get event :name) (romacs-agent-ui--brief (plist-get event :input)))
                              'face 'romacs-agent-tool 'romacs-agent-tool id)
       (puthash id (copy-marker (1- romacs-agent-ui--end)) romacs-agent-ui--tool-lines))
      (state
       (let* ((output (or (plist-get event :output) ""))
              (failed (eq state 'failed))
              (line (gethash id romacs-agent-ui--tool-lines)))
         (save-excursion
           (let ((inhibit-read-only t))
             (when line
               (goto-char line)
               (insert (propertize (if failed "  ✗" "  ✓") 'face (if failed 'romacs-agent-error 'romacs-agent-dim)
                                   'read-only t))
               (forward-line 1))
             (unless line (goto-char romacs-agent-ui--end))
             ;; Before the end marker when the call is the transcript's
             ;; last line, so what follows comes after its output.
             (insert-before-markers
              (propertize (concat (string-trim-right output) "\n")
                          'face 'romacs-agent-dim 'read-only t 'romacs-agent-output id
                          'invisible 'romacs-agent-output)))))))))

(defun romacs-agent-ui--render (session event)
  "Show SESSION's EVENT in its chat buffer, when it has one and has not
shown it: the journal's replay and the live records meet without a gap
or a repeat."
  (when-let* ((buffer (romacs-agent-buffer session)) ((buffer-live-p buffer))
              ((> (or (plist-get event :seq) 0) (buffer-local-value 'romacs-agent-ui--shown buffer))))
    (with-current-buffer buffer
      (when (plist-get event :last) (setq romacs-agent-ui--shown (plist-get event :seq)))
      (pcase (plist-get event :type)
        ('input
         (romacs-agent-ui--line session event (concat "❯ " (plist-get event :text)) 'face 'romacs-agent-user
                                'romacs-agent-turn t))
        ('delta
         (unless (eq romacs-agent-ui--open 'text)
           (romacs-agent-ui--line session event "")
           (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
           (setq romacs-agent-ui--open 'text))
         (romacs-agent-ui--insert (plist-get event :text)))
        ('thinking
         (unless (eq romacs-agent-ui--open 'thinking)
           (romacs-agent-ui--line session event "∴ " 'face 'romacs-agent-dim)
           (save-excursion (let ((inhibit-read-only t)) (goto-char romacs-agent-ui--end) (delete-char -1)))
           (setq romacs-agent-ui--open 'thinking))
         (romacs-agent-ui--insert (plist-get event :text) 'face 'romacs-agent-dim))
        ('tool (romacs-agent-ui--tool session event))
        ('approval
         (romacs-agent-ui--line session event
                                (format "⚠ %s asks: %s  %s   [y] allow  [n] deny"
                                        (plist-get event :tool) (romacs-agent-ui--brief (plist-get event :input))
                                        (or (plist-get event :description) ""))
                                'face 'romacs-agent-approval 'romacs-agent-approval (plist-get event :request-id)))
        ('answered
         (romacs-agent-ui--line session event (format "  %s" (or (plist-get event :behavior) "answered"))
                                'face 'romacs-agent-dim))
        ('done
         (romacs-agent-ui--line session event
                                (format "— %s%s" (if (plist-get event :error) "failed" "done")
                                        (if (numberp (plist-get event :cost)) (format "  $%.4f" (plist-get event :cost)) ""))
                                'face (if (plist-get event :error) 'romacs-agent-error 'romacs-agent-dim)))
        ('status
         (unless (member (plist-get event :text) '("requesting" "compacting"))
           (romacs-agent-ui--line session event (format "· %s" (plist-get event :text)) 'face 'romacs-agent-dim)))
        ('error (romacs-agent-ui--line session event (format "! %s" (plist-get event :text)) 'face 'romacs-agent-error))
        ('exit (romacs-agent-ui--line session event (format "· harness exited (%s)" (plist-get event :status))
                                      'face 'romacs-agent-dim))
        ('session nil)
        (_ nil))
      (force-mode-line-update))))

(defun romacs-agent-ui--on-event (session event)
  (romacs-agent-ui--render session event)
  (romacs-agent-ui--dashboard-changed))

(add-hook 'romacs-agent-event-functions #'romacs-agent-ui--on-event)

;;; The chat buffer

(defun romacs-agent-ui--header ()
  (let ((session romacs-agent-ui--session))
    (format " %s · %s · %s · %s%s"
            (romacs-agent-harness session) (or (romacs-agent-model session) "…")
            (abbreviate-file-name (or (romacs-agent-cwd session) ""))
            (romacs-agent-state session)
            (if (numberp (romacs-agent-cost session)) (format " · $%.4f" (romacs-agent-cost session)) ""))))

(defun romacs-agent-ui--in-composer-p ()
  (and romacs-agent-ui--end (>= (point) romacs-agent-ui--end)))

(defun romacs-agent-ui--transcript-key (command)
  "COMMAND in the transcript, the key's own insertion in the composer."
  `(menu-item "" ,command :filter ,(lambda (command) (if (romacs-agent-ui--in-composer-p) nil command))))

(defvar romacs-agent-chat-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agent-chat-return)
    (define-key map (kbd "TAB") #'romacs-agent-chat-toggle)
    (define-key map (kbd "C-c C-c") #'romacs-agent-chat-interrupt)
    (define-key map (kbd "C-c C-k") #'romacs-agent-chat-stop)
    (define-key map (kbd "C-c C-r") #'romacs-agent-chat-resume)
    (define-key map (kbd "C-c C-a") #'romacs-agent-chat-respond)
    (define-key map (kbd "C-c C-j") #'romacs-agent-chat-composer)
    (define-key map (kbd "M-p") #'romacs-agent-chat-history-previous)
    (define-key map "n" (romacs-agent-ui--transcript-key #'romacs-agent-chat-next-turn))
    (define-key map "p" (romacs-agent-ui--transcript-key #'romacs-agent-chat-previous-turn))
    (define-key map "y" (romacs-agent-ui--transcript-key #'romacs-agent-chat-allow))
    (define-key map "N" (romacs-agent-ui--transcript-key #'romacs-agent-chat-deny))
    map)
  "Keys of an agent chat buffer.")

(define-derived-mode romacs-agent-chat-mode fundamental-mode "Agent"
  "An agent session's transcript and composer."
  (setq-local desktop-save-buffer #'romacs-agent-ui--desktop-save)
  (setq-local header-line-format '(:eval (romacs-agent-ui--header)))
  (setq-local buffer-invisibility-spec (list 'romacs-agent-output))
  (visual-line-mode 1))

(defun romacs-agent-show (session)
  "Show SESSION's chat buffer, made from its journal when new."
  (let ((buffer (romacs-agent-buffer session)))
    (unless (buffer-live-p buffer)
      (setq buffer (get-buffer-create (format "*agent: %s*" (romacs-agent-title session))))
      (setf (romacs-agent-buffer session) buffer)
      (with-current-buffer buffer
        (romacs-agent-chat-mode)
        (setq romacs-agent-ui--session session
              romacs-agent-ui--tool-lines (make-hash-table :test #'equal))
        (let ((inhibit-read-only t)) (erase-buffer))
        (setq romacs-agent-ui--end (point-marker))
        (set-marker-insertion-type romacs-agent-ui--end nil)
        (romacs-agent-ui--replay session)))
    (pop-to-buffer buffer)
    (goto-char (point-max))
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

(defun romacs-agent-ui--replay (session)
  "Render SESSION's journal into the current chat buffer."
  ;; A copy parses, so the live session's own state is not replayed over.
  (let ((scratch (copy-romacs-agent session)))
    (setf (romacs-agent-seq scratch) 0
          (romacs-agent-streamed scratch) (make-hash-table :test #'equal)
          (romacs-agent-approvals scratch) (make-hash-table :test #'equal))
    (let ((romacs-agent-event-functions (list (lambda (_ event) (romacs-agent-ui--render session event)))))
      (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record))))))

(defun romacs-agent-ui--composer-text ()
  (string-trim (buffer-substring-no-properties romacs-agent-ui--end (point-max))))

(defun romacs-agent-chat-return ()
  "In the composer, send its text; on a tool line, show its output."
  (interactive)
  (if (not (romacs-agent-ui--in-composer-p))
      (romacs-agent-chat-toggle)
    (let ((text (romacs-agent-ui--composer-text)))
      (unless (string-empty-p text)
        (romacs-agent-send romacs-agent-ui--session text)
        (push text romacs-agent-ui--history)
        (delete-region romacs-agent-ui--end (point-max))))))

(defun romacs-agent-chat-toggle ()
  "Show or hide the output of the tool call on this line."
  (interactive)
  (let ((id (get-text-property (line-beginning-position) 'romacs-agent-tool)))
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
  (interactive)
  (goto-char (point-max)))

(defun romacs-agent-chat-history-previous ()
  "Put the previous prompt in the composer."
  (interactive)
  (when romacs-agent-ui--history
    (delete-region romacs-agent-ui--end (point-max))
    (goto-char (point-max))
    (insert (car romacs-agent-ui--history))
    (setq romacs-agent-ui--history (append (cdr romacs-agent-ui--history) (list (car romacs-agent-ui--history))))))

(defun romacs-agent-ui--move-to (property direction)
  (let ((pos (point)))
    (while (and (setq pos (if (> direction 0) (next-single-property-change pos property)
                            (previous-single-property-change pos property)))
                (not (get-text-property pos property))))
    (when pos (goto-char pos))))

(defun romacs-agent-chat-next-turn () "Next prompt." (interactive) (romacs-agent-ui--move-to 'romacs-agent-turn 1))
(defun romacs-agent-chat-previous-turn () "Previous prompt." (interactive) (romacs-agent-ui--move-to 'romacs-agent-turn -1))

(defun romacs-agent-chat-interrupt () "Interrupt the turn." (interactive) (romacs-agent-interrupt romacs-agent-ui--session))
(defun romacs-agent-chat-stop ()
  "Stop the session."
  (interactive)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop romacs-agent-ui--session)))
(defun romacs-agent-chat-resume () "Resume the session." (interactive) (romacs-agent-resume romacs-agent-ui--session))

(defun romacs-agent-ui--approval-here ()
  "The approval request on this line, else the session's only one."
  (or (get-text-property (line-beginning-position) 'romacs-agent-approval)
      (let ((ids (hash-table-keys (romacs-agent-approvals romacs-agent-ui--session))))
        (and (= (length ids) 1) (car ids)))))

(defun romacs-agent-chat-allow ()
  "Allow the tool call the harness asks about."
  (interactive)
  (let ((id (romacs-agent-ui--approval-here)))
    (unless id (user-error "No approval here"))
    (romacs-agent-respond romacs-agent-ui--session id t)))

(defun romacs-agent-chat-deny ()
  "Deny the tool call the harness asks about."
  (interactive)
  (let ((id (romacs-agent-ui--approval-here)))
    (unless id (user-error "No approval here"))
    (romacs-agent-respond romacs-agent-ui--session id nil)))

(defun romacs-agent-chat-respond ()
  "Answer the approval the harness waits for."
  (interactive)
  (pcase (read-char-choice "Allow this tool call? (y/n) " '(?y ?n))
    (?y (romacs-agent-chat-allow))
    (?n (romacs-agent-chat-deny))))

;;; The dashboard

(defvar romacs-agents-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agents-open)
    (define-key map "+" #'romacs-agent-start)
    (define-key map "i" #'romacs-agents-interrupt)
    (define-key map "k" #'romacs-agents-stop)
    (define-key map "r" #'romacs-agents-resume)
    (define-key map "n" #'next-line)
    (define-key map "p" #'previous-line)
    map)
  "Keys of the agents dashboard.")

(define-derived-mode romacs-agents-mode tabulated-list-mode "Agents"
  "Every agent session this server knows."
  (setq tabulated-list-format [("Session" 28 t) ("Harness" 8 t) ("State" 16 t) ("Cost" 9 t) ("Directory" 40 t)])
  (setq tabulated-list-entries #'romacs-agents--entries)
  (tabulated-list-init-header))

(defun romacs-agents--entries ()
  (mapcar (lambda (session)
            (list (romacs-agent-id session)
                  (vector (or (romacs-agent-title session) "") (symbol-name (romacs-agent-harness session))
                          (symbol-name (romacs-agent-state session))
                          (if (numberp (romacs-agent-cost session)) (format "$%.4f" (romacs-agent-cost session)) "")
                          (abbreviate-file-name (or (romacs-agent-cwd session) "")))))
          (romacs-agent-sessions)))

(defvar romacs-agents--refresh nil)
(defun romacs-agent-ui--dashboard-changed ()
  "Refresh the dashboard soon: many events make one refresh."
  (when (and (get-buffer "*romacs-agents*") (not romacs-agents--refresh))
    (setq romacs-agents--refresh
          (run-at-time 0.2 nil (lambda ()
                                 (setq romacs-agents--refresh nil)
                                 (when-let* ((buffer (get-buffer "*romacs-agents*")))
                                   (with-current-buffer buffer (tabulated-list-print t))))))))

;;;###autoload
(defun romacs-agents ()
  "Show every agent session."
  (interactive)
  (romacs-agent-restore)
  (with-current-buffer (get-buffer-create "*romacs-agents*")
    (romacs-agents-mode)
    (tabulated-list-print)
    (pop-to-buffer (current-buffer))))

(defun romacs-agents--at-point ()
  (or (romacs-agent-get (tabulated-list-get-id)) (user-error "No session here")))

(defun romacs-agents-open () "Open this session's chat." (interactive) (romacs-agent-show (romacs-agents--at-point)))
(defun romacs-agents-interrupt () "Interrupt this session." (interactive) (romacs-agent-interrupt (romacs-agents--at-point)))
(defun romacs-agents-stop ()
  "Stop this session."
  (interactive)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop (romacs-agents--at-point))))
(defun romacs-agents-resume () "Resume this session." (interactive) (romacs-agent-resume (romacs-agents--at-point)))

(provide 'romacs-agent-ui)
;;; romacs-agent-ui.el ends here

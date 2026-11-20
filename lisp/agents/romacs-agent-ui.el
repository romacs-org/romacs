;;; romacs-agent-ui.el --- Agent sessions' state, header and workers -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's own interface to agent sessions (romacs-agent.el), beside the
;; chat itself (romacs-agent-log.el, its menu romacs-agent-menu.el):
;;
;; - One state for a session (`romacs-agent-ui--state'), every view's:
;;   the header, the workers' rows, the status bar and the agent board
;;   (romacs-agent-board.el, `romacs-agents').
;; - What runs under a session: its delegated sessions and its harness's
;;   own subagents, as rows, summarised in its header.
;; - A chat's header: harness, permission mode, model, context used.

;;; Code:
(require 'romacs-truncate)
(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-key-hint)
(require 'romacs-agent-rows)
(require 'romacs-agent-ask)
(require 'romacs-markdown)
(require 'romacs-agent-turns)
(require 'romacs-agent-log)
(require 'romacs-agent-menu)

;;;###autoload
(defvar romacs-agents-map
  (let ((map (make-sparse-keymap)))
    (define-key map "n" #'romacs-agent-start)
    (define-key map "s" #'romacs-agents)
    (define-key map "a" #'romacs-agents)
    map)
  "Global prefix map for starting and finding agent sessions.
\\{romacs-agents-map}")

;; Global, so under C-x (C-c C-<key> is a major mode's).  Its keys are plain
;; letters: gud's and edebug's under C-x C-a are control keys, and the
;; prefix maps merge.
;;;###autoload (define-key ctl-x-map "\C-a" romacs-agents-map)
;; gud binds its own global map on C-x C-a as it loads: the agents' map
;; goes under it, gud's keys first (LISP-GUD-PREFIX).
;;;###autoload (with-eval-after-load 'gud (unless (keymap-parent gud-global-map) (set-keymap-parent gud-global-map romacs-agents-map)))

(declare-function romacs-agents "romacs-agent-board" ())
(declare-function romacs-agent-resume-time "romacs-agent-tools" (session))
(declare-function romacs-agent-tools--cancel-resume "romacs-agent-tools" (session &optional why))

;;; Faces

(defface romacs-agent-prompt '((t :inherit highlight :extend t))
  "A user's prompt in a chat: a band across the window, so what the user
said stands apart from what agents and sessions said.")
(defface romacs-agent-note '((t :inherit romacs-agent-dim :slant italic))
  "A note of what happened to the session (a compaction, a model switch, a
fork), said by no one: the quietest row.")
(defface romacs-agent-rule
  '((((background light)) :foreground "gray70")
    (t :foreground "gray30"))
  "The rule closing a turn and above the composer.")
(defface romacs-agent-running '((t :inherit romacs-working-running))
  "What is running: a tool's mark, the live rule.  The working thread's
colour, which no theme's syntax faces can make read as an error.")
(defface romacs-agent-dim '((t :inherit shadow))
  "Settled rows, thinking, folded output and metadata.")
(defface romacs-agent-done '((t :inherit success))
  "A settled mark.")
(defface romacs-agent-approval '((t :inherit romacs-working-waiting :weight bold))
  "An approval the harness waits for.")
(defface romacs-agent-plan '((t :inherit font-lock-constant-face))
  "A plan presented, waiting to be taken up." :group 'romacs-agent)
(defface romacs-agent-error '((t :inherit error))
  "A failed tool call or turn.")
(defface romacs-agent-model '((t :inherit font-lock-type-face))
  "The model in a chat's header.")
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
(defface romacs-agent-context '((t :inherit font-lock-number-face))
  "Context tokens in a chat's header.")
(defface romacs-agent-mode '((t :inherit font-lock-builtin-face))
  "The harness's permission mode in a chat's header.")
(defface romacs-agent-section '((t :inherit font-lock-function-name-face :weight bold))
  "A project heading on the home page.")
(defface romacs-agent-delegate '((t :inherit font-lock-function-name-face :weight bold))
  "Face of a delegated task's title.")
(defface romacs-agent-message '((t :inherit font-lock-function-name-face :weight bold))
  "The other session on a message's card: its sender, or the one it goes to.")
(defface romacs-agent-message-bar '((t :inherit font-lock-function-name-face))
  "The bar before each line of a message between sessions.")
(defface romacs-agent-ui-sent '((t :inherit shadow :slant italic))
  "A message this chat sent another thread: set apart from words for the
user.")
(defface romacs-agent-link '((t :inherit link))
  "Face of a link to a delegated task's thread.")
(defface romacs-agent-unread '((t :weight bold))
  "A session whose last turn ended since its chat was shown.")

(defvar-local romacs-agent-ui--session nil)
(defvar-local romacs-agent-ui--asks nil
  "The chat's approval events by request id: what each asked.")

(defun romacs-agent-ui--asks ()
  "This chat's `romacs-agent-ui--asks', made when first wanted."
  (or romacs-agent-ui--asks (setq romacs-agent-ui--asks (make-hash-table :test #'equal))))
(defvar-local romacs-agent-ui--end nil
  "Marker at the transcript's end; the composer follows it.")
(defvar-local romacs-agent-ui--draft-images nil
  "The image files of the message being written, in order.")
(defvar-local romacs-agent-ui--messages nil
  "An open message's id to (START . LENGTH): where its text starts and how
much of it its deltas wrote.")
(defvar-local romacs-agent-ui--faced nil
  "An open message's id to how much of its text is faced as Markdown.")

(defun romacs-agent-ui--faced ()
  "This chat's `romacs-agent-ui--faced', made when first wanted."
  (or romacs-agent-ui--faced (setq romacs-agent-ui--faced (make-hash-table :test #'equal))))
(defvar-local romacs-agent-ui--shown 0
  "The last journal record this buffer shows whole.")
(defvar-local romacs-agent-ui--open nil
  "The kind of block the transcript ends in: `text' (prose), a run's
`thinking' or `sub' (a subagent's words), or nil.")
(defvar-local romacs-agent-ui--history nil)
(defvar-local romacs-agent-ui--composer nil
  "The overlay at the composer's start: the live rule and the empty hint.")
(defvar-local romacs-agent-ui--fitted nil
  "The width the rules' fills last fitted.")
(defvar-local romacs-agent-ui--staying nil
  "Markers that stay before text the transcript's end takes, as a
window's start does: where what an event draws starts.")

(defun romacs-agent-ui--clock (seconds)
  "SECONDS as a clock: 12s, 4:26 or 1:02:03."
  (let ((s (max 0 (floor seconds))))
    (cond ((< s 60) (format "%ds" s))
          ((< s 3600) (format "%d:%02d" (/ s 60) (% s 60)))
          (t (format "%d:%02d:%02d" (/ s 3600) (/ (% s 3600) 60) (% s 60))))))

;;; What the board and the header share

(require 'keymap-popup)
(require 'romacs-agent-diff)
(require 'romacs-agent-rollback)
(require 'romacs-agent-renew)
(require 'romacs-agent-handoff)
(require 'romacs-agent-idle)

(defalias 'romacs-agent-ui--tool-name #'romacs-agent-rows-name)

(defun romacs-agent-ui--delegation (name input)
  "A harness's own subagent's (TITLE TYPE), when tool NAME's INPUT starts
one, else nil: the workers' rows name it."
  (when (and (hash-table-p input) (member name '("Agent" "Task")))
    (list (or (gethash "description" input) "task") (gethash "subagent_type" input))))

(defun romacs-agent-ui--selection-label (selection)
  "SELECTION, a model and effort, for a person: unnamed, the harness's own."
  (mapconcat #'identity (or (delq nil (list (plist-get selection :model) (plist-get selection :effort)))
                            '("harness's own"))
             " · "))

(defun romacs-agent-ui--context-share (session)
  "How full SESSION's context is, as \"NN%\", or nil when it is not known."
  (let ((used (romacs-agent-context-used session)) (window (romacs-agent-context-window session)))
    (and (numberp used) (numberp window) (> window 0)
         (format "%d%%" (round (* 100.0 (/ used (float window))))))))

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

(defvar romacs-agent-ui-changed-hook nil
  "Run once per batch of events that may change sessions' states, after
their rows and the status bar: views of every session (the agent board)
follow.")

(defun romacs-agent-ui--batch ()
  "Bring what sessions' events change up to date at the end of this batch
of them: rows of `romacs-agent-ui--children-changed', the status bar,
then `romacs-agent-ui-changed-hook'."
  (unless romacs-agent-ui--children-timer
    (setq romacs-agent-ui--children-timer (run-at-time 0 nil #'romacs-agent-ui--children-flush))))

(defun romacs-agent-ui--children-changed (session)
  "Make SESSION's rows again at the end of this batch of events."
  (puthash session t romacs-agent-ui--children-changed)
  (romacs-agent-ui--batch))

(defun romacs-agent-ui--children-flush ()
  (setq romacs-agent-ui--children-timer nil)
  (let ((sessions (hash-table-keys romacs-agent-ui--children-changed)))
    (clrhash romacs-agent-ui--children-changed)
    (romacs-agent-ui--flush-rows sessions)
    (romacs-agent-ui--status-bar)
    ;; While a start restores, its views are told once, by the batch its
    ;; end runs (`romacs-agent-restored-hook'): not a turn of its loading
    ;; at a time, each redrawing every row.
    (unless romacs-agent--restoring (run-hooks 'romacs-agent-ui-changed-hook))))


(defun romacs-agent-ui--flush-rows (sessions)
  "Make SESSIONS' rows again; their headers and an open actions menu
follow, and the parent of one whose state that changed, in the next batch."
  (dolist (session sessions)
    (romacs-agent-ui--summarize-children session)
    (when-let* (((romacs-agent-ui--announce session))
                (parent (romacs-agent-loaded (plist-get (romacs-agent-options session) :parent))))
      (romacs-agent-ui--children-changed parent))
    (romacs-agent-ui--header-refresh session)
    ;; Its chat's actions menu, when open, lists its workers as they are.
    (when-let* ((buffer (gethash (romacs-agent-id session) romacs-agent-log--chats))
                ((buffer-live-p buffer))
                ((buffer-live-p keymap-popup--buffer))
                ((eq buffer (plist-get (keymap-popup--session-state keymap-popup--buffer) :source))))
      (keymap-popup--refresh keymap-popup--buffer))))

(defun romacs-agent-ui--track-subagent (session event)
  "Keep what tool EVENT says of SESSION's harness's own subagents."
  (let ((id (plist-get event :id)) (table (romacs-agent-subagents session)))
    (if (eq (plist-get event :state) 'started)
        (when (and (member (plist-get event :name) romacs-agent-ui--subagent-tools)
                   (not (plist-get event :parent)))
          (unless table (setq table (setf (romacs-agent-subagents session) (make-hash-table :test #'equal))))
          (puthash id (list :tool id :subagent t :state "running" :time (plist-get event :time)
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

(defvar romacs-agent-ui--task-tools (make-hash-table :test #'eq :weakness 'key)
  "Each session's running harness tasks: a table of task id to the tool id
whose row runs it.  A task's event finds its row by it, not by a scan.")

(defun romacs-agent-ui--task-tools (session)
  "SESSION's table of running harness tasks (`romacs-agent-ui--task-tools')."
  (or (gethash session romacs-agent-ui--task-tools)
      (puthash session (make-hash-table :test #'equal) romacs-agent-ui--task-tools)))

(defun romacs-agent-ui--forget-task (session row)
  "ROW no longer runs its task: SESSION's index forgets it."
  (when-let* ((task (plist-get row :task)))
    (remhash task (romacs-agent-ui--task-tools session))))

(defun romacs-agent-ui--track-task (session event)
  "Keep what task EVENT says of the work SESSION's harness runs under a tool
call: its subagents, and commands.  The task's row is its tool's, made
here for a tool that is not a subagent's: hidden while it runs in the
foreground, as the tool's own card shows it."
  (let* ((table (or (romacs-agent-subagents session)
                    (setf (romacs-agent-subagents session) (make-hash-table :test #'equal))))
         (id (or (plist-get event :id) (gethash (plist-get event :task) (romacs-agent-ui--task-tools session))))
         (row (and id (or (gethash id table)
                          (list :tool id :state "running" :time (plist-get event :time) :title (plist-get event :title)
                                :hidden (not (plist-get event :background)))))))
    (when row
     (puthash id
             (pcase (plist-get event :state)
               ('started (puthash (plist-get event :task) id (romacs-agent-ui--task-tools session))
                         (plist-put row :task (plist-get event :task)))
               ('backgrounded (plist-put row :hidden nil))
               ('progress (plist-put (plist-put row :activity (plist-get event :activity))
                                     :time (or (plist-get event :time) (plist-get row :time))))
               (end (let ((failed (eq end 'failed)))
                      (romacs-agent-ui--forget-task session row)
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
               (romacs-agent-ui--forget-task session row)
               (puthash id (plist-put (plist-put (plist-put row :state "cancelled") :task nil)
                                      :time (or time (plist-get row :time)))
                        table)))))
    (romacs-agent-ui--children-changed session)))

(defconst romacs-agent-ui--state-statuses
  '("detached" "attached" "stopped" "supervisor not reachable" "cancelled" "plan mode" "plan mode off")
  "The `status' events that change what a session's state reads.")

(defun romacs-agent-ui--state-event-p (event)
  "Whether EVENT may change its session's state (`romacs-agent-ui--state')."
  (or (memq (plist-get event :type) '(run session input approval answered plan done exit unstarted settled))
      (and (eq (plist-get event :type) 'status) (member (plist-get event :text) romacs-agent-ui--state-statuses))))

(defvar romacs-agent-ui--announced (make-hash-table :test #'eq :weakness 'key)
  "Each session's state key as its views were last told of it.")

(defun romacs-agent-ui--announce (session)
  "Note SESSION's state key as its views' now; return whether it moved."
  (let ((key (romacs-agent-ui--state-key session)))
    (unless (eq key (gethash session romacs-agent-ui--announced))
      (puthash session key romacs-agent-ui--announced)
      t)))

(defun romacs-agent-ui--state-moved-p (session event)
  "Whether SESSION's EVENT may have changed a state its views show.  An
event whose kind can is, a `changed' one (a write no harness event says)
only when the state's key differs from the one its views were last told:
an option or a selection that leaves it as it was redraws no row."
  (if (eq (plist-get event :type) 'changed)
      (romacs-agent-ui--announce session)
    (and (romacs-agent-ui--state-event-p event)
         (progn (romacs-agent-ui--announce session) t))))

(defun romacs-agent-ui--track-children (session event)
  "Note what EVENT changes of what runs under SESSION, or above it.
Return whether SESSION's state may have moved (`romacs-agent-ui--state-moved-p'),
decided once per event."
  (let ((type (plist-get event :type))
        (moved (romacs-agent-ui--state-moved-p session event)))
    (pcase type
      ('tool (romacs-agent-ui--track-subagent session event))
      ('task (romacs-agent-ui--track-task session event))
      ('input (romacs-agent-ui--end-subagents session t nil nil))
      ((or 'done 'exit) (romacs-agent-ui--end-subagents session nil (eq type 'exit) (plist-get event :time))))
    (when moved
      (when-let* ((parent (plist-get (romacs-agent-options session) :parent))
                  (known (romacs-agent-loaded parent)))
        (romacs-agent-ui--children-changed known)))
    moved))

;; A row's :key is its state's (`romacs-agent-ui--states'), made with
;; the rows: drawing, ranking and its parent's state only read it.

(defun romacs-agent-ui--child-row (child)
  "Delegated session CHILD as a row."
  (list :session child :key (romacs-agent-ui--state-key child) :title (romacs-agent-title child)
        :time (romacs-agent-last-time child)))

(defun romacs-agent-ui--row-key (row)
  "Harness task ROW's state key, by its status: a failure is news until a
turn of its session starts, a completion never (its session's turn tells it)."
  (let ((key (romacs-agent-ui--status-key (plist-get row :state))))
    (if (or (eq key 'completed) (and (eq key 'failed) (not (plist-get row :unseen)))) 'idle key)))

(defun romacs-agent-ui--row-rank (row)
  (romacs-agent-ui--rank (plist-get row :key)))

(defun romacs-agent-ui--summarize-children (session)
  "Make SESSION's rows, ranked, then the newest first, and its summary."
  (let ((rows (append (mapcar #'romacs-agent-ui--child-row (romacs-agent-children session))
                      (and (romacs-agent-subagents session)
                           (mapcar (lambda (row) (append (list :key (romacs-agent-ui--row-key row)) row))
                                   (cl-remove-if (lambda (row) (plist-get row :hidden))
                                                 (hash-table-values (romacs-agent-subagents session))))))))
    (setf (romacs-agent-children-rows session)
          (sort rows (lambda (a b)
                       (let ((x (romacs-agent-ui--row-rank a)) (y (romacs-agent-ui--row-rank b)))
                         (if (= x y) (> (or (plist-get a :time) 0) (or (plist-get b :time) 0)) (< x y)))))
          (romacs-agent-children-summary session)
          (romacs-agent-ui--workers-summary (romacs-agent-children-rows session)))))

(defconst romacs-agent-ui--worker-kinds
  '((approval "✋" "Needs approval" romacs-agent-approval)
    (question "❓" "Asks you" romacs-agent-approval)
    (failed "❗" "Failed" romacs-agent-error)
    (subagents "🤖" "Working" romacs-agent-running)
    (threads "🧵" "Threads live" romacs-agent-running))
  "What a session's header counts of its workers, as (KIND GLYPH WORD
FACE): what wants the user, then its harness's own subagents running,
then its delegated threads still live.")

(defun romacs-agent-ui--worker-kind (row)
  "The kinds worker ROW counts as (`romacs-agent-ui--worker-kinds')."
  (let ((key (plist-get row :key)))
    (delq nil (list (and (memq key '(approval question failed)) key)
                    (cond ((plist-get row :session) (and (memq key romacs-agent-ui--live-keys) 'threads))
                          ;; A command the harness runs is no subagent.
                          ((and (eq key 'working) (plist-get row :subagent)) 'subagents))))))

(defun romacs-agent-ui--workers-summary (rows)
  "ROWS, a session's workers ranked, as its header's counts by kind
\(`romacs-agent-ui--worker-kinds'), or nil when none counts: hovering lists
every worker by title and state."
  (let* ((kinds (mapcan #'romacs-agent-ui--worker-kind rows))
         (parts (delq nil (mapcar (lambda (kind) (let ((count (cl-count (car kind) kinds)))
                                                   (and (> count 0) (romacs-agent-ui--count kind count))))
                                  romacs-agent-ui--worker-kinds))))
    (when parts
      (let ((text (mapconcat #'identity parts (propertize (if romacs-agent-glyphs " " " · ") 'face 'romacs-agent-dim)))
            (listing (mapconcat (lambda (row) (format "%s: %s" (or (plist-get row :title) "task")
                                                      (downcase (nth 2 (romacs-agent-ui--state-of (plist-get row :key))))))
                                (seq-take rows 20) "\n")))
        (add-text-properties 0 (length text) (list 'help-echo listing) text)
        text))))

(defvar romacs-agent-ui--turn-starts (make-hash-table :test #'eq :weakness 'key)
  "Each session's last turn start seen live: (TIME . DONE-SEQ), the turn
end it came after.")

(defun romacs-agent-ui--see-failed-children (session event)
  "SESSION's live turn starts at EVENT, a message after a turn's end (one
steering the turn under way starts none): a failure of a task it
delegated that was there when its turn before started has been in view a
whole turn, and is seen now, as opening the task's chat sees it."
  (when (and (eq (plist-get event :type) 'input) (romacs-agent--live-p session))
    (let ((before (gethash session romacs-agent-ui--turn-starts)))
      (unless (and before (eql (cdr before) (romacs-agent-done-seq session)))
        (when before
          (dolist (child (romacs-agent-children session))
            (when (and (eq (romacs-agent-ui--state-key child) 'failed)
                       (<= (or (romacs-agent-last-time child) 0) (car before)))
              (romacs-agent-mark-seen child))))
        (puthash session (cons (or (plist-get event :time) (float-time)) (romacs-agent-done-seq session))
                 romacs-agent-ui--turn-starts)))))

(defun romacs-agent-ui--on-event (session event)
  "Follow SESSION's EVENT in what runs under it and above it, its header,
and the views a changed state moves (the chat draws it itself)."
  (romacs-agent-ui--see-failed-children session event)
  ;; A report comes with each request: the views follow only what it changes.
  (when (and (eq (plist-get event :type) 'rate-limit)
             (not (equal (romacs-agent-ui--pools-line) romacs-agent-ui--pools-shown)))
    (romacs-agent-ui--batch))
  (let ((moved (romacs-agent-ui--track-children session event)))
    (romacs-agent-ui--header-refresh session)
    (when moved (romacs-agent-ui--batch))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-ui--on-event)

;;; A session's state
;;
;; One vocabulary for every view: a chat's header, its workers' rows and
;; summary, and the status bar.  The states rank as T3 Code ranks its
;; threads' (Sidebar.logic.ts), what wants the user first.

(defcustom romacs-agent-glyphs t
  "Whether chats say a state by a glyph (🤖 working, ✋ waiting for you, ❗
failed...) where the frame can draw it: the word shows on hover."
  :type 'boolean :group 'romacs-agent
  ;; Sessions' worker summaries, the status bar and chats' headers are
  ;; made ahead: made again when it is set, never by loading this file
  ;; (which remakes the headers once its definitions are in, at its end).
  :initialize #'custom-initialize-default
  :set (lambda (symbol value)
         (set-default symbol value)
         (romacs-agent-ui--glyphs-changed)))

(defconst romacs-agent-ui--states
  (seq-map-indexed
   (lambda (state rank) (append state (list rank)))
   '((approval "✋" "Needs approval" romacs-agent-approval)
     (question "❓" "Asks you" romacs-agent-approval)
     (failed "❗" "Failed" romacs-agent-error)
     (working "🤖" "Working" romacs-agent-running)
     (waiting "⏳" "Waiting on its workers" romacs-agent-running)
     (plan "📋" "Plan ready" romacs-agent-plan)
     (completed "✅" "Completed" romacs-agent-done)
     ;; Nothing to say: no glyph.
     (idle nil "Idle" romacs-agent-dim)
     (detached "🔌" "Detached" romacs-agent-dim)
     (unstarted "❗" "Never started" romacs-agent-error)
     (ended "⏹" "Ended" romacs-agent-dim)
     (history "📜" "History" romacs-agent-dim)
     (settled nil "Settled" romacs-agent-dim)))
  "Every state a session or its worker is in, as (KEY GLYPH WORD FACE
RANK), RANK its place here.  The states ranked before `idle' are news.")

(defconst romacs-agent-ui--live-keys '(approval question working waiting)
  "The states something still goes on in.")

(defun romacs-agent-ui--state-of (key)
  "The state KEY names."
  (assq key romacs-agent-ui--states))

(defun romacs-agent-ui--rank (key)
  (nth 4 (romacs-agent-ui--state-of key)))

(defun romacs-agent-ui--news-p (key)
  "Whether state KEY says something to the user."
  (< (romacs-agent-ui--rank key) (romacs-agent-ui--rank 'idle)))

(defun romacs-agent-ui--plan-ready-p (session)
  "Whether SESSION's last turn, in plan mode, presented a plan and ended."
  (and (plist-get (romacs-agent-options session) :plan)
       (romacs-agent-planned-seq session)
       (eql (romacs-agent-planned-seq session) (romacs-agent-input-seq session))
       (not (romacs-agent-failed session))))

(defun romacs-agent-ui--workers-live-p (session)
  "Whether one of SESSION's workers still goes on.  Its rows are ranked
\(`romacs-agent-ui--summarize-children'): the live ones rank no later
than `waiting', so the finished ones are never read."
  (let ((last (romacs-agent-ui--rank 'waiting)))
    (cl-loop for row in (romacs-agent-children-rows session)
             while (<= (romacs-agent-ui--row-rank row) last)
             thereis (memq (plist-get row :key) romacs-agent-ui--live-keys))))

(defun romacs-agent-ui--state-key (session)
  "SESSION's state key (`romacs-agent-ui--states'), from its data alone:
its workers' as their rows were last made.  What goes on comes first,
then a settled session says nothing more.  A delegated session's
completion went to its parent: not news to the user."
  (let ((state (romacs-agent-state session)))
    (cond ((eq state 'waiting-approval)
           (let ((questions (romacs-agent-questions session)))
             (if (and questions (not (seq-difference (romacs-agent-asks session) questions))) 'question 'approval)))
          ;; Cancelled, its failure is the user's own doing.
          ((and (romacs-agent-failed session) (romacs-agent-unseen-p session) (not (romacs-agent-cancelled session)))
           'failed)
          ((memq state '(starting running)) 'working)
          ((eq state 'history) 'history)
          ((romacs-agent-ui--workers-live-p session) 'waiting)
          ((romacs-agent-settled-p session) 'settled)
          ((romacs-agent-cancelled session) 'ended)
          ((eq state 'detached) 'detached)
          ((eq state 'unstarted) 'unstarted)
          ((romacs-agent-ui--plan-ready-p session) 'plan)
          ((and (romacs-agent-unseen-p session) (not (romacs-agent-failed session))
                (not (plist-get (romacs-agent-options session) :parent)))
           'completed)
          ((eq state 'ended) 'ended)
          (t 'idle))))

(defun romacs-agent-ui--state (session)
  "SESSION's state, as (KEY GLYPH WORD FACE RANK)."
  (romacs-agent-ui--state-of (romacs-agent-ui--state-key session)))

(defun romacs-agent-ui--status-key (word)
  "A task's status WORD, as `romacs-agent-tools--status' or a harness
task's row says it, as a state key."
  (pcase word
    ((or "running" "starting") 'working)
    ("waiting" 'approval)
    ("failed" 'failed)
    ("completed" 'completed)
    ((or "idle" "detached") (intern word))
    (_ 'ended)))

(defun romacs-agent-ui--glyph (glyph word face)
  "GLYPH, or WORD where glyphs are off or GLYPH cannot be drawn, in FACE,
WORD its help."
  (propertize (if (and romacs-agent-glyphs (char-displayable-p (aref glyph 0))) glyph word)
              'face face 'help-echo word))

(defun romacs-agent-ui--label (state)
  "STATE's glyph (`romacs-agent-ui--glyph'), or nil for a state without one."
  (pcase-let ((`(,_ ,glyph ,word ,face) state))
    (and glyph (romacs-agent-ui--glyph glyph word face))))

(defun romacs-agent-ui--count (state count)
  "COUNT of what is in STATE: its glyph and the count, else the count and
its word."
  (pcase-let* ((`(,_ ,glyph ,word ,face) state)
               (shown (romacs-agent-ui--glyph glyph word face))
               (word (downcase word)))
    (propertize (if (equal (substring-no-properties shown) (nth 2 state)) (format "%d %s" count word)
                  (format "%s%d" glyph count))
                'face face 'help-echo (format "%d %s" count word))))

(defun romacs-agent-ui--rollup (keys)
  "State KEYS as a line of each news state's count (🤖2 ✋1), in rank
order, or nil when none is news."
  (let ((parts (delq nil (mapcar (lambda (state)
                                   (let ((count (cl-count (car state) keys)))
                                     (and (> count 0) (romacs-agent-ui--news-p (car state))
                                          (romacs-agent-ui--count state count))))
                                 romacs-agent-ui--states))))
    (and parts (mapconcat #'identity parts (propertize (if romacs-agent-glyphs " " " · ") 'face 'romacs-agent-dim)))))

;;; A state's mark
;;
;; One mark for each kind of state, the same in every view of agents (the
;; board's rows, a chat's chips): what waits for the user ◆, failed ✗,
;; working ●, done ✓, nothing more ·.

(defface romacs-agent-chip '((t :weight bold :inverse-video t))
  "A state chip: a short word on its state's colour, set off from text.")
(defface romacs-agent-chip-done '((t :inherit (success romacs-agent-chip)))
  "A chip of something that finished well.")
(defface romacs-agent-chip-failed '((t :inherit (error romacs-agent-chip)))
  "A chip of something that failed.")
(defface romacs-agent-chip-working '((t :inherit (romacs-working-running romacs-agent-chip)))
  "A chip of something still working.")
(defface romacs-agent-chip-you '((t :inherit (romacs-working-waiting romacs-agent-chip)))
  "A chip of something waiting for the user.")
(defconst romacs-agent-ui--marks
  '((you "◆" romacs-agent-approval romacs-agent-chip-you "needs you")
    (failed "✗" romacs-agent-error romacs-agent-chip-failed "failed")
    (working "●" romacs-agent-running romacs-agent-chip-working "working")
    (done "✓" romacs-agent-done romacs-agent-chip-done "done")
    (idle "·" romacs-agent-dim nil "idle"))
  "Each kind of state (`romacs-agent-ui--chip-kind') as (KIND MARK FACE
CHIP-FACE WORD), what wants the user first: the one mark every view of
agents says it by.")

(defun romacs-agent-ui--chip-kind (key)
  "The kind of state KEY (`romacs-agent-ui--states'): `you' waits for the
user, `failed', `working', `done' finished well and unseen, `idle' says
nothing more."
  (pcase key
    ((or 'approval 'question 'plan) 'you)
    ((or 'failed 'unstarted) 'failed)
    ((or 'working 'waiting) 'working)
    ('completed 'done)
    (_ 'idle)))

(defun romacs-agent-ui--mark (kind)
  "KIND's mark (`romacs-agent-ui--marks') in its face, its word on hover."
  (pcase-let ((`(,_ ,mark ,face ,_ ,word) (assq kind romacs-agent-ui--marks)))
    (propertize mark 'face face 'help-echo word)))

(defun romacs-agent-ui--kind-counts (keys)
  "State KEYS as each kind's mark and count, what wants the user first
\(◆1 ●2 ✗1 ✓3), or nil when none counts; `idle' is not counted."
  (let ((kinds (mapcar #'romacs-agent-ui--chip-kind keys)))
    (and-let* ((parts (delq nil (mapcar (lambda (entry)
                                          (let ((count (cl-count (car entry) kinds)))
                                            (and (> count 0) (not (eq (car entry) 'idle))
                                                 (concat (romacs-agent-ui--mark (car entry))
                                                         (propertize (number-to-string count) 'face (nth 2 entry))))))
                                        romacs-agent-ui--marks))))
      (string-join parts " "))))

(defvar romacs-mode-line-agent)

;; Sessions a new server restored raised no events: their news shows.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-ui--batch)

(defun romacs-agent-ui--pools-line ()
  "The pools with an account at or near its limit, each as ⛽ its harness
and how many of its accounts have room, its accounts in the hover: nil
while every account has room and none warns."
  (let ((parts nil))
    (dolist (cell romacs-agent--drivers)
      (let* ((pool (romacs-agent-pool (car cell)))
             (rooms (mapcar (lambda (i) (romacs-agent-room-of (plist-get i :name))) pool)))
        (when (cl-some (lambda (room) (memq (plist-get room :status) '(warning rejected))) rooms)
          (push (propertize
                 (format "⛽%s %d/%d" (car cell)
                         (cl-count-if (lambda (i) (romacs-agent-room (plist-get i :name))) pool) (length pool))
                 'face (if (cl-some (lambda (i) (romacs-agent-room (plist-get i :name))) pool) 'warning 'romacs-agent-error)
                 'help-echo (mapconcat
                             (lambda (i)
                               (let ((room (romacs-agent-room-of (plist-get i :name))))
                                 (format "%s: %s%s%s" (plist-get i :name)
                                         (if room (plist-get room :status) "no report")
                                         (if (numberp (plist-get room :used)) (format ", %d%% used" (round (* 100 (plist-get room :used)))) "")
                                         (if (numberp (plist-get room :resets))
                                             (format-time-string ", resets %a %H:%M" (plist-get room :resets)) ""))))
                             pool "\n"))
                parts))))
    (and parts (mapconcat #'identity (nreverse parts) " "))))

(defvar romacs-agent-ui--pools-shown nil "The pools line the status bar shows.")

(defun romacs-agent-ui--status-bar ()
  "Show every session's news in the status bar (`romacs-mode-line-agent'),
and the pools whose accounts near their limits."
  (setq romacs-agent-ui--pools-shown (romacs-agent-ui--pools-line))
  (let ((keys nil))
    (maphash (lambda (_ session) (push (romacs-agent-ui--state-key session) keys)) romacs-agent--sessions)
    (let ((rollup (let ((parts (delq nil (list (romacs-agent-ui--rollup keys) romacs-agent-ui--pools-shown))))
                    (and parts (mapconcat #'identity parts " ")))))
      (unless (equal rollup (and (boundp 'romacs-mode-line-agent) (default-value 'romacs-mode-line-agent)))
        (setq-default romacs-mode-line-agent rollup)
        (force-mode-line-update t)))))

;;; The header
;;
;; A value, never a form: the buffer's `header-line-format' is the string
;; `romacs-agent-ui--header-show' last made, so showing it runs no Lisp.
;; It is made where the board's views are told a session changed: after
;; each of the session's events (a `changed' one too, for what no harness
;; event says), when the batch remakes the session's rows, and when what
;; draws a glyph changes.  It makes and writes nothing while the header
;; would read the same.  The string is the whole line, the same for every
;; window: what depends on a window is display's, which clips the band
;; where the window ends.

(defun romacs-agent-ui--harness-icon (harness)
  (let ((icon (or (plist-get (cdr (gethash harness romacs-agent--driver-table)) :icon) "●")))
    (if (char-displayable-p (aref icon 0)) icon (symbol-name harness))))

(defun romacs-agent-ui--effort (effort)
  "EFFORT for a person, in its level's face: one Romacs does not know in
`romacs-agent-effort', none as \"default\"."
  (if effort
      (let ((face (intern (concat "romacs-agent-effort-" effort))))
        (propertize effort 'face (if (facep face) face 'romacs-agent-effort)))
    (propertize "default" 'face 'romacs-agent-effort-default)))

(defun romacs-agent-ui--header-inputs (session)
  "All that SESSION's header reads of it, a plist, with whether glyphs
are on: the header is made from it (`romacs-agent-ui--header-text'), and
made again when it differs."
  (let ((options (romacs-agent-options session)) (switch (romacs-agent-pending-selection session)))
    (list :harness (romacs-agent-harness session) :provider (romacs-agent-provider session)
          :mode (if (plist-get options :plan) 'plan (romacs-agent-permission-mode session))
          :model (romacs-agent-model session) :effort (plist-get options :effort)
          :switch (and switch (romacs-agent-ui--selection-label switch))
          :workers (romacs-agent-children-summary session)
          :share (romacs-agent-ui--context-share session)
          :resume (romacs-agent-resume-time session)
          :state (romacs-agent-ui--state-key session)
          :glyphs romacs-agent-glyphs)))

(defun romacs-agent-ui--header-text (inputs)
  "A chat's header for INPUTS (`romacs-agent-ui--header-inputs'), only what
is worth a glance, what matters most first: harness, its state, model and
effort, the instance when not the harness's own, a permission mode other
than bypassing them, a selection waiting, its workers, and how full its
context is.  The whole line: a window too narrow for it shows its start,
display clipping the band where the window ends."
  (cl-destructuring-bind (&key harness provider mode model effort switch workers share state resume &allow-other-keys)
      inputs
    (mapconcat
     #'identity
     (delq nil
           (list (romacs-agent-ui--harness-icon harness)
                 (romacs-agent-ui--label (romacs-agent-ui--state-of state))
                 (concat (propertize (or (romacs-agent-ui--instance nil harness model nil) "...")
                                     'face 'romacs-agent-model 'help-echo (and provider (format "account %s" provider)))
                         (if effort (concat " " (romacs-agent-ui--effort effort)) ""))
                 (cond ((eq mode 'plan) (propertize "plan" 'face 'warning))
                       ((and mode (not (equal mode "bypassPermissions"))) (propertize mode 'face 'romacs-agent-mode)))
                 (and switch (propertize (format "→ %s" switch) 'face 'warning))
                 (and resume (propertize (format-time-string "⏰%H:%M" resume) 'face 'warning
                                         'help-echo "Resumes by itself when its limit resets; z in the actions menu cancels"))
                 workers
                 (and share (propertize share 'face 'romacs-agent-context))))
     "  ")))

(defvar-local romacs-agent-ui--header nil
  "What the chat's header was last made from (`romacs-agent-ui--header-inputs').")

(defun romacs-agent-ui--header-format (text)
  "TEXT as a header line shows it: each `%' said literally."
  (string-replace "%" "%%" text))

(defun romacs-agent-ui--header-show (&optional force)
  "Make this chat's header say what its session says now.
While what the header reads is what it was made from, nothing is made;
while it would read the same, nothing is written, so display sees no
change.  FORCE makes it again whatever it was made from."
  (let ((inputs (romacs-agent-ui--header-inputs romacs-agent-ui--session)))
    (unless (and (not force) (equal-including-properties inputs romacs-agent-ui--header))
      (let ((header (romacs-agent-ui--header-format (romacs-agent-ui--header-text inputs))))
        (setq romacs-agent-ui--header inputs)
        (unless (equal-including-properties header header-line-format)
          (setq header-line-format header)
          (force-mode-line-update))))))

(defun romacs-agent-ui--header-refresh (session)
  "Make the header of SESSION's chat say what SESSION says now: its
workers' states, which its own events do not tell it, among it."
  (when-let* ((buffer (gethash (romacs-agent-id session) romacs-agent-log--chats))
              ((buffer-live-p buffer))
              ;; A copy a journal is replayed into names the same buffer.
              ((eq (buffer-local-value 'romacs-agent-log--session buffer) session)))
    (with-current-buffer buffer (romacs-agent-log--header-show))))

(defun romacs-agent-ui--headers-remake ()
  "Make every chat's header again, a transcript's view's too, whose
session is no recorded one."
  (dolist (buffer (buffer-list))
    (when (buffer-local-value 'romacs-agent-log--session buffer)
      (with-current-buffer buffer
        (setq romacs-agent-log--header-made nil)
        (romacs-agent-log--header-show)))))

(defun romacs-agent-ui--glyphs-changed ()
  "What draws a glyph changed (`romacs-agent-glyphs', the frame's font):
every session's workers' line, the status bar and every chat's header are
made again."
  (mapc #'romacs-agent-ui--summarize-children (romacs-agent-sessions))
  (romacs-agent-ui--status-bar)
  (romacs-agent-ui--headers-remake))

(add-hook 'after-setting-font-hook #'romacs-agent-ui--glyphs-changed)

;;; Words rows share

(defun romacs-agent-ui--ago (time)
  "How long ago TIME was, compactly."
  (if (not (numberp time)) ""
    (let ((seconds (max 0 (- (float-time) time))))
      (cond ((< seconds 60) "now")
            ((< seconds 3600) (format "%dm" (/ seconds 60)))
            ((< seconds 86400) (format "%dh" (/ seconds 3600)))
            (t (format "%dd" (/ seconds 86400)))))))

(defun romacs-agent-ui--instance (provider harness model default)
  "The model column: PROVIDER when not DEFAULT, then MODEL without its
HARNESS's prefix (claude-opus-5-5 is opus-5-5)."
  (let ((provider (and provider (not (equal provider default)) provider))
        (model (if (and model harness) (string-remove-prefix (concat (symbol-name harness) "-") model) model)))
    (mapconcat #'identity (delq nil (list provider model)) " · ")))

;; Loaded into a running session, its open chats' headers are this file's:
;; an older version's form names functions this one has changed.
(romacs-agent-ui--headers-remake)

(provide 'romacs-agent-ui)
;;; romacs-agent-ui.el ends here

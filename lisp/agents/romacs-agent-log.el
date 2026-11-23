;;; romacs-agent-log.el --- An agent session's chat, as its log  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent session's chat: what its harness did, in order, as text, and
;; a composer below it to write the next message.  It shows every event
;; a session's records give (romacs-agent.el's events): its prompts, its
;; reply as it streams, its thinking, each tool call with its input and
;; its output's first lines, its subagents' work under their calls, every
;; approval it asks for and who answered it, its errors, and each turn's
;; end.  Nothing is folded or hidden.
;;
;; The buffer is the comint model:
;;
;;   transcript   read-only text, appended to
;;   activity     live delegated threads and harness subagents
;;   divider      one line: what the session does now
;;   composer     editable, RET sends
;;
;; Every character is text: no field, no intangible or invisible text,
;; no overlay strings, so motion and search work everywhere at any time.
;; The view never moves point.  What changes in place: a tool's output
;; is put under its call (what a running command printed so far gives way
;; to its whole output as it ends), a subagent's events under its Agent
;; call, a finished message gets its Markdown faces, an answered approval
;; loses its warning, and the separator is rewritten when the state
;; changes.
;;
;; The view draws from events alone, as its source gives them
;; (`romacs-agent-log--source'): for this editor's own sessions, live ones
;; through `romacs-agent-event-functions', and a session's history by
;; replaying its records into a copy (`romacs-agent--scratch'), the same
;; events drawn by the same code.  What the chat does to its session goes
;; through its source too.  A turn's output longer than
;; `romacs-agent-log-output-lines' shows its first lines and how many more:
;; RET on it shows all of it.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-glyphs)
(require 'romacs-agent-rows)
(require 'romacs-agent-store)

(require 'romacs-working)
(require 'romacs-agent-log-images)
(require 'romacs-agent-log-activity)

;; The actions the chat's menu offers on its session are the chat's of
;; before (romacs-agent-ui.el), which read the session from here too.
(defvar romacs-agent-ui--session)
(defvar romacs-agent-ui-changed-hook)
(declare-function romacs-agent-log-attach-place "romacs-agent-log-attach" ())
(declare-function romacs-agent-log-faces-soon "romacs-agent-log-faces" ())
(declare-function romacs-agent-log-rounds-you "romacs-agent-log-rounds" (text))
(declare-function romacs-agent-log-rounds-incoming "romacs-agent-log-rounds" (event incoming))
(declare-function romacs-agent-log-source-queue "romacs-agent-log-queue" (source session))
(declare-function romacs-agent-log-rounds-end "romacs-agent-log-rounds" ())
(declare-function romacs-agent-incoming-of "romacs-agent-tools" (text))
(declare-function romacs-agent-log-faces-output "romacs-agent-log-faces" (text))
(declare-function romacs-agent-log-faces-drop "romacs-agent-log-faces" ())
(declare-function romacs-agent-log-faces-end "romacs-agent-log-faces" (start at))
(declare-function romacs-agent-ask-open "romacs-agent-ask" (session id questions &optional place))
(declare-function romacs-agent-ui--header-text "romacs-agent-ui" (inputs))
(declare-function romacs-agent-ui--header-inputs "romacs-agent-ui" (session))
(declare-function romacs-agents "romacs-agent-board" ())
(declare-function romacs-agent-menu--session "romacs-agent-menu" ())
(declare-function romacs-agent-events-p "romacs-agent-events" (session))
(declare-function romacs-agent-events-end "romacs-agent-events" (session))
(declare-function romacs-agent-events-note-seq "romacs-agent-events" (session type end))
(declare-function romacs-agent--claude-questions "romacs-agent-claude" (input))
(declare-function romacs-agent--codex-questions "romacs-agent-codex" (params))
(declare-function romacs-agent-history-events "romacs-agent-history" (harness imported))

(defcustom romacs-agent-log-output-lines 5
  "How many lines of a tool's output, or of its input, a chat shows
under its call; RET on it shows all of it."
  :type 'natnum :group 'romacs-agent)

(defcustom romacs-agent-log-output-width 96
  "How many columns of each line of a tool's output a chat shows under
its call, a longer one cut with an ellipsis: a line a screen row in a
window of ordinary width.  RET on it shows all of it."
  :type 'natnum :group 'romacs-agent)

(defcustom romacs-agent-log-records 4000
  "The most of a session's latest records its chat draws as it opens,
whatever turns they are: a turn longer than that opens at its end."
  :type 'natnum :group 'romacs-agent)

(defcustom romacs-agent-log-turns 10
  "How many of a session's latest turns its chat shows as it opens; the
line above them brings earlier ones."
  :type 'natnum :group 'romacs-agent)

;;; Faces

(defface romacs-agent-log-prompt '((t :inherit bold))
  "A message the user sent.")
(defface romacs-agent-log-prompt-mark '((t :inherit (romacs-working-running bold)))
  "The mark before a message the user sent: the theme's colour for work.")
(defface romacs-agent-log-thinking '((t :inherit (shadow italic)))
  "What the agent thinks.")
(defface romacs-agent-log-tool '((t :inherit shadow))
  "A tool call's row within a turn.")
(defface romacs-agent-log-tool-failed '((t :inherit error))
  "A tool call whose harness reported failure.")
(defface romacs-agent-log-output '((t :inherit shadow))
  "A tool's output, and its input's lines.")

(defface romacs-agent-log-header-key '((t :inherit help-key-binding :weight bold))
  "A key the header names: the theme's key colour, bold, readable on the
header line.")

(defun romacs-agent-log--derive (&rest _)
  "Make a tool's output, a chat's dim words, its thinking and its rules
the theme's dim colour raised to 4.5:1 on its background, read, never
hidden; and a key the header names the theme's key colour raised to
4.5:1 on the header line: each as its default spec.  A theme's or the
user's own spec still wins."
  (let ((colour (romacs-color-legible (face-foreground 'shadow nil t))))
    ;; A tool's output, every dim word of a chat's rows, its thinking and
    ;; a turn's end: read, 4.5:1.
    (pcase-dolist (`(,face . ,more) '((romacs-agent-log-output) (romacs-agent-dim) (romacs-agent-log-note)
                                      (romacs-agent-log-rule) (romacs-agent-log-tool)
                                      (romacs-agent-log-thinking :slant italic)))
      (when (facep face)
        (face-spec-set face (if colour `((t :foreground ,colour ,@more)) `((t :inherit shadow ,@more))) 'face-defface-spec))))
  (let* ((key (face-foreground 'help-key-binding nil t))
         (colour (and key (romacs-color-legible key (face-background 'header-line nil t)))))
    (face-spec-set 'romacs-agent-log-header-key
                   (if colour `((t :foreground ,colour :weight bold)) '((t :inherit help-key-binding :weight bold)))
                   'face-defface-spec)))

(add-hook 'enable-theme-functions #'romacs-agent-log--derive)
(add-hook 'disable-theme-functions #'romacs-agent-log--derive)
(defface romacs-agent-log-note '((t :inherit shadow))
  "A status line: what the harness or Romacs says of the session.")
(defface romacs-agent-log-rule '((t :inherit shadow))
  "A turn's end, and the separator above the composer.")
;; Its faces derived once its dim faces are defined.
(romacs-agent-log--derive)
(defface romacs-agent-log-ask '((t :inherit romacs-working-waiting :extend t :weight bold))
  "An approval or a question that waits on the user.")
(defface romacs-agent-log-allowed '((t :inherit success))
  "An answer that allowed.")
(defface romacs-agent-log-denied '((t :inherit error))
  "An answer that denied, and an error.")

;;; State

(defvar-local romacs-agent-log--session nil "The session this chat shows.")
(defvar-local romacs-agent-log--end nil
  "Marker: the transcript's end, where the next event goes.")
(defvar-local romacs-agent-log--divider nil
  "Marker where the divider begins, after any live activity.")
(defvar-local romacs-agent-log--text-map nil
  "The keys of this buffer's transcript text, when not a chat's
\(`romacs-agent-log-transcript-map'): a view with no composer has its own.")
(defvar-local romacs-agent-log--rows nil "The rows the separator was last drawn with.")
(defvar-local romacs-agent-log-queue--editing nil
  "The queued message the composer holds to edit, or nil
\(romacs-agent-log-queue.el).")
(defvar-local romacs-agent-log--input nil
  "Marker: the composer's start, after the separator.")
(defvar-local romacs-agent-log--seq 0 "The seq of the last record drawn.")
(defvar-local romacs-agent-log--active-start nil
  "Start of the current session's transcript, after earlier handoffs.")
(defvar-local romacs-agent-log--from nil
  "Where the drawn records begin (a place, as `romacs-agent--journal-tail'
gives it), or nil when they begin at the session's start.")
(cl-defstruct (romacs-agent-log-call (:constructor romacs-agent-log-call--make) (:copier nil))
  "A tool call drawn open: what comes under it goes in its place."
  (id nil :documentation "The call's id.")
  (name nil :documentation "Its tool, as `romacs-agent-rows-name' says it.")
  (input nil :documentation "Its input, a table, or nil.")
  (depth 0 :documentation "How deep under subagents' calls it is.")
  (parent nil :documentation "The id of the call it is under, or nil.")
  (start nil :documentation "A marker where its row begins.")
  (under nil :documentation "A marker where what comes under it goes.")
  (kind nil :documentation "The kind of a subagent's text it holds.")
  (held "" :documentation "That text's line under way.")
  (live nil :documentation "A marker where its output streaming as it runs begins.")
  (streamed nil :documentation "That output, its last 64K.")
  (count 0 :documentation "The line breaks of that output.")
  (task nil :documentation "The id of the work its harness runs under it.")
  (activity nil :documentation "What that work said it does, last.")
  (ended nil :documentation "Its output drawn while that work goes on.")
  (mark nil :documentation "A leading mark on older rows, before their tool text.")
  (indent 0 :documentation "The characters occupied by a subagent's bars.")
  (task-ended nil :documentation "That work over before the call ended."))

(defvar-local romacs-agent-log--blocks nil
  "The calls drawn open, a table of `romacs-agent-log-call' by id.")

(defun romacs-agent-log--block (id)
  "The open call ID, or nil."
  (and id romacs-agent-log--blocks (gethash id romacs-agent-log--blocks)))

(defun romacs-agent-log--close-block (block)
  (set-marker (romacs-agent-log-call-under block) nil)
  (set-marker (romacs-agent-log-call-start block) nil)
  (remhash (romacs-agent-log-call-id block) romacs-agent-log--blocks))
(defvar-local romacs-agent-log--current nil
  "The id of the call at the transcript's top level that runs now, or nil.")
(defvar-local romacs-agent-log--early nil
  "Output that came before its call, an alist of ID to TEXT.")
(cl-defstruct (romacs-agent-log-stream (:constructor romacs-agent-log-stream--make) (:copier nil))
  "What streams at the transcript's end."
  (kind nil :documentation "`text' or `thinking'.")
  (start nil :documentation "A marker where its text began, nil until its first text.")
  (at nil :documentation "A marker where its next text goes, before its line's newline.")
  (held 0 :documentation "The line breaks it ended in so far, held until text follows:
a message ends with no trail of blank lines."))

(defvar-local romacs-agent-log--stream nil
  "The `romacs-agent-log-stream' at the transcript's end, or nil.")

(defun romacs-agent-log--reply-stream ()
  "The stream of the agent's words at the end, once it shows text, or nil."
  (let ((stream romacs-agent-log--stream))
    (and stream (eq (romacs-agent-log-stream-kind stream) 'text) (romacs-agent-log-stream-start stream)
         stream)))

(defvar-local romacs-agent-log--root-kind nil
  "The kind of the last entry drawn at the transcript's top level, not
under a call: what the agent's next words stand apart from.")
(cl-defstruct (romacs-agent-log-ask (:constructor romacs-agent-log-ask--make) (:copier nil))
  "An approval block drawn open."
  (id nil :documentation "Its request's id.")
  (start nil :documentation "A marker where its block begins.")
  (end nil :documentation "A marker where it ends."))

(defvar-local romacs-agent-log--asks nil
  "The approval blocks drawn open, `romacs-agent-log-ask's, the newest first.")

(defun romacs-agent-log--ask-of (id)
  "The approval block drawn open for request ID, or nil."
  (cl-find id romacs-agent-log--asks :key #'romacs-agent-log-ask-id :test #'equal))
(defvar-local romacs-agent-log--attributed nil
  "Request ids whose answer was drawn with who gave it.")
(defvar romacs-agent-log--closes-deferred nil
  "Non-nil while a source's history is drawn whole, what it waits on now
said only after: an ask open at a turn's end is said to have ended once
the history is drawn, unless an answer in it settles it.")

(cl-defstruct (romacs-agent-log-owed (:constructor romacs-agent-log-owed--make) (:copier nil))
  "What is owed for an ask's end: its note, put where it ended once its
source lets the ask go."
  (id nil :documentation "The ask's request id.")
  (entry nil :documentation "The note, saying why it ended.")
  (at nil :documentation "A marker where it ended."))

(defvar-local romacs-agent-log--ended-asks nil
  "Asks open when their turn ended or harness exited, still listed as
waited on by their source, `romacs-agent-log-owed's: each note is put
where its ask ended once the source lets the ask go
\(`romacs-agent-log--sync-asks'), as another machine's does after.")

(defun romacs-agent-log--owed (id)
  "What is owed for ask ID's end, or nil."
  (cl-find id romacs-agent-log--ended-asks :key #'romacs-agent-log-owed-id :test #'equal))

(defun romacs-agent-log--drop-ended (id)
  "Forget what is owed for ask ID's end, and return it: (ENTRY . AT)."
  (when-let* ((owed (romacs-agent-log--owed id)))
    (setq romacs-agent-log--ended-asks (delq owed romacs-agent-log--ended-asks))
    (prog1 (cons (romacs-agent-log-owed-entry owed) (marker-position (romacs-agent-log-owed-at owed)))
      (set-marker (romacs-agent-log-owed-at owed) nil))))
(defvar romacs-agent-log--replaying nil
  "While a replay draws, the copy of the session its records are read
into: what the session waited on then, not now.")
(defvar-local romacs-agent-log--state nil "What the separator last said.")
(defvar-local romacs-agent-log--width nil "The columns the separator was last made for.")
(defvar-local romacs-agent-log--prompts nil
  "The user's prompts this chat has drawn, the newest first: what M-p recalls.")
(defvar-local romacs-agent-log--recalled nil
  "The cells of `romacs-agent-log--prompts' M-p went through, the one the
composer shows first; nil when it shows the draft.")
(defvar-local romacs-agent-log--draft-kept nil
  "The composer's text as M-p first found it, given back past the newest.")
(defvar-local romacs-agent-log--filled nil
  "Whether the chat has drawn its session's records: a restored chat does
when a window first shows it.")

(defvar romacs-agent-log--chats (make-hash-table :test #'equal)
  "Each session id's chat buffer.")

;;; The source

;; Where a chat reads its session's events and acts on it: this editor's
;; own sessions, or another machine's through the agents service.  Each
;; source is a type with a method of each generic here; a chat draws the
;; same from any.

(cl-defstruct (romacs-agent-log-local-source (:constructor romacs-agent-log--local-source) (:copier nil))
  "The sessions this editor runs: their records replayed, their events
drawn as they are raised, acted on here.")

(defconst romacs-agent-log-local (romacs-agent-log--local-source)
  "This editor's own sessions, as a chat's source.")

(defvar-local romacs-agent-log--source romacs-agent-log-local
  "Where this chat reads its session and acts on it (;;; The source).")

(cl-defgeneric romacs-agent-log-source-fill (source session)
  "Draw SESSION's last turns in the current chat, from SOURCE: at once, or
as they come.")

(cl-defgeneric romacs-agent-log-source-earlier (source session)
  "Draw SESSION's turns before those the current chat shows, from SOURCE,
above them.")

(cl-defgeneric romacs-agent-log-source-approvals (source session)
  "What SESSION waits on now, as SOURCE says: a table of request id to
request, as `romacs-agent-approvals' is.")

(cl-defgeneric romacs-agent-log-source-send (source session text &optional when)
  "Send SESSION the user's TEXT through SOURCE: now, into the turn under
way when one runs; WHEN `later', once that turn ends; `interrupt', that
turn interrupted, then first of the messages waiting (now when none runs).
An empty TEXT is not sent.")

(cl-defgeneric romacs-agent-log-source-interrupt (source session)
  "Interrupt SESSION's turn under way, through SOURCE.")

(cl-defgeneric romacs-agent-log-source-respond (source session id allow reason)
  "Answer SESSION's request ID as the user, through SOURCE: ALLOW as
`romacs-agent-respond' takes it, REASON telling the agent why it is denied.")

(cl-defgeneric romacs-agent-log-source-ask (source session id questions)
  "Open the layer that answers SESSION's request ID, its QUESTIONS, through SOURCE.")

(cl-defgeneric romacs-agent-log-source-seen (source session)
  "SESSION's chat has been shown to its end, through SOURCE.")

(cl-defgeneric romacs-agent-log-source-request-event (source session id request)
  "The approval event that asks REQUEST, ID, of what SOURCE's session
waits on (`romacs-agent-log-source-approvals').")

(cl-defgeneric romacs-agent-log-source-chats (source)
  "SOURCE's chat buffers, a table by session id.")

(cl-defgeneric romacs-agent-log-source-answers-p (_source _session _event)
  "Whether this editor answers SESSION's approval EVENT itself, by its
policy, as SOURCE has it: no other source's requests."
  nil)

(cl-defgeneric romacs-agent-log-source-commands-p (_source)
  "Whether this editor's session commands (its model, rename, fork, stop
and the others of the chat's menu) act on SOURCE's sessions."
  nil)

(cl-defgeneric romacs-agent-log-source-restorable-p (_source)
  "Whether a chat of SOURCE's is saved with the desktop and restored
\(`romacs-agent-log-desktop-restore'): it is not, as of no source here."
  nil)

(defun romacs-agent-log--set-session (session)
  "Make SESSION the current chat's, named by its title; the session this
editor's session commands act on (`romacs-agent-ui--session'), when they
act on its source's."
  (setq romacs-agent-log--session session)
  (setq-local romacs-agent-ui--session (and (romacs-agent-log--commands-p) session))
  (let ((name (romacs-agent-log--buffer-name session)))
    (unless (equal (buffer-name) name) (rename-buffer name t)))
  (romacs-agent-log--header-show))

;;; Drawing

(defun romacs-agent-log--sealed (text)
  "TEXT as the transcript holds it: read-only, with the transcript's keys,
passing nothing to what is typed after it."
  (let ((text (copy-sequence text)))
    (add-text-properties 0 (length text) '(read-only t front-sticky (read-only) rear-nonsticky t) text)
    (let ((position 0))
      (while (setq position (text-property-any position (length text) 'keymap nil text))
        (let ((end (next-single-property-change position 'keymap text (length text))))
          (put-text-property position end 'keymap (or romacs-agent-log--text-map romacs-agent-log-transcript-map) text)
          (setq position end))))
    text))

(defmacro romacs-agent-log--writing (&rest body)
  "Run BODY, which writes the transcript: read-only lifted, no undo kept,
no hook told of it, and the buffer no more modified than it was (it is
no edit of the user's: only a draft typed is)."
  (declare (indent 0) (debug t))
  `(with-silent-modifications ,@body))

(defun romacs-agent-log--put (text &optional block)
  "Insert TEXT, sealed, at the transcript's end, or at the end of BLOCK, an
open call (`romacs-agent-log--blocks'): it ends after TEXT then, and so do
the calls it is under that ended where it did, while the calls under it
end before TEXT."
  (romacs-agent-log--writing
    (save-excursion
      (let ((at (if block (marker-position (romacs-agent-log-call-under block)) romacs-agent-log--end)))
        (goto-char at)
        ;; What the agent's next words stand apart from: an entry drawn
        ;; at the top level.
        (unless block
          (when-let* ((kind (and (> (length text) 0) (get-text-property 0 'romacs-agent-log-entry text))))
            (setq romacs-agent-log--root-kind kind)))
        (insert (romacs-agent-log--sealed text))
        (while block
          (let ((under (romacs-agent-log-call-under block)))
            (when (= under at) (set-marker under (point))))
          (setq block (romacs-agent-log--block (romacs-agent-log-call-parent block))))))))

(defun romacs-agent-log--indent-width (depth)
  "How many columns a subagent's line DEPTH deep is indented by."
  (if (zerop depth) 0 (+ (* 2 (1- depth)) (string-width (romacs-agent-glyph 'subagent)) 1)))

(defun romacs-agent-log--marks-changed ()
  "A mark of `romacs-agent-glyph-alist' changed: every chat's separator,
the composer's mark in it, is written again, a narrowed chat's too."
  (dolist (buffer (buffer-list))
    (when (markerp (buffer-local-value 'romacs-agent-log--input buffer))
      (with-current-buffer buffer
        (if (not (buffer-narrowed-p))
            (romacs-agent-log--separator t)
          ;; The narrowing's end stays before the separator written at it.
          (let ((beg (point-min-marker)) (end (copy-marker (point-max))))
            (unwind-protect (progn (widen) (romacs-agent-log--separator t))
              (narrow-to-region beg end)
              (set-marker beg nil)
              (set-marker end nil))))))))

(add-hook 'romacs-agent-glyph-alist-changed-hook #'romacs-agent-log--marks-changed)

(defun romacs-agent-log--indent (text depth)
  "TEXT with every line indented for a subagent DEPTH deep."
  (if (zerop depth) text
    (let ((bar (propertize (concat (make-string (* 2 (1- depth)) ?\s) (romacs-agent-glyph 'subagent) " ") 'face 'romacs-agent-log-rule)))
      ;; The bar opens what its line does.
      (replace-regexp-in-string "^\\(.\\)"
                                (lambda (match)
                                  (concat (propertize bar 'romacs-agent-log-full (get-text-property 0 'romacs-agent-log-full match)) match))
                                text t t))))

(defun romacs-agent-log--where (event)
  "Where EVENT is drawn: (BLOCK . DEPTH), under its parent's open call,
else (nil . 0), at the end."
  (let ((parent (romacs-agent-log--block (plist-get event :parent))))
    (if parent (cons parent (1+ (romacs-agent-log-call-depth parent))) (cons nil 0))))

(defun romacs-agent-log--close-stream (&optional discard)
  "End what streams: a message gets its Markdown faces, unless DISCARD,
its text about to go."
  (when-let* ((stream romacs-agent-log--stream))
    (setq romacs-agent-log--stream nil)
    (when-let* ((start (romacs-agent-log-stream-start stream))
                (at (romacs-agent-log-stream-at stream)))
      (if discard (romacs-agent-log-faces-drop)
        (when (eq (romacs-agent-log-stream-kind stream) 'text)
          (romacs-agent-log-faces-end start at)
          (romacs-agent-log-tables-draw start at)))
      (set-marker start nil)
      (set-marker at nil))))

(defun romacs-agent-log--blank-p ()
  "Whether the transcript ends in a blank line, or is empty: a blank line
before what comes next is there already."
  (or (<= romacs-agent-log--end (1+ (point-min)))
      (equal (buffer-substring-no-properties (- romacs-agent-log--end 2) romacs-agent-log--end) "\n\n")))

(defun romacs-agent-log--entry (text kind event)
  "TEXT as one entry of KIND for EVENT: a line or more, ending in a newline,
known to n and p."
  (let ((text (if (string-suffix-p "\n" text) text (concat text "\n"))))
    (add-text-properties 0 1 (list 'romacs-agent-log-entry kind) text)
    (when-let* ((seq (plist-get event :seq))) (put-text-property 0 (length text) 'romacs-agent-log-seq seq text))
    text))

(defun romacs-agent-log--line (event kind text &optional face)
  "Draw TEXT, in FACE, as EVENT's entry of KIND, where EVENT goes."
  (pcase-let ((`(,at . ,depth) (romacs-agent-log--where event)))
    (if at (romacs-agent-log--flush at) (romacs-agent-log--close-stream))
    (romacs-agent-log--put (romacs-agent-log--indent
                            (romacs-agent-log--entry (if face (propertize text 'face face) text) kind event)
                            depth)
                           at)))

(defun romacs-agent-log--stream-split (text)
  "TEXT as (LEAD BODY TRAIL): the count of line breaks before its body,
its body, and the count after."
  (let* ((lead (or (string-match-p "[^\n]" text) (length text)))
         (end (if (= lead (length text)) lead (string-match-p "\n*\\'" text lead))))
    (list lead (substring text lead end) (- (length text) end))))

(defun romacs-agent-log--whole-lines (text)
  "TEXT as (LINES . REST): its whole lines without their last newline, nil
if none ended, and the start of a line after them."
  (if-let* ((cut (string-match-p "\n[^\n]*\\'" text)))
      (cons (substring text 0 cut) (substring text (1+ cut)))
    (cons nil text)))

(defun romacs-agent-log--thinking-marks (body opens)
  "BODY, thinking, in its face, each paragraph after its first opened with
the mark, its first too if it OPENS one: none reads as the answer."
  (propertize (let* ((mark (concat (romacs-agent-glyph 'thinking) " "))
                     (hang (romacs-agent-glyph-hang mark)))
                (concat (if opens mark "")
                        ;; A paragraph's later lines hang under its first's words.
                        (replace-regexp-in-string "\n\n?" (lambda (breaks) (concat breaks (if (= (length breaks) 2) mark hang)))
                                                  body t t)))
              'face 'romacs-agent-log-thinking))

(defun romacs-agent-log--stream-chunk (kind first held body)
  "What BODY, a stream's text of KIND, puts at its end after HELD line
breaks: at most one blank line parts paragraphs, and its FIRST text,
the entry's start, has none before it."
  (let ((parted (cond (first "")
                      ;; A thinking paragraph's line goes on under its words.
                      ((and (eq kind 'thinking) (= held 1))
                       (concat "\n" (romacs-agent-glyph-hang (concat (romacs-agent-glyph 'thinking) " "))))
                      (t (make-string (min held 2) ?\n))))
        (body (replace-regexp-in-string "\n\n\n+" "\n\n" body)))
    (let ((chunk (concat parted (if (eq kind 'thinking)
                                    (romacs-agent-log--thinking-marks body (or first (>= held 2)))
                                  body))))
      (when first (put-text-property 0 1 'romacs-agent-log-entry kind chunk))
      chunk)))

(defun romacs-agent-log--stream-of (kind)
  "The stream of KIND at the end: the one there, or a new one, the other
kind's closed."
  (let ((stream romacs-agent-log--stream))
    (if (and stream (eq (romacs-agent-log-stream-kind stream) kind)) stream
      (romacs-agent-log--close-stream)
      (setq romacs-agent-log--stream (romacs-agent-log-stream--make :kind kind)))))

(defun romacs-agent-log--stream-open (stream)
  "Give STREAM its line at the end, a blank line apart from thinking or
work that the agent's words follow."
  (when (and (eq (romacs-agent-log-stream-kind stream) 'text)
             (memq romacs-agent-log--root-kind '(thinking tool))
             (not (romacs-agent-log--blank-p)))
    (romacs-agent-log--put "\n"))
  ;; Its text goes before its line's newline: the separator is never on it.
  (let ((start (copy-marker romacs-agent-log--end)))
    (romacs-agent-log--put "\n")
    (setf (romacs-agent-log-stream-start stream) start
          (romacs-agent-log-stream-at stream) (copy-marker start t)))
  (setq romacs-agent-log--root-kind (romacs-agent-log-stream-kind stream)))

(defun romacs-agent-log--stream-root (kind text)
  "Stream TEXT of KIND at the transcript's end."
  (pcase-let* ((`(,lead ,body ,trail) (romacs-agent-log--stream-split text))
               (stream (romacs-agent-log--stream-of kind))
               (held (+ (romacs-agent-log-stream-held stream) lead)))
    (if (string-empty-p body)
        (setf (romacs-agent-log-stream-held stream) held)
      (let* ((first (not (romacs-agent-log-stream-start stream)))
             (chunk (romacs-agent-log--stream-chunk kind first held body)))
        (when first (romacs-agent-log--stream-open stream))
        (romacs-agent-log--writing
          (save-excursion (goto-char (romacs-agent-log-stream-at stream))
                          (insert (romacs-agent-log--sealed chunk))))
        (when (eq kind 'text) (romacs-agent-log-faces-soon))
        (setf (romacs-agent-log-stream-held stream) trail)))))

(defun romacs-agent-log--stream-under (block kind text depth)
  "Stream TEXT of KIND under BLOCK's call, DEPTH deep, in whole lines,
the rest held for what follows."
  (let ((same (eq (romacs-agent-log-call-kind block) kind)))
    (unless same (romacs-agent-log--flush block))
    (pcase-let ((`(,lines . ,rest) (romacs-agent-log--whole-lines
                                    (concat (if same (romacs-agent-log-call-held block) "") text))))
      (setf (romacs-agent-log-call-kind block) kind
            (romacs-agent-log-call-held block) rest)
      (when lines (romacs-agent-log--under block kind lines depth)))))

(defun romacs-agent-log--stream (event kind text)
  "Stream TEXT, KIND `text' or `thinking', at the end, or under EVENT's
parent call."
  (pcase-let ((`(,block . ,depth) (romacs-agent-log--where event)))
    (if block (romacs-agent-log--stream-under block kind text depth)
      (romacs-agent-log--stream-root kind text))))

(defun romacs-agent-log--under (block kind text depth)
  "Put TEXT, whole lines of KIND, under BLOCK, DEPTH deep."
  (unless (string-blank-p text)
    (romacs-agent-log--put (romacs-agent-log--indent
                            (concat (propertize (replace-regexp-in-string "\n\n\n+" "\n\n" (string-trim text "\n+" "[ \t\n]+"))
                                                'face (and (eq kind 'thinking) 'romacs-agent-log-thinking))
                                    "\n")
                            depth)
                           block)))

(defun romacs-agent-log--flush (block)
  "Put what BLOCK holds of a subagent's text under it."
  (let ((held (romacs-agent-log-call-held block)))
    (unless (string-empty-p held)
      (setf (romacs-agent-log-call-held block) "")
      (romacs-agent-log--under block (romacs-agent-log-call-kind block) held (1+ (romacs-agent-log-call-depth block))))))

(defun romacs-agent-log--lines (text limit)
  "TEXT's first LIMIT lines, each cut to
`romacs-agent-log-output-width' columns, and how many it has:
\(SHOWN . COUNT)."
  (pcase-let ((`(,lines . ,count)
               ;; A character is a column at least: read no more of a line
               ;; than its width needs, and one more for the ellipsis.
               (romacs-agent-rows-glimpse (or text "") limit (1+ romacs-agent-log-output-width) " \t\n\r")))
    (cons (mapconcat (lambda (line) (truncate-string-to-width line romacs-agent-log-output-width nil nil (truncate-string-ellipsis)))
                     lines "\n")
          count)))

(defun romacs-agent-log--more (count limit whole)
  "What a preview of LIMIT lines of WHOLE leaves out, in the note face:
how many more lines of its COUNT, or its size when COUNT is nil (past
what the chat reads), and the key that shows all; nil when nothing is
left out."
  (when (or (not count) (> count limit))
    (propertize (concat (if count
                            (let ((n (- count limit)))
                              (format "… %d more line%s" n (if (= n 1) "" "s")))
                          (format "… %s in all" (romacs-agent-rows-size whole)))
                        (format " · %s shows all" (romacs-agent-log--key 'romacs-agent-log-open)))
                'face 'romacs-agent-log-note)))

(defconst romacs-agent-log--tool-indent "  "
  "Indent of a tool row within its turn, after any subagent bars.")

(defun romacs-agent-log--output (output face full)
  "OUTPUT under its call, its mark first, its first lines in FACE, then how
many more there are.  FULL, (INPUT . OUTPUT), is what RET on it shows."
  (pcase-let ((`(,shown . ,count) (romacs-agent-log--lines output romacs-agent-log-output-lines)))
    ;; OUTPUT may be part of what the call answered: past what the chat
    ;; reads, the answer's size, never the part's count.
    (unless (romacs-agent-rows-readable-p (cdr full)) (setq count nil))
    (let* ((more (romacs-agent-log--more count romacs-agent-log-output-lines (cdr full)))
           (prefix (concat romacs-agent-log--tool-indent "  " (romacs-agent-glyph 'output) " "))
           (hang (concat "\n" (romacs-agent-glyph-hang prefix)))
           (shown (propertize (replace-regexp-in-string "\n" hang (if (string-empty-p shown) "(no output)" shown) t t)
                              'face face))
           (shown (if (eq face 'romacs-agent-log-output) (romacs-agent-log-faces-output shown) shown))
           (text (concat (propertize prefix 'face 'romacs-agent-log-rule)
                         shown
                         (if more (concat hang more) "")
                         "\n")))
      (propertize text 'romacs-agent-log-full full))))

(defun romacs-agent-log--json (value)
  "VALUE, a tool's input, as indented JSON text."
  (condition-case nil
      (with-temp-buffer
        (insert (json-serialize value :null-object nil :false-object :false))
        (json-pretty-print-buffer)
        (buffer-string))
    (error (format "%S" value))))

(defun romacs-agent-log--call-row (name input &optional failed)
  "The row of a call of tool NAME with INPUT, a table, as it starts: its
renderer's, else its name and a brief.  FAILED leaves the fallback mark off."
  (let* ((input (if (hash-table-p input) input (make-hash-table :test #'equal)))
         (renderer (romacs-agent-rows-get name))
         (row (and renderer
                   (condition-case err (funcall (plist-get renderer :row) input)
                     (error (message "The row of %s failed: %s" name (error-message-string err)) nil)))))
    (or row (concat (unless failed (concat (romacs-agent-glyph 'tool) " ")) name
                    ;; One line of it, its own description when it gives
                    ;; one: RET shows all of it.
                    (let* ((described (gethash "description" input))
                           (brief (truncate-string-to-width
                                   (car (split-string (if (stringp described) described (romacs-agent-rows--brief input)) "\n"))
                                   48 nil nil (truncate-string-ellipsis))))
                      (cond ((string-empty-p brief) "")
                            ((stringp described) (concat " " brief))
                            (t (concat "(" brief ")"))))))))

(defconst romacs-agent-log--row-chars 240
  "The most characters a call's row holds, whatever its renderer says:
RET shows the rest.")

(defun romacs-agent-log--one-line (text limit)
  "TEXT on one line, cut to LIMIT characters.
Read only a bounded prefix.  RET shows the whole text."
  (and text
       (let* ((read (min (length text) (* 2 limit)))
              (row (replace-regexp-in-string "[ \t]*[\n\r]+[ \t]*" " " (substring text 0 read) t t))
              (ellipsis (truncate-string-ellipsis)))
         (if (or (< read (length text)) (> (length row) limit))
             (concat (substring row 0 (min (length row) (- limit (length ellipsis)))) ellipsis)
           row))))

(defun romacs-agent-log--style-tool-row (row &optional failed)
  "Dim ROW while keeping the renderer's outcome faces.
Mark FAILED calls in the error face."
  (let ((text (romacs-agent-log--one-line
               (if failed (concat (romacs-agent-glyph 'denied) " " row) row)
               (- romacs-agent-log--row-chars (length romacs-agent-log--tool-indent)))))
    (add-face-text-property 0 (length text)
                            (if failed 'romacs-agent-log-tool-failed 'romacs-agent-log-tool)
                            (not failed) text)
    text))

(defun romacs-agent-log--call (name input)
  "The first line of tool NAME with INPUT, indented within the turn."
  (concat romacs-agent-log--tool-indent
          (romacs-agent-log--style-tool-row (romacs-agent-log--call-row name input))))

(defun romacs-agent-log--replace-tool-row (block row &optional failed)
  "Replace BLOCK's row text with ROW, styling it for FAILED."
  (save-excursion
    (let* ((start (romacs-agent-log-call-start block))
           (at (+ start (romacs-agent-log-call-indent block)))
           (mark (romacs-agent-log-call-mark block))
           (prefix (if mark (concat mark " ") romacs-agent-log--tool-indent))
           (body (+ at (length prefix)))
           (end (progn (goto-char at) (line-end-position))))
      (when (and (<= body end) (equal (buffer-substring-no-properties at body) prefix))
        (let ((text (romacs-agent-log--style-tool-row row failed))
              (properties (list 'romacs-agent-log-seq (get-text-property at 'romacs-agent-log-seq)
                                'romacs-agent-log-full (get-text-property at 'romacs-agent-log-full)
                                'romacs-agent-log-tool (get-text-property at 'romacs-agent-log-tool))))
          (romacs-agent-log--writing
            (goto-char body)
            (delete-region body end)
            (insert (romacs-agent-log--sealed (apply #'propertize text properties)))))))))

(defun romacs-agent-log--ended-row (block output)
  "BLOCK's row, its call ended well with OUTPUT, rewritten to say what came
of it (`romacs-agent-rows-ended'), after its mark."
  (let* ((name (romacs-agent-log-call-name block))
         (input (let ((input (romacs-agent-log-call-input block))) (if (hash-table-p input) input (make-hash-table :test #'equal))))
         (ended (condition-case err (romacs-agent-rows-ended name input (romacs-agent-log--call-row name input) output)
                  (error (message "The row of %s failed: %s" name (error-message-string err)) nil))))
    (when ended (romacs-agent-log--replace-tool-row block ended))))

(defun romacs-agent-log--failure (output)
  "A failed call's OUTPUT as its error says it: the harness's tags gone."
  (string-trim (replace-regexp-in-string "</?tool_use_error>" "" (or output ""))))

(defun romacs-agent-log--failed-text (output full)
  "What goes under a call that failed with OUTPUT: why, RET showing FULL.
Anything else, a command's output or a subagent's report, is RET away:
a call that ended well is its row (while a command runs, its last lines
show)."
  (romacs-agent-log--output (romacs-agent-log--failure (romacs-agent-rows-clip output)) 'romacs-agent-log-denied full))

(defun romacs-agent-log--live (block text)
  "BLOCK's output streamed so far grows by TEXT: its last lines show under
its call, in place of those shown before."
  (let* ((streamed (concat (romacs-agent-log-call-streamed block) text))
         (streamed (substring streamed (max 0 (- (length streamed) 65536))))
         (count (+ (romacs-agent-log-call-count block) (cl-count ?\n text)))
         ;; Lines whole and the one under way.
         (total (+ count (if (string-suffix-p "\n" streamed) 0 1)))
         (shown (last (let ((lines (split-string streamed "\n")))
                        ;; The empty piece after a final newline is no line.
                        (if (string-suffix-p "\n" streamed) (butlast lines) lines))
                      romacs-agent-log-output-lines))
         (prefix (concat romacs-agent-log--tool-indent "  " (romacs-agent-glyph 'running) " "))
         (hang (romacs-agent-glyph-hang prefix))
         (under (romacs-agent-log-call-under block))
         (live (or (romacs-agent-log-call-live block) (copy-marker under))))
    (setf (romacs-agent-log-call-streamed block) streamed
          (romacs-agent-log-call-live block) live
          (romacs-agent-log-call-count block) count)
    (romacs-agent-log--writing (delete-region live under))
    (romacs-agent-log--put (romacs-agent-log--indent
                            (concat (propertize (concat prefix
                                                        (if (> total (length shown))
                                                            (format "running: %d lines so far, the last %d\n" total (length shown))
                                                          (format "running: %d line%s so far\n" total (if (= total 1) "" "s"))))
                                                'face 'romacs-agent-log-note)
                                    (propertize (concat hang (string-join shown (concat "\n" hang)) "\n") 'face 'romacs-agent-log-output))
                            (romacs-agent-log-call-depth block))
                           block)))

(defun romacs-agent-log--tool (event)
  "Draw tool EVENT: its call as it starts, its output under it as it ends."
  (pcase (plist-get event :state)
    ('started (romacs-agent-log--tool-started event))
    ((or 'completed 'failed)
     (let ((block (romacs-agent-log--block (plist-get event :id))))
       (when (equal (plist-get event :id) romacs-agent-log--current) (setq romacs-agent-log--current nil))
       (if (and block (not (romacs-agent-log-call-ended block)))
           (romacs-agent-log--tool-ended block event)
         (romacs-agent-log--tool-unseen event))))))

(defun romacs-agent-log--tool-started (event)
  "Draw EVENT's call as it starts, open for what comes under it."
  (pcase-let ((`(,parent . ,depth) (romacs-agent-log--where event)))
    (if parent (romacs-agent-log--flush parent) (romacs-agent-log--close-stream))
    (let* ((id (plist-get event :id))
           (input (plist-get event :input))
           (name (romacs-agent-rows-name (plist-get event :name)))
           (place (lambda () (copy-marker (if parent (romacs-agent-log-call-under parent) romacs-agent-log--end))))
           (start (funcall place)))
      (romacs-agent-log--put (romacs-agent-log--indent
                              (propertize (romacs-agent-log--entry (romacs-agent-log--call name input) 'tool event)
                                          'romacs-agent-log-full (list input) 'romacs-agent-log-tool name)
                              depth)
                             parent)
      ;; Its row in place: what an earlier call puts after it, at the
      ;; same place, goes before it.
      (set-marker-insertion-type start t)
      (when (zerop depth) (setq romacs-agent-log--current id))
      (unless romacs-agent-log--blocks (setq romacs-agent-log--blocks (make-hash-table :test #'equal)))
      (puthash id (romacs-agent-log-call--make :id id :name name :input input :depth depth
                                               :parent (and parent (romacs-agent-log-call-id parent))
                                               :start start :under (funcall place)
                                               :indent (1- (length (romacs-agent-log--indent "x" depth))))
               romacs-agent-log--blocks)
      (when-let* ((early (alist-get id romacs-agent-log--early nil nil #'equal)))
        (setf (alist-get id romacs-agent-log--early nil t #'equal) nil)
        (romacs-agent-log--live (romacs-agent-log--block id) early)))))

(defun romacs-agent-log--goes-on-p (block)
  "Whether BLOCK's call stays open as it ends: work its harness runs under
it (a subagent, a command in the background) goes on."
  (and (or (member (romacs-agent-log-call-name block) '("Agent" "Task")) (romacs-agent-log-call-task block))
       (not (romacs-agent-log-call-task-ended block))))

(defun romacs-agent-log--tool-ended (block event)
  "Draw BLOCK's call ended, as EVENT says: its row says what came of it,
RET on it shows all."
  (let ((output (or (plist-get event :output) ""))
        (failed (eq (plist-get event :state) 'failed))
        (start (romacs-agent-log-call-start block)))
    (romacs-agent-log--flush block)
    ;; What streamed as it ran gives way to the whole output.
    (when-let* ((live (romacs-agent-log-call-live block)))
      (romacs-agent-log--writing (delete-region live (romacs-agent-log-call-under block)))
      (set-marker live nil))
    ;; Some calls fill their input in only as they end.
    (when-let* ((input (plist-get event :input))) (setf (romacs-agent-log-call-input block) input))
    (let ((full (cons (romacs-agent-log-call-input block) output)))
      (romacs-agent-log--writing
        (put-text-property start (save-excursion (goto-char start) (line-end-position)) 'romacs-agent-log-full full))
      (if failed
          (progn
            (romacs-agent-log--replace-tool-row
             block (romacs-agent-log--call-row (romacs-agent-log-call-name block) (car full) t) t)
            (romacs-agent-log--put (romacs-agent-log--indent (romacs-agent-log--failed-text output full)
                                                             (romacs-agent-log-call-depth block))
                                   block))
        (romacs-agent-log--ended-row block output)))
    (if (romacs-agent-log--goes-on-p block)
        (setf (romacs-agent-log-call-ended block) t)
      (romacs-agent-log--close-block block))))

(defun romacs-agent-log--unseen-row (name output failed)
  "The row of a call of NAME never drawn, ended with OUTPUT, FAILED or not."
  (let ((row (if failed (format "%s failed" name)
               (or (plist-get (romacs-agent-rows-texts name nil output) :ended)
                   (concat (romacs-agent-glyph 'tool) " " name " ended" (romacs-agent-rows-outcome output))))))
    (concat romacs-agent-log--tool-indent
            (romacs-agent-log--style-tool-row row failed))))

(defun romacs-agent-log--tool-unseen (event)
  "Draw EVENT's call, ended but never drawn (it ended after the turn, or
before this chat's first record): at the end, with its output."
  (romacs-agent-log--close-stream)
  (let* ((output (or (plist-get event :output) ""))
         (failed (eq (plist-get event :state) 'failed))
         (full (cons nil output)))
    (romacs-agent-log--put
     (propertize (concat (romacs-agent-log--entry
                          (romacs-agent-log--unseen-row (romacs-agent-rows-name (plist-get event :name)) output failed)
                          'tool event)
                         (if failed (romacs-agent-log--failed-text output full) ""))
                 'romacs-agent-log-full full))))

(defun romacs-agent-log--task-block (event)
  "The call EVENT, a task's, is under: by its tool's id, else its task's."
  (or (romacs-agent-log--block (plist-get event :id))
      (when-let* ((task (plist-get event :task)) (blocks romacs-agent-log--blocks))
        (catch 'found
          (maphash (lambda (_ block) (when (equal (romacs-agent-log-call-task block) task) (throw 'found block))) blocks)
          nil))))

(defun romacs-agent-log--task-text (event block subagent)
  "What task EVENT says in the transcript under BLOCK, its call, if any.
SUBAGENT progress belongs to the live view."
  (let ((state (plist-get event :state)))
    (pcase state
      ('backgrounded "moved to the background")
      ('progress (let ((activity (plist-get event :activity)))
                   (and activity (not subagent)
                        (not (and block (equal activity (romacs-agent-log-call-activity block))))
                        (format "now: %s" activity))))
      ;; Its summary, but a subagent's: its call's output, drawn, is its report.
      ((or 'completed 'failed 'stopped)
       (unless (and subagent (not block) (not (plist-get event :summary)))
         (concat (symbol-name state)
                 (when-let* ((summary (and (not (and subagent block (romacs-agent-log-call-ended block)))
                                           (plist-get event :summary))))
                   (concat ": " summary))))))))

(defun romacs-agent-log--task (session event)
  "Draw SESSION's task EVENT, what work under a call says
of itself, under that call; with its call gone, at the end.  Its work
over, a call already ended closes; one still running closes as it ends."
  (let* ((block (romacs-agent-log--task-block event))
         (state (plist-get event :state))
         (subagent (or (plist-get event :subagent)
                       (and block (member (romacs-agent-log-call-name block) '("Agent" "Task")))
                       (plist-get (romacs-agent-ui-worker session (plist-get event :id)) :subagent)))
         (text (romacs-agent-log--task-text event block subagent)))
    (when block
      (when (and text (eq state 'progress)) (setf (romacs-agent-log-call-activity block) (plist-get event :activity)))
      (when (and (eq state 'started) (plist-get event :task)) (setf (romacs-agent-log-call-task block) (plist-get event :task))))
    (cond ((not text))
          ((not block) (romacs-agent-log--note event (concat (if subagent "subagent " "task ") text)))
          (t (romacs-agent-log--flush block)
             (romacs-agent-log--put (romacs-agent-log--indent (propertize (concat "· " text "\n") 'face 'romacs-agent-log-note)
                                                              (1+ (romacs-agent-log-call-depth block)))
                                    block)))
    (when (and block (memq state '(completed failed stopped)))
      (if (romacs-agent-log-call-ended block)
          (romacs-agent-log--close-block block)
        (setf (romacs-agent-log-call-task-ended block) t)))))

(defun romacs-agent-log--key (command &optional map)
  "COMMAND's key in MAP (the transcript's), as `substitute-command-keys' says it."
  (substitute-command-keys (format "\\<%s>\\[%s]" (or map 'romacs-agent-log-transcript-map) command)))

(defun romacs-agent-log--keys (&rest pairs)
  "PAIRS of COMMAND and WORD as the keys an approval offers, each the
key its block binds the command to."
  (mapconcat (lambda (pair) (concat (romacs-agent-log--key (car pair) 'romacs-agent-log-ask-map) " " (cadr pair)))
             (seq-partition pairs 2) "  "))

(defun romacs-agent-log--ask (session event)
  "Draw approval EVENT: a block at the end that waits on the user, unless
Romacs answers it by its policy, which draws that answer and who gave it.
Live, in a chat no window shows, the echo area says so too."
  (unless (romacs-agent-log-source-answers-p romacs-agent-log--source session event)
    (romacs-agent-log--ask-block event)
    (when (and (romacs-agent--live-p session) (not (get-buffer-window (current-buffer) 'visible)))
      (message "%s %s waits on you: %s"
               (propertize (romacs-agent-glyph 'asks) 'face 'romacs-agent-log-ask) (romacs-agent-title session)
               (substitute-command-keys "\\[romacs-agents] shows it")))))

(defun romacs-agent-log--ask-head (event)
  "What EVENT, a request, asks of the user, in a few words."
  (let ((who (if (plist-get event :parent) "A subagent" "The agent")))
    (cond ((plist-get event :questions) (format "%s asks you:" who))
          ((plist-get event :plan) (format "%s presents its plan: proceed?" who))
          (t (format "%s wants to run %s:" who (romacs-agent-rows-name (plist-get event :tool)))))))

(defun romacs-agent-log--ask-body (event)
  "What EVENT, a request, asks about: its questions and their options, a
call's input, or nil for a plan, shown above it."
  (let ((questions (plist-get event :questions)))
    (cond (questions
           (mapconcat (lambda (q)
                        (concat "  " (car q)
                                (when-let* ((options (plist-get (cdr q) :options)))
                                  (concat "\n    " (string-join options " · ")))))
                      questions "\n"))
          ((plist-get event :plan) nil)
          (t (let ((json (romacs-agent-log--json (plist-get event :input))))
               (pcase-let ((`(,shown . ,count) (romacs-agent-log--lines json 30)))
                 (if-let* ((more (romacs-agent-log--more count 30 json)))
                     (concat shown "\n" more)
                   shown)))))))

(defun romacs-agent-log--ask-keys (event)
  "The keys that answer EVENT, a request, and what each does."
  (cond ((plist-get event :questions)
         (romacs-agent-log--keys 'romacs-agent-log-allow "choose" 'romacs-agent-log-deny "decline"))
        ((plist-get event :plan)
         (romacs-agent-log--keys 'romacs-agent-log-allow "proceed" 'romacs-agent-log-deny "keep planning, saying why"))
        (t (romacs-agent-log--keys 'romacs-agent-log-allow "allow" 'romacs-agent-log-allow-always "allow for this session"
                                   'romacs-agent-log-deny "deny"))))

(defun romacs-agent-log--ask-text (event)
  "The approval block of EVENT, a request: heading, why, what it asks, keys."
  (let ((why (plist-get event :description))
        (body (romacs-agent-log--ask-body event)))
    (concat (propertize (concat (romacs-agent-glyph 'asks) " " (romacs-agent-log--ask-head event)) 'face 'romacs-agent-log-ask)
            (if why (concat "\n  " (propertize why 'face 'romacs-agent-log-note)) "")
            ;; Its question and choices in the text's own face: the
            ;; decision reads first, after the heading.
            (if body (concat "\n" (propertize (replace-regexp-in-string "^" "  " body)
                                               'face (if (plist-get event :questions) 'default 'romacs-agent-log-output)))
              "")
            "\n" (propertize (concat "  " (romacs-agent-log--ask-keys event) "\n") 'romacs-agent-log-keys t))))

(defun romacs-agent-log--ask-block (event)
  "Draw EVENT, a request, as a block its keys answer."
  (romacs-agent-log--close-stream)
  (let ((id (plist-get event :request-id))
        (start (copy-marker romacs-agent-log--end)))
    (romacs-agent-log--put (propertize (romacs-agent-log--entry (romacs-agent-log--ask-text event) 'ask event)
                                       'romacs-agent-log-request id
                                       'romacs-agent-log-full (list (plist-get event :input))
                                       'romacs-agent-log-questions (plist-get event :questions)
                                       'romacs-agent-log-plan (plist-get event :plan)
                                       'keymap romacs-agent-log-ask-map))
    (push (romacs-agent-log-ask--make :id id :start start :end (copy-marker romacs-agent-log--end))
          romacs-agent-log--asks)))

(defun romacs-agent-log--settle-ask (id)
  "The approval block of request ID answered: no longer a warning."
  (when-let* ((ask (romacs-agent-log--ask-of id)))
    (let ((start (romacs-agent-log-ask-start ask)) (end (romacs-agent-log-ask-end ask)))
      (romacs-agent-log--writing
        ;; Its keys answer nothing any more: their row goes.
        (when-let* ((keys (text-property-any start end 'romacs-agent-log-keys t)))
          (delete-region keys (or (next-single-property-change keys 'romacs-agent-log-keys nil end) end)))
        (add-face-text-property start end 'romacs-agent-log-note)
        ;; It answers nothing any more: no keys, no request to answer.
        (remove-text-properties start end '(keymap nil romacs-agent-log-request nil))
        (put-text-property start end 'keymap (or romacs-agent-log--text-map romacs-agent-log-transcript-map))))
    (setq romacs-agent-log--asks (delq ask romacs-agent-log--asks))))

(defun romacs-agent-log--close-asks (session event how)
  "SESSION's turn ended or its harness exited (HOW, `turn-end' or `exit',
at EVENT) with approvals drawn open: each it no longer waits on (then,
in a replay) settled, and said why.  One answered was not delivered: a
recorded answer settles its block."
  (let ((approvals (if romacs-agent-log--replaying (romacs-agent-approvals romacs-agent-log--replaying)
                     (romacs-agent-log-source-approvals romacs-agent-log--source session))))
    (dolist (ask romacs-agent-log--asks)
      (let* ((id (romacs-agent-log-ask-id ask))
             (why (cond ((member id romacs-agent-log--attributed) "an answer not delivered: the harness never took it")
                        ((eq how 'exit) "the harness exited with it unanswered")
                        (t "the turn ended with it unanswered"))))
        (cond ((and (not (gethash id approvals)) (not romacs-agent-log--closes-deferred))
               (romacs-agent-log--settle-ask id)
               (romacs-agent-log--note event why)
               (romacs-agent-log--drop-ended id))
              ;; Still listed by a source that says so later, or drawn
              ;; from a source's history before what it waits on now:
              ;; said, where it first ended, once it lets it go, unless
              ;; an answer settles it.  A later turn's end is not its.
              ((and (not romacs-agent-log--replaying) (not (romacs-agent-log--owed id)))
               (romacs-agent-log--close-stream)
               (push (romacs-agent-log-owed--make
                      :id id :entry (romacs-agent-log--entry (propertize (concat "· " why) 'face 'romacs-agent-log-note) 'note event)
                      :at (copy-marker romacs-agent-log--end))
                     romacs-agent-log--ended-asks)))))))

(defun romacs-agent-log--answers-text (by answers)
  "What BY answered to questions, ANSWERS a table of question to answer:
one question, its answer; more, each question with its own."
  (concat (pcase by ("user" "answered: ") ("policy" "answered by Romacs: ")
                 ("harness" "answered by the harness: ") (_ "answered: "))
          (let ((one (= (hash-table-count answers) 1)))
            (mapconcat (lambda (question)
                         (if one (format "%s" (gethash question answers))
                           (format "%s → %s" question (gethash question answers))))
                       (hash-table-keys answers) " · "))))

(defun romacs-agent-log--decision-text (by allowed)
  "Who decided, BY, and whether ALLOWED."
  (pcase by
    ("user" (if allowed "allowed by you" "denied by you"))
    ("policy" (if allowed "allowed by Romacs (romacs-agent-approval-policy)" "denied by Romacs"))
    ("harness" (if allowed "allowed by the harness" "denied by the harness"))
    (_ (if allowed "allowed" "denied"))))

(defun romacs-agent-log--answer-text (actor decision scope reason &optional answers)
  "Who answered, and how: ACTOR a plist of :kind (\"user\", \"policy\",
\"harness\" or \"unknown\"), :login and :client; DECISION allow or deny;
ANSWERS, a table of question to answer, what was answered: a question's
answer says what was chosen, never that a tool was allowed.  An answer
from a client other than this editor says which."
  (let* ((by (plist-get actor :kind))
         (client (plist-get actor :client))
         (via (if (and client (not (equal client "editor"))) (format " (via %s)" client) ""))
         (allowed (equal decision "allow")))
    (if (and (hash-table-p answers) allowed)
        (concat (propertize (concat (romacs-agent-glyph 'allowed) " ") 'face 'romacs-agent-log-allowed) (romacs-agent-log--answers-text by answers) via)
      (concat (propertize (concat (romacs-agent-glyph (if allowed 'allowed 'denied)) " ") 'face (if allowed 'romacs-agent-log-allowed 'romacs-agent-log-denied))
              (romacs-agent-log--decision-text by allowed)
              via
              (if (equal scope "session") " for this session" "")
              (if (and reason (not (string-empty-p reason))) (concat ": " reason) "")))))

(defun romacs-agent-log--answered (event)
  "Draw answer EVENT, from the harness's records: its block settled, and,
unless a note said who gave it, that it was answered."
  (let ((id (plist-get event :request-id)))
    (romacs-agent-log--settle-ask id)
    ;; Settled: no longer left to say its turn ended with it open.
    (romacs-agent-log--drop-ended id)
    ;; Settled by the harness over an answer it never took.
    (when (and (eq (plist-get event :how) 'harness) (member id romacs-agent-log--attributed))
      (romacs-agent-log--note event "an answer not delivered: the harness settled it first"))
    (unless (member id romacs-agent-log--attributed)
      (romacs-agent-log--line event 'answer
                              (pcase (plist-get event :behavior)
                                ("allow" (concat (propertize (concat (romacs-agent-glyph 'allowed) " ") 'face 'romacs-agent-log-allowed) "allowed (who: not recorded)"))
                                ("deny" (concat (propertize (concat (romacs-agent-glyph 'denied) " ") 'face 'romacs-agent-log-denied) "denied (who: not recorded)"))
                                (_ (propertize (if (eq (plist-get event :how) 'harness)
                                                   "· settled by the harness: no longer asked"
                                                 "· settled: the harness no longer asks")
                                               'face 'romacs-agent-log-note)))))))

(defun romacs-agent-log--incomings (event)
  "What input EVENT brought from elsewhere, as plists of :from :state
:text (`romacs-agent-log-rounds-incoming'), or nil for the user's own
words: the messages Romacs delivered, else what its text says it is."
  (if-let* ((messages (plist-get event :messages)))
      ;; Each as Romacs delivered it: whom from, a wake's state or the
      ;; message's kind, its own body.
      (mapcar (lambda (m) (list :from (or (plist-get (plist-get m :from) :name) "a session")
                                :state (or (plist-get (plist-get m :wake) :state) (plist-get m :kind))
                                :text (or (plist-get m :text) (plist-get event :text))))
              messages)
    (romacs-agent-incoming-of (plist-get event :text))))

(defun romacs-agent-log--prompt (event)
  "Draw input EVENT: the user's message, opening a round, or what came
from elsewhere, opening one of its own."
  (if-let* ((incomings (romacs-agent-log--incomings event)))
      (dolist (incoming incomings) (romacs-agent-log-rounds-incoming event incoming))
    (romacs-agent-log--prompt-own event)))

(defun romacs-agent-log--prompt-own (event)
  "Draw input EVENT, the user's own message, the images it links under it."
  (let* ((text (or (plist-get event :text) ""))
         (images (plist-get event :images))
         (_ (push (string-trim text) romacs-agent-log--prompts))
         (shown (romacs-agent-log-images-message (propertize (string-trim text) 'face 'romacs-agent-log-prompt)))
         (head (cond ((plist-get event :steer) (propertize (concat (romacs-agent-glyph 'steer) " ") 'face 'romacs-agent-log-prompt))
                     (t (propertize (concat (romacs-agent-glyph 'prompt) " ") 'face 'romacs-agent-log-prompt-mark)))))
    (romacs-agent-log--line event 'prompt
                            (let ((line (concat (if (romacs-agent-log--blank-p) "" "\n")
                                                head
                                                (car shown)
                                                (if images (propertize (format "  [%d image%s]" (length images) (if (cdr images) "s" ""))
                                                                       'face 'romacs-agent-log-note)
                                                  "")
                                                (if (cdr shown) (concat "\n" (cdr shown)) ""))))
                              ;; The turn it starts, by its request, on all of it
                              ;; (where prompt motion lands too): a diff or a
                              ;; rollback of the turn at point reads it.
                              (let ((line (romacs-agent-log-rounds-you line)))
                                (if-let* ((request (plist-get event :request)))
                                    (propertize line 'romacs-agent-log-turn request)
                                  line))))))

(defun romacs-agent-log--done (event)
  "Draw turn end EVENT: a rule with how long it took, when it ended and
why it failed; what it cost RET on it shows, never the rule."
  (romacs-agent-log--line
   event 'turn
   (let ((cost (plist-get event :cost)))
     (concat (when-let* ((reason (or (plist-get event :reason) (and (plist-get event :error) (plist-get event :text)))))
               (concat (propertize (concat (romacs-agent-glyph 'denied) " " reason) 'face 'romacs-agent-log-denied) "\n"))
             (propertize (concat (romacs-agent-glyph 'turn-end) " " (string-join (delq nil (list
                                                      (and (plist-get event :interrupted) "interrupted")
                                                      (when-let* ((s (plist-get event :duration))) (format "%ds" (round s)))
                                                      ;; A transcript's turns may not say when they ended.
                                                      (when-let* ((time (plist-get event :time))) (format-time-string "%H:%M" time))))
                                      " · "))
                         'face 'romacs-agent-log-rule
                         'romacs-agent-log-full (and cost (cons nil (format "This turn cost $%.2f." cost))))))))

(defun romacs-agent-log--note (event text)
  (romacs-agent-log--line event 'note (concat "· " text) 'romacs-agent-log-note))

(defun romacs-agent-log--offers-proceed-p (session event)
  "Whether plan EVENT of SESSION is offered to proceed with by leaving plan
mode: its driver marks it :proceed (its plan is no request), SESSION is
in plan mode, and the plan is live, not replayed."
  (and (plist-get event :proceed) (plist-get (romacs-agent-options session) :plan)
       (not romacs-agent-log--replaying)))

(defun romacs-agent-log--retire-proceed ()
  "The plan offered to proceed with is no more: its text offers nothing."
  (save-restriction
    (widen)
    (when-let* ((at (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-proceed t)))
      (let ((end (or (next-single-property-change at 'romacs-agent-log-proceed nil romacs-agent-log--end)
                     romacs-agent-log--end)))
        (romacs-agent-log--writing
          (remove-text-properties at end '(romacs-agent-log-proceed nil))
          (put-text-property at end 'keymap (or romacs-agent-log--text-map romacs-agent-log-transcript-map)))))))

(defun romacs-agent-log--proceed-offered ()
  "Where the chat offers to proceed with a plan by leaving plan mode, or
nil; however the chat is narrowed."
  (when romacs-agent-log--end
    (save-restriction
      (widen)
      (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-proceed t))))

(defun romacs-agent-log-proceed ()
  "Proceed with the plan the chat offers: its session leaves plan mode,
back in the mode it had, and the next message carries it out."
  (interactive nil romacs-agent-log-mode)
  (unless (romacs-agent-log--proceed-offered) (user-error "No plan to proceed with"))
  (romacs-agent-set-plan (romacs-agent-menu--session) nil)
  ;; Switched: its row offers nothing more, nor the separator.
  (romacs-agent-log--retire-proceed)
  (save-restriction (widen) (romacs-agent-log--separator))
  (romacs-agent-log--header-show))

(defun romacs-agent-log--tool-output (event)
  "Draw EVENT, output a call streams as it runs, under its call; before
its call (Codex's first line can be), held for it, its last 64K, until
its turn ends."
  (when-let* ((text (plist-get event :text)))
    (if-let* ((block (romacs-agent-log--block (plist-get event :id))))
        (romacs-agent-log--live block text)
      (let* ((id (plist-get event :id))
             (held (concat (alist-get id romacs-agent-log--early nil nil #'equal) text)))
        (setf (alist-get id romacs-agent-log--early nil nil #'equal)
              (substring held (max 0 (- (length held) 65536))))))))

(defun romacs-agent-log--attributed (event)
  "Draw EVENT, who answered a request: said, its block settled once its
answer is written."
  (push (plist-get event :request-id) romacs-agent-log--attributed)
  (romacs-agent-log--line event 'answer
                          ;; A service older than :actor says who alone.
                          (romacs-agent-log--answer-text (or (plist-get event :actor)
                                                             (list :kind (or (plist-get event :by) "unknown")))
                                                         (plist-get event :decision)
                                                         (plist-get event :scope) (plist-get event :reason)
                                                         (plist-get event :answers))))

(defun romacs-agent-log--plan (session event)
  "Draw EVENT, SESSION's plan; offered, its text takes y, the separator
says so."
  (let ((text (concat (propertize (concat (romacs-agent-glyph 'plan) " Plan\n") 'face 'romacs-agent-log-prompt) (plist-get event :text))))
    (when (romacs-agent-log--offers-proceed-p session event)
      (romacs-agent-log--retire-proceed)
      (setq text (propertize text 'romacs-agent-log-proceed t 'keymap romacs-agent-log-plan-map)))
    (romacs-agent-log--line event 'plan text)))

(defun romacs-agent-log--compacted (event)
  "Draw EVENT, the context compacted: its summary's first lines, all of it
on RET from anywhere in the entry."
  (let* ((summary (or (plist-get event :text) ""))
         (lines (romacs-agent-log--lines summary romacs-agent-log-output-lines)))
    (romacs-agent-log--line event 'note
                            (propertize
                             (concat "· context compacted\n" (car lines)
                                     (if-let* ((more (romacs-agent-log--more (cdr lines) romacs-agent-log-output-lines summary)))
                                         (concat "\n" more)
                                       ""))
                             'romacs-agent-log-full (cons nil summary))
                            'romacs-agent-log-note)))

(defun romacs-agent-log--exit (session event)
  "Draw EVENT, SESSION's harness exited: a turn it cut is a round ended,
and an exit a per-turn driver's turn did not mean is said."
  (setq romacs-agent-log--current nil romacs-agent-log--early nil)
  (romacs-agent-log--close-asks session event 'exit)
  (romacs-agent-log--close-stream)
  (romacs-agent-log-rounds-end)
  (unless (and (romacs-agent--per-turn-p session) (eql (plist-get event :status) 0))
    (romacs-agent-log--note event (format "harness exited (%s)" (plist-get event :status)))))

(defun romacs-agent-log--selection-label (selection)
  "SELECTION, a model and effort, as a note says it."
  (mapconcat #'identity (or (delq nil (list (plist-get selection :model) (plist-get selection :effort)))
                            '("harness's own"))
             " · "))

(defun romacs-agent-log--unknown (event)
  "Draw EVENT, what Romacs does not read of its harness: named, its whole
on RET."
  (let* ((raw (plist-get event :raw))
         (name (or (plist-get event :method)
                   (and (hash-table-p raw) (stringp (gethash "type" raw)) (gethash "type" raw))
                   (and (stringp raw) (truncate-string-to-width (car (split-string raw "\n")) 100
                                                                nil nil (truncate-string-ellipsis)))
                   "a line Romacs does not read")))
    (romacs-agent-log--line event 'note
                            (propertize (concat "· " name)
                                        'romacs-agent-log-full (cons nil (if (stringp raw) raw (romacs-agent-log--json raw))))
                            'romacs-agent-log-note)))

(defun romacs-agent-log-draw (session event)
  "Draw SESSION's EVENT in the current chat."
  (pcase (plist-get event :type)
    ('input (romacs-agent-log--prompt event))
    ;; A per-turn driver's delivery waiting: its `input' draws it.
    ('incoming nil)
    ('arrival (romacs-agent-log-rounds-incoming event event))
    ('delta (when-let* ((text (plist-get event :text))) (romacs-agent-log--stream event 'text text)))
    ('thinking (when-let* ((text (plist-get event :text))) (romacs-agent-log--stream event 'thinking text)))
    ('message-end (if-let* ((block (romacs-agent-log--block (plist-get event :parent))))
                      (romacs-agent-log--flush block)
                    (romacs-agent-log--close-stream)))
    ('tool (romacs-agent-log--tool event))
    ('task (romacs-agent-log--task session event))
    ('tool-output (romacs-agent-log--tool-output event))
    ('approval (romacs-agent-log--ask session event))
    ;; Who answered: said, the block settled once its answer is written.
    ('attributed (romacs-agent-log--attributed event))
    ('answered (romacs-agent-log--answered event))
    ('plan-switch (unless (plist-get event :on) (romacs-agent-log--retire-proceed)))
    ('plan (romacs-agent-log--plan session event))
    ('done (setq romacs-agent-log--current nil romacs-agent-log--early nil)
           (romacs-agent-log--close-asks session event 'turn-end)
           (romacs-agent-log--done event)
           (romacs-agent-log-rounds-end))
    ('error (romacs-agent-log--line event 'error (concat (romacs-agent-glyph 'denied) " " (or (plist-get event :text) "error")) 'romacs-agent-log-denied))
    ('unstarted (romacs-agent-log--line event 'error (concat (romacs-agent-glyph 'denied) " " (or (plist-get event :text) "never started"))
                                        'romacs-agent-log-denied))
    ;; What a session's state says (the header shows it) is no line.
    ('status (when-let* ((text (plist-get event :text))
                         ((not (member text '("requesting" "selection armed")))))
               (romacs-agent-log--note event text)))
    ('compacted (romacs-agent-log--compacted event))
    ('exit (romacs-agent-log--exit session event))
    ('renewed (romacs-agent-log--note event (format "renewed: a new conversation%s"
                                                    (if-let* ((why (plist-get event :why))) (format " (%s)" why) ""))))
    ('forked (romacs-agent-log--note event (format "forked from %s" (or (plist-get event :title) (plist-get event :from)))))
    ;; A rollback takes the plan offered back with its turns.
    ('rolled-back (romacs-agent-log--retire-proceed)
                  (romacs-agent-log-turns-rolled-back event))
    ('replied (romacs-agent-log--note event (format "replied to %s" (plist-get event :message))))
    ('selection (romacs-agent-log--note event (format "%s → %s"
                                                      (romacs-agent-log--selection-label (plist-get event :from))
                                                      (romacs-agent-log--selection-label (plist-get event :to)))))
    ('unknown (romacs-agent-log--unknown event))
    ;; State, not content: the header and separator say it.
    (_ nil)))

;;; The separator and header

(defun romacs-agent-log--doing (session)
  "What SESSION does now, as the separator says it: (STATE . WORDS).
WORDS is nil when the working thread says it alone: busy or idle."
  (pcase (cond ((romacs-agent-log--pending) 'waiting-approval)
               ;; A plan offered waits on the user as a request does.
               ((and (eq (romacs-agent-state session) 'idle) (not (romacs-agent-failed session))
                     (romacs-agent-log--proceed-offered))
                'plan-offered)
               (t (romacs-agent-state session)))
    ('plan-offered
     (cons 'waiting (substitute-command-keys
                     (concat (romacs-agent-glyph 'asks) " plan ready: \\<romacs-agent-log-mode-map>\\[romacs-agent-log-allow] proceeds, leaving plan mode · type to keep planning"))))
    ('waiting-approval
     (let ((question (when-let* ((ask (car (romacs-agent-log--pending))))
                       (get-text-property (romacs-agent-log-ask-start ask) 'romacs-agent-log-questions))))
       (cons 'waiting (substitute-command-keys
                       (concat (romacs-agent-glyph 'asks) " waits on you: \\<romacs-agent-log-mode-map>\\[romacs-agent-log-allow] "
                               (if question "choose" "allow") " · \\[romacs-agent-log-deny] "
                               (if question "decline" "deny") " · \\[romacs-agent-log-next-ask] goes to it")))))
    ;; Busy or idle, the thread's motion says; what runs, its row does.
    ((or 'starting 'running) (cons 'working nil))
    ;; A turn that failed and is not yet seen: said so, until the next.
    ('idle (if (romacs-agent-failed session) (cons 'failed "failed") (cons 'idle nil)))
    (state (cons 'idle (format "%s" state)))))

(defconst romacs-agent-log--thread-cols 8
  "Characters of the working thread that starts the separator.")

(defun romacs-agent-log--work (doing)
  "The working thread's work for DOING (`romacs-working')."
  (pcase (car doing) ('waiting 'waiting) ('working 'running) ('failed 'failed) (_ 'idle)))

(defun romacs-agent-log--rule-width ()
  "The columns the narrowest window showing the chat leaves its separator:
its last width when none shows it."
  (if-let* ((windows (get-buffer-window-list (current-buffer) nil t)))
      (apply #'min (mapcar #'window-body-width windows))
    (or romacs-agent-log--width 80)))

(defvar romacs-agent-log-separator-rows-function nil
  "A function of no arguments giving the rows the separator shows between
its rule and the composer's prompt, each ending in a newline (what waits
to be sent: romacs-agent-log-queue.el), or nil for none.")

(defun romacs-agent-log--divider-text (doing width)
  "The divider for DOING, (STATE . WORDS), in WIDTH columns."
  (let ((face (pcase (car doing) ('waiting 'romacs-agent-log-ask) ('working 'romacs-working-running)
                ('failed 'romacs-agent-log-denied) (_ 'romacs-agent-log-rule))))
    (let* ((label (if (cdr doing) (concat " " (propertize (cdr doing) 'face (list face 'bold)) " ") " "))
           ;; A dim rule after it fences the composer off the transcript,
           ;; never wrapping; none when that is short.
           (rule (- width 1 romacs-agent-log--thread-cols (string-width label))))
      (concat (romacs-working-still romacs-agent-log--thread-cols (romacs-agent-log--work doing))
              label
              (if (>= rule 8) (propertize (make-string rule ?─) 'face 'romacs-agent-log-rule) "")))))

(defun romacs-agent-log--separator-field (text)
  "TEXT as read-only chrome, including its boundary with the composer."
  (propertize (romacs-agent-log--sealed text) 'field 'romacs-agent-log-separator))

(defun romacs-agent-log--divider-marker ()
  "The divider's marker, also in a chat predating the activity view."
  (or romacs-agent-log--divider
      (setq romacs-agent-log--divider (copy-marker romacs-agent-log--end t))))

(defun romacs-agent-log--write-separator (divider rows)
  "Write DIVIDER and ROWS below the live activity, preserving the composer."
  (let* ((marker (romacs-agent-log--divider-marker))
        (start (marker-position marker))
        (end (marker-position romacs-agent-log--end))
        (old (- romacs-agent-log--input start))
        (prompt (concat rows (propertize (concat (romacs-agent-glyph 'composer) " ") 'face 'romacs-agent-log-prompt))))
    (goto-char start)
    (insert (romacs-agent-log--separator-field (concat divider "\n" prompt)))
    (delete-region (point) (+ (point) old))
    (set-marker romacs-agent-log--input (point))
    (set-marker marker start)
    (when-let* ((bounds (romacs-agent-log-activity-bounds))) (set-marker (cdr bounds) start))
    (set-marker romacs-agent-log--end end)))

(defun romacs-agent-log--activity-refresh ()
  "Show the current session's active workers above the divider."
  (when (and romacs-agent-log--input romacs-agent-log--width)
    (save-restriction
      (widen)
      (let ((text (romacs-agent-log-activity-lines
                   (romacs-agent-children-rows romacs-agent-log--session) romacs-agent-log--width))
            (end (marker-position romacs-agent-log--end)))
        (romacs-agent-log-activity-set (romacs-agent-log--separator-field text) end)
        (set-marker romacs-agent-log--end end)))))

(defun romacs-agent-log--separator (&optional force)
  "Write the separator again when what the session does, or the room the
windows showing it leave, changed."
  (let* ((doing (romacs-agent-log--doing romacs-agent-log--session))
         (width (romacs-agent-log--rule-width))
         (rows (and romacs-agent-log-separator-rows-function (funcall romacs-agent-log-separator-rows-function)))
         (refit (not (eql width romacs-agent-log--width))))
    (when (or force (not (equal doing romacs-agent-log--state)) refit
              (not (equal-including-properties rows romacs-agent-log--rows)))
      (setq romacs-agent-log--state doing romacs-agent-log--width width romacs-agent-log--rows rows)
      (let ((on (get-text-property (point) 'romacs-agent-log-queued)))
        (romacs-agent-log--writing
          (save-excursion
            ;; The new one put before the old one goes: a point in the
            ;; composer stays there, one at the transcript's end stays there.
            (romacs-agent-log--write-separator (romacs-agent-log--divider-text doing width) rows))
          (romacs-agent-log-attach-place))
      ;; Point on a queued row stays on it while it waits.
      (when-let* ((on) (at (text-property-any romacs-agent-log--end romacs-agent-log--input 'romacs-agent-log-queued on)))
        (goto-char at)))
      ;; The thread moves as the session does, drawn by the display.
      (romacs-working-set (current-buffer) (romacs-agent-log--work doing)))
    (when refit (romacs-agent-log--activity-refresh))))

(defvar-local romacs-agent-log--title nil "The title the chat's name was made from.")
(defvar-local romacs-agent-log--header-made nil
  "What the header was last made from: made again only when it differs.")

(defun romacs-agent-log--header-inputs (session)
  "What SESSION's header reads (`romacs-agent-ui--header-inputs'), as this
chat shows it: a plan it offers waits on the user, as a request does."
  (let ((inputs (romacs-agent-ui--header-inputs session)))
    (if (and (romacs-agent-log--proceed-offered) (not (memq (plist-get inputs :state) '(approval question))))
        (plist-put (copy-sequence inputs) :state 'approval)
      inputs)))

(defun romacs-agent-log--waiting-count ()
  "How many things wait on the user here: its approvals, and a plan offered."
  (+ (length romacs-agent-log--asks) (if (romacs-agent-log--proceed-offered) 1 0)))

(defun romacs-agent-log--header-snapshot (session)
  "Read the values from SESSION and this chat that the header needs."
  (let* ((state (romacs-agent-state session))
         (pending (romacs-agent-log--waiting-count))
         (start (romacs-agent-turn-start session))
         (elapsed (and start (zerop pending) (memq state '(starting running))
                       (round (- (float-time) start)))))
    (list (romacs-agent-log--header-inputs session) pending elapsed (eq state 'history))))

(defun romacs-agent-log--header-show ()
  "Make the header say what the session says now: a value, never a form,
so a redisplay runs no Lisp for it; made only when what it reads changed."
  (let ((inputs (romacs-agent-log--header-snapshot romacs-agent-log--session)))
    (unless (equal-including-properties inputs romacs-agent-log--header-made)
      (setq romacs-agent-log--header-made inputs)
      (let ((header (string-replace "%" "%%" (apply #'romacs-agent-log--header inputs))))
        (unless (equal-including-properties header header-line-format)
          (setq header-line-format header)
          (force-mode-line-update))))))

(defun romacs-agent-log--header (inputs pending elapsed history)
  "Render the chat header from INPUTS, PENDING requests and ELAPSED seconds.
HISTORY says the chat shows a read-only transcript."
  (concat (romacs-agent-ui--header-text inputs)
          (when (> pending 0)
            (propertize (format "  %s %d wait%s on you" (romacs-agent-glyph 'asks) pending (if (= pending 1) "s" ""))
                        'face 'romacs-agent-log-ask))
          (when history
            (concat " read-only · "
                    (propertize (substitute-command-keys
                                 "\\<romacs-agent-history-text-map>\\[romacs-agent-history-resume-this]")
                                'face 'romacs-agent-log-header-key)
                    " resumes"))
          (when elapsed
            (propertize (format "  %ds · %s stops" elapsed
                                (substitute-command-keys "\\<romacs-agent-log-mode-map>\\[romacs-agent-log-interrupt]"))
                        'face 'romacs-agent-log-note))))

;;; Events

(defun romacs-agent-log--on-event (session event)
  "Draw SESSION's EVENT in its chat, once: a record drawn already is not."
  (when-let* ((buffer (gethash (romacs-agent-id session) romacs-agent-log--chats))
              ((buffer-live-p buffer))
              ;; Not drawn yet: its fill will draw this.
              ((buffer-local-value 'romacs-agent-log--filled buffer)))
    (with-current-buffer buffer (romacs-agent-log--draw-live session event))))

(defun romacs-agent-log--draw-live (session event)
  "Draw SESSION's live EVENT in the current chat, once: a record drawn
already is not.  Each source's live events come here."
  (let ((seq (plist-get event :seq)))
    (when (or (null seq) (> seq romacs-agent-log--seq))
      (unless (and (eq session romacs-agent-log--session)
                   (equal (romacs-agent-title session) romacs-agent-log--title))
        (setq romacs-agent-log--title (romacs-agent-title session))
        (romacs-agent-log--set-session session))
      (romacs-agent-log-draw session event)
      (romacs-agent-log--progress event)
      (when (and seq (plist-get event :last)) (setq romacs-agent-log--seq seq))
      (romacs-agent-log--separator)
      (when (eq (car romacs-agent-log--state) 'working) (romacs-agent-log--tick-start))
      (romacs-agent-log--header-show))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-log--on-event)

(defun romacs-agent-log--progress (event)
  "Step the working thread on for EVENT when it is progress: a call begun
or ended, a line of its output, a message ended."
  (pcase (plist-get event :type)
    ((or 'tool 'message-end) (romacs-working-advance (current-buffer)))
    ;; A line of output each.
    ('tool-output (romacs-working-advance (current-buffer) (cl-count ?\n (or (plist-get event :text) ""))))))

;;; Replay

(defun romacs-agent-log--replay (session from end)
  "Draw SESSION's records from place FROM to place END at the transcript's
end, replayed into a copy so the session is not touched."
  (let ((scratch (romacs-agent-replay-copy session from)) (buffer (current-buffer)))
    (let ((romacs-agent-event-functions
           (list (lambda (_ event)
                   (with-current-buffer buffer
                     (let ((romacs-agent-log--replaying scratch))
                       (romacs-agent-log-draw session event))
                     (when-let* ((seq (plist-get event :seq))) (setq romacs-agent-log--seq (max romacs-agent-log--seq seq))))))))
      (romacs-agent--read-journal session (lambda (record) (romacs-agent-replay-record scratch record)) from end))
    (romacs-agent-log--close-stream)))

(defun romacs-agent-log--earlier-line (from)
  (romacs-agent-log--sealed
   (propertize (concat (propertize (format "%s earlier turns · %s shows them" (romacs-agent-glyph 'earlier) (romacs-agent-log--key 'romacs-agent-log-open))
                                   'face 'romacs-agent-log-note)
                       "\n")
               'romacs-agent-log-earlier from 'romacs-agent-log-entry 'earlier)))

(defun romacs-agent-log-draw-imported (session imported)
  "Draw the harness transcript IMPORTED names (:path, :size) for SESSION
at the transcript's end: its end, from a prompt on
\(`romacs-agent-history-events').  Return the events drawn."
  (require 'romacs-agent-history)
  (let* ((path (plist-get imported :path))
         (read (romacs-agent-history-events (romacs-agent-harness session) imported)))
    (if (null read)
        (romacs-agent-log--line nil 'error (format "%s %s is gone" (romacs-agent-glyph 'denied) (abbreviate-file-name path)) 'romacs-agent-log-denied)
      (when (cdr read)
        (romacs-agent-log--note nil (format "earlier turns are in %s" (abbreviate-file-name path))))
      (dolist (event (car read)) (romacs-agent-log-draw session event))
      (romacs-agent-log--close-stream))
    (car read)))

(cl-defmethod romacs-agent-log-source-fill ((_source romacs-agent-log-local-source) session)
  "Draw SESSION's last `romacs-agent-log-turns' turns from its records."
  (let* ((from (romacs-agent-tail-start session romacs-agent-log-turns romacs-agent-log-records)))
    (setq romacs-agent-log--from from)
    (when from
      (romacs-agent-log--writing
        (save-excursion
          (goto-char (or romacs-agent-log--active-start (point-min)))
          (insert (romacs-agent-log--earlier-line from)))))
    ;; From its start: an imported session's harness transcript first.
    (when-let* (((null from)) (imported (romacs-agent-imported session)))
      (romacs-agent-log-draw-imported session imported))
    (romacs-agent-log--replay session from nil)
    (romacs-agent-log--sync-asks session)))

(defun romacs-agent-log--sync-asks (session)
  "What SESSION waits on now is what its chat shows waiting, whatever its
records drew: a block no longer waited on settled, one waited on drawn."
  (let ((approvals (romacs-agent-log-source-approvals romacs-agent-log--source session)))
    (dolist (id (mapcar #'romacs-agent-log-ask-id romacs-agent-log--asks))
      (unless (gethash id approvals)
        (romacs-agent-log--settle-ask id)
        ;; Said where it ended: what streams since goes on unbroken.
        (when-let* ((ended (romacs-agent-log--drop-ended id)))
          (romacs-agent-log--writing
            (save-excursion (goto-char (cdr ended)) (insert (romacs-agent-log--sealed (car ended))))))))
    (maphash (lambda (id request)
               (unless (romacs-agent-log--ask-of id)
                 (romacs-agent-log--ask-block (romacs-agent-log-source-request-event romacs-agent-log--source session id request))))
             approvals)))

(defun romacs-agent-log-earlier ()
  "Show the turns before those this chat shows, above them."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-source-earlier romacs-agent-log--source romacs-agent-log--session))

(cl-defmethod romacs-agent-log-source-earlier ((_source romacs-agent-log-local-source) session)
  "Draw SESSION's turns before those the chat shows from its records."
  (let* ((end romacs-agent-log--from)
         (from (and end (romacs-agent-tail-start session romacs-agent-log-turns romacs-agent-log-records end))))
    (unless end (user-error "This chat shows the session from its start"))
    (romacs-agent-log--draw-above session from (lambda () (romacs-agent-log--replay session from end)))))

(defun romacs-agent-log--draw-above (session from draw)
  "Put above what the current chat shows the turns DRAW draws of SESSION,
called in a chat of its own: the live end is untouched.  They begin at
FROM, nil at the session's start."
  (let ((text (with-temp-buffer
                (romacs-agent-log-mode)
                (setq romacs-agent-log--session session
                      romacs-agent-log--end (point-max-marker)
                      romacs-agent-log--input (point-max-marker))
                (set-marker-insertion-type romacs-agent-log--end t)
                (funcall draw)
                (romacs-agent-log--close-stream)
                ;; No approval here offers its keys: one settled later
                ;; is settled, and one still waited on has its block at
                ;; the chat's end (`romacs-agent-log--sync-asks').
                (dolist (id (mapcar #'romacs-agent-log-ask-id romacs-agent-log--asks)) (romacs-agent-log--settle-ask id))
                (buffer-substring (point-min) romacs-agent-log--end))))
    (save-restriction
      (narrow-to-region (or romacs-agent-log--active-start (point-min)) (point-max))
      (romacs-agent-log--writing
        (save-excursion
          (goto-char (point-min))
          (let ((line-end (line-beginning-position 2)))
            (when (get-text-property (point) 'romacs-agent-log-earlier) (delete-region (point) line-end)))
          (insert (if from (romacs-agent-log--earlier-line from) "") text)
          ;; A rollback either side of the new page's start reaches over it.
          (romacs-agent-log-turns--dim-above (point)))))
    ;; Its tables were drawn at no window's width.
    (romacs-agent-log-tables-refit))
  (setq romacs-agent-log--from from))

;;; The chat

(defun romacs-agent-log--buffer-name (session)
  (format "*agent: %s*" (or (romacs-agent-title session) (romacs-agent-id session))))

(defun romacs-agent-log-chat (session &optional later source)
  "SESSION's chat buffer, made when it has none, and drawn from its
records now, or with LATER when a window first shows it.  SOURCE is
where the chat reads and acts on SESSION, this editor by default
\(`romacs-agent-log--source')."
  (let* ((source (or source romacs-agent-log-local))
         (chats (romacs-agent-log-source-chats source))
         (buffer (gethash (romacs-agent-id session) chats)))
    (unless (buffer-live-p buffer)
      (setq buffer (generate-new-buffer (romacs-agent-log--buffer-name session)))
      (with-current-buffer buffer
        (romacs-agent-log-mode)
        (setq romacs-agent-log--source source)
        (unless (romacs-agent-log-source-restorable-p source)
          (setq-local desktop-save-buffer nil
                      warm-restart-left-out "another source's chat: open it again from its board"))
        (romacs-agent-log--set-session session)
        (setq romacs-agent-log--end (point-min-marker)
              romacs-agent-log--input (point-min-marker))
        (set-marker-insertion-type romacs-agent-log--end t)
        (romacs-agent-log--separator t)
        (romacs-working-show buffer buffer (romacs-agent-log--divider-marker) romacs-agent-log--thread-cols
                             (romacs-agent-log--work romacs-agent-log--state))
        (puthash (romacs-agent-id session) buffer chats)
        (add-hook 'window-buffer-change-functions #'romacs-agent-log--shown nil t)))
    (unless later (with-current-buffer buffer (romacs-agent-log--ensure-filled)))
    buffer))

(defun romacs-agent-log--clear ()
  "Empty the current chat's transcript, to draw its session afresh: what it
drew forgotten, its markers let go, its composer kept."
  ;; What streams is dropped, not faced: its text goes too.
  (romacs-agent-log--close-stream 'discard)
  (romacs-agent-log-activity-set "")
  (when romacs-agent-log--blocks
    (mapc #'romacs-agent-log--close-block (hash-table-values romacs-agent-log--blocks)))
  (dolist (ask romacs-agent-log--asks)
    (set-marker (romacs-agent-log-ask-start ask) nil)
    (set-marker (romacs-agent-log-ask-end ask) nil))
  (dolist (owed romacs-agent-log--ended-asks) (set-marker (romacs-agent-log-owed-at owed) nil))
  (romacs-agent-log--writing
    (delete-region (or romacs-agent-log--active-start (point-min)) romacs-agent-log--end))
  (setq romacs-agent-log--current nil romacs-agent-log--early nil romacs-agent-log--asks nil
        romacs-agent-log--attributed nil romacs-agent-log--ended-asks nil
        romacs-agent-log--prompts nil romacs-agent-log--seq 0
        romacs-agent-log--from nil romacs-agent-log--root-kind nil))

(defun romacs-agent-log--archive-handoff (target)
  "Set aside the current session's visible transcript before TARGET."
  (romacs-agent-log--close-stream)
  (let* ((start (save-excursion
                  (goto-char (or romacs-agent-log--active-start (point-min)))
                  (when (get-text-property (point) 'romacs-agent-log-earlier) (forward-line 1))
                  (point)))
         (history (buffer-substring-no-properties start romacs-agent-log--end)))
    (romacs-agent-log--clear)
    (romacs-agent-log--writing
      (save-excursion
        (goto-char romacs-agent-log--end)
        (insert (romacs-agent-log--sealed history))))
    (romacs-agent-log--note nil (format "Continued with %s" (romacs-agent-provider target)))
    (setq romacs-agent-log--active-start (copy-marker romacs-agent-log--end))))

(defun romacs-agent-log-continue (target)
  "Continue this buffer with TARGET, preserving the draft and earlier text.
Earlier turns' commands belong to their original session, available from
the board.  TARGET owns the composer and all subsequent live events."
  (let* ((source romacs-agent-log--session)
         (chats (romacs-agent-log-source-chats romacs-agent-log--source))
         (buffer (current-buffer)))
    (unless (eq (gethash (romacs-agent-id source) chats) buffer)
      (error "The source no longer owns this chat"))
    (save-excursion
      (when romacs-agent-log--recalled
        (romacs-agent-log--recall nil))
      (romacs-agent-log--archive-handoff target)
      (remhash (romacs-agent-id source) chats)
      (puthash (romacs-agent-id target) buffer chats)
      (setq romacs-agent-log--filled nil
            romacs-agent-log--recalled nil
            romacs-agent-log--draft-kept nil
            romacs-agent-log-turns--rollbacks nil)
      (romacs-agent-log--set-session target)
      (romacs-agent-log--ensure-filled)
      (when (get-buffer-window buffer t)
        (romacs-agent-log-source-seen romacs-agent-log--source target)))))

(defun romacs-agent-log--ensure-filled ()
  (unless romacs-agent-log--filled
    (setq romacs-agent-log--filled t)
    (romacs-agent-log-source-fill romacs-agent-log--source romacs-agent-log--session)
    (romacs-agent-log--separator t)
    (romacs-agent-log--activity-refresh)
    (romacs-agent-log--header-show)))

(defun romacs-agent-log--refit (_frame)
  "A window started or stopped showing a buffer, or it or its frame went:
each chat still shown, whatever its source, fits its rule to the windows
showing it.  One shown nowhere is fitted when it shows again."
  (dolist (buffer (delete-dups (mapcar #'window-buffer (window-list-1 nil 'nomini t))))
    (with-current-buffer buffer
      (when (and (derived-mode-p 'romacs-agent-log-mode) romacs-agent-log--input)
        (romacs-agent-log--separator)))))

(add-hook 'window-buffer-change-functions #'romacs-agent-log--refit)
(add-hook 'after-delete-frame-functions #'romacs-agent-log--refit)

(defun romacs-agent-log--shown (_window)
  "A window shows this chat: drawn now, when it was not, its header made
again, and its clock going while its turn runs."
  (romacs-agent-log--ensure-filled)
  (romacs-agent-log--activity-refresh)
  (romacs-agent-log--header-show)
  (when (memq (romacs-agent-state romacs-agent-log--session) '(starting running waiting-approval))
    (romacs-agent-log--tick-start)))

(defun romacs-agent-log--headers-refresh ()
  "Make the header of every shown chat say what its session says now: run
after a batch of events remade sessions' states (their workers')."
  (dolist (window (window-list-1 nil 'nomini 'visible))
    (with-current-buffer (window-buffer window)
      (when (derived-mode-p 'romacs-agent-log-mode)
        (romacs-agent-log--activity-refresh)
        (romacs-agent-log--header-show)))))

(add-hook 'romacs-agent-ui-changed-hook #'romacs-agent-log--headers-refresh)

(defun romacs-agent-log-show (session &optional action source)
  "Show SESSION's chat, as `pop-to-buffer' does with ACTION; SOURCE's
session, this editor's by default."
  (let ((buffer (romacs-agent-log-chat session nil source)))
    (pop-to-buffer buffer action)
    (goto-char (point-max))
    (recenter -1)
    (romacs-agent-log-source-seen romacs-agent-log--source session)
    buffer))


(defun romacs-agent-show (session &optional action source)
  "Show SESSION's chat, as `pop-to-buffer' does with ACTION; SOURCE's
session, this editor's by default."
  (romacs-agent-log-show session action source))
;; A session outlives the editor in its supervisor: a restored chat
;; (desktop.el, warm-restart.el) carries which session it shows and the
;; composer's unsent text; its transcript is drawn from the records.
(defun romacs-agent-log--desktop-save (_dirname)
  ;; A queued message in the composer to edit is no draft: it waits
  ;; where it was, as it was, for the next session.  Gone from the queue
  ;; meanwhile, the composer's text is all there is of it: a draft.
  (list :session (romacs-agent-id romacs-agent-log--session)
        :draft (if (and romacs-agent-log-queue--editing
                        (memq romacs-agent-log-queue--editing
                              (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session)))
                   ""
                 (buffer-substring-no-properties romacs-agent-log--input (point-max)))))

(defun romacs-agent-log-desktop-restore (_file _name misc)
  "The chat of MISC's session with its draft, drawn when a window first
shows it; its session loaded only then, as its records are read."
  (when-let* ((id (plist-get misc :session))
              (session (or (romacs-agent-loaded id) (romacs-agent-store-row-session id) (romacs-agent-get id)))
              (buffer (romacs-agent-log-chat session t)))
    (with-current-buffer buffer
      (save-excursion (goto-char (point-max)) (insert (or (plist-get misc :draft) ""))))
    buffer))

(defvar desktop-buffer-mode-handlers)
(defvar warm-restart-left-out)
(add-to-list 'desktop-buffer-mode-handlers '(romacs-agent-log-mode . romacs-agent-log-desktop-restore))
;; A desktop or warm-restart image saved before the old chat view went
;; names its mode: its session and draft come back in this chat.
(define-obsolete-function-alias 'romacs-agent-chat-mode #'romacs-agent-log-mode "0.0.21")
(add-to-list 'desktop-buffer-mode-handlers '(romacs-agent-chat-mode . romacs-agent-log-desktop-restore))

;;; Commands

(defun romacs-agent-log--loaded (session)
  "SESSION, the chat's, loaded: a restored chat's is its row until used."
  (let ((id (romacs-agent-id session)))
    (unless (eq (romacs-agent-loaded id) session)
      (romacs-agent-log--set-session (or (romacs-agent-get id) session)))
    romacs-agent-log--session))

(cl-defmethod romacs-agent-log-source-approvals ((_source romacs-agent-log-local-source) session)
  (romacs-agent-approvals session))

(cl-defmethod romacs-agent-log-source-send ((_source romacs-agent-log-local-source) session text &optional when)
  (let ((session (romacs-agent-log--loaded session)))
    (pcase when
      ((guard (string-empty-p text)) (when (and (eq when 'interrupt) (romacs-agent-log--running-p session))
                                       (romacs-agent-interrupt session)))
      ;; Its user's: theirs to change while it waits.
      ('later (romacs-agent-send-later session text nil nil nil nil t))
      ((and 'interrupt (guard (romacs-agent-log--running-p session)))
       (romacs-agent-interrupt session)
       (romacs-agent-send-later session text t nil nil nil t))
      (_ (romacs-agent-send session text nil nil nil nil t)))))

(cl-defmethod romacs-agent-log-source-interrupt ((_source romacs-agent-log-local-source) session)
  (romacs-agent-interrupt (romacs-agent-log--loaded session)))

(cl-defmethod romacs-agent-log-source-respond ((_source romacs-agent-log-local-source) session id allow reason)
  (romacs-agent-respond (romacs-agent-log--loaded session) id allow reason 'user))

(cl-defmethod romacs-agent-log-source-ask ((_source romacs-agent-log-local-source) session id questions)
  (romacs-agent-ask-open (romacs-agent-log--loaded session) id questions))

(cl-defmethod romacs-agent-log-source-seen ((_source romacs-agent-log-local-source) session)
  (romacs-agent-mark-seen session))

(cl-defmethod romacs-agent-log-source-request-event ((_source romacs-agent-log-local-source) session id request)
  (romacs-agent-request-event session id request))

(cl-defmethod romacs-agent-log-source-chats ((_source romacs-agent-log-local-source))
  romacs-agent-log--chats)

(cl-defmethod romacs-agent-log-source-answers-p ((_source romacs-agent-log-local-source) session event)
  (romacs-agent--auto-answer-p session event))

(cl-defmethod romacs-agent-log-source-commands-p ((_source romacs-agent-log-local-source))
  t)

(cl-defmethod romacs-agent-log-source-restorable-p ((_source romacs-agent-log-local-source))
  t)

(defun romacs-agent-log--commands-p ()
  "Whether this editor's session commands act on the chat's session."
  (romacs-agent-log-source-commands-p romacs-agent-log--source))

(defun romacs-agent-log--act (function &rest args)
  "Call FUNCTION, a source's generic, with the chat's source, its session and ARGS."
  (apply function romacs-agent-log--source romacs-agent-log--session args))

(defvar romacs-agent-log-draft-replaced-hook nil
  "Run in a chat whose composer was emptied, sent or replaced by a
recalled prompt: what was pending on its draft lands nowhere.")

(defun romacs-agent-log--draft ()
  (string-trim (buffer-substring-no-properties romacs-agent-log--input (point-max))))

(declare-function romacs-agent-log-queue-put-back "romacs-agent-log-queue" ())

(defun romacs-agent-log--deliver (send)
  "Call SEND with the composer's text, then empty it; it stays when SEND
fails.  A queued message the composer holds to edit goes back in its
place instead (`romacs-agent-log-queue-put-back'), sent by no one."
  (let ((text (romacs-agent-log--draft)))
    (if romacs-agent-log-queue--editing (romacs-agent-log-queue-put-back)
     (unless (string-empty-p text)
      (funcall send text)
      (setq romacs-agent-log--recalled nil)
      (delete-region romacs-agent-log--input (point-max))
      (run-hooks 'romacs-agent-log-draft-replaced-hook)
      ;; Sent: nothing typed is left.
      (set-buffer-modified-p nil)))))

(defun romacs-agent-log--running-p (&optional session)
  "Whether SESSION, the chat's by default, runs a turn."
  (memq (romacs-agent-state (or session romacs-agent-log--session)) '(starting running waiting-approval)))

(defun romacs-agent-log-send ()
  "On the transcript, open the entry at point; in the composer, send its
text (`romacs-agent-log-steer')."
  (interactive nil romacs-agent-log-mode)
  (if (< (point) romacs-agent-log--input)
      (romacs-agent-log-open)
    (romacs-agent-log-steer)))

(defun romacs-agent-log-steer ()
  "Send the composer's text now, wherever point is: into the turn under
way, when one runs."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log--deliver (lambda (text) (romacs-agent-log--act #'romacs-agent-log-source-send text))))

(defun romacs-agent-log-queue ()
  "Send the composer's text once the turn under way ends."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log--deliver (lambda (text) (romacs-agent-log--act #'romacs-agent-log-source-send text 'later))))

(defun romacs-agent-log-interrupt ()
  "Interrupt the turn under way."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log--act #'romacs-agent-log-source-interrupt))

(defun romacs-agent-log-interrupt-and-send ()
  "Interrupt the turn under way; the composer's text is sent once it ends."
  (interactive nil romacs-agent-log-mode)
  (if (string-empty-p (romacs-agent-log--draft))
      (romacs-agent-log--act #'romacs-agent-log-source-send "" 'interrupt)
    (romacs-agent-log--deliver (lambda (text) (romacs-agent-log--act #'romacs-agent-log-source-send text 'interrupt)))))

(defun romacs-agent-log--pending ()
  "The approvals this chat shows that the live session still waits on,
the oldest first, `romacs-agent-log-ask's."
  (let ((approvals (romacs-agent-log--act #'romacs-agent-log-source-approvals)))
    (reverse (cl-remove-if-not (lambda (ask) (gethash (romacs-agent-log-ask-id ask) approvals)) romacs-agent-log--asks))))

(defun romacs-agent-log--ask-here ()
  "The approval to answer: the one at point, else the oldest pending; its
block is shown, and it is checked the session still waits on it."
  (let* ((at (get-text-property (point) 'romacs-agent-log-request))
         (pending (romacs-agent-log--pending))
         (ask (if at (cl-find at pending :key #'romacs-agent-log-ask-id :test #'equal) (car pending))))
    (unless ask (user-error (if at "That request is answered" "Nothing waits on you")))
    ;; Shown where it is asked: point at it says which is answered.  One
    ;; outside what a narrowed chat shows is not answered blind.
    (unless at
      (unless (<= (point-min) (romacs-agent-log-ask-start ask) (point-max))
        (user-error "What waits on you is outside what the chat shows: widen it"))
      (goto-char (romacs-agent-log-ask-start ask)))
    (romacs-agent-log-ask-id ask)))

(defun romacs-agent-log-allow (&optional always)
  "Allow the request at point, else the oldest pending; with ALWAYS, for
the session.  Questions open their answering layer."
  (interactive nil romacs-agent-log-mode)
  (if (and (not always) (not (romacs-agent-log--pending)) (romacs-agent-log--proceed-offered))
      ;; Nothing asked, a plan offered (a harness whose plan is no
      ;; request): allowing is proceeding with it.
      (romacs-agent-log-proceed)
    (romacs-agent-log--allow-ask always)))

(defun romacs-agent-log--allow-ask (always)
  "Allow the request at point, else the oldest pending; with ALWAYS, for
the session."
  (let* ((id (romacs-agent-log--ask-here))
         (questions (get-text-property (point) 'romacs-agent-log-questions)))
    (cond (questions (romacs-agent-log--act #'romacs-agent-log-source-ask id questions))
          ;; A plan is proceeded with, or not: no rule for the session.
          ((and always (get-text-property (point) 'romacs-agent-log-plan))
           (user-error (substitute-command-keys
                        "A plan is proceeded with once: \\[romacs-agent-log-allow] proceeds")))
          (t (romacs-agent-log--act #'romacs-agent-log-source-respond id (if always (list :always t) t) nil)))))

(defun romacs-agent-log-allow-always ()
  "Allow the request at point, else the oldest pending, for the session."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-allow t))

(defun romacs-agent-log-deny (reason)
  "Deny the request at point, else the oldest pending, telling the agent
REASON when given."
  (interactive (list (read-string "Deny, telling the agent (RET: nothing): ")) romacs-agent-log-mode)
  (let ((id (romacs-agent-log--ask-here)))
    (romacs-agent-log--act #'romacs-agent-log-source-respond id nil (and (not (string-empty-p reason)) reason))))

(defun romacs-agent-log-next-ask ()
  "Go to the next approval that waits on you, round them all."
  (interactive nil romacs-agent-log-mode)
  (let* ((pending (romacs-agent-log--pending))
         (next (or (cl-find-if (lambda (ask) (> (romacs-agent-log-ask-start ask) (point))) pending) (car pending))))
    (unless next (user-error "Nothing waits on you"))
    (goto-char (romacs-agent-log-ask-start next))
    (recenter)))

(defun romacs-agent-log--move (n)
  "Move to the Nth entry from point (back when N is negative)."
  (let ((pos (point)))
    (dotimes (_ (abs n))
      (setq pos (if (> n 0)
                    (let ((next (next-single-property-change pos 'romacs-agent-log-entry)))
                      (while (and next (not (get-text-property next 'romacs-agent-log-entry)))
                        (setq next (next-single-property-change next 'romacs-agent-log-entry)))
                      (or next pos))
                  (let ((prev (previous-single-property-change pos 'romacs-agent-log-entry)))
                    (while (and prev (> prev (point-min)) (not (get-text-property prev 'romacs-agent-log-entry)))
                      (setq prev (previous-single-property-change prev 'romacs-agent-log-entry)))
                    (or prev pos)))))
    (goto-char (min pos romacs-agent-log--end))))

(defun romacs-agent-log-next (n)
  "Move to the next entry; with N, the Nth."
  (interactive "p" romacs-agent-log-mode)
  (romacs-agent-log--move n))

(defun romacs-agent-log-previous (n)
  "Move to the previous entry; with N, the Nth."
  (interactive "p" romacs-agent-log-mode)
  (romacs-agent-log--move (- n)))

(defun romacs-agent-log--move-kind (kind n)
  "Move to the Nth entry of KIND from point (back when N is negative)."
  (let ((pos (point)) (forward (> n 0)))
    (dotimes (_ (abs n))
      (let ((found nil))
        (while (and (not found)
                    (let ((next (if forward
                                    (next-single-property-change pos 'romacs-agent-log-entry nil romacs-agent-log--end)
                                  (previous-single-property-change pos 'romacs-agent-log-entry nil (point-min)))))
                      (and (/= next pos) (setq pos next))))
          (setq found (eq (get-text-property pos 'romacs-agent-log-entry) kind)))
        (unless found (user-error "No more prompts"))))
    (goto-char pos)))

(defun romacs-agent-log-next-prompt (n)
  "Move to the next prompt; with N, the Nth."
  (interactive "p" romacs-agent-log-mode)
  (romacs-agent-log--move-kind 'prompt n))

(defun romacs-agent-log-previous-prompt (n)
  "Move to the previous prompt; with N, the Nth."
  (interactive "p" romacs-agent-log-mode)
  (romacs-agent-log--move-kind 'prompt (- n)))

(defun romacs-agent-log--open-delegated-thread (id)
  "Open the delegated thread with ID in this window."
  (if-let* ((child (romacs-agent-get id)))
      (romacs-agent-show child '(display-buffer-same-window))
    (user-error "That delegated thread is no longer available")))

(defun romacs-agent-log-open ()
  "Open the delegated thread, earlier turns, or full tool call at point."
  (interactive nil romacs-agent-log-mode)
  (cond ((get-text-property (point) 'romacs-agent-log-delegated-thread)
         (romacs-agent-log--open-delegated-thread (get-text-property (point) 'romacs-agent-log-delegated-thread)))
        ((get-text-property (point) 'romacs-agent-log-earlier) (romacs-agent-log-earlier))
        ((get-text-property (point) 'romacs-agent-log-full)
         (let ((full (get-text-property (point) 'romacs-agent-log-full))
               (tool (get-text-property (point) 'romacs-agent-log-tool))
               (title (string-trim (buffer-substring-no-properties (line-beginning-position) (line-end-position)))))
           (with-current-buffer (get-buffer-create "*agent output*")
             (let ((inhibit-read-only t))
               (erase-buffer)
               (insert title "\n")
               (when-let* ((details (and tool (romacs-agent-rows-details tool (car full) (cdr full)))))
                 (insert "\n" details "\n"))
               (when (car full) (insert "\nInput:\n" (romacs-agent-log--json (car full)) "\n"))
               (when (cdr full) (insert "\nOutput:\n" (cdr full) "\n"))
               (goto-char (point-min)))
             (view-mode 1)
             (pop-to-buffer (current-buffer)))))
        (t (user-error "Nothing to open here"))))

(defun romacs-agent-log-copy ()
  "Copy the entry at point."
  (interactive nil romacs-agent-log-mode)
  (let* ((start (if (get-text-property (point) 'romacs-agent-log-entry) (point)
                  ;; The change is after the entry's marked first character.
                  (let ((after (previous-single-property-change (point) 'romacs-agent-log-entry)))
                    (cond ((null after) (point-min))
                          ((and (> after (point-min)) (get-text-property (1- after) 'romacs-agent-log-entry)) (1- after))
                          (t after)))))
         (end (or (next-single-property-change (1+ start) 'romacs-agent-log-entry) romacs-agent-log--end)))
    ;; Its text, its bars and other decoration left out.
    (kill-new (string-trim (substring-no-properties
                            (romacs-agent-log--undecorated (buffer-substring start (min end romacs-agent-log--end))))))
    (message "Copied")))

(defun romacs-agent-log-type (n)
  "Type the key's character N times at the composer's end: typed on the
transcript, it goes there."
  (interactive "p" romacs-agent-log-mode)
  (goto-char (point-max))
  (self-insert-command n))

(defun romacs-agent-log--composer-plain (beg end _length)
  "Text typed or yanked into the composer is plain: nothing of the
transcript's comes with it."
  (when (and (markerp romacs-agent-log--input) (< beg end) (>= beg romacs-agent-log--input)
             (or (text-properties-at beg)
                 (and (< (1+ beg) end)
                      (< (next-property-change beg nil end) end))))
    (let ((inhibit-modification-hooks t) (inhibit-read-only t)) (set-text-properties beg end nil))))

(defun romacs-agent-log-composer ()
  "Go to the composer."
  (interactive nil romacs-agent-log-mode)
  (goto-char (point-max)))

(defun romacs-agent-log--recall (cells)
  "Show the prompt of the first of CELLS in the composer, or the draft."
  (unless romacs-agent-log--recalled
    (setq romacs-agent-log--draft-kept (buffer-substring-no-properties romacs-agent-log--input (point-max))))
  (setq romacs-agent-log--recalled cells)
  (delete-region romacs-agent-log--input (point-max))
  (run-hooks 'romacs-agent-log-draft-replaced-hook)
  (goto-char (point-max))
  (insert (if cells (car (car cells)) (or romacs-agent-log--draft-kept ""))))

(defun romacs-agent-log-history-previous ()
  "Put the previous prompt in the composer."
  (interactive nil romacs-agent-log-mode)
  (let ((cell (if romacs-agent-log--recalled (cdr (car romacs-agent-log--recalled)) romacs-agent-log--prompts)))
    (unless cell (user-error "No earlier prompt"))
    (romacs-agent-log--recall (cons cell romacs-agent-log--recalled))))

(defun romacs-agent-log-history-next ()
  "Put the next prompt in the composer; past the newest, empty it."
  (interactive nil romacs-agent-log-mode)
  (unless romacs-agent-log--recalled (user-error "No later prompt"))
  (romacs-agent-log--recall (cdr romacs-agent-log--recalled)))

;;; The clock

;; One timer redraws the header of every shown chat whose turn runs, once
;; a second; it ends when none runs.
(defvar romacs-agent-log--ticker nil)

(defun romacs-agent-log--tick ()
  (let ((running nil))
    (dolist (window (window-list-1 nil 'nomini 'visible))
      (with-current-buffer (window-buffer window)
        (when (and (derived-mode-p 'romacs-agent-log-mode)
                   (memq (romacs-agent-state romacs-agent-log--session) '(starting running waiting-approval)))
          (setq running t)
          (romacs-agent-log--header-show))))
    (unless (or running (not (timerp romacs-agent-log--ticker)))
      (cancel-timer romacs-agent-log--ticker)
      (setq romacs-agent-log--ticker nil))))

(defun romacs-agent-log--tick-start ()
  (unless (timerp romacs-agent-log--ticker)
    (setq romacs-agent-log--ticker (run-at-time 1 1 #'romacs-agent-log--tick))))

;;; Keys and mode

(defvar-keymap romacs-agent-log-transcript-map
  :doc "Keys of a chat's transcript: a `keymap' text property, as a view's.
A character typed on it goes to the composer."
  "<remap> <self-insert-command>" #'romacs-agent-log-type
  "SPC" #'scroll-up-command
  "DEL" #'scroll-down-command
  "?" #'romacs-agent-menu-map-popup
  "n" #'romacs-agent-log-next
  "p" #'romacs-agent-log-previous
  "M-n" #'romacs-agent-log-next-prompt
  "M-p" #'romacs-agent-log-previous-prompt
  "RET" #'romacs-agent-log-open
  "w" #'romacs-agent-log-copy
  "e" #'romacs-agent-log-earlier
  "u" #'romacs-agent-chat-rollback
  "D" #'romacs-agent-chat-diff
  "g" #'romacs-agent-log-composer)

(defvar-keymap romacs-agent-log-plan-map
  :doc "Keys of a plan to proceed with by leaving plan mode."
  :parent romacs-agent-log-transcript-map
  "y" #'romacs-agent-log-proceed)

(defvar-keymap romacs-agent-log-ask-map
  :doc "Keys of an approval waiting on the user."
  :parent romacs-agent-log-transcript-map
  "y" #'romacs-agent-log-allow
  "!" #'romacs-agent-log-allow-always
  "d" #'romacs-agent-log-deny)

(defvar-keymap romacs-agent-log-mode-map
  :doc "Keys of an agent chat; the composer edits as any buffer does.
The session's commands are the menu's (romacs-agent-menu.el), here
under C-c as well."
  "RET" #'romacs-agent-log-send
  "DEL" #'romacs-agent-log-attach-delete-backward
  "<remap> <yank>" #'romacs-agent-log-attach-yank
  "C-j" #'newline
  "S-<return>" #'newline
  ;; The turn.
  "C-c C-c" #'romacs-agent-log-interrupt
  "C-c C-i" #'romacs-agent-log-interrupt
  "C-c C-k" #'romacs-agent-log-interrupt-and-send
  "C-c C-q" #'romacs-agent-log-queue
  "C-c C-s" #'romacs-agent-log-steer
  "C-c C-a" #'romacs-agent-chat-attach
  ;; What waits on the user.
  "C-c C-y" #'romacs-agent-log-allow
  "C-c C-d" #'romacs-agent-log-deny
  "C-c C-p" #'romacs-agent-log-next-ask
  ;; The session.
  "C-c C-n" #'romacs-agent-chat-new
  "C-c C-r" #'romacs-agent-chat-rename
  "C-c C-e" #'romacs-agent-chat-effort
  ;; Elsewhere.
  "C-c C-j" #'romacs-agent-log-composer
  "C-c C-b" #'romacs-agents
  "C-c C-t" #'romacs-agents
  "C-c C-o" #'romacs-agent-menu-map-popup
  "C-c C-/" #'describe-mode
  "M-p" #'romacs-agent-log-history-previous
  "M-n" #'romacs-agent-log-history-next)


(defvar romacs-agent-log--face-lists (make-hash-table :test #'equal)
  "One object for each distinct face list the chat's decoration uses: the
display realizes a face list by its identity, so equal lists made anew
for each message cost a realization each redraw (romacs#377).")

(defun romacs-agent-log--shared-faces (text)
  "TEXT, its face lists each replaced by the one object of its kind in
`romacs-agent-log--face-lists'; TEXT itself, changed in place."
  (let ((at 0) (end (length text)))
    (while (< at end)
      (let ((next (next-single-property-change at 'face text end))
            (face (get-text-property at 'face text)))
        (when (consp face)
          (put-text-property at next 'face
                             (or (gethash face romacs-agent-log--face-lists)
                                 (let ((kept (copy-sequence face))) (puthash kept kept romacs-agent-log--face-lists)))
                             text))
        (setq at next))))
  text)

(defun romacs-agent-log--undecorated (text)
  "TEXT without its decoration, the runs marked `romacs-agent-decor'."
  (if (not (stringp text)) text
    (let ((at 0) (parts nil) (text (romacs-agent-log-tables-source text)))
      (while (< at (length text))
        (let ((next (next-single-property-change at 'romacs-agent-decor text (length text))))
          (unless (get-text-property at 'romacs-agent-decor text)
            (push (substring text at next) parts))
          (setq at next)))
      (apply #'concat (nreverse parts)))))

(define-derived-mode romacs-agent-log-mode fundamental-mode "Agent"
  "An agent session's chat: its transcript, and below, the composer.
\\{romacs-agent-log-mode-map}
On the transcript's text:
\\{romacs-agent-log-transcript-map}"
  :interactive nil
  (setq-local truncate-lines nil word-wrap t)
  ;; A reply's Markdown marks read as their faces (romacs-agent-log-faces.el).
  (add-to-invisibility-spec 'romacs-markdown-markup)
  (setq-local scroll-conservatively 101)
  (setq-local header-line-format "")
  (setq-local desktop-save-buffer #'romacs-agent-log--desktop-save)
  ;; A quote's bar is decoration: never in what one copies.
  (add-function :filter-return (local 'filter-buffer-substring-function) #'romacs-agent-log--undecorated)
  ;; The separator's rule fits the window it is shown in.
  (add-hook 'window-size-change-functions (lambda (_) (when romacs-agent-log--input (romacs-agent-log--separator))) nil t)
  (add-hook 'window-size-change-functions #'romacs-agent-log-tables-refit nil t)
  (romacs-agent-log-attach-setup)
  (add-hook 'after-change-functions #'romacs-agent-log--composer-plain nil t))

(provide 'romacs-agent-log)
;; Its turns' place in the transcript, which needs it loaded.
(require 'romacs-agent-log-turns)
;; Its replies' faces as they stream.
(require 'romacs-agent-log-faces)
(require 'romacs-agent-log-tables)
(require 'romacs-agent-log-attach)
;; Its rounds, told at a glance.
(require 'romacs-agent-log-rounds)
(require 'romacs-agent-log-queue)
;;; romacs-agent-log.el ends here

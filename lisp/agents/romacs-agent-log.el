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
;;   separator    one line: what the session does now
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
;; The view draws from events alone: live ones through
;; `romacs-agent-event-functions', and a session's history by replaying its
;; records into a copy (`romacs-agent--scratch'), the same events drawn by
;; the same code.  A turn's output longer than
;; `romacs-agent-log-output-lines' shows its first lines and how many more:
;; RET on it shows all of it.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-rows)
(require 'romacs-agent-store)

(require 'keymap-popup)
(require 'romacs-working)

;; The actions the chat's menu offers on its session are the chat's of
;; before (romacs-agent-ui.el), which read the session from here too.
(defvar romacs-agent-ui--session)
(defvar romacs-agent-ui-changed-hook)
(declare-function romacs-agent-chat-model "romacs-agent-ui" (model effort))
(declare-function romacs-agent-chat-effort "romacs-agent-ui" (effort))
(declare-function romacs-agent-chat-plan "romacs-agent-ui" ())
(declare-function romacs-agent-chat-handoff "romacs-agent-ui" (provider model next))
(declare-function romacs-agent-chat-next-account "romacs-agent-ui" ())
(declare-function romacs-agent-chat-new "romacs-agent-ui" ())
(declare-function romacs-agent-chat-rename "romacs-agent-ui" (title))
(declare-function romacs-agent-chat-fork "romacs-agent-ui" ())
(declare-function romacs-agent-chat-renew "romacs-agent-ui" ())
(declare-function romacs-agent-chat-resume "romacs-agent-ui" ())
(declare-function romacs-agent-chat-stop "romacs-agent-ui" ())
(declare-function romacs-markdown-fontify-region "romacs-markdown" (beg end &optional from))
(declare-function romacs-agent-ask-open "romacs-agent-ask" (session id questions &optional place))
(declare-function romacs-agent-ui--header-text "romacs-agent-ui" (inputs))
(declare-function romacs-agent-ui--header-inputs "romacs-agent-ui" (session))
(declare-function romacs-agents "romacs-agent-board" ())
(declare-function romacs-agent-events-p "romacs-agent-events" (session))
(declare-function romacs-agent-events-end "romacs-agent-events" (session))
(declare-function romacs-agent-events-note-seq "romacs-agent-events" (session type end))
(declare-function romacs-agent--claude-questions "romacs-agent-claude" (input))
(declare-function romacs-agent--codex-questions "romacs-agent-codex" (params))
(declare-function romacs-agent-history-events "romacs-agent-history" (harness imported))

(defcustom romacs-agent-log-output-lines 8
  "How many lines of a tool's output, or of its input, a chat shows
under its call; RET on it shows all of it."
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
(defface romacs-agent-log-thinking '((t :inherit (shadow italic)))
  "What the agent thinks.")
(defface romacs-agent-log-tool '((t :inherit romacs-working-running))
  "A tool call's mark and name: the theme's own colour for work, never a
syntax face's, which a theme may draw as red as an error.")
(defface romacs-agent-log-output '((t :inherit shadow))
  "A tool's output, and its input's lines.")
(defface romacs-agent-log-note '((t :inherit shadow))
  "A status line: what the harness or Romacs says of the session.")
(defface romacs-agent-log-ask '((t :inherit romacs-working-waiting :extend t :weight bold))
  "An approval or a question that waits on the user.")
(defface romacs-agent-log-allowed '((t :inherit success))
  "An answer that allowed.")
(defface romacs-agent-log-denied '((t :inherit error))
  "An answer that denied, and an error.")
(defface romacs-agent-log-rule '((t :inherit shadow))
  "A turn's end, and the separator above the composer.")

;;; State

(defvar-local romacs-agent-log--session nil "The session this chat shows.")
(defvar-local romacs-agent-log--end nil
  "Marker: the transcript's end, where events are drawn; the separator follows it.")
(defvar-local romacs-agent-log--input nil
  "Marker: the composer's start, after the separator.")
(defvar-local romacs-agent-log--seq 0 "The seq of the last record drawn.")
(defvar-local romacs-agent-log--from nil
  "Where the drawn records begin (a place, as `romacs-agent--journal-tail'
gives it), or nil when they begin at the session's start.")
(defvar-local romacs-agent-log--blocks nil
  "Open tool calls by id, a table of (ID . PLIST): :under, a marker
where what comes under the call goes; :start, one where its row begins;
:depth; :input, its input; :live, a marker where its output streaming
as it runs begins, and :streamed, that output; :task, the id of the work
its harness runs under it; :ended, its output drawn while that work
goes on.")

(defun romacs-agent-log--block (id)
  "The open call ID, as (ID . PLIST), or nil."
  (and id romacs-agent-log--blocks (gethash id romacs-agent-log--blocks)))

(defun romacs-agent-log--close-block (block)
  (set-marker (plist-get (cdr block) :under) nil)
  (set-marker (plist-get (cdr block) :start) nil)
  (remhash (car block) romacs-agent-log--blocks))
(defvar-local romacs-agent-log--current nil
  "The id of the call at the transcript's top level that runs now, or nil.")
(defvar-local romacs-agent-log--early nil
  "Output that came before its call, an alist of ID to TEXT.")
(defvar-local romacs-agent-log--stream nil
  "What streams at the transcript's end: (KIND START AT NEWLINES), KIND
`text' or `thinking', START a marker where its text began, AT one where
the next text goes, before the newline that ends its line so far, and
NEWLINES the line breaks it ended in so far, held until text follows.")
(defvar-local romacs-agent-log--asks nil
  "Approval blocks drawn, by request id: (START . END) markers.")
(defvar-local romacs-agent-log--attributed nil
  "Request ids whose answer was drawn with who gave it.")
(defvar-local romacs-agent-log--delivered nil
  "Request ids whose answer the harness's records have: it took it.")
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

(defun romacs-agent-log--set-session (session)
  "Make SESSION the current chat's, named by its title."
  (setq romacs-agent-log--session session)
  (setq-local romacs-agent-ui--session session)
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
          (put-text-property position end 'keymap romacs-agent-log-transcript-map text)
          (setq position end))))
    text))

(defmacro romacs-agent-log--writing (&rest body)
  "Run BODY, which writes the transcript: read-only lifted, no undo kept,
and no hook told of it (it is no edit of the user's)."
  (declare (indent 0) (debug t))
  `(let ((inhibit-read-only t) (inhibit-modification-hooks t) (buffer-undo-list t)) ,@body))

(defun romacs-agent-log--put (text &optional block)
  "Insert TEXT, sealed, at the transcript's end, or at the end of BLOCK, an
open call (`romacs-agent-log--blocks'): it ends after TEXT then, and so do
the calls it is under that ended where it did, while the calls under it
end before TEXT."
  (romacs-agent-log--writing
    (save-excursion
      (let ((at (if block (marker-position (plist-get (cdr block) :under)) romacs-agent-log--end)))
        (goto-char at)
        (insert (romacs-agent-log--sealed text))
        (while block
          (let ((under (plist-get (cdr block) :under)))
            (when (= under at) (set-marker under (point))))
          (setq block (romacs-agent-log--block (plist-get (cdr block) :parent))))))))

(defun romacs-agent-log--indent (text depth)
  "TEXT with every line indented for a subagent DEPTH deep."
  (if (zerop depth) text
    (let ((bar (propertize (concat (make-string (* 2 (1- depth)) ?\s) "│ ") 'face 'romacs-agent-log-rule)))
      (replace-regexp-in-string "^\\(.\\)" (lambda (match) (concat bar match)) text t t))))

(defun romacs-agent-log--where (event)
  "Where EVENT is drawn: (BLOCK . DEPTH), under its parent's open call,
else (nil . 0), at the end."
  (let ((parent (romacs-agent-log--block (plist-get event :parent))))
    (if parent (cons parent (1+ (plist-get (cdr parent) :depth))) (cons nil 0))))

(defun romacs-agent-log--close-stream ()
  "End what streams: a message gets its Markdown faces."
  (when romacs-agent-log--stream
    (pcase-let ((`(,kind ,start ,at ,_) romacs-agent-log--stream))
      (setq romacs-agent-log--stream nil)
      (when (and (eq kind 'text) (fboundp 'romacs-markdown-fontify-region))
        (romacs-agent-log--writing
          (ignore-errors (romacs-markdown-fontify-region start romacs-agent-log--end))))
      (set-marker start nil)
      (set-marker at nil))))

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

(defun romacs-agent-log--stream (event kind text)
  "Stream TEXT, KIND `text' or `thinking', at the end, or under EVENT's
parent call."
  (pcase-let ((`(,at . ,depth) (romacs-agent-log--where event)))
    (if at
        ;; Under a call: its text goes in whole lines, the rest held.
        (let* ((props (cdr at))
               (held (if (eq (plist-get props :kind) kind) (plist-get props :held) "")))
          (unless (eq (plist-get props :kind) kind) (romacs-agent-log--flush at))
          (setcdr at (plist-put (plist-put (cdr at) :kind kind) :held (concat held text)))
          (let ((whole (plist-get (cdr at) :held)))
            (when-let* ((cut (string-match "\n[^\n]*\\'" whole)))
              (setcdr at (plist-put (cdr at) :held (substring whole (1+ cut))))
              (romacs-agent-log--under at kind (substring whole 0 cut) depth))))
      (let* ((new (not (and romacs-agent-log--stream (eq (car romacs-agent-log--stream) kind))))
             ;; Line breaks around its text wait for text after them: a
             ;; message ends without a trail of blank lines, and at most
             ;; one blank line parts its paragraphs.
             (body (string-trim text "\n+" "\n+"))
             (lead (if (string-empty-p body) (length text) (string-match-p "[^\n]" text)))
             (held (+ (if new 0 (nth 3 romacs-agent-log--stream)) lead)))
        (when new
          (romacs-agent-log--close-stream)
          ;; Its line ended at once, its text put before that newline: the
          ;; separator is never on its line.
          (let ((start (copy-marker romacs-agent-log--end)))
            (romacs-agent-log--put "\n")
            (setq romacs-agent-log--stream (list kind start (copy-marker start t) 0))))
        (if (string-empty-p body)
            (setcar (nthcdr 3 romacs-agent-log--stream) held)
          (let* ((trail (- (length text) lead (length body)))
                 (body (replace-regexp-in-string "\n\n\n+" "\n\n" body))
                 (chunk (concat (if new (if (eq kind 'thinking) (propertize "∴ " 'face 'romacs-agent-log-thinking) "")
                                  (make-string (min held 2) ?\n))
                                (if (eq kind 'thinking) (propertize body 'face 'romacs-agent-log-thinking) body))))
            ;; Its first character starts the entry.
            (when new (put-text-property 0 1 'romacs-agent-log-entry kind chunk))
            (romacs-agent-log--writing
              (save-excursion
                (goto-char (nth 2 romacs-agent-log--stream))
                (insert (romacs-agent-log--sealed chunk))))
            (setcar (nthcdr 3 romacs-agent-log--stream) trail)))))))

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
  (let ((held (plist-get (cdr block) :held)))
    (when (and held (not (string-empty-p held)))
      (setcdr block (plist-put (cdr block) :held ""))
      (romacs-agent-log--under block (plist-get (cdr block) :kind) held (1+ (plist-get (cdr block) :depth))))))

(defun romacs-agent-log--report (output)
  "A subagent's OUTPUT as its report: without the frame Claude Code puts
around it for its model (the hand-back notice before it, the agent's id
and its usage after it).  RET still shows all of it."
  (let ((report (string-trim-right
                 (if (string-match "\\`\\[Subagent hand-back\\]\\(?:.\\|\n\\)*?The report follows:[ \t]*\n" output)
                     (substring output (match-end 0))
                   output))))
    ;; Its last usage block, then its id line, at its end only.
    (when (string-suffix-p "</usage>" report)
      (let ((open nil) (from 0))
        (while (setq from (string-search "<usage>" report from))
          (setq open from from (1+ from)))
        ;; A block, closed by the report's last tag: none quoted inside it.
        (when (and open (= (string-search "</usage>" report open) (- (length report) (length "</usage>"))))
          (setq report (string-trim-right (substring report 0 open))))))
    (when (string-match "\n[ \t]*agentId: [^\n]*\\'" report)
      (setq report (substring report 0 (match-beginning 0))))
    (string-trim report)))

(defun romacs-agent-log--lines (text limit)
  "TEXT's first LIMIT lines and how many it has: (SHOWN . COUNT)."
  (let* ((lines (split-string (string-trim-right (or text "")) "\n"))
         (count (length lines)))
    (cons (string-join (seq-take lines limit) "\n") count)))

(defun romacs-agent-log--output (output face full)
  "OUTPUT under its call, `⎿' first, its first lines in FACE, then how
many more there are; FULL, (INPUT . OUTPUT), is what RET on it shows."
  (pcase-let ((`(,shown . ,count) (romacs-agent-log--lines output romacs-agent-log-output-lines)))
    (let ((text (concat (propertize "  ⎿ " 'face 'romacs-agent-log-rule)
                        (propertize (replace-regexp-in-string "\n" "\n    " (if (string-empty-p shown) "(no output)" shown) t t)
                                    'face face)
                        (if (> count romacs-agent-log-output-lines)
                            (propertize (format "\n    … %d more lines · %s shows all" (- count romacs-agent-log-output-lines)
                                                (romacs-agent-log--key 'romacs-agent-log-open))
                                        'face 'romacs-agent-log-note)
                          "")
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

(defun romacs-agent-log--call (event)
  "The first line of tool call EVENT: its renderer's row, else its name and
the brief of its input."
  (let* ((name (romacs-agent-rows-name (plist-get event :name)))
         (input (let ((input (plist-get event :input))) (if (hash-table-p input) input (make-hash-table :test #'equal))))
         (renderer (romacs-agent-rows-get name))
         (row (and renderer (ignore-errors (funcall (plist-get renderer :row) input)))))
    (concat (propertize "● " 'face 'romacs-agent-log-tool)
            (or row (concat (propertize name 'face 'romacs-agent-log-tool)
                            ;; One line of it: RET shows all of it.
                            (let ((brief (truncate-string-to-width
                                          (car (split-string (romacs-agent-rows--brief input) "\n"))
                                          100 nil nil (truncate-string-ellipsis))))
                              (if (string-empty-p brief) "" (concat "(" brief ")"))))))))

(defun romacs-agent-log--live (block text)
  "BLOCK's output streamed so far grows by TEXT: its last lines show under
its call, in place of those shown before."
  (let* ((props (cdr block))
         (streamed (concat (plist-get props :streamed) text))
         (streamed (substring streamed (max 0 (- (length streamed) 65536))))
         (count (+ (or (plist-get props :count) 0) (cl-count ?\n text)))
         ;; Lines whole and the one under way.
         (total (+ count (if (string-suffix-p "\n" streamed) 0 1)))
         (shown (last (let ((lines (split-string streamed "\n")))
                        ;; The empty piece after a final newline is no line.
                        (if (string-suffix-p "\n" streamed) (butlast lines) lines))
                      romacs-agent-log-output-lines))
         (under (plist-get props :under))
         (live (or (plist-get props :live) (copy-marker under))))
    (setcdr block (plist-put (plist-put (plist-put props :streamed streamed) :live live) :count count))
    (romacs-agent-log--writing (delete-region live under))
    (romacs-agent-log--put (romacs-agent-log--indent
                            (concat (propertize (if (> total (length shown))
                                                    (format "  ⋮ running: %d lines so far, the last %d\n" total (length shown))
                                                  (format "  ⋮ running: %d line%s so far\n" total (if (= total 1) "" "s")))
                                                'face 'romacs-agent-log-note)
                                    (propertize (concat "    " (string-join shown "\n    ") "\n") 'face 'romacs-agent-log-output))
                            (plist-get props :depth))
                           block)))

(defun romacs-agent-log--tool (event)
  "Draw tool EVENT: its call as it starts, its output under it as it ends."
  (let ((id (plist-get event :id)))
    (pcase (plist-get event :state)
      ('started
       (pcase-let ((`(,at . ,depth) (romacs-agent-log--where event)))
         (if at (romacs-agent-log--flush at) (romacs-agent-log--close-stream))
         (let* ((input (plist-get event :input))
                (text (propertize (romacs-agent-log--entry (romacs-agent-log--call event) 'tool event)
                                  'romacs-agent-log-full (list input)))
                (start (copy-marker (if at (plist-get (cdr at) :under) romacs-agent-log--end))))
           (romacs-agent-log--put (romacs-agent-log--indent text depth) at)
           ;; Its row in place: what an earlier call puts after it, at the
           ;; same place, goes before it.
           (set-marker-insertion-type start t)
           (unless romacs-agent-log--blocks (setq romacs-agent-log--blocks (make-hash-table :test #'equal)))
           (when (zerop depth) (setq romacs-agent-log--current id))
           ;; What comes under it goes here, in order, before what follows it.
           (puthash id (list id :under (copy-marker (if at (plist-get (cdr at) :under) romacs-agent-log--end))
                             :start start :depth depth :input input :parent (car at)
                             :name (romacs-agent-rows-name (plist-get event :name)))
                    romacs-agent-log--blocks)
           (when-let* ((early (alist-get id romacs-agent-log--early nil nil #'equal)))
             (setf (alist-get id romacs-agent-log--early nil t #'equal) nil)
             (romacs-agent-log--live (romacs-agent-log--block id) early)))))
      ((or 'completed 'failed)
       (let* ((block (romacs-agent-log--block id))
              (output (or (plist-get event :output) ""))
              (failed (eq (plist-get event :state) 'failed)))
         (when (equal id romacs-agent-log--current) (setq romacs-agent-log--current nil))
         (if (and block (not (plist-get (cdr block) :ended)))
             (let* ((props (cdr block)) (under (plist-get props :under)))
               (romacs-agent-log--flush block)
               ;; What streamed as it ran gives way to the whole output.
               (when-let* ((live (plist-get props :live)))
                 (romacs-agent-log--writing (delete-region live under))
                 (set-marker live nil))
               ;; Its input as it ended: some calls fill theirs in only then.
               (when-let* ((input (plist-get event :input))
                           ((not (equal input (plist-get props :input)))))
                 (setcdr block (plist-put (cdr block) :input input))
                 (romacs-agent-log--writing
                   (put-text-property (plist-get props :start)
                                      (save-excursion (goto-char (plist-get props :start)) (line-end-position))
                                      'romacs-agent-log-full (list input))))
               (romacs-agent-log--put (romacs-agent-log--indent
                                       (romacs-agent-log--output (if (member (plist-get props :name) '("Agent" "Task"))
                                                                     (romacs-agent-log--report output)
                                                                   output)
                                                                 (if failed 'romacs-agent-log-denied 'romacs-agent-log-output)
                                                                 (cons (plist-get (cdr block) :input) output))
                                       (plist-get props :depth))
                                      block)
               ;; A call stays open while work its harness runs under it
               ;; (a subagent, a command in the background) goes on.
               (if (and (or (member (plist-get props :name) '("Agent" "Task")) (plist-get props :task))
                        (not (plist-get props :task-ended)))
                   (setcdr block (plist-put (cdr block) :ended t))
                 (romacs-agent-log--close-block block)))
           ;; Its call was never drawn (it ended after the turn, or before
           ;; this chat's first record): said at the end, with its output.
           (romacs-agent-log--close-stream)
           (romacs-agent-log--put (concat (romacs-agent-log--entry
                                           (concat (propertize "● " 'face 'romacs-agent-log-tool)
                                                   (propertize (format "%s ended" (or (plist-get event :name) "a call"))
                                                               'face 'romacs-agent-log-note))
                                           'tool event)
                                          (romacs-agent-log--output output 'romacs-agent-log-output (cons nil output))))))))))

(defun romacs-agent-log--task-block (event)
  "The call EVENT, a task's, is under: by its tool's id, else its task's."
  (or (romacs-agent-log--block (plist-get event :id))
      (when-let* ((task (plist-get event :task)) (blocks romacs-agent-log--blocks))
        (catch 'found
          (maphash (lambda (_ block) (when (equal (plist-get (cdr block) :task) task) (throw 'found block))) blocks)
          nil))))

(defun romacs-agent-log--task (event)
  "Draw task EVENT, what work under a call (a subagent, a command) says
of itself, under that call: its start in the background, what it does
when that changes, and how it ended; with its call gone, at the end."
  (let* ((block (romacs-agent-log--task-block event))
         (state (plist-get event :state))
         (text (pcase state
                 ('backgrounded "moved to the background")
                 ('progress (let ((activity (plist-get event :activity)))
                              (and activity (not (and block (equal activity (plist-get (cdr block) :activity))))
                                   (progn (when block (setcdr block (plist-put (cdr block) :activity activity)))
                                          (format "now: %s" activity)))))
                 ;; Its summary, but a subagent's: its call's output, drawn, is its report.
                 ((or 'completed 'failed 'stopped)
                  (concat (symbol-name state)
                          (if-let* ((summary (and (not (and block (plist-get (cdr block) :ended)
                                                            (member (plist-get (cdr block) :name) '("Agent" "Task"))))
                                                  (plist-get event :summary))))
                              (concat ": " summary)
                            ""))))))
    (when (and block (eq state 'started) (plist-get event :task))
      (setcdr block (plist-put (cdr block) :task (plist-get event :task))))
    (when text
      (if (not block)
          (romacs-agent-log--note event (concat "task " text))
        (romacs-agent-log--flush block)
        (romacs-agent-log--put (romacs-agent-log--indent (propertize (concat "· " text "\n") 'face 'romacs-agent-log-note)
                                                         (1+ (plist-get (cdr block) :depth)))
                               block)))
    ;; Its work over, a call already ended closes; one still running
    ;; closes as it ends.
    (when (and block (memq state '(completed failed stopped)))
      (if (plist-get (cdr block) :ended)
          (romacs-agent-log--close-block block)
        (setcdr block (plist-put (cdr block) :task-ended t))))))

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
  (unless (romacs-agent--auto-answer-p session event)
    (romacs-agent-log--ask-block event)
    (when (and (romacs-agent--live-p session) (not (get-buffer-window (current-buffer) 'visible)))
      (message "%s %s waits on you: %s"
               (propertize "⚠" 'face 'romacs-agent-log-ask) (romacs-agent-title session)
               (substitute-command-keys "\\[romacs-agents] shows it")))))

(defun romacs-agent-log--ask-block (event)
  (romacs-agent-log--close-stream)
  (let* ((id (plist-get event :request-id))
         (questions (plist-get event :questions))
         (plan (plist-get event :plan))
         (input (plist-get event :input))
         (who (if (plist-get event :parent) "A subagent" "The agent"))
         (head (cond (questions (format "%s asks you:" who))
                     (plan (format "%s presents its plan: proceed?" who))
                     (t (format "%s wants to run %s:" who (romacs-agent-rows-name (plist-get event :tool))))))
         (body (cond (questions
                      (mapconcat (lambda (q)
                                   (concat "  " (car q)
                                           (when-let* ((options (plist-get (cdr q) :options)))
                                             (concat "\n    " (string-join options " · ")))))
                                 questions "\n"))
                     (plan nil)
                     (t (pcase-let ((`(,shown . ,count) (romacs-agent-log--lines (romacs-agent-log--json input) 30)))
                          (if (> count 30)
                              (format "%s\n… %d more lines · %s shows all" shown (- count 30) (romacs-agent-log--key 'romacs-agent-log-open))
                            shown)))))
         (why (plist-get event :description))
         (keys (cond (questions (romacs-agent-log--keys 'romacs-agent-log-allow "choose" 'romacs-agent-log-deny "decline"))
                     (plan (romacs-agent-log--keys 'romacs-agent-log-allow "proceed" 'romacs-agent-log-deny "keep planning, saying why"))
                     (t (romacs-agent-log--keys 'romacs-agent-log-allow "allow" 'romacs-agent-log-allow-always "allow for this session"
                                                'romacs-agent-log-deny "deny"))))
         (text (concat (propertize (concat "⚠ " head) 'face 'romacs-agent-log-ask)
                       (if why (concat "\n  " (propertize why 'face 'romacs-agent-log-note)) "")
                       ;; Its question and choices in the text's own face: the
                       ;; decision reads first, after the heading.
                       (if body (concat "\n" (propertize (replace-regexp-in-string "^" "  " body)
                                                          'face (if questions 'default 'romacs-agent-log-output)))
                         "")
                       "\n" (propertize (concat "  " keys "\n") 'romacs-agent-log-keys t))))
    (let ((start (copy-marker romacs-agent-log--end)))
      (romacs-agent-log--put (propertize (romacs-agent-log--entry text 'ask event)
                                         'romacs-agent-log-request id
                                         'romacs-agent-log-full (list input)
                                         'romacs-agent-log-questions questions
                                         'keymap romacs-agent-log-ask-map))
      (push (cons id (cons start (copy-marker romacs-agent-log--end))) romacs-agent-log--asks))))

(defun romacs-agent-log--settle-ask (id)
  "The approval block of request ID answered: no longer a warning."
  (when-let* ((block (assoc id romacs-agent-log--asks)))
    (pcase-let ((`(,_ ,start . ,end) block))
      (romacs-agent-log--writing
        ;; Its keys answer nothing any more: their row goes.
        (when-let* ((keys (text-property-any start end 'romacs-agent-log-keys t)))
          (delete-region keys (or (next-single-property-change keys 'romacs-agent-log-keys nil end) end)))
        (add-face-text-property start end 'romacs-agent-log-note)
        (remove-text-properties start end '(keymap nil))
        (put-text-property start end 'keymap romacs-agent-log-transcript-map)))
    (setq romacs-agent-log--asks (delq block romacs-agent-log--asks))))

(defun romacs-agent-log--close-asks (session event how)
  "SESSION's turn ended or its harness exited (HOW, `turn-end' or `exit',
at EVENT) with approvals drawn open: each it no longer waits on settled,
and said why.  One answered whose answer the harness's records never had
was not delivered."
  (dolist (ask romacs-agent-log--asks)
    (when-let* ((id (car ask))
                ((not (gethash id (romacs-agent-approvals session)))))
      (romacs-agent-log--settle-ask id)
      (romacs-agent-log--note event (cond ((and (member id romacs-agent-log--attributed)
                                                (not (member id romacs-agent-log--delivered)))
                                           "an answer not delivered: the harness never took it")
                                          ((eq how 'exit) "the harness exited with it unanswered")
                                          (t "the turn ended with it unanswered"))))))

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
        (concat (propertize "✔ " 'face 'romacs-agent-log-allowed)
                (if (equal by "user") (concat "answered" via ": ") "answered by Romacs: ")
                ;; One question: its answer; more: each question with its own.
                (let ((one (= (hash-table-count answers) 1)))
                  (mapconcat (lambda (question)
                               (if one (format "%s" (gethash question answers))
                                 (format "%s → %s" question (gethash question answers))))
                             (hash-table-keys answers) " · ")))
      (concat (propertize (if allowed "✔ " "✘ ") 'face (if allowed 'romacs-agent-log-allowed 'romacs-agent-log-denied))
              (pcase by
                ("user" (concat (if allowed "allowed by you" "denied by you") via))
                ("policy" (if allowed "allowed by Romacs (romacs-agent-approval-policy)" "denied by Romacs"))
                ("harness" (if allowed "allowed by the harness" "denied by the harness"))
                (_ (if allowed "allowed" "denied")))
              (if (equal scope "session") " for this session" "")
              (if (and reason (not (string-empty-p reason))) (concat ": " reason) "")))))

(defun romacs-agent-log--answered (event)
  "Draw answer EVENT, from the harness's records: its block settled, and,
unless a note said who gave it, that it was answered."
  (let ((id (plist-get event :request-id)))
    (push id romacs-agent-log--delivered)
    (romacs-agent-log--settle-ask id)
    (unless (member id romacs-agent-log--attributed)
      (romacs-agent-log--line event 'answer
                              (pcase (plist-get event :behavior)
                                ("allow" (concat (propertize "✔ " 'face 'romacs-agent-log-allowed) "allowed (who: not recorded)"))
                                ("deny" (concat (propertize "✘ " 'face 'romacs-agent-log-denied) "denied (who: not recorded)"))
                                (_ (propertize (if (eq (plist-get event :how) 'harness)
                                                   "· settled by the harness: no longer asked"
                                                 "· settled: the harness no longer asks")
                                               'face 'romacs-agent-log-note)))))))

(defun romacs-agent-log--prompt (event)
  "Draw input EVENT: the user's message, or one from another session."
  (let* ((messages (plist-get event :messages))
         (text (or (plist-get event :text) ""))
         (images (plist-get event :images))
         (_ (unless messages (push (string-trim text) romacs-agent-log--prompts)))
         (head (cond ((plist-get event :steer) (propertize "↪ " 'face 'romacs-agent-log-prompt))
                     (messages (propertize "✉ " 'face 'romacs-agent-log-note))
                     (t (propertize "❯ " 'face 'romacs-agent-log-prompt)))))
    (romacs-agent-log--line event 'prompt
                            (concat (if (and (null messages) (> romacs-agent-log--end (point-min))) "\n" "")
                                    head
                                    (propertize (string-trim text) 'face (if messages nil 'romacs-agent-log-prompt))
                                    (if images (propertize (format "  [%d image%s]" (length images) (if (cdr images) "s" ""))
                                                           'face 'romacs-agent-log-note)
                                      "")))))

(defun romacs-agent-log--done (event)
  "Draw turn end EVENT: a rule with its time, cost and why it failed."
  (romacs-agent-log--line
   event 'turn
   (concat (when-let* ((reason (or (plist-get event :reason) (and (plist-get event :error) (plist-get event :text)))))
             (concat (propertize (format "✘ %s" reason) 'face 'romacs-agent-log-denied) "\n"))
           (propertize (concat "✻ " (string-join (delq nil (list
                                                    (and (plist-get event :interrupted) "interrupted")
                                                    (when-let* ((s (plist-get event :duration))) (format "%ds" (round s)))
                                                    (when-let* ((c (plist-get event :cost))) (format "$%.2f" c))
                                                    (format-time-string "%H:%M" (plist-get event :time))))
                                    " · "))
                       'face 'romacs-agent-log-rule))))

(defun romacs-agent-log--note (event text)
  (romacs-agent-log--line event 'note (concat "· " text) 'romacs-agent-log-note))

(defun romacs-agent-log-draw (session event)
  "Draw SESSION's EVENT in the current chat."
  (pcase (plist-get event :type)
    ('input (romacs-agent-log--prompt event))
    ('incoming (romacs-agent-log--prompt event))
    ('delta (when-let* ((text (plist-get event :text))) (romacs-agent-log--stream event 'text text)))
    ('thinking (when-let* ((text (plist-get event :text))) (romacs-agent-log--stream event 'thinking text)))
    ('message-end (if-let* ((block (romacs-agent-log--block (plist-get event :parent))))
                      (romacs-agent-log--flush block)
                    (romacs-agent-log--close-stream)))
    ('tool (romacs-agent-log--tool event))
    ('task (romacs-agent-log--task event))
    ('tool-output (when-let* ((text (plist-get event :text)))
                    (if-let* ((block (romacs-agent-log--block (plist-get event :id))))
                        (romacs-agent-log--live block text)
                      ;; Before its call (Codex's first line can be): held for
                      ;; it, its last 64K, until its turn ends.
                      (let* ((id (plist-get event :id))
                             (held (concat (alist-get id romacs-agent-log--early nil nil #'equal) text)))
                        (setf (alist-get id romacs-agent-log--early nil nil #'equal)
                              (substring held (max 0 (- (length held) 65536))))))))
    ('approval (romacs-agent-log--ask session event))
    ;; Who answered: said, the block settled once its answer is written.
    ('attributed (push (plist-get event :request-id) romacs-agent-log--attributed)
                 (romacs-agent-log--line event 'answer
                                         (romacs-agent-log--answer-text (plist-get event :actor) (plist-get event :decision)
                                                                        (plist-get event :scope) (plist-get event :reason)
                                                                        (plist-get event :answers))))
    ('answered (romacs-agent-log--answered event))
    ('plan (romacs-agent-log--line event 'plan (concat (propertize "📋 Plan\n" 'face 'romacs-agent-log-prompt)
                                                       (plist-get event :text))))
    ('done (setq romacs-agent-log--current nil romacs-agent-log--early nil)
           (romacs-agent-log--close-asks session event 'turn-end)
           (romacs-agent-log--done event))
    ('error (romacs-agent-log--line event 'error (concat "✘ " (or (plist-get event :text) "error")) 'romacs-agent-log-denied))
    ('unstarted (romacs-agent-log--line event 'error (concat "✘ " (or (plist-get event :text) "never started"))
                                        'romacs-agent-log-denied))
    ;; What a session's state says (the header shows it) is no line.
    ('status (when-let* ((text (plist-get event :text))
                         ((not (member text '("requesting" "selection armed")))))
               (romacs-agent-log--note event text)))
    ('compacted (romacs-agent-log--note event (concat "context compacted\n"
                                                      (car (romacs-agent-log--lines (plist-get event :text)
                                                                                    romacs-agent-log-output-lines)))))
    ('exit (setq romacs-agent-log--current nil romacs-agent-log--early nil)
           (romacs-agent-log--close-asks session event 'exit)
           (unless (and (romacs-agent--per-turn-p session) (eql (plist-get event :status) 0))
             (romacs-agent-log--note event (format "harness exited (%s)" (plist-get event :status)))))
    ('renewed (romacs-agent-log--note event (format "renewed: a new conversation%s"
                                                    (if-let* ((why (plist-get event :why))) (format " (%s)" why) ""))))
    ('forked (romacs-agent-log--note event (format "forked from %s" (or (plist-get event :title) (plist-get event :from)))))
    ('rolled-back (romacs-agent-log--note event (format "rolled back %s turn(s)" (or (plist-get event :turns) "?"))))
    ('replied (romacs-agent-log--note event (format "replied to %s" (plist-get event :message))))
    ('selection (cl-flet ((label (selection)
                            (mapconcat #'identity (or (delq nil (list (plist-get selection :model) (plist-get selection :effort)))
                                                      '("harness's own"))
                                       " · ")))
                  (romacs-agent-log--note event (format "%s → %s" (label (plist-get event :from)) (label (plist-get event :to))))))
    ;; What Romacs does not read of its harness: named, its whole on RET.
    ('unknown (let ((raw (plist-get event :raw)))
                (romacs-agent-log--line event 'note
                                        (propertize (concat "· " (or (plist-get event :method)
                                                                     (and (hash-table-p raw) (stringp (gethash "type" raw))
                                                                          (gethash "type" raw))
                                                                     (and (stringp raw) (truncate-string-to-width
                                                                                         (car (split-string raw "\n")) 100
                                                                                         nil nil (truncate-string-ellipsis)))
                                                                     "a line Romacs does not read"))
                                                    'romacs-agent-log-full (cons nil (if (stringp raw) raw (romacs-agent-log--json raw))))
                                        'romacs-agent-log-note)))
    ;; State, not content: the header and separator say it.
    (_ nil)))

;;; The separator and header

(defun romacs-agent-log--doing (session)
  "What SESSION does now, as the separator says it: (STATE . WORDS)."
  (pcase (if (romacs-agent-log--pending) 'waiting-approval (romacs-agent-state session))
    ('waiting-approval
     (let ((question (when-let* ((ask (car (romacs-agent-log--pending))))
                       (get-text-property (cadr ask) 'romacs-agent-log-questions))))
       (cons 'waiting (substitute-command-keys
                       (concat "⚠ waits on you: \\<romacs-agent-log-mode-map>\\[romacs-agent-log-allow] "
                               (if question "choose" "allow") " · \\[romacs-agent-log-deny] "
                               (if question "decline" "deny") " · \\[romacs-agent-log-next-ask] goes to it")))))
    ((or 'starting 'running)
     (cons 'working (if-let* ((block (romacs-agent-log--block romacs-agent-log--current)))
                        (format "running %s" (plist-get (cdr block) :name))
                      "working")))
    ;; A turn that failed and is not yet seen: said so, until the next.
    ('idle (if (romacs-agent-failed session) (cons 'failed "failed") (cons 'idle "idle")))
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

(defun romacs-agent-log--separator-text (doing width)
  "The separator for DOING, (STATE . WORDS), in WIDTH columns: the working
thread, what the session does, then the composer's prompt."
  (let ((face (pcase (car doing) ('waiting 'romacs-agent-log-ask) ('working 'romacs-working-running)
                ('failed 'romacs-agent-log-denied) (_ 'romacs-agent-log-rule))))
    (let* ((label (concat " " (propertize (cdr doing) 'face (list face 'bold)) " "))
           ;; A dim rule after it fences the composer off the transcript,
           ;; never wrapping; none when that is short.
           (rule (- width 1 romacs-agent-log--thread-cols (string-width label))))
      (concat (romacs-working-still romacs-agent-log--thread-cols (romacs-agent-log--work doing))
              label
              (if (>= rule 8) (propertize (make-string rule ?─) 'face 'romacs-agent-log-rule) "")
              "\n" (propertize "❯ " 'face 'romacs-agent-log-prompt)))))

(defun romacs-agent-log--separator (&optional force)
  "Write the separator again when what the session does, or the room the
windows showing it leave, changed."
  (let ((doing (romacs-agent-log--doing romacs-agent-log--session))
        (width (romacs-agent-log--rule-width)))
    (when (or force (not (equal doing romacs-agent-log--state)) (not (eql width romacs-agent-log--width)))
      (setq romacs-agent-log--state doing romacs-agent-log--width width)
      (romacs-agent-log--writing
        (save-excursion
          ;; The new one put before the old one goes: a point in the
          ;; composer stays there, one at the transcript's end stays there.
          (let ((start (marker-position romacs-agent-log--end))
                (old (- romacs-agent-log--input romacs-agent-log--end)))
            (goto-char start)
            ;; A field of its own: word motion and kills in the composer
            ;; stop at its start, as in a comint buffer; line motion
            ;; crosses it (a field binds it only within a line).
            (insert (propertize (romacs-agent-log--sealed (romacs-agent-log--separator-text doing width))
                                'field 'romacs-agent-log-separator))
            (delete-region (point) (+ (point) old))
            (set-marker romacs-agent-log--input (point))
            (set-marker romacs-agent-log--end start))))
      ;; The thread moves as the session does, drawn by the display.
      (romacs-working-set (current-buffer) (romacs-agent-log--work doing)))))

(defvar-local romacs-agent-log--title nil "The title the chat's name was made from.")
(defvar-local romacs-agent-log--header-made nil
  "What the header was last made from: made again only when it differs.")

(defun romacs-agent-log--header-show ()
  "Make the header say what the session says now: a value, never a form,
so a redisplay runs no Lisp for it; made only when what it reads changed."
  (let* ((session romacs-agent-log--session)
         (start (romacs-agent-turn-start session))
         (running (and (null romacs-agent-log--asks) (memq (romacs-agent-state session) '(starting running))))
         (inputs (list (romacs-agent-ui--header-inputs session) (length romacs-agent-log--asks)
                       (and start running (round (- (float-time) start))))))
    (unless (equal inputs romacs-agent-log--header-made)
      (setq romacs-agent-log--header-made inputs)
      (let ((header (string-replace "%" "%%" (romacs-agent-log--header))))
        (unless (equal-including-properties header header-line-format)
          (setq header-line-format header)
          (force-mode-line-update))))))

(defun romacs-agent-log--header ()
  "The header line: the session's own header, what waits on the user, and
how long the turn has run."
  (let* ((session romacs-agent-log--session)
         (pending (length romacs-agent-log--asks))
         (start (romacs-agent-turn-start session)))
    (concat (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs session))
            (if (> pending 0)
                (propertize (format "  ⚠ %d wait%s on you" pending (if (= pending 1) "s" ""))
                            'face 'romacs-agent-log-ask)
              "")
            (if (and start (zerop pending) (memq (romacs-agent-state session) '(starting running)))
                (propertize (format "  %ds · %s stops" (round (- (float-time) start))
                                    (substitute-command-keys "\\<romacs-agent-log-mode-map>\\[romacs-agent-log-interrupt]"))
                            'face 'romacs-agent-log-note)
              ""))))

;;; Events

(defun romacs-agent-log--on-event (session event)
  "Draw SESSION's EVENT in its chat, once: a record drawn already is not."
  (when-let* ((buffer (gethash (romacs-agent-id session) romacs-agent-log--chats))
              ((buffer-live-p buffer))
              ;; Not drawn yet: its fill will draw this.
              ((buffer-local-value 'romacs-agent-log--filled buffer)))
    (with-current-buffer buffer
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
          (romacs-agent-log--header-show))))))

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
  (let ((scratch (romacs-agent--scratch session)) (buffer (current-buffer))
        ;; A replay writes nothing of the session (its manifest).
        (romacs-agent--manifest-frozen t))
    ;; Its run at FROM, as the session numbered its requests then (Codex's).
    (setf (romacs-agent-driver-state scratch) (list :run (romacs-agent-log--run-at session from)))
    (let ((romacs-agent-event-functions
           (list (lambda (_ event)
                   (with-current-buffer buffer
                     (romacs-agent-log-draw session event)
                     (when-let* ((seq (plist-get event :seq))) (setq romacs-agent-log--seq (max romacs-agent-log--seq seq))))))))
      (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)) from end))
    (romacs-agent-log--close-stream)))

(defun romacs-agent-log--run-at (session from)
  "The run of SESSION's harness its records are in at place FROM: the seq
of the last run note before it in the store, else the live session's."
  (or (and from (romacs-agent-events-p session) (romacs-agent-events-note-seq session "run" from))
      (plist-get (romacs-agent-driver-state session) :run)))

(defun romacs-agent-log--earlier-line (from)
  (romacs-agent-log--sealed
   (propertize (concat (propertize (format "↑ earlier turns · %s shows them" (romacs-agent-log--key 'romacs-agent-log-open))
                                   'face 'romacs-agent-log-note)
                       "\n")
               'romacs-agent-log-earlier from 'romacs-agent-log-entry 'earlier)))

(defun romacs-agent-log--fill ()
  "Draw the current chat's session's last `romacs-agent-log-turns' turns."
  (let* ((session romacs-agent-log--session)
         (from (romacs-agent-log--start session (car (romacs-agent--journal-tail session romacs-agent-log-turns)))))
    (setq romacs-agent-log--from from)
    (when from (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line from)))))
    ;; From its start: an imported session's harness transcript first.
    (when-let* (((null from))
                (imported (romacs-agent-imported session))
                ((require 'romacs-agent-history nil t))
                (read (romacs-agent-history-events (romacs-agent-harness session) imported)))
      (when (cdr read)
        (romacs-agent-log--note nil (format "earlier turns are in %s" (abbreviate-file-name (plist-get imported :path)))))
      (dolist (event (car read)) (romacs-agent-log-draw session event))
      (romacs-agent-log--close-stream))
    (romacs-agent-log--replay session from nil)
    (romacs-agent-log--sync-asks session)))

(defun romacs-agent-log--start (session from)
  "Where SESSION's chat begins drawing its records: FROM, a turn's start,
unless more than `romacs-agent-log-records' records follow it in the
store, then that many before its end."
  (if-let* (((romacs-agent-events-p session))
            (end (romacs-agent-events-end session))
            ((> (- end (or from 0)) romacs-agent-log-records)))
      (- end romacs-agent-log-records)
    from))

(defun romacs-agent-log--sync-asks (session)
  "What SESSION waits on now is what its chat shows waiting, whatever its
records drew: a block no longer waited on settled, one waited on drawn."
  (let ((approvals (romacs-agent-approvals session)))
    (dolist (ask romacs-agent-log--asks)
      (unless (gethash (car ask) approvals) (romacs-agent-log--settle-ask (car ask))))
    (maphash (lambda (id request)
               (unless (assoc id romacs-agent-log--asks)
                 (romacs-agent-log--ask-block (romacs-agent-log--request-event id request))))
             approvals)))

(defun romacs-agent-log--request-event (id request)
  "An approval event for request ID, REQUEST as its harness asked it: a
table (Claude Code's), else (WIRE METHOD PARAMS) (Codex's); its questions
read as its harness's events read them."
  (if (hash-table-p request)
      (let ((tool (gethash "tool_name" request)) (input (gethash "input" request)))
        (list :type 'approval :request-id id :tool tool :input input
              :description (gethash "description" request) :plan (equal tool "ExitPlanMode")
              :questions (and (equal tool "AskUserQuestion") (romacs-agent--claude-questions input))))
    (pcase-let ((`(,_ ,method ,params) request))
      (list :type 'approval :request-id id :tool method :input params
            :questions (and (equal method "item/tool/requestUserInput") (romacs-agent--codex-questions params))))))

(defun romacs-agent-log-earlier ()
  "Show the turns before those this chat shows, above them."
  (interactive nil romacs-agent-log-mode)
  (let* ((session romacs-agent-log--session)
         (end romacs-agent-log--from)
         (tail (and end (romacs-agent--journal-tail session romacs-agent-log-turns end)))
         (from (let ((from (car tail)))
                 (if (and (romacs-agent-events-p session) (> (- end (or from 0)) romacs-agent-log-records))
                     (- end romacs-agent-log-records)
                   from))))
    (unless end (user-error "This chat shows the session from its start"))
    ;; Drawn into a buffer of its own, then put above: the live end is untouched.
    (let ((text (with-temp-buffer
                  (romacs-agent-log-mode)
                  (setq romacs-agent-log--session session
                        romacs-agent-log--end (point-max-marker)
                        romacs-agent-log--input (point-max-marker))
                  (set-marker-insertion-type romacs-agent-log--end t)
                  (romacs-agent-log--replay session from end)
                  (buffer-substring (point-min) romacs-agent-log--end))))
      (romacs-agent-log--writing
        (save-excursion
          (goto-char (point-min))
          (let ((line-end (line-beginning-position 2)))
            (when (get-text-property (point) 'romacs-agent-log-earlier) (delete-region (point) line-end)))
          (insert (if from (romacs-agent-log--earlier-line from) "") text))))
    (setq romacs-agent-log--from from)))

;;; The chat

(defun romacs-agent-log--buffer-name (session)
  (format "*agent: %s*" (or (romacs-agent-title session) (romacs-agent-id session))))

(defun romacs-agent-log-chat (session &optional later)
  "SESSION's chat buffer, made when it has none, and drawn from its
records now, or with LATER when a window first shows it."
  (let ((buffer (gethash (romacs-agent-id session) romacs-agent-log--chats)))
    (unless (buffer-live-p buffer)
      (setq buffer (generate-new-buffer (romacs-agent-log--buffer-name session)))
      (with-current-buffer buffer
        (romacs-agent-log-mode)
        (romacs-agent-log--set-session session)
        (setq romacs-agent-log--end (point-min-marker)
              romacs-agent-log--input (point-min-marker))
        (set-marker-insertion-type romacs-agent-log--end t)
        (romacs-agent-log--separator t)
        (romacs-working-show buffer buffer romacs-agent-log--end romacs-agent-log--thread-cols
                             (romacs-agent-log--work romacs-agent-log--state))
        (puthash (romacs-agent-id session) buffer romacs-agent-log--chats)
        (add-hook 'window-buffer-change-functions #'romacs-agent-log--shown nil t)))
    (unless later (with-current-buffer buffer (romacs-agent-log--ensure-filled)))
    buffer))

(defun romacs-agent-log--ensure-filled ()
  (unless romacs-agent-log--filled
    (setq romacs-agent-log--filled t)
    (romacs-agent-log--fill)
    (romacs-agent-log--separator t)
    (romacs-agent-log--header-show)))

(defun romacs-agent-log--refit (_frame)
  "A window started or stopped showing a buffer, or it or its frame went:
each chat's rule fits the windows that still show it."
  (maphash (lambda (_id buffer)
             (when (buffer-live-p buffer)
               (with-current-buffer buffer
                 (when romacs-agent-log--input (romacs-agent-log--separator)))))
           romacs-agent-log--chats))

(add-hook 'window-buffer-change-functions #'romacs-agent-log--refit)
(add-hook 'after-delete-frame-functions #'romacs-agent-log--refit)

(defun romacs-agent-log--shown (_window)
  "A window shows this chat: drawn now, when it was not, its header made
again, and its clock going while its turn runs."
  (romacs-agent-log--ensure-filled)
  (romacs-agent-log--header-show)
  (when (memq (romacs-agent-state romacs-agent-log--session) '(starting running waiting-approval))
    (romacs-agent-log--tick-start)))

(defun romacs-agent-log--headers-refresh ()
  "Make the header of every shown chat say what its session says now: run
after a batch of events remade sessions' states (their workers')."
  (dolist (window (window-list-1 nil 'nomini 'visible))
    (with-current-buffer (window-buffer window)
      (when (derived-mode-p 'romacs-agent-log-mode) (romacs-agent-log--header-show)))))

(add-hook 'romacs-agent-ui-changed-hook #'romacs-agent-log--headers-refresh)

(defun romacs-agent-log-show (session &optional action)
  "Show SESSION's chat, as `pop-to-buffer' does with ACTION."
  (let ((buffer (romacs-agent-log-chat session)))
    (pop-to-buffer buffer action)
    (goto-char (point-max))
    (recenter -1)
    (romacs-agent-mark-seen session)
    buffer))

;; A session outlives the editor in its supervisor: a restored chat
;; (desktop.el, warm-restart.el) carries which session it shows and the
;; composer's unsent text; its transcript is drawn from the records.
(defun romacs-agent-log--desktop-save (_dirname)
  (list :session (romacs-agent-id romacs-agent-log--session)
        :draft (buffer-substring-no-properties romacs-agent-log--input (point-max))))

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
(add-to-list 'desktop-buffer-mode-handlers '(romacs-agent-log-mode . romacs-agent-log-desktop-restore))

;;; Commands

(defun romacs-agent-log--loaded ()
  "The chat's session, loaded: a restored chat's is its row until used."
  (let ((id (romacs-agent-id romacs-agent-log--session)))
    (unless (eq (romacs-agent-loaded id) romacs-agent-log--session)
      (romacs-agent-log--set-session (or (romacs-agent-get id) romacs-agent-log--session)))
    romacs-agent-log--session))

(defun romacs-agent-log--draft ()
  (string-trim (buffer-substring-no-properties romacs-agent-log--input (point-max))))

(defun romacs-agent-log--deliver (send)
  "Call SEND with the composer's text, then empty it; it stays when SEND fails."
  (let ((text (romacs-agent-log--draft)))
    (unless (string-empty-p text)
      (funcall send text)
      (setq romacs-agent-log--recalled nil)
      (delete-region romacs-agent-log--input (point-max)))))

(defun romacs-agent-log--running-p ()
  (memq (romacs-agent-state romacs-agent-log--session) '(starting running waiting-approval)))

(defun romacs-agent-log-send ()
  "Send the composer's text: into the turn under way, when one runs."
  (interactive nil romacs-agent-log-mode)
  (if (< (point) romacs-agent-log--input)
      (romacs-agent-log-open)
    (romacs-agent-log--deliver (lambda (text) (romacs-agent-send (romacs-agent-log--loaded) text)))))

(defun romacs-agent-log-queue ()
  "Send the composer's text once the turn under way ends."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log--deliver (lambda (text) (romacs-agent-send-later (romacs-agent-log--loaded) text))))

(defun romacs-agent-log-interrupt ()
  "Interrupt the turn under way."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-interrupt (romacs-agent-log--loaded)))

(defun romacs-agent-log-interrupt-and-send ()
  "Interrupt the turn under way; the composer's text is sent once it ends."
  (interactive nil romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (if (not (romacs-agent-log--running-p))
        (romacs-agent-log--deliver (lambda (text) (romacs-agent-send session text)))
      (romacs-agent-interrupt session)
      (romacs-agent-log--deliver (lambda (text) (romacs-agent-send-later session text t))))))

(defun romacs-agent-log--pending ()
  "The approvals this chat shows that the live session still waits on,
the oldest first: (ID START . END)."
  (let ((approvals (romacs-agent-approvals romacs-agent-log--session)))
    (reverse (cl-remove-if-not (lambda (ask) (gethash (car ask) approvals)) romacs-agent-log--asks))))

(defun romacs-agent-log--ask-here ()
  "The approval to answer: the one at point, else the oldest pending; its
block is shown, and it is checked the session still waits on it."
  (let* ((at (get-text-property (point) 'romacs-agent-log-request))
         (pending (romacs-agent-log--pending))
         (ask (if at (assoc at pending) (car pending))))
    (unless ask (user-error (if at "That request is answered" "Nothing waits on you")))
    (unless at
      (goto-char (cadr ask))
      (message "Answering: %s" (string-trim (buffer-substring-no-properties (cadr ask) (line-end-position)))))
    (car ask)))

(defun romacs-agent-log-allow (&optional always)
  "Allow the request at point, else the oldest pending; with ALWAYS, for
the session.  Questions open their answering layer."
  (interactive nil romacs-agent-log-mode)
  (let* ((id (romacs-agent-log--ask-here))
         (questions (get-text-property (point) 'romacs-agent-log-questions)))
    (cond (questions (romacs-agent-ask-open (romacs-agent-log--loaded) id questions))
          (t (romacs-agent-respond (romacs-agent-log--loaded) id (if always (list :always t) t) nil 'user)))))

(defun romacs-agent-log-allow-always ()
  "Allow the request at point, else the oldest pending, for the session."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-allow t))

(defun romacs-agent-log-deny (reason)
  "Deny the request at point, else the oldest pending, telling the agent
REASON when given."
  (interactive (list (read-string "Deny, telling the agent (RET: nothing): ")) romacs-agent-log-mode)
  (let ((id (romacs-agent-log--ask-here)))
    (romacs-agent-respond (romacs-agent-log--loaded) id nil (and (not (string-empty-p reason)) reason) 'user)))

(defun romacs-agent-log-next-ask ()
  "Go to the next approval that waits on you, round them all."
  (interactive nil romacs-agent-log-mode)
  (let* ((pending (romacs-agent-log--pending))
         (next (or (cl-find-if (lambda (ask) (> (cadr ask) (point))) pending) (car pending))))
    (unless next (user-error "Nothing waits on you"))
    (goto-char (cadr next))
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

(defun romacs-agent-log-open ()
  "Open what is at point: earlier turns, or a call's whole input and output."
  (interactive nil romacs-agent-log-mode)
  (cond ((get-text-property (point) 'romacs-agent-log-earlier) (romacs-agent-log-earlier))
        ((get-text-property (point) 'romacs-agent-log-full)
         (let ((full (get-text-property (point) 'romacs-agent-log-full))
               (title (string-trim (buffer-substring-no-properties (line-beginning-position) (line-end-position)))))
           (with-current-buffer (get-buffer-create "*agent output*")
             (let ((inhibit-read-only t))
               (erase-buffer)
               (insert title "\n")
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
                  (or (previous-single-property-change (point) 'romacs-agent-log-entry) (point-min))))
         (end (or (next-single-property-change (1+ start) 'romacs-agent-log-entry) romacs-agent-log--end)))
    (kill-new (string-trim (buffer-substring-no-properties start (min end romacs-agent-log--end))))
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
  (when (and (markerp romacs-agent-log--input) (< beg end) (>= beg romacs-agent-log--input))
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

;;;; The actions menu

(defun romacs-agent-log--menu-title ()
  "The actions menu's title: whose chat it is."
  (concat "Agent · " (or (and romacs-agent-log--session (romacs-agent-title romacs-agent-log--session)) "chat")))

(defun romacs-agent-log--workers ()
  "The sessions this chat's delegated that are at work, those waiting on
the user first: a finished one is the board's."
  (when romacs-agent-log--session
    (let ((live (seq-filter (lambda (child) (memq (romacs-agent-state child) '(starting running waiting-approval)))
                            (romacs-agent-children romacs-agent-log--session))))
      (append (seq-filter (lambda (child) (eq (romacs-agent-state child) 'waiting-approval)) live)
              (seq-remove (lambda (child) (eq (romacs-agent-state child) 'waiting-approval)) live)))))

(defun romacs-agent-log--worker-label (index)
  "The menu's row of the worker at INDEX: a mark of what it does, then its
title, so a long title is what the menu cuts; nil when there is none."
  (when-let* ((child (nth index (romacs-agent-log--workers))))
    (concat (if (eq (romacs-agent-state child) 'waiting-approval)
                (propertize "⚠" 'face 'romacs-working-waiting)
              (propertize "●" 'face 'romacs-working-running))
            " " (romacs-agent-title child))))

(defun romacs-agent-log-open-worker ()
  "Open the chat of the worker on the menu row of the digit just pressed."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-show (or (nth (- last-command-event ?1) (romacs-agent-log--workers))
                         (user-error "No such worker"))
                     '(display-buffer-same-window)))

(keymap-popup-define romacs-agent-log-actions-map
  "Actions of an agent chat: what waits on you, the turn, what it runs on,
the session, the view."
  :popup-key "?"
  :exit-key "C-g"
  :description #'romacs-agent-log--menu-title
  :group ("Waits on you" :if (lambda () (and romacs-agent-log--session (romacs-agent-log--pending))))
  "y" ("Allow" romacs-agent-log-allow)
  "!" ("Allow for this session" romacs-agent-log-allow-always)
  "d" ("Deny" romacs-agent-log-deny)
  "j" ("Go to it" romacs-agent-log-next-ask)
  :group "Turn"
  "i" ("Interrupt" romacs-agent-log-interrupt :inapt-if (lambda () (not (romacs-agent-log--running-p))))
  "k" ("Interrupt, then send" romacs-agent-log-interrupt-and-send)
  "q" ("Send when the turn ends" romacs-agent-log-queue)
  :group "Runs on"
  "m" ("Model" romacs-agent-chat-model)
  "E" ("Effort" romacs-agent-chat-effort)
  "P" ("Plan mode on or off" romacs-agent-chat-plan)
  "H" ("Go on under another instance" romacs-agent-chat-handoff)
  "N" ("Next account with room" romacs-agent-chat-next-account)
  :row
  :group "Session"
  "b" ("Agent board" romacs-agents)
  "n" ("New session" romacs-agent-chat-new)
  "R" ("Rename" romacs-agent-chat-rename)
  "f" ("Fork" romacs-agent-chat-fork)
  "F" ("Fresh conversation, briefed" romacs-agent-chat-renew)
  "r" ("Resume" romacs-agent-chat-resume :inapt-if #'romacs-agent-log--running-p)
  "x" ("Stop" romacs-agent-chat-stop)
  :group "View"
  "e" ("Earlier turns" romacs-agent-log-earlier)
  "w" ("Copy the entry at point" romacs-agent-log-copy)
  :group ("Workers" :if #'romacs-agent-log--workers)
  "1" ((lambda () (romacs-agent-log--worker-label 0)) romacs-agent-log-open-worker)
  "2" ((lambda () (romacs-agent-log--worker-label 1)) romacs-agent-log-open-worker)
  "3" ((lambda () (romacs-agent-log--worker-label 2)) romacs-agent-log-open-worker)
  "4" ((lambda () (romacs-agent-log--worker-label 3)) romacs-agent-log-open-worker)
  "5" ((lambda () (romacs-agent-log--worker-label 4)) romacs-agent-log-open-worker)
  "6" ((lambda () (romacs-agent-log--worker-label 5)) romacs-agent-log-open-worker)
  "7" ((lambda () (romacs-agent-log--worker-label 6)) romacs-agent-log-open-worker)
  "8" ((lambda () (romacs-agent-log--worker-label 7)) romacs-agent-log-open-worker)
  "9" ((lambda () (romacs-agent-log--worker-label 8)) romacs-agent-log-open-worker))

(defvar-keymap romacs-agent-log-transcript-map
  :doc "Keys of a chat's transcript: a `keymap' text property, as a view's.
A character typed on it goes to the composer."
  "<remap> <self-insert-command>" #'romacs-agent-log-type
  "SPC" #'scroll-up-command
  "DEL" #'scroll-down-command
  "?" #'romacs-agent-log-actions-map-popup
  "n" #'romacs-agent-log-next
  "p" #'romacs-agent-log-previous
  "M-n" #'romacs-agent-log-next-prompt
  "M-p" #'romacs-agent-log-previous-prompt
  "RET" #'romacs-agent-log-open
  "w" #'romacs-agent-log-copy
  "e" #'romacs-agent-log-earlier
  "g" #'romacs-agent-log-composer)

(defvar-keymap romacs-agent-log-ask-map
  :doc "Keys of an approval waiting on the user."
  :parent romacs-agent-log-transcript-map
  "y" #'romacs-agent-log-allow
  "!" #'romacs-agent-log-allow-always
  "d" #'romacs-agent-log-deny)

(defvar-keymap romacs-agent-log-mode-map
  :doc "Keys of an agent chat; the composer edits as any buffer does."
  "RET" #'romacs-agent-log-send
  "C-j" #'newline
  "S-<return>" #'newline
  "C-c C-c" #'romacs-agent-log-interrupt
  "C-c C-k" #'romacs-agent-log-interrupt-and-send
  "C-c C-q" #'romacs-agent-log-queue
  "C-c C-y" #'romacs-agent-log-allow
  "C-c C-d" #'romacs-agent-log-deny
  "C-c C-p" #'romacs-agent-log-next-ask
  "C-c C-j" #'romacs-agent-log-composer
  "C-c C-b" #'romacs-agents
  "C-c C-o" #'romacs-agent-log-actions-map-popup
  "M-p" #'romacs-agent-log-history-previous
  "M-n" #'romacs-agent-log-history-next)

(put 'romacs-agent-log-actions-map-popup 'command-modes '(romacs-agent-log-mode))

(define-derived-mode romacs-agent-log-mode fundamental-mode "Agent"
  "An agent session's chat: its transcript, and below, the composer.
\\{romacs-agent-log-mode-map}
On the transcript's text:
\\{romacs-agent-log-transcript-map}"
  :interactive nil
  (setq-local truncate-lines nil word-wrap t)
  (setq-local scroll-conservatively 101)
  (setq-local header-line-format "")
  (setq-local desktop-save-buffer #'romacs-agent-log--desktop-save)
  ;; The separator's rule fits the window it is shown in.
  (add-hook 'window-size-change-functions (lambda (_) (when romacs-agent-log--input (romacs-agent-log--separator))) nil t)
  (add-hook 'after-change-functions #'romacs-agent-log--composer-plain nil t))

(provide 'romacs-agent-log)
;;; romacs-agent-log.el ends here

;;; romacs-agent-turns.el --- A session's turns, a row each, in SQLite  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each turn of a session is a row of the `turns' table of the agents'
;; sessions.sqlite: where it begins and ends in the session's records
;; (seqs), when, its prompt, its reply as the chat shows it, a line a
;; tool call, its cost.  The rows are written from a session's records,
;; replayed after its latest row into a copy, from the events every
;; harness's adapter gives (input, tool, done), whatever holds the records
;; (a journal or SQLite): by a batch process, a moment after a turn ends
;; and at a restore, never on the editor; a turn becomes a row only once
;; it ends.  So where a chat's last turns begin is one indexed query, and
;; text can be searched (`turns_fts').

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-store)
(require 'romacs-agent-drawn)

(defconst romacs-agent-turns--schema
  '("create table if not exists turns (
       session text not null, first_seq integer not null, last_seq integer, t real,
       prompt text, reply text, tools text, cost real, primary key (session, first_seq)) without rowid")
  "The turns' table, made with the database.")

(defconst romacs-agent-turns--added
  '(("fold" . "text") ("items" . "text") ("images" . "text") ("messages" . "text") ("duration" . "real")
    ("status" . "text") ("reason" . "text") ("request" . "integer"))
  "The turns' columns added after the table was first made, each (NAME
. TYPE): added where missing as the database opens.")

(defconst romacs-agent-turns--search-schema
  '("create virtual table if not exists turns_fts using fts5 (session unindexed, first_seq unindexed, prompt, reply, tools)")
  "The turns' text, searchable: made where SQLite has FTS5.")

(defvar romacs-agent-turns--made nil "The database the turns' tables were made in.")

(defun romacs-agent-turns--db ()
  "The database, its turns' tables made; nil without SQLite."
  (when-let* ((db (romacs-agent-store--open)))
    ;; The editor reads: the writer made them (`romacs-agent-store--made').
    (unless (or (eq romacs-agent-turns--made db) (romacs-agent-store-readonly-p))
      (dolist (statement romacs-agent-turns--schema) (sqlite-execute db statement))
      ;; Columns added since: the session's folded state, on one row of its
      ;; own (`romacs-agent-turns-fold-put'), and what a chat draws a turn
      ;; from (`romacs-agent-turns-page').
      (let ((columns (mapcar #'cadr (sqlite-select db "pragma table_info(turns)"))))
        (dolist (column romacs-agent-turns--added)
          (unless (member (car column) columns)
            (sqlite-execute db (format "alter table turns add column %s %s" (car column) (cdr column))))))
      (ignore-errors (dolist (statement romacs-agent-turns--search-schema) (sqlite-execute db statement)))
      (romacs-agent-drawn-table db)
      (setq romacs-agent-turns--made db))
    db))

(add-hook 'romacs-agent-store-made-functions #'romacs-agent-turns--db)

;;; Writing

(defvar romacs-agent-turns--open (make-hash-table :test #'eq :weakness 'key)
  "Each session's turn under way: a plist of :first, :prompt, :time,
:images, :tools (each (ID NAME TEXT STATE), the latest first), :items
\(what a chat draws of it, the latest first) and :by-id (its tools' and
asks' items, by id).")

;;; What a chat draws of a turn: its items

;; A turn's row holds its items in order, each a JSON object with its
;; `kind', the seqs of its first and last records (`seq') and its time
;; (`t'): text and thinking (`message', `parent', `text'), a tool (`id',
;; `name', `parent', `state', `t_end', `output_bytes', its `input' bounded,
;; and its `output' when short), an ask (`request_id', `tool',
;; `questions', `answers', `behavior'), an input written into the turn
;; (`text', `steer', `request', `images'), and plan, status, error and
;; compacted (`text').  A tool's long output is read when asked for
;; (`romacs-agent-turns-item').

(defconst romacs-agent-turns--cut 2048
  "The characters of a string a tool's input keeps in its item.")

(defconst romacs-agent-turns--inline 1024
  "The bytes of output up to which a tool's item holds it.")

(defconst romacs-agent-turns--bodies '("old_string" "new_string" "content")
  "The keys of an editing tool's input whose text its item holds as a size.")

(defun romacs-agent-turns--bounded (value &optional editing key)
  "VALUE, a tool's input or part of it, as its item keeps it: strings cut
at `romacs-agent-turns--cut' characters, and for an EDITING tool the text
under KEY in `romacs-agent-turns--bodies' replaced by its size in bytes."
  (cond ((and editing (stringp value) (member key romacs-agent-turns--bodies)) (string-bytes value))
        ((stringp value) (if (> (length value) romacs-agent-turns--cut)
                             (concat (substring value 0 romacs-agent-turns--cut) "…")
                           value))
        ((hash-table-p value)
         (let ((copy (make-hash-table :test #'equal :size (hash-table-count value))))
           (maphash (lambda (k v) (puthash k (romacs-agent-turns--bounded v editing k) copy)) value)
           copy))
        ((vectorp value) (vconcat (mapcar (lambda (v) (romacs-agent-turns--bounded v editing)) value)))
        ((null value) :null)
        ((or (numberp value) (memq value '(t :null :false))) value)
        ;; A plist (a message's sender, its wake): an object of its fields.
        ((and (consp value) (keywordp (car value)) (proper-list-p value))
         (let ((object (romacs-mcp--object)))
           (cl-loop for (key field) on value by #'cddr
                    when (keywordp key)
                    do (puthash (substring (symbol-name key) 1) (romacs-agent-turns--bounded field editing) object))
           object))
        ;; Nothing else is JSON: said as text.
        (t (format "%S" value))))

(defun romacs-agent-turns--questions (questions)
  "An approval's QUESTIONS, each (QUESTION :header HEADER :options LABELS
:notes ((LABEL . DESCRIPTION) ...) :multi MULTI), as its item keeps them."
  (vconcat (mapcar (lambda (question)
                     (let ((notes (romacs-mcp--object)))
                       (pcase-dolist (`(,label . ,description) (plist-get (cdr question) :notes))
                         (when (stringp label) (puthash label (format "%s" description) notes)))
                       (romacs-mcp--object "question" (format "%s" (car question))
                                           "header" (romacs-agent-turns--null (plist-get (cdr question) :header))
                                           "options" (vconcat (seq-filter #'stringp (plist-get (cdr question) :options)))
                                           "notes" notes
                                           "multi" (if (plist-get (cdr question) :multi) t :false))))
                   (seq-filter #'consp questions))))

(defun romacs-agent-turns--messages (messages)
  "An input's MESSAGES (who said what in it), each a plist, as JSON
objects of their fields."
  (vconcat (mapcar (lambda (message)
                     (let ((object (romacs-mcp--object)))
                       (cl-loop for (key value) on message by #'cddr
                                when (keywordp key)
                                do (puthash (substring (symbol-name key) 1) (romacs-agent-turns--bounded value) object))
                       object))
                   (seq-filter #'consp messages))))

(defun romacs-agent-turns--images (images)
  "An input's IMAGES (their files), as a vector of their names."
  (vconcat (seq-filter #'stringp images)))

(defun romacs-agent-turns--add (turn kind event &rest fields)
  "A new item of KIND (a string) for EVENT in TURN, with FIELDS (key, value
pairs), as the latest: returned."
  (let ((item (apply #'romacs-mcp--object "kind" kind
                     "seq" (vector (plist-get event :seq) (plist-get event :seq))
                     "t" (or (plist-get event :time) :null)
                     fields)))
    (plist-put turn :items (cons item (plist-get turn :items)))
    item))

(defun romacs-agent-turns--grow (item event)
  "ITEM's records go on to EVENT's."
  (aset (gethash "seq" item) 1 (plist-get event :seq)))

(defun romacs-agent-turns--null (value) (or value :null))

(defun romacs-agent-turns--text (turn kind event)
  "EVENT's text, joined to TURN's latest item when it is of KIND (text or
thinking) and of the same message and parent, else a new one."
  (let ((last (car (plist-get turn :items)))
        (message (romacs-agent-turns--null (plist-get event :message)))
        (parent (romacs-agent-turns--null (plist-get event :parent))))
    (if (and last (equal (gethash "kind" last) kind) (equal (gethash "parent" last) parent)
             (or (eq message :null) (equal (gethash "message" last) message)))
        ;; Its pieces kept, the newest first, joined once at its turn's end.
        (progn (puthash "text" (cons (or (plist-get event :text) "") (gethash "text" last)) last)
               (romacs-agent-turns--grow last event))
      (romacs-agent-turns--add turn kind event "message" message "parent" parent "text" (list (or (plist-get event :text) ""))))))

(defvar romacs-agent-turns-tool-texts-function nil
  "Called with a tool's name, its whole input and its output (nil while it
runs), as the fill has them: a plist of :row and :ended, the texts its row
shows (counts the bounded input cannot give), kept in its item; nil, none.
The chat's rows set it (romacs-agent-rows).")

(require 'romacs-agent-rows nil t)

(defun romacs-agent-turns--texts (item name input output)
  "Keep in ITEM its row's texts for tool NAME of INPUT and OUTPUT
\(`romacs-agent-turns-tool-texts-function')."
  (when-let* ((texts (and romacs-agent-turns-tool-texts-function
                          (ignore-errors (funcall romacs-agent-turns-tool-texts-function name input output)))))
    (puthash "row" (romacs-agent-turns--null (plist-get texts :row)) item)
    (puthash "ended" (romacs-agent-turns--null (plist-get texts :ended)) item)))

(defun romacs-agent-turns--item (turn event)
  "Fold EVENT into TURN's items."
  (let ((by-id (plist-get turn :by-id)))
    (pcase (plist-get event :type)
      ('delta (romacs-agent-turns--text turn "text" event))
      ('thinking (romacs-agent-turns--text turn "thinking" event))
      ('tool
       (let ((known (gethash (plist-get event :id) by-id))
             (inputs (plist-get turn :inputs)))
         (if (not known)
             (let ((item (romacs-agent-turns--add
                       turn "tool" event "id" (plist-get event :id) "name" (romacs-agent-turns--null (plist-get event :name))
                       "parent" (romacs-agent-turns--null (plist-get event :parent))
                       "state" (if (memq (plist-get event :state) '(completed failed)) (symbol-name (plist-get event :state)) "running")
                       "t_end" :null "output_bytes" 0
                       "input" (romacs-agent-turns--bounded (plist-get event :input)
                                                            (member (plist-get event :name) '("Edit" "Write" "MultiEdit" "NotebookEdit"))))))
               ;; Its whole input, kept until its turn's row is written.
               (puthash (plist-get event :id) (cons (plist-get event :name) (plist-get event :input)) inputs)
               (romacs-agent-turns--texts item (plist-get event :name) (plist-get event :input) (plist-get event :output))
               (puthash (plist-get event :id) item by-id))
           (when (memq (plist-get event :state) '(completed failed))
             (let ((output (or (plist-get event :output) ""))
                   (whole (gethash (plist-get event :id) inputs)))
               ;; Its input as it ended, when its end gives one.
               (when (and whole (plist-get event :input))
                 (setcdr whole (plist-get event :input))
                 (puthash "input" (romacs-agent-turns--bounded (plist-get event :input)
                                                               (member (car whole) '("Edit" "Write" "MultiEdit" "NotebookEdit")))
                          known))
               (romacs-agent-turns--texts known (car whole) (cdr whole) output)
               (puthash "state" (symbol-name (plist-get event :state)) known)
               (puthash "t_end" (or (plist-get event :time) :null) known)
               (puthash "output_bytes" (string-bytes output) known)
               (when (<= (string-bytes output) romacs-agent-turns--inline) (puthash "output" output known))
               (romacs-agent-turns--grow known event))))))
      ('approval
       (puthash (plist-get event :request-id)
                (romacs-agent-turns--add turn "ask" event "request_id" (plist-get event :request-id)
                                         "tool" (romacs-agent-turns--null (plist-get event :tool))
                                         "user" (if (plist-get event :user) t :false)
                                         "plan" (if (plist-get event :plan) t :false)
                                         "input" (romacs-agent-turns--bounded
                                                  (plist-get event :input)
                                                  (member (plist-get event :tool) '("Edit" "Write" "MultiEdit" "NotebookEdit")))
                                         "description" (romacs-agent-turns--null (plist-get event :description))
                                         "questions" (romacs-agent-turns--questions (plist-get event :questions))
                                         "answers" :null "behavior" :null)
                by-id))
      ('answered
       (when-let* ((ask (gethash (plist-get event :request-id) by-id)))
         (puthash "answers" (romacs-agent-turns--bounded (plist-get event :answers)) ask)
         (puthash "behavior" (romacs-agent-turns--null (plist-get event :behavior)) ask)
         (romacs-agent-turns--grow ask event)))
      ('input (romacs-agent-turns--add turn "input" event "text" (or (plist-get event :text) "")
                                       "steer" (if (plist-get event :steer) t :false)
                                       "request" (romacs-agent-turns--null (plist-get event :request))
                                       "images" (romacs-agent-turns--images (plist-get event :images))
                                       "messages" (romacs-agent-turns--messages (plist-get event :messages))))
      ((and type (or 'plan 'status 'error 'compacted))
       (romacs-agent-turns--add turn (symbol-name type) event "text" (or (plist-get event :text) ""))))))

(defun romacs-agent-turns--ended-items (turn)
  "TURN's items in order, a tool still running at its end interrupted, a
text's pieces joined."
  (let ((items (reverse (plist-get turn :items))))
    (dolist (item items)
      (when (equal (gethash "state" item) "running") (puthash "state" "interrupted" item))
      (when (listp (gethash "text" item)) (puthash "text" (apply #'concat (reverse (gethash "text" item))) item)))
    (vconcat items)))

(defun romacs-agent-turns--status (done)
  "How the turn DONE (its end's event) ended: completed, failed or
interrupted."
  (cond ((plist-get done :interrupted) "interrupted")
        ((plist-get done :error) "failed")
        (t "completed")))

(defun romacs-agent-turns--summary (input)
  "A tool call's INPUT (a table), as one short line: its first text."
  (let ((text (and (hash-table-p input)
                   (catch 'found
                     (maphash (lambda (_ value) (when (and (stringp value) (not (string-empty-p value))) (throw 'found value)))
                              input)
                     nil))))
    (if text (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " text) 60 nil nil "…") "")))

(defun romacs-agent-turns--tools-text (tools)
  "TOOLS, a turn's calls (the latest first), a line each: `NAME · TEXT · ok'."
  (mapconcat (lambda (tool)
               (pcase-let ((`(,_ ,name ,text ,state) tool))
                 (string-join (delq nil (list name (and (not (string-empty-p text)) text)
                                              (pcase state ('completed "ok") ('failed "failed") (_ "…"))))
                              " · ")))
             (reverse tools) "\n"))

(defvar romacs-agent-turns--held :none
  "Rows made while a fill reads the records, written once it is done: a
read of records in SQLite holds a snapshot no write can be made inside.
`:none' outside a fill.")

(defvar romacs-agent-turns-draw-function nil
  "Called with a session and one of its turn rows, as
`romacs-agent-turns-page' gives them, for the row's drawn form
\(`romacs-agent-drawn-form'): bound where turn rows are written off the
editor, by a worker VM or the fill's batch process, to
`romacs-agent-ui-draw-turn'.  Nil in the editor, which draws no finished
turn but the one it just showed.")

(defun romacs-agent-turns--page-row (row)
  "ROW, a turn's columns first_seq, last_seq, t, prompt, reply, items, cost,
images, messages, duration, status, reason and request, as
`romacs-agent-turns-page' gives it."
  (pcase-let ((`(,first ,last ,time ,prompt ,reply ,items ,cost ,images ,messages ,duration ,status ,reason ,request) row))
    (list :first-seq first :last-seq last :t time :prompt prompt :reply reply
          :items (romacs-agent-turns--parse items) :cost cost
          :images (romacs-agent-turns--parse images) :messages (romacs-agent-turns--parse messages)
          :duration duration :status status :reason reason :request request)))

(defconst romacs-agent-turns--page-columns
  "first_seq, last_seq, t, prompt, reply, items, cost, images, messages, duration, status, reason, request"
  "The columns of a row `romacs-agent-turns--page-row' reads.")

(defun romacs-agent-turns--draw (session row)
  "SESSION's turn ROW (as `romacs-agent-turns--put' writes it) drawn, or nil:
no drawing here, or the drawing failed (the chat then shows the turn as
not drawn yet)."
  (when romacs-agent-turns-draw-function
    (pcase-let ((`(,_ ,first ,last ,time ,prompt ,reply ,_tools ,cost ,items ,images ,messages ,duration ,status ,reason ,request) row))
      (condition-case err
          (funcall romacs-agent-turns-draw-function session
                   (romacs-agent-turns--page-row (list first last time prompt reply items cost images messages
                                                       duration status reason request)))
        (error (message "romacs-agent-turns: drawing %s %s: %s" (car row) first (error-message-string err)) nil)))))

(defun romacs-agent-turns--write (session last)
  "Write SESSION's turn under way, ended at seq LAST, as its row, drawn
where rows are drawn (`romacs-agent-turns-draw-function'): at once, or as
a fill ends (`romacs-agent-turns--held')."
  (when-let* ((turn (gethash session romacs-agent-turns--open)))
    (remhash session romacs-agent-turns--open)
    (let* ((done (plist-get turn :done))
           (started (plist-get turn :time))
           (row (list (romacs-agent-id session) (plist-get turn :first) last (or started (float-time))
                      (plist-get turn :prompt) (romacs-agent-reply session)
                      (romacs-agent-turns--tools-text (plist-get turn :tools)) (romacs-agent-cost session)
                      (romacs-agent--json (romacs-agent-turns--ended-items turn))
                      (romacs-agent--json (romacs-agent-turns--images (plist-get turn :images)))
                      (romacs-agent--json (romacs-agent-turns--messages (plist-get turn :messages)))
                      (and started (plist-get done :time) (- (plist-get done :time) started))
                      (romacs-agent-turns--status done) (plist-get done :reason)
                      (plist-get turn :request))))
      (let ((entry (cons row (romacs-agent-turns--draw session row))))
        (if (listp romacs-agent-turns--held) (push entry romacs-agent-turns--held)
          (romacs-agent-turns--put (list entry)))))))

(defun romacs-agent-turns--put (entries)
  "Write ENTRIES, each (ROW . FORM): a turn's row, its text's index and its
drawn form when it has one (`romacs-agent-drawn-put'), in one
transaction: never two of one, nor a form without its row."
  (when-let* ((db (romacs-agent-turns--db)))
    (condition-case err
        (progn
          (sqlite-execute db "begin immediate")
          (pcase-dolist (`(,row . ,form) entries)
            ;; Its folded state, on it, is left as it is.
            (sqlite-execute db (concat "insert into turns (session, first_seq, last_seq, t, prompt, reply, tools, cost,"
                                       " items, images, messages, duration, status, reason, request)"
                                       " values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) on conflict (session, first_seq) do update set"
                                       " last_seq = excluded.last_seq, t = excluded.t, prompt = excluded.prompt,"
                                       " reply = excluded.reply, tools = excluded.tools, cost = excluded.cost,"
                                       " items = excluded.items, images = excluded.images, messages = excluded.messages,"
                                       " duration = excluded.duration,"
                                       " status = excluded.status, reason = excluded.reason, request = excluded.request")
                            row)
            (when (sqlite-select db "select 1 from sqlite_master where name = 'turns_fts'")
              (sqlite-execute db "delete from turns_fts where session = ? and first_seq = ?" (list (nth 0 row) (nth 1 row)))
              (sqlite-execute db "insert into turns_fts values (?, ?, ?, ?, ?)"
                              (list (nth 0 row) (nth 1 row) (nth 4 row) (nth 5 row) (nth 6 row))))
            (when form (romacs-agent-drawn-put db (nth 0 row) (nth 1 row) form)))
          (sqlite-execute db "commit")
          (pcase-dolist (`(,row . ,form) entries)
            (when form (romacs-agent-turns--drawn (nth 0 row) (list (nth 1 row))))))
      (error (ignore-errors (sqlite-execute db "rollback"))
             (signal (car err) (cdr err))))))

(defun romacs-agent-turns-drawn-keep (id first-seq form)
  "Keep FORM as session ID's turn from FIRST-SEQ drawn (`romacs-agent-drawn-put'):
the writer's, for a turn the editor just drew."
  (let ((db (or (romacs-agent-turns--db) (error "No agents store to keep a drawn turn in"))))
    (romacs-agent-drawn-put db id first-seq form)
    t))

(defvar romacs-agent-turns--drawn-kept :none
  "The forms the fill's batch process kept, to tell the editor: a list of
\(ID . FIRST-SEQ), newest first; `:none' where none is told.")

(defun romacs-agent-turns--drawn (id first-seqs)
  "Note that session ID's turns from FIRST-SEQS were kept drawn."
  (when (listp romacs-agent-turns--drawn-kept)
    (dolist (first first-seqs) (push (cons id first) romacs-agent-turns--drawn-kept))))

(defun romacs-agent-turns--drawn-told (output)
  "Tell the chats of the turns OUTPUT, the fill's batch process's, says
were kept drawn (`romacs-agent-ui-drawn-ready'): they show them."
  (when (fboundp 'romacs-agent-ui-drawn-ready)
    (with-temp-buffer
      (insert output)
      (goto-char (point-min))
      (while (re-search-forward "^(romacs-agent-turns-drawn " nil t)
        (goto-char (match-beginning 0))
        (pcase (condition-case nil (read (current-buffer)) (error nil))
          (`(romacs-agent-turns-drawn ,(and id (pred stringp)) ,(and seqs (pred consp)))
           (with-demoted-errors "romacs-agent-turns: telling a chat: %S" (romacs-agent-ui-drawn-ready id seqs)))
          (_ (forward-line 1)))))))

(defun romacs-agent-turns--drawn-wanted (id _first-seqs)
  "A chat shows session ID's turns with no form of this version: its rows
are filled and drawn soon, by the fill's batch process."
  (romacs-agent-turns-fill-soon (list id)))

(add-hook 'romacs-agent-drawn-wanted-functions #'romacs-agent-turns--drawn-wanted)

(defun romacs-agent-turns--event (session event)
  "Fold EVENT into SESSION's turn under way; a turn's end writes it."
  (when (romacs-agent-dir session)
    (let ((turn (gethash session romacs-agent-turns--open)) (type (plist-get event :type)))
      (cond
       ;; A prompt begins a turn; an input written into one is its item.
       ((and (eq type 'input) (not turn))
        (unless (plist-get event :steer)
          (puthash session (list :first (plist-get event :seq) :prompt (plist-get event :text) :tools nil
                                 :time (plist-get event :time) :images (plist-get event :images)
                                 :messages (plist-get event :messages)
                                 :request (plist-get event :request)
                                 :items nil :by-id (make-hash-table :test #'equal)
                                 :inputs (make-hash-table :test #'equal) :done nil)
                   romacs-agent-turns--open)))
       ;; Only its end makes a turn a row: one cut by an exit may yet be
       ;; resumed and end, its row written then.
       ((eq type 'done)
        (when turn (plist-put turn :done event))
        (romacs-agent-turns--write session (or (plist-get event :seq) (romacs-agent-seq session))))
       (turn
        (romacs-agent-turns--item turn event)
        (when (eq type 'tool)
          (let* ((tools (plist-get turn :tools)) (known (assoc (plist-get event :id) tools)))
            (if known (setf (nth 3 known) (plist-get event :state))
              (push (list (plist-get event :id) (plist-get event :name)
                          (romacs-agent-turns--summary (plist-get event :input)) (plist-get event :state))
                    tools))
            (plist-put turn :tools tools))))))))

(defvar romacs-agent-turns--due nil "The ids of sessions whose rows are to be caught up, by a timer.")

(defun romacs-agent-turns--ended (session event)
  "A live turn's end: SESSION's rows are caught up a moment later, by a
batch process (`romacs-agent-turns-fill-soon'), not here."
  (when (and (eq (plist-get event :type) 'done) (romacs-agent-dir session)
             (eq (romacs-agent-loaded (romacs-agent-id session)) session))
    (unless romacs-agent-turns--due
      (run-with-timer 2 nil (lambda () (let ((ids romacs-agent-turns--due))
                                         (setq romacs-agent-turns--due nil)
                                         (romacs-agent-turns-fill-soon ids)))))
    (cl-pushnew (romacs-agent-id session) romacs-agent-turns--due :test #'equal)))

(add-hook 'romacs-agent-event-functions #'romacs-agent-turns--ended)

;;; Filling

(defun romacs-agent-turns--last (session)
  "The seq SESSION's latest row ends at, or nil with no row."
  (when-let* ((db (romacs-agent-turns--db)))
    (caar (sqlite-select db "select max(last_seq) from turns where session = ? and first_seq > 0" (list (romacs-agent-id session))))))

(defun romacs-agent-turns-fill (session)
  "Write the rows of SESSION's turns its rows do not have yet: its records
after its latest row (all of them with none) replayed into a copy, as
they came.  The number written."
  (let* ((last (romacs-agent-turns--last session))
         (from (if last (romacs-agent--journal-after session last) 0)))
    (if (not from) 0
      (let ((scratch (romacs-agent--scratch session)) (before (romacs-agent-turns-count session)))
        (let ((romacs-agent-event-functions (list #'romacs-agent-turns--event)) (romacs-agent-turns--held nil))
          ;; A message said whole later is read whole, not each of its
          ;; stream's deltas (its harness's `:streamed-skip'), as a chat's
          ;; replay reads it.
          (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)) from nil
                                      (romacs-agent--harness-get session :streamed-skip))
          (when romacs-agent-turns--held (romacs-agent-turns--put (nreverse romacs-agent-turns--held))))
        ;; A turn under way at the records' end is not one yet.
        (remhash scratch romacs-agent-turns--open)
        (- (romacs-agent-turns-count session) before)))))

(defvar romacs-agent-events-store)

(defconst romacs-agent-turns--redraw-chunk 50
  "The forms a redraw writes a transaction.")

(defun romacs-agent-turns-redraw (session)
  "Draw SESSION's turn rows that have no form of this drawing's version
\(`romacs-agent-drawn-version'), from their rows, where rows are drawn
\(`romacs-agent-turns-draw-function'), `romacs-agent-turns--redraw-chunk'
forms a transaction.  The number drawn."
  (let ((db (romacs-agent-turns--db)) (id (romacs-agent-id session)) (drawn 0))
    (when (and db romacs-agent-turns-draw-function)
      (let ((todo (mapcar #'car (if (sqlite-select db "select 1 from sqlite_master where name = 'turn_drawn'")
                                     (sqlite-select db (concat "select t.first_seq from turns t left join turn_drawn d"
                                                               " on d.session = t.session and d.first_seq = t.first_seq"
                                                               " where t.session = ? and t.first_seq > 0"
                                                               " and (d.version is null or d.version <> ?) order by t.first_seq")
                                                    (list id romacs-agent-drawn-version))
                                   (sqlite-select db "select first_seq from turns where session = ? and first_seq > 0 order by first_seq" (list id))))))
        (while todo
          (let* ((chunk (seq-take todo romacs-agent-turns--redraw-chunk))
                 (rows (sqlite-select db (format "select %s from turns where session = ? and first_seq in (%s)"
                                                 romacs-agent-turns--page-columns (mapconcat (lambda (_) "?") chunk ","))
                                      (cons id chunk)))
                 (forms (delq nil (mapcar (lambda (row)
                                            (when-let* ((form (condition-case err
                                                                  (funcall romacs-agent-turns-draw-function session (romacs-agent-turns--page-row row))
                                                                (error (message "romacs-agent-turns: drawing %s %s: %s" id (car row) (error-message-string err))
                                                                       nil))))
                                              (cons (car row) form)))
                                          rows))))
            (setq todo (nthcdr romacs-agent-turns--redraw-chunk todo))
            (when forms
              (condition-case err
                  (progn
                    (sqlite-execute db "begin immediate")
                    (pcase-dolist (`(,first . ,form) forms) (romacs-agent-drawn-put db id first form))
                    (sqlite-execute db "commit")
                    (romacs-agent-turns--drawn id (mapcar #'car forms))
                    (cl-incf drawn (length forms)))
                (error (ignore-errors (sqlite-execute db "rollback"))
                       (signal (car err) (cdr err)))))))))
    drawn))

(defun romacs-agent-turns-fill-batch ()
  "`romacs-agent-turns-fill' for the sessions the command line names (all
recorded ones, with none), in a batch process: each fills from where its
rows end, nothing when its records end there too, and its rows with no
form of this drawing's version are drawn (`romacs-agent-turns-redraw').
It moves no session's records to SQLite: the editor alone does."
  (require 'romacs-agent-ui)
  (let ((written 0) (drawn 0) (romacs-agent-events-store nil) (t0 (float-time))
        (romacs-agent-turns-draw-function #'romacs-agent-ui-draw-turn)
        (romacs-agent-turns--drawn-kept nil)
        (ids (prog1 command-line-args-left (setq command-line-args-left nil))))
    (dolist (dir (if ids (mapcar (lambda (id) (expand-file-name id (romacs-agent--state-root))) ids)
                   (directory-files (romacs-agent--state-root) t "\\`[0-9a-f]\\{8\\}-")))
      (when (file-exists-p (expand-file-name "manifest.json" dir))
        (cl-letf (((symbol-function 'romacs-agent--attach) #'ignore))
          (romacs-agent--load dir))
        (when-let* ((session (romacs-agent-loaded (file-name-nondirectory dir))))
          ;; Whether records came since its rows is the records' to say: a
          ;; session loaded beside its running supervisor has an older seq.
          (setq written (+ written (condition-case err (romacs-agent-turns-fill session)
                                     (error (message "romacs-agent-turns: %s: %s" dir (error-message-string err)) 0))))
          (setq drawn (+ drawn (condition-case err (romacs-agent-turns-redraw session)
                                 (error (message "romacs-agent-turns: %s: %s" dir (error-message-string err)) 0))))
          (remhash (romacs-agent-id session) romacs-agent--sessions))))
    ;; The forms kept, a line a session, for the editor to tell its chats.
    (let ((sessions nil))
      (pcase-dolist (`(,id . ,first) romacs-agent-turns--drawn-kept)
        (let ((known (assoc id sessions)))
          (if known (push first (cdr known)) (push (list id first) sessions))))
      (pcase-dolist (`(,id . ,firsts) sessions)
        (princ (format "%S\n" (list 'romacs-agent-turns-drawn id (sort firsts #'<))))))
    (princ (format "romacs-agent-turns: %d turns written, %d drawn again, in %.0f s\n" written drawn (- (float-time) t0)))))

(defvar romacs-agent-turns--filling nil "The batch process catching the rows up, while it runs.")

(defvar romacs-agent-turns--waiting nil
  "The ids a fill is still to catch up once the one running ends: t for
all of them.")

(defun romacs-agent-turns-fill-soon (&optional ids)
  "Catch the rows of the sessions IDS names (all of them, with none) up with
their records in a batch process of this build, off the editor; while one
runs, once it ends."
  (when (and (not noninteractive) (romacs-agent-turns--db))
    (if (process-live-p romacs-agent-turns--filling)
        (setq romacs-agent-turns--waiting
              (if (or (null ids) (eq romacs-agent-turns--waiting t)) t
                (cl-union ids romacs-agent-turns--waiting :test #'equal)))
      (setq romacs-agent-turns--filling
            (make-process
             :name "romacs-agent-turns-fill" :noquery t :connection-type 'pipe
             :buffer (generate-new-buffer " *romacs-agent-turns-fill*" t)
             :command (append (list (expand-file-name invocation-name invocation-directory) "--batch"
                                    "-l" "romacs-agent" "-f" "romacs-agent-turns-fill-batch")
                              ids)
             :sentinel (lambda (process _event)
                         (unless (process-live-p process)
                           (when (buffer-live-p (process-buffer process))
                             (romacs-agent-turns--drawn-told (with-current-buffer (process-buffer process) (buffer-string)))
                             (kill-buffer (process-buffer process)))
                           (when-let* ((waiting romacs-agent-turns--waiting))
                             (setq romacs-agent-turns--waiting nil)
                             (romacs-agent-turns-fill-soon (unless (eq waiting t) waiting))))))))))

;; Restored: what ran while no editor was there gets its rows, off the editor.
(add-hook 'romacs-agent-restored-hook #'romacs-agent-turns-fill-soon)

;;; Reading

(defun romacs-agent-turns-first-seqs (session n &optional before)
  "The first seqs of SESSION's latest N turns (before seq BEFORE when
given), the latest first."
  (when-let* ((db (romacs-agent-turns--db)))
    (mapcar #'car (sqlite-select db (concat "select first_seq from turns where session = ? and first_seq > 0"
                                            (if before " and first_seq < ?" "") " order by first_seq desc limit ?")
                                 (append (list (romacs-agent-id session)) (and before (list before)) (list n))))))

(defun romacs-agent-turns-count (session)
  "How many turns SESSION has rows for."
  (or (and-let* ((db (romacs-agent-turns--db)))
        (caar (sqlite-select db "select count(*) from turns where session = ? and first_seq > 0" (list (romacs-agent-id session)))))
      0))

(defun romacs-agent-turns-search (query &optional limit)
  "The turns whose text matches QUERY (FTS5 syntax), the best first: each
\(SESSION FIRST-SEQ SNIPPET), at most LIMIT (20)."
  (when-let* ((db (romacs-agent-turns--db)))
    (ignore-errors
      (sqlite-select db (concat "select session, first_seq, snippet(turns_fts, -1, '[', ']', '…', 12) from turns_fts"
                                " where turns_fts match ? order by rank limit ?")
                     (list query (or limit 20))))))

(defun romacs-agent-turns--parse (text)
  "TEXT, an item list's JSON, as a vector of tables (null and false nil)."
  (if (stringp text) (json-parse-string text :object-type 'hash-table :null-object nil :false-object nil) []))

(defun romacs-agent-turns-page (session &optional before limit)
  "What a chat draws SESSION's finished turns from: a plist of :rows, its
LIMIT (20) latest turns before seq BEFORE, newest first, each a plist of
:first-seq :last-seq :t :prompt :reply :items (a vector of tables, as
`romacs-agent-turns--item' makes them) :cost :images (its prompt's files)
:messages (who said what in it, tables) :duration :status :reason
:request (its prompt's request id); :after, the seq its latest row ends
at (nil with no row), after
which its records are the turn under way, drawn from them; and :more,
whether it has rows before this page's."
  (when-let* ((db (romacs-agent-turns--db)))
    (let* ((id (romacs-agent-id session)) (limit (or limit 20))
           (rows (sqlite-select db (concat "select " romacs-agent-turns--page-columns
                                           " from turns where session = ? and first_seq > 0"
                                           (if before " and first_seq < ?" "") " order by first_seq desc limit ?")
                                (append (list id) (and before (list before)) (list (1+ limit))))))
      (list :rows (mapcar #'romacs-agent-turns--page-row (seq-take rows limit))
            :after (romacs-agent-turns--last session)
            :more (> (length rows) limit)))))

(defun romacs-agent-turns-first-after (session seq)
  "SESSION's earliest turn after seq SEQ, as a row of
`romacs-agent-turns-page' gives it, or nil with none."
  (when-let* ((db (romacs-agent-turns--db))
              (row (car (sqlite-select db (concat "select " romacs-agent-turns--page-columns
                                                  " from turns where session = ? and first_seq > ? and first_seq > 0"
                                                  " order by first_seq limit 1")
                                       (list (romacs-agent-id session) seq)))))
    (romacs-agent-turns--page-row row)))

(defun romacs-agent-turns--tool (session id)
  "SESSION's tool item ID, from its turns' rows, or nil."
  (when-let* ((db (romacs-agent-turns--db))
              (text (caar (sqlite-select db (concat "select j.value from turns, json_each(turns.items) j where turns.session = ?"
                                                   " and turns.items is not null and json_extract(j.value, '$.kind') = 'tool'"
                                                   " and json_extract(j.value, '$.id') = ? limit 1")
                                         (list (romacs-agent-id session) id)))))
    (json-parse-string text :object-type 'hash-table :null-object nil :false-object nil)))

(defconst romacs-agent-turns--lookup-bytes (* 8 1024 1024)
  "How far back from its journal's end a tool's output is looked for in a
session whose records are still in a journal; in SQLite it is a seq.")

(defun romacs-agent-turns--record-tools (session seq key)
  "The tool events SESSION's record SEQ gives, read into a copy, each (ID
. VALUE), VALUE its KEY (:input or :output): those with KEY, in order.
Nil when a journal's record lies further back than
`romacs-agent-turns--lookup-bytes'."
  (let* ((events (romacs-agent-events-p session))
         (from (if events seq (romacs-agent--journal-file-after session seq romacs-agent-turns--lookup-bytes t)))
         (end (if events (1+ seq) (and (integerp from) (romacs-agent--journal-file-after session seq romacs-agent-turns--lookup-bytes))))
         (scratch (romacs-agent--scratch session)) (found nil))
    (when (and (integerp from) (integerp end))
      (let ((romacs-agent-event-functions
             (list (lambda (_session event)
                     (when (and (eq (plist-get event :type) 'tool) (plist-member event key)
                                (if (eq key :output) (memq (plist-get event :state) '(completed failed)) t))
                       (push (cons (plist-get event :id) (plist-get event key)) found))))))
        (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)) from end)))
    (nreverse found)))

(defun romacs-agent-turns-item (session id)
  "SESSION's tool ID as its row opens: (:input INPUT :output OUTPUT), its
whole input from the record that started it and its output from its item
when it holds it, else from the record that completed it; nil when no row
has it.  Of the tools a record gives, the one of ID, else the only one: an
id a harness numbers by its turn reads otherwise in a copy.  A journal's
record further back than `romacs-agent-turns--lookup-bytes' gives nil."
  (when-let* ((item (romacs-agent-turns--tool session id)))
    (cl-flet ((pick (found) (or (cdr (assoc id found)) (and (= (length found) 1) (cdar found)))))
      (let ((seqs (gethash "seq" item)))
        (list :input (pick (romacs-agent-turns--record-tools session (aref seqs 0) :input))
              :output (or (gethash "output" item)
                          (pick (romacs-agent-turns--record-tools session (aref seqs 1) :output))))))))

;;; A session's folded state

;; What a session's records fold to (`romacs-agent--kept-text'), so a
;; restore reads it and the records after its seq instead of them all,
;; is kept on one of its rows: its latest turn's, else a row of its own
;; with no turn (first seq 0).  One copy a session, moved as it is written.

(defun romacs-agent-turns-fold-put (id text)
  "Keep TEXT as the session ID's folded state, its only copy; t once kept.
A failure signals, rolled back: the writer sees it."
  (let ((db (or (romacs-agent-turns--db) (error "No agents store to keep %s's state in" id))))
    (condition-case err
        (progn
          (sqlite-execute db "begin immediate")
          (sqlite-execute db "update turns set fold = null where session = ? and fold is not null" (list id))
          (when (zerop (sqlite-execute db (concat "update turns set fold = ? where session = ? and first_seq ="
                                                  " (select max(first_seq) from turns where session = ?)")
                                       (list text id id)))
            (sqlite-execute db "insert into turns (session, first_seq, fold) values (?, 0, ?)" (list id text)))
          (sqlite-execute db "commit")
          t)
      (error (ignore-errors (sqlite-execute db "rollback"))
             (signal (car err) (cdr err))))))

(defun romacs-agent-turns-fold (id)
  "The session ID's folded state as kept, or nil.  Looked for from its
latest turn back, where it is kept (`romacs-agent-turns-fold-put'): a
row's fold is its last column, read past its turn's text, so a walk from
its first turn would read every turn it has."
  (when-let* ((db (romacs-agent-turns--db)))
    (caar (sqlite-select db "select fold from turns where session = ? and fold is not null order by first_seq desc limit 1"
                         (list id)))))

(provide 'romacs-agent-turns)
;;; romacs-agent-turns.el ends here

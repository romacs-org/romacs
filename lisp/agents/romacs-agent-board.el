;;; romacs-agent-board.el --- Agent threads on a board  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The board: every agent thread by project, in a buffer of its own mode
;; (`romacs-agent-board-mode'), shown as `display-buffer' places it: the
;; selected window by default; `romacs-popup-display-buffer' in
;; `display-buffer-alist' floats it over the frame.  A thread's row says
;; its state (`romacs-agent-ui--state-key'), what wants the user first, then
;; the newest; the threads it delegated fold under it.  Settling a thread
;; acknowledges it: it moves to the Settled section (M-RET) until its next
;; activity.  The harnesses' own transcripts of a project follow its
;; threads.
;;
;; Rows are plists made by pure functions of the sessions' summaries
;; (`romacs-agent-summary') and the board's view (`romacs-agent-board-rows');
;; the buffer is a list (romacs-list.el) drawn from them, as wide
;; as its window, again after each batch of session events while the
;; board shows and when a window shows it anew, never while it is hidden.

;;; Code:
(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-summary)
(require 'romacs-agent-glyphs)
(require 'romacs-agent-store)
(require 'romacs-agent-ui)
(require 'romacs-key-hint)
(require 'romacs-truncate)
(require 'romacs-agent-history)
(require 'romacs-list)

(defface romacs-agent-board-row '((t :inherit romacs-list-row))
  "The agent board's row at point."
  :group 'romacs-agent)

(defconst romacs-agent-board--name "*romacs-agents*")

;;; The view: what the user chose, kept in the board's buffer

(defvar-local romacs-agent-board--settled nil "Non-nil: the Settled section shows.")
(defvar-local romacs-agent-board--words nil "Filter terms every thread shown matches, or nil.")
(defvar-local romacs-agent-board--folds nil "Session id -> `open' or `closed', as the user left it.")
(defvar-local romacs-agent-board--all-history nil "Every project's transcripts show, not its newest few.")
(defvar-local romacs-agent-board--project nil
  "The project whose threads the board shows, its directory; nil: every
project with threads or transcripts, a line each.  The board's level
\(`romacs-list-level-variable').")

(defun romacs-agent-board--view ()
  "The board's view, from its buffer's choices, for `romacs-agent-board-rows'."
  (list :settled romacs-agent-board--settled :words romacs-agent-board--words
        :folds (or romacs-agent-board--folds (setq romacs-agent-board--folds (make-hash-table :test #'equal)))
        :project romacs-agent-board--project
        :all-history romacs-agent-board--all-history
        ;; No transcript listed yet, their first scan under way.
        :reading (and (null romacs-agent-history--listing) (romacs-agent-history-indexing-p))))

;;; Rows

(defvar romacs-agent-board--index nil
  "While rows are made, the summaries they are made of by id, and by their
parent's id: (BY-ID . CHILDREN).")

(defun romacs-agent-board--index-of (summaries)
  "SUMMARIES by id, and by their parent's id (`romacs-agent-board--index')."
  (let ((by-id (make-hash-table :test #'equal)) (children (make-hash-table :test #'equal)))
    (dolist (summary (reverse summaries))
      (puthash (romacs-agent-summary-id summary) summary by-id)
      (when-let* ((parent (romacs-agent-summary-parent summary)))
        (push summary (gethash parent children))))
    (cons by-id children)))

(defun romacs-agent-board--parent (summary)
  "The summary of the session SUMMARY's was delegated by, when the board
knows it, or nil."
  (gethash (romacs-agent-summary-parent summary) (car romacs-agent-board--index)))

(defun romacs-agent-board--children (summary)
  "The summaries of the sessions SUMMARY's delegated."
  (gethash (romacs-agent-summary-id summary) (cdr romacs-agent-board--index)))

(defvar romacs-agent-board-source romacs-agent-log-local
  "Whose sessions the board shows and opens: a chat's source
\(`romacs-agent-log--source'), this editor's own or another machine's,
one at a time.")

(cl-defgeneric romacs-agent-board-source-live (source id)
  "SOURCE's session ID to act on, as it is now, or nil when it is gone.")

(cl-defgeneric romacs-agent-board-source-history (_source)
  "The harness transcripts the board lists with SOURCE's sessions, newest
first: none, as of a source here."
  nil)

(cl-defmethod romacs-agent-board-source-history ((_source romacs-agent-log-local-source))
  (romacs-agent-history-items))

(defun romacs-agent-board--project (cwd)
  "A project's name for CWD, a directory: as every row of it says it."
  (abbreviate-file-name (directory-file-name (expand-file-name (or cwd "~")))))

(defun romacs-agent-board--text (summary)
  "What a filter's words match in SUMMARY: title, project, provider, model."
  (downcase (string-join (delq nil (list (romacs-agent-summary-title summary) (romacs-agent-summary-cwd summary)
                                         (romacs-agent-summary-provider summary) (romacs-agent-summary-model summary)))
                         " ")))

(defun romacs-agent-board--text-matches-p (text words &optional parent)
  "Whether lower-case TEXT and PARENT match all filter terms in WORDS."
  (cl-every (lambda (word)
              (if (string-prefix-p "parent:" word)
                  (equal parent (substring word (length "parent:")))
                (string-search (downcase word) text)))
            words))

(defun romacs-agent-board--matches-p (summary words)
  "Whether SUMMARY matches all filter terms in WORDS."
  (or (null words)
      (romacs-agent-board--text-matches-p (romacs-agent-board--text summary) words
                                          (romacs-agent-summary-parent summary))))

(defun romacs-agent-board--sorted (summaries keys)
  "SUMMARIES by rank of their KEYS (a table), then the newest first."
  (sort (copy-sequence summaries)
        (lambda (a b)
          (let ((x (romacs-agent-ui--rank (gethash a keys))) (y (romacs-agent-ui--rank (gethash b keys))))
            (if (= x y)
                (> (or (romacs-agent-summary-last-time a) 0) (or (romacs-agent-summary-last-time b) 0))
              (< x y))))))

(defun romacs-agent-board--marks (sessions keys view)
  "A table of each of SESSIONS to whether it shows in VIEW: `own', its
section and the filter's words are its own; `context', only for a thread
it delegated that shows (its row is dim); else absent.  One pass over
every session and its delegated ones."
  (let ((marks (make-hash-table :test #'eq)) (settled (and (plist-get view :settled) t)))
    (cl-labels ((mark (session)
                  (pcase (gethash session marks 'unknown)
                    ('unknown
                     (let ((own (and (eq (eq (gethash session keys) 'settled) settled)
                                     (romacs-agent-board--matches-p session (plist-get view :words))))
                           (under nil))
                       ;; Every child is marked: none is left for a later pass.
                       (dolist (child (romacs-agent-board--children session))
                         (when (mark child) (setq under t)))
                       (puthash session (cond (own 'own) (under 'context)) marks)))
                    (value value))))
      (mapc #'mark sessions))
    marks))

(defun romacs-agent-board--open-p (session children keys view)
  "Whether SESSION's CHILDREN show under it: as the user left it, else
while filtering or while one of them wants the user or works."
  (pcase (gethash (romacs-agent-summary-id session) (plist-get view :folds))
    ('open t)
    ('closed nil)
    (_ (or (plist-get view :words)
           (cl-some (lambda (child)
                      (let ((key (gethash child keys)))
                        (or (eq key 'failed) (memq key romacs-agent-ui--live-keys))))
                    children)))))

(defun romacs-agent-board--hidden-context-p (session marks)
  "Whether SESSION is settled and only needed to reach visible children."
  (and (eq (gethash session marks) 'context)
       (eql (romacs-agent-summary-settled-seq session) romacs-agent--inactive-settlement)))

(defun romacs-agent-board--shown-children (session marks)
  "SESSION's marked children, with hidden parents skipped."
  (mapcan (lambda (child)
            (when (gethash child marks)
              (if (romacs-agent-board--hidden-context-p child marks)
                  (romacs-agent-board--shown-children child marks)
                (list child))))
          (romacs-agent-board--children session)))

(defun romacs-agent-board--thread (session depth keys marks view)
  "SESSION's row, DEPTH deep, then those of the threads it delegated when
they show under it (always, when it shows only for them), as VIEW shows
them; KEYS has every session's key, MARKS whether it shows."
  (let* ((children (romacs-agent-board--sorted (romacs-agent-board--shown-children session marks) keys))
         (context (eq (gethash session marks) 'context))
         (hidden (romacs-agent-board--hidden-context-p session marks))
         (open (and children (or context (romacs-agent-board--open-p session children keys view))))
         (rows (and open (mapcan (lambda (child)
                                  (romacs-agent-board--thread child (if hidden depth (1+ depth)) keys marks view))
                                children))))
    (if hidden rows
      (cons (list :kind 'thread :id (romacs-agent-summary-id session) :summary session :key (gethash session keys)
                  :depth depth :open open :context context
                  :children (mapcar (lambda (child) (gethash child keys)) children))
            rows))))

(defun romacs-agent-board--tree-keys (session keys marks)
  "The keys of SESSION and of every thread under it that shows."
  (let ((under (mapcan (lambda (child) (romacs-agent-board--tree-keys child keys marks))
                       (romacs-agent-board--shown-children session marks))))
    (if (romacs-agent-board--hidden-context-p session marks)
        under
      (cons (gethash session keys) under))))

(defun romacs-agent-board--history-rows (cwd items view)
  "Project CWD's transcript ITEMS (newest first) as rows: its newest few,
then a row saying how many more, unless VIEW shows them all."
  (let* ((words (plist-get view :words))
         (items (seq-filter (lambda (item)
                              (romacs-agent-board--text-matches-p (downcase (or (plist-get item :title) "")) words))
                            items))
         (shown (if (plist-get view :all-history) items (seq-take items romacs-agent-history-per-project))))
    (append (mapcar (lambda (item) (list :kind 'history :id (plist-get item :path) :item item)) shown)
            (and (> (length items) (length shown))
                 (list (list :kind 'more :id (cons 'more cwd)
                             :count (- (length items) (length shown))))))))

(defun romacs-agent-board--settled-count (session keys)
  "Count settled threads in SESSION's tree according to KEYS."
  (+ (if (eq (gethash session keys) 'settled) 1 0)
     (cl-loop for child in (romacs-agent-board--children session)
              sum (romacs-agent-board--settled-count child keys))))

(defun romacs-agent-board--project-line (project threads transcripts keys marks)
  "PROJECT, which has THREADS and TRANSCRIPTS, as one line: what its
threads are at and how many, and how many transcripts."
  (let ((all (mapcan (lambda (session) (romacs-agent-board--tree-keys session keys marks)) threads)))
    (append project (list :keys all :folded (length all) :transcripts (length transcripts)))))

(defun romacs-agent-board--project-rows (cwd threads transcripts keys marks view settled-count)
  "Rows for project CWD and its THREADS and TRANSCRIPTS in VIEW: with no
project chosen and no filter, its line alone when it has threads or
transcripts; with one, its threads and transcripts when it is that project."
  (let ((project (list :kind 'project :id cwd :cwd cwd :settled-count settled-count))
        (chosen (plist-get view :project)))
    (cond
     ((plist-get view :words) (romacs-agent-board--project-threads project threads transcripts keys marks view))
     ((null chosen) (and (or threads transcripts)
                         (list (romacs-agent-board--project-line project threads transcripts keys marks))))
     ((equal chosen cwd) (romacs-agent-board--project-threads project threads transcripts keys marks view)))))

(defun romacs-agent-board--project-threads (project threads transcripts keys marks view)
  "PROJECT's row, then its THREADS and TRANSCRIPTS as rows in VIEW."
  (when-let* ((rows (append (mapcan (lambda (session) (romacs-agent-board--thread session 0 keys marks view)) threads)
                            (romacs-agent-board--history-rows (plist-get project :cwd) transcripts view))))
    (cons (plist-put project :keys (cl-loop for row in rows when (eql (plist-get row :depth) 0)
                                           collect (plist-get row :key)))
          rows)))

(defun romacs-agent-board--archive (history)
  "Group newest-first HISTORY by project, in order of each project's first item."
  (let ((items (make-hash-table :test #'equal)) order)
    (dolist (item history)
      (let ((cwd (romacs-agent-board--project (plist-get item :cwd))))
        (unless (gethash cwd items) (push cwd order))
        (push item (gethash cwd items))))
    (dolist (cwd order) (puthash cwd (nreverse (gethash cwd items)) items))
    (cons (nreverse order) items)))

(defun romacs-agent-board--prepare (sessions history)
  "Index SESSIONS, order their roots and group HISTORY for repeated views."
  (let* ((index (romacs-agent-board--index-of sessions))
         (romacs-agent-board--index index)
         (keys (make-hash-table :test #'eq))
         (settled (make-hash-table :test #'equal))
         roots)
    (dolist (session sessions) (puthash session (romacs-agent-summary-state-key session) keys))
    (dolist (session (romacs-agent-board--sorted sessions keys))
      (unless (romacs-agent-board--parent session)
        (let ((cwd (romacs-agent-board--project (romacs-agent-summary-cwd session))))
          (cl-incf (gethash cwd settled 0) (romacs-agent-board--settled-count session keys))
          (push (cons cwd session) roots))))
    (list sessions index keys (nreverse roots) (romacs-agent-board--archive history) settled)))

(defun romacs-agent-board--listing-rows (listing view)
  "Project a prepared LISTING into rows for VIEW."
  (pcase-let* ((`(,sessions ,romacs-agent-board--index ,keys ,roots ,archive ,settled) listing)
               (marks (romacs-agent-board--marks sessions keys view))
               (threads (make-hash-table :test #'equal))
               (active (not (plist-get view :settled)))
               (order nil))
    (dolist (root roots)
      (when (gethash (cdr root) marks)
        (unless (gethash (car root) threads) (push (car root) order))
        (push (cdr root) (gethash (car root) threads))))
    (when active
      (dolist (cwd (car archive))
        (unless (gethash cwd threads) (push cwd order))))
    (append
     (mapcan (lambda (cwd)
               (romacs-agent-board--project-rows cwd (nreverse (gethash cwd threads))
                                                 (and active (gethash cwd (cdr archive))) keys marks view (and active (gethash cwd settled))))
             (nreverse order))
     (when (and active (null (car archive)) (plist-get view :reading))
       (list (list :kind 'reading :id 'reading))))))

(defun romacs-agent-board-rows (sessions history view)
  "The board's rows for SESSIONS, their summaries (`romacs-agent-summary'),
and HISTORY (transcript items, newest
first), as VIEW (`romacs-agent-board--view') shows them: per project with
anything to show, its row, its threads (those wanting the user first,
then the newest, with what each delegated) and, off the Settled section,
its transcripts.  A thread shows in the section it is in, a thread above
it dim for context.  Project rows count settled threads off that section.
With no project chosen (VIEW's :project) and no filter, each project
with threads or transcripts shows its row alone, saying how many threads
it has and what they are at, or how many transcripts when it has none;
with one chosen, that project's rows alone.  Each row is a
plist of :kind (`project', `thread', `history', `more' or `reading', the
transcripts' first scan under way) and :id, which names it across
redraws; a thread's :summary is its session's."
  (romacs-agent-board--listing-rows (romacs-agent-board--prepare sessions history) view))

;;; Columns

(defcustom romacs-agent-board-columns
  '((romacs-agent-board-state . romacs-agent-board-state-width)
    (romacs-agent-board-title . fill)
    (romacs-agent-board-doing . 12)
    (romacs-agent-board-context . 4)
    (romacs-agent-board-instance . 18)
    (romacs-agent-board-time . 4))
  "A thread's row on the agent board, left to right after its fold: each
\(FUNCTION . WIDTH).  FUNCTION, called with the row (a plist, see
`romacs-agent-board-rows', its :summary the session's), gives its text
or nil.  WIDTH is a number of columns, `fill' for the column taking
what the others leave, or a function giving the number.  A FUNCTION
with a non-nil `romacs-list-live' property is asked again every second
for a thread at work, while the board shows (`romacs-list-cells')."
  :type '(repeat (cons function (choice integer (const fill) function)))
  :group 'romacs-agent)

(defun romacs-agent-board-state (row)
  "ROW's state: its glyph, or its word."
  (when-let* ((key (plist-get row :key)))
    (romacs-agent-ui--label (romacs-agent-ui--state-of key (romacs-agent-summary-watch-unit (plist-get row :summary))))))

(defun romacs-agent-board-state-width ()
  "The state column's width: its widest label's, a word cut to nine."
  (min 9 (apply #'max 1 (mapcar (lambda (state) (string-width (or (romacs-agent-ui--label state) "")))
                                romacs-agent-ui--states))))

(defun romacs-agent-board-title (row)
  "ROW's title: dim when settled or only there for context, bold unread."
  (let ((summary (plist-get row :summary)))
    (propertize (or (romacs-agent-summary-title summary) "")
                'face (cond ((or (plist-get row :context) (eq (plist-get row :key) 'settled)) 'romacs-agent-dim)
                            ((romacs-agent--unseen-p (romacs-agent-summary-seen-seq summary)
                                                     (romacs-agent-summary-done-seq summary))
                             'romacs-agent-unread)
                            (t 'default)))))

(defun romacs-agent-board-doing (row)
  "What ROW's folded threads are doing, else the tool its turn runs."
  (or (and (plist-get row :children) (not (plist-get row :open))
           (romacs-agent-ui--rollup (plist-get row :children)))
      (and (eq (plist-get row :key) 'working)
           (when-let* ((tool (romacs-agent-summary-active-tool (plist-get row :summary))))
             (propertize (romacs-agent-ui--tool-name tool) 'face 'romacs-agent-running)))))

(defun romacs-agent-board-context (row)
  "How full ROW's context is."
  (when-let* ((summary (plist-get row :summary))
              (share (romacs-agent-ui--context-share (romacs-agent-summary-context-used summary)
                                                     (romacs-agent-summary-context-window summary))))
    (propertize share 'face 'romacs-agent-context)))

(defun romacs-agent-board-instance (row)
  "ROW's harness and model; its account on hover."
  (let* ((summary (plist-get row :summary)) (harness (romacs-agent-summary-harness summary)))
    (propertize (romacs-agent-ui--instance (symbol-name harness) harness
                                           (or (romacs-agent-summary-model summary)
                                               (romacs-agent-summary-selected-model summary))
                                           nil)
                'face 'romacs-agent-model 'help-echo (format "account %s" (romacs-agent-summary-provider summary)))))

(defun romacs-agent-board--span (seconds)
  "SECONDS, compactly."
  (cond ((< seconds 60) (format "%ds" seconds))
        ((< seconds 3600) (format "%dm" (/ seconds 60)))
        (t (format "%dh" (/ seconds 3600)))))

(defun romacs-agent-board-time (row)
  "How long ROW's turn has run, else how long ago it last did something."
  (let* ((summary (plist-get row :summary)) (start (romacs-agent-summary-turn-start summary)))
    (propertize (if (and (eq (plist-get row :key) 'working) (numberp start))
                    (romacs-agent-board--span (floor (- (float-time) start)))
                  (romacs-agent-ui--ago (romacs-agent-summary-last-time summary)))
                'face 'romacs-agent-dim)))

(put 'romacs-agent-board-doing 'romacs-list-live t)
(put 'romacs-agent-board-time 'romacs-list-live t)

;;; Lines

(defconst romacs-agent-board--max-indent-depth 6
  "The deepest a delegated thread's row indents, two columns a level (12
columns): deeper ones line up with it, so a row's indent costs the same
however long its chain, and a chain's titles stay in view.")

(defun romacs-agent-board--fold-mark (row)
  "ROW's fold mark, open or folded, or blank when it has no threads: as
wide as the wider mark, so rows line up whichever they show."
  (let ((blank (make-string (string-width (romacs-agent-glyph-padded 'board-open 'board-closed)) ?\s)))
    (cond ((null (plist-get row :children)) blank)
          ((plist-get row :open) (romacs-agent-glyph-padded 'board-open 'board-closed))
          (t (romacs-agent-glyph-padded 'board-closed 'board-open)))))

(defun romacs-agent-board--thread-line (row width)
  "Thread ROW in a line WIDTH columns wide: its fold, then its columns
\(`romacs-agent-board-columns')."
  (romacs-list-cells romacs-agent-board-columns row width
                     (concat (make-string (* 2 (min (plist-get row :depth) romacs-agent-board--max-indent-depth)) ?\s)
                             (romacs-agent-board--fold-mark row) " ")
                     (eq (plist-get row :key) 'working)))

(defun romacs-agent-board--line (row width)
  "ROW as a line WIDTH columns wide."
  (pcase (plist-get row :kind)
    ('project
     ;; Its name always shows, then what its threads are at, then how
     ;; many when folded, cut from the end only when nothing else is
     ;; left; the path takes the room after them.
     (let* ((name (romacs-truncate (concat (romacs-agent-board--project-mark (plist-get row :keys))
                                           (propertize (file-name-nondirectory (plist-get row :cwd)) 'face 'romacs-agent-section))
                                   width))
            (room (- width (string-width name)))
            (tail (romacs-truncate
                   (concat (if-let* ((rollup (romacs-agent-ui--rollup (plist-get row :keys)))) (concat "  " rollup) "")
                           (if-let* ((count (plist-get row :settled-count)) ((> count 0)))
                               (propertize (format "  %s %d" (romacs-agent-glyph 'board-settled) count) 'face 'romacs-agent-dim)
                             "")
                           (romacs-agent-board--project-count row))
                   room)))
       (concat name
               (romacs-truncate (propertize (concat "  " (plist-get row :cwd)) 'face 'romacs-agent-dim)
                                (- room (string-width tail)))
               tail)))
    ('thread (romacs-agent-board--thread-line row width))
    ('history (let ((item (plist-get row :item)))
                (propertize (romacs-truncate (concat "  " (romacs-agent-glyph 'board-history) " " (plist-get item :title) "  " (let ((harness (plist-get item :harness))) (if harness (format "%s" harness) ""))
                                                     "  " (romacs-agent-ui--ago (plist-get item :mtime)))
                                             width)
                            'face 'romacs-agent-dim)))
    ('more (romacs-key-hint (format "  %s %d more  (\\<romacs-agent-board-mode-map>\\[romacs-agent-board-history] shows all)" (romacs-agent-glyph 'board-history)
                                    (plist-get row :count))
                            'romacs-agent-dim))
    ('reading (propertize (romacs-truncate (concat "  " (romacs-agent-glyph 'board-history) " Reading harness transcripts…") width) 'face 'romacs-agent-dim))))

(defun romacs-agent-board--project-count (row)
  "How many threads project ROW's line holds, or with none, how many
transcripts; empty off the project list."
  (let ((threads (or (plist-get row :folded) 0))
        (transcripts (or (plist-get row :transcripts) 0)))
    (propertize (cond ((> threads 0)
                       (format "  %s %d thread%s" (romacs-agent-glyph 'board-threads) threads (if (= threads 1) "" "s")))
                      ((> transcripts 0)
                       (format "  %s %d transcript%s" (romacs-agent-glyph 'board-history) transcripts (if (= transcripts 1) "" "s")))
                      (t ""))
                'face 'romacs-agent-dim)))

(defun romacs-agent-board--project-mark (keys)
  "A project's mark before its name: a bar in the colour of the most
pressing state of its threads, KEYS (what wants the user, a failure,
work), dim when none presses; then its folder."
  (let* ((top (car (sort (seq-filter (lambda (key) (romacs-agent-ui--news-p key)) (copy-sequence keys))
                         (lambda (a b) (< (romacs-agent-ui--rank a) (romacs-agent-ui--rank b))))))
         (face (if top (nth 3 (romacs-agent-ui--state-of top)) 'romacs-agent-dim)))
    (concat (propertize "▌" 'face face) " "
            (if-let* ((icon (romacs-icon 'dired 'romacs-agent-section))) (concat icon " ") ""))))

(defun romacs-agent-board--keys-of (rows)
  "The states of every thread ROWS show or count."
  (mapcan (lambda (row) (if (eq (plist-get row :kind) 'project) (copy-sequence (plist-get row :keys))
                          (and (plist-get row :key) (list (plist-get row :key)))))
          rows))

(defun romacs-agent-board--live-p (rows)
  "Whether one of the threads ROWS show or count works: the banner moves."
  (and (seq-intersection (romacs-agent-board--keys-of rows) romacs-agent-ui--live-keys) t))

(defconst romacs-agent-board--banner-faces '(romacs-working-running romacs-working-waiting romacs-agent-done)
  "The banner's colours: running, waiting on the user, done.")

(defvar romacs-agent-board-mode-map)

(defun romacs-agent-board--keys-text (width)
  "The board's few keys within WIDTH; \\[describe-mode] says every one."
  (romacs-truncate
   (romacs-key-hint (concat "\\<romacs-agent-board-mode-map>\\[romacs-agent-board-open] open  "
                            (if romacs-agent-board--project "\\[romacs-agent-board-projects] projects  " "")
                            "\\[romacs-agent-board-fold] fold  \\[romacs-agent-board-settle] settle  "
                            "\\[romacs-agent-board-toggle-settled] settled  \\[describe-mode] keys")
                    'romacs-agent-dim)
   width))

;;; Drawing

(defun romacs-agent-board--empty ()
  "What the board says with no row."
  (romacs-key-hint (if romacs-agent-board--settled "Nothing is settled."
                     "No agent threads: \\<romacs-agent-board-mode-map>\\[romacs-agent-board-new] starts one.")
                   'romacs-agent-dim))

(defvar-local romacs-agent-board--listing nil
  "The board's prepared listing, paired with the source it was read from.")

(defun romacs-agent-board--listing (source)
  "Return the current listing from SOURCE, reading it once after a change."
  (unless (and romacs-agent-board--listing (eq (car romacs-agent-board--listing) source))
    (setq romacs-agent-board--listing
          (cons source (romacs-agent-board--prepare (romacs-agent-summary nil source)
                                                   (romacs-agent-board-source-history source)))))
  (cdr romacs-agent-board--listing))

(defun romacs-agent-board--render (buffer &optional columns)
  "Draw BUFFER's board COLUMNS wide, by default as wide as its windows
\(`romacs-list-render')."
  (with-current-buffer buffer
    (let ((source romacs-agent-board-source))
      ;; Its rows are no one's while they are drawn: a draw cut short
      ;; leaves them so.
      (setq romacs-agent-board--drawn-source 'drawing)
      (romacs-list-render buffer (romacs-agent-board--listing-rows (romacs-agent-board--listing source)
                                                                   (romacs-agent-board--view))
                          columns)
      ;; Its rows are SOURCE's once drawn: only then are they acted on as such.
      (setq romacs-agent-board--drawn-source source))))

(defun romacs-agent-board--fresh (row)
  "ROW of its session's summary read again, for its cells at work."
  (let ((summary (or (romacs-agent-summary-kept (plist-get row :id))
                     (romacs-agent-summary (plist-get row :id) romacs-agent-board-source))))
    (if (or (null summary) (eq summary (plist-get row :summary))) row
      (plist-put (copy-sequence row) :summary summary))))

(defun romacs-agent-board--shown ()
  "The board's buffer, while it shows."
  (when-let* ((buffer (get-buffer romacs-agent-board--name))
              ((romacs-list-windows buffer)))
    buffer))

(defun romacs-agent-board--refresh ()
  "Invalidate the listing and draw it again while the board shows."
  (let ((shown (romacs-agent-board--shown)))
    (when-let* ((buffer (or shown (get-buffer romacs-agent-board--name))))
      (with-current-buffer buffer (setq romacs-agent-board--listing nil))
      (when shown (romacs-agent-board--render buffer)))))

(defun romacs-agent-board--local-refresh ()
  "Refresh the board when its listing is this editor's own."
  (when (eq romacs-agent-board-source romacs-agent-log-local)
    (romacs-agent-board--refresh)))

(defun romacs-agent-board--listing-event (_session event)
  "Invalidate the local listing when EVENT can move its filter inputs."
  (when-let* (((memq (plist-get event :type) '(changed selection cwd)))
              ((eq romacs-agent-board-source romacs-agent-log-local))
              (buffer (get-buffer romacs-agent-board--name)))
    (with-current-buffer buffer (setq romacs-agent-board--listing nil))))

(add-hook 'romacs-agent-event-functions #'romacs-agent-board--listing-event)
(add-hook 'romacs-agent-ui-changed-hook #'romacs-agent-board--local-refresh)
;; The sessions' rows made: those left unloaded show.
(add-hook 'romacs-agent-store-filled-hook #'romacs-agent-board--local-refresh)
(add-hook 'romacs-agent-store-settlement-hook #'romacs-agent-ui--batch)
;; The transcripts listed anew, aside: the board shows them.
(add-hook 'romacs-agent-history-indexed-hook #'romacs-agent-board--local-refresh)

(defun romacs-agent-board--redraw (&optional listing-changed)
  "Draw the board, re-reading the listing when LISTING-CHANGED is non-nil."
  (when listing-changed (setq romacs-agent-board--listing nil))
  (romacs-agent-board--render (current-buffer)))

;;; Commands

(defvar-local romacs-agent-board--drawn-source nil
  "The source whose sessions the board shows, drawn whole
\(`romacs-agent-board-source'); `drawing' while it is drawn, nil when it
never was.")

(defun romacs-agent-board--acting ()
  "Refuse, unless this editor acts on the sessions the board shows, drawn
whole: none while it is drawn."
  (unless (and (not (eq romacs-agent-board--drawn-source 'drawing))
               (romacs-agent-log-source-commands-p (or romacs-agent-board--drawn-source romacs-agent-board-source)))
    (user-error "These sessions are another machine's: shown, not acted on from here")))

(defun romacs-agent-board--session ()
  "The session on this line, to act on."
  (romacs-agent-board--acting)
  (romacs-agent-board--live (or (plist-get (romacs-list-row) :summary) (user-error "No thread on this line"))))

(defconst romacs-agent-board--here '(display-buffer-same-window)
  "Where what the board opens shows: in its window, as dired's files do.")

(defun romacs-agent-board--show-chat (session)
  "Show SESSION's chat in the board's window."
  (romacs-agent-show session romacs-agent-board--here romacs-agent-board-source))

(defun romacs-agent-board-open ()
  "Open the thread on this line, the transcript read-only, or a project:
its threads, or on its heading among them, its newest thread; on
\"more\", list every transcript."
  (interactive nil romacs-agent-board-mode)
  (let ((row (or (romacs-list-row) (user-error "Nothing on this line"))))
    (if (plist-get row :folded)
        (romacs-list-enter (plist-get row :cwd))
      (romacs-agent-board--open-row row))))

(defun romacs-agent-board--open-row (row)
  "Open ROW, a project's heading among its threads, a thread or a transcript."
  (when (eq (plist-get row :kind) 'project)
    ;; A project: its first row, what wants the user most, else the newest.
    (romacs-list-next 1)
    (setq row (romacs-list-row))
    (when (memq (plist-get row :kind) '(project nil))
      (romacs-list-previous 1)
      (user-error "Nothing to open in this project")))
  (pcase (plist-get row :kind)
    ('thread (romacs-agent-board--show-chat (romacs-agent-board--live (plist-get row :summary))))
    ('history (romacs-agent-history-open (plist-get row :item) romacs-agent-board--here))
    ('more (romacs-agent-board-history))))

(defun romacs-agent-board--live (summary)
  "The session of SUMMARY as it is now, to act on, unless no longer known:
then the board is drawn again."
  (or (romacs-agent-board-source-live romacs-agent-board-source (romacs-agent-summary-id summary))
      (progn (romacs-agent-board--redraw t)
             (user-error "That thread is gone: the board is drawn again"))))

(cl-defmethod romacs-agent-board-source-live ((_source romacs-agent-log-local-source) id)
  "Session ID, loaded first when it is not."
  (romacs-agent-get id))

(defun romacs-agent-board-projects ()
  "Show every project with threads or transcripts again, a line each."
  (interactive nil romacs-agent-board-mode)
  (unless romacs-agent-board--project (user-error "Every project shows already"))
  (romacs-list-enter nil))

(defun romacs-agent-board--fold (open)
  "Show the threads this one delegated when OPEN is t, hide them when
nil, flip them when `toggle'; on a delegated thread, its parent's.  On a
project's line, its threads alone; on its heading among them, every
project again."
  (let ((row (romacs-list-row)))
    (cond ((eq (plist-get row :kind) 'project)
           (romacs-list-enter (and (plist-get row :folded) (plist-get row :cwd))))
          (t (unless (plist-get row :children)
               (when-let* ((summary (plist-get row :summary))
                           (parent (romacs-agent-summary-parent summary))
                           ((romacs-list--find parent)))
                 (setq row (list :id parent :open t :children t))))
             (unless (plist-get row :children) (user-error "No threads under this one"))
             (puthash (plist-get row :id)
                      (if (if (eq open 'toggle) (not (plist-get row :open)) open) 'open 'closed)
                      (plist-get (romacs-agent-board--view) :folds))))
    (romacs-agent-board--redraw)
    (romacs-list-goto (plist-get row :id))))

(defun romacs-agent-board--project-row ()
  "The row of the project point's line is in, or nil."
  (save-excursion
    (let ((row (romacs-list-row)))
      (while (and row (not (eq (plist-get row :kind) 'project)) (zerop (forward-line -1)))
        (setq row (romacs-list-row)))
      (and (eq (plist-get row :kind) 'project) row))))

(defun romacs-agent-board-fold ()
  "Fold or unfold the threads this one delegated, or a project's."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--fold 'toggle))

(defun romacs-agent-board-unfold ()
  "Show the threads this one delegated."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--fold t))

(defun romacs-agent-board-close-fold ()
  "Hide the threads this one delegated, or its parent's."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--fold nil))

(defun romacs-agent-board-approve ()
  "Allow the tool call this thread waits on; a question opens its chat,
where it is answered."
  (interactive nil romacs-agent-board-mode)
  (let* ((session (romacs-agent-board--session))
         (asks (romacs-agent-asks session))
         (tool (car (seq-difference asks (romacs-agent-questions session)))))
    (cond (tool (romacs-agent-respond session tool t nil 'user))
          (asks (romacs-agent-board--show-chat session))
          (t (user-error "%s waits on no approval" (romacs-agent-title session))))))

(defun romacs-agent-board-interrupt ()
  "Interrupt this thread's turn."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-interrupt (romacs-agent-board--session)))

(defun romacs-agent-board-stop ()
  "Stop this thread's harness."
  (interactive nil romacs-agent-board-mode)
  (let ((session (romacs-agent-board--session)))
    (when (yes-or-no-p (format "Stop %s? " (romacs-agent-title session)))
      (romacs-agent-stop session))))

(defun romacs-agent-board-resume ()
  "Resume this thread, or go on with this transcript as a thread."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--acting)
  (let ((row (romacs-list-row)))
    (if (eq (plist-get row :kind) 'history)
        (romacs-agent-history-resume (plist-get row :item) romacs-agent-board--here)
      (romacs-agent-resume (romacs-agent-board--session)))))

(defun romacs-agent-board-settle ()
  "Hide this thread in Settled until explicitly brought back with this key.
Say when its delegated tasks are still at work."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--acting)
  (let* ((row (romacs-list-row))
         (summary (or (plist-get row :summary) (user-error "No thread on this line"))))
    (unless (romacs-agent-settle-recorded (romacs-agent-summary-id summary) t)
      (user-error "That thread is gone"))
    (romacs-agent-board--redraw t)
    (when (and (not (eq (plist-get row :key) 'settled))
               (cl-some (lambda (child)
                          (and (plist-get child :id)
                               (memq (plist-get child :key) romacs-agent-ui--live-keys)))
                        (romacs-agent-summary-children-rows summary)))
      (message "Delegated tasks are still at work. Their results will wait until you bring this thread back."))))

(defun romacs-agent-board--finished (rows)
  "The ids of the threads of ROWS (top-level and delegated) with nothing
going on."
  (cl-loop for row in rows
           when (and (eq (plist-get row :kind) 'thread)
                     (not (plist-get row :context))
                     (not (memq (plist-get row :key) romacs-agent-ui--live-keys))
                     (not (eq (plist-get row :key) 'settled)))
           collect (plist-get row :id)))

(defun romacs-agent-board-settle-finished ()
  "Settle every thread shown with nothing going on."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--acting)
  (let ((finished (romacs-agent-board--finished (romacs-list-shown-rows))))
    (unless finished (user-error "No finished threads"))
    (when (y-or-n-p (format "Settle %d finished thread%s? " (length finished) (if (cdr finished) "s" "")))
      (dolist (id finished)
        (romacs-agent-settle-recorded id))
      (romacs-agent-board--redraw t))))

(defun romacs-agent-board-new ()
  "Start a new thread, in this row's project, where the board's sessions
run (`romacs-agent-board-source')."
  (interactive nil romacs-agent-board-mode)
  ;; Its row's directory is the drawn rows' source's: started there only.
  (unless (eq (or romacs-agent-board--drawn-source romacs-agent-board-source) romacs-agent-board-source)
    (user-error "The board shows another source's sessions: draw it again (g)"))
  (let* ((row (romacs-list-row))
         (cwd (cond ((plist-get row :summary) (romacs-agent-summary-cwd (plist-get row :summary)))
                    ((plist-get row :cwd))
                    ((plist-get row :item) (plist-get (plist-get row :item) :cwd)))))
    (romacs-agent-board-source-new romacs-agent-board-source cwd)))

(cl-defgeneric romacs-agent-board-source-new (source cwd)
  "Start a session where SOURCE's sessions run, in directory CWD offered.")

(cl-defmethod romacs-agent-board-source-new ((_source romacs-agent-log-local-source) cwd)
  (apply #'funcall-interactively #'romacs-agent-start
         (romacs-agent--start-arguments (if cwd (file-name-as-directory cwd) (romacs-agent--project-default)))))

(defun romacs-agent-board-filter ()
  "Show only the threads whose title, project, provider or model have
every word typed, the board following as they are typed.
A parent:ID term matches the threads directly delegated by session ID."
  (interactive nil romacs-agent-board-mode)
  (let ((board (current-buffer)) (before romacs-agent-board--words))
    (cl-flet ((filter (text)
                (with-current-buffer board
                  (setq romacs-agent-board--words (split-string text nil t))
                  (romacs-agent-board--redraw))))
      (condition-case nil
          (filter (minibuffer-with-setup-hook
                      (lambda ()
                        (add-hook 'after-change-functions
                                  (lambda (&rest _) (when (buffer-live-p board) (filter (minibuffer-contents-no-properties))))
                                  nil t))
                    (read-from-minibuffer "Filter: " (string-join before " "))))
        (quit (filter (string-join before " ")))))))

(defun romacs-agent-board-toggle-settled ()
  "Show the Settled section, or the board again, at the current project."
  (interactive nil romacs-agent-board-mode)
  (let ((project (plist-get (romacs-agent-board--project-row) :id))
        (heading (eq (plist-get (romacs-list-row) :kind) 'project)))
    (setq romacs-agent-board--settled (not romacs-agent-board--settled))
    (romacs-agent-board--redraw)
    (if-let* ((at (romacs-list--find project)))
        (progn
          (goto-char at)
          (unless (or heading (plist-get (romacs-list-row) :folded))
            (romacs-list-next 1)))
      (goto-char (point-min))
      (romacs-list-next 2))))

(defun romacs-agent-board-history ()
  "Show every project's transcripts, or its newest few."
  (interactive nil romacs-agent-board-mode)
  (setq romacs-agent-board--all-history (not romacs-agent-board--all-history))
  (romacs-agent-board--redraw))

(defun romacs-agent-board--revert (&rest _)
  "Draw the board again, the harnesses' transcripts listed anew: its
`revert-buffer-function'."
  (romacs-agent-history-list)
  (romacs-agent-board--redraw t))

(defvar-keymap romacs-agent-board-mode-map
  :doc "Keys of the agent board, besides `special-mode''s."
  :parent romacs-list-mode-map
  "f" #'romacs-agent-board-unfold
  "b" #'romacs-agent-board-close-fold
  "TAB" #'romacs-agent-board-fold
  "<backtab>" #'romacs-agent-board-projects
  "S-<tab>" #'romacs-agent-board-projects
  "^" #'romacs-agent-board-projects
  "DEL" #'romacs-agent-board-projects
  "RET" #'romacs-agent-board-open
  "a" #'romacs-agent-board-approve
  "i" #'romacs-agent-board-interrupt
  "k" #'romacs-agent-board-stop
  "r" #'romacs-agent-board-resume
  "s" #'romacs-agent-board-settle
  "S" #'romacs-agent-board-settle-finished
  "c" #'romacs-agent-board-new
  "/" #'romacs-agent-board-filter
  "M-RET" #'romacs-agent-board-toggle-settled
  "h" #'romacs-agent-board-history)

(define-derived-mode romacs-agent-board-mode romacs-list-mode "Agents"
  "The agent board: every agent thread by project, what wants you first.
\\{romacs-agent-board-mode-map}"
  :interactive nil
  (setq-local revert-buffer-function #'romacs-agent-board--revert
              hl-line-face 'romacs-agent-board-row
              romacs-list-redraw-function #'romacs-agent-board--redraw
              romacs-list-line-function #'romacs-agent-board--line
              romacs-list-group-kind 'project
              romacs-list-rule-face 'romacs-agent-rule
              romacs-list-empty-function #'romacs-agent-board--empty
              romacs-list-hint-function #'romacs-agent-board--keys-text
              romacs-list-banner 'thread
              romacs-list-banner-faces romacs-agent-board--banner-faces
              romacs-list-live-function #'romacs-agent-board--live-p
              romacs-list-fresh-function #'romacs-agent-board--fresh
              romacs-list-level-variable 'romacs-agent-board--project))

(defun romacs-agent-board--buffer ()
  (or (get-buffer romacs-agent-board--name)
      (with-current-buffer (get-buffer-create romacs-agent-board--name)
        (romacs-agent-board-mode)
        (current-buffer))))

(defun romacs-agent-board-open-threads (session)
  "Show the agent board filtered to threads delegated by SESSION."
  (with-current-buffer (romacs-agent-board--buffer)
    (setq romacs-agent-board--words (list (concat "parent:" (romacs-agent-id session)))
          romacs-agent-board--settled nil))
  (romacs-agents))

;;;###autoload
(defun romacs-agents ()
  "Show the agent board: every agent thread by project, those that want
you first, then the newest, with what each delegated folded under it.
It shows in the selected window, unless `display-buffer-alist' says
otherwise (`romacs-popup-display-buffer' floats it over the frame).  Its
keys act on the thread at point.
\\{romacs-agent-board-mode-map}"
  (interactive)
  (romacs-agent-know-sessions)
  (romacs-agent-history-list)
  (let ((buffer (romacs-agent-board--buffer)))
    ;; Drawn first: a window sized to its text fits it.
    (with-current-buffer buffer (romacs-agent-board--redraw t))
    (when-let* ((window (display-buffer buffer '(display-buffer-same-window)))
                ;; A frame taking no focus (a floating menu) takes keys of its own.
                ((not (frame-parameter (window-frame window) 'no-accept-focus))))
      (unless (eq (window-frame window) (selected-frame)) (select-frame-set-input-focus (window-frame window)))
      (select-window window))))

(provide 'romacs-agent-board)
;;; romacs-agent-board.el ends here

;;; romacs-agent-board.el --- Agent threads on a board  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The board: every agent thread by project, in a buffer of its own mode
;; (`romacs-agent-board-mode'), shown as `display-buffer' places it: the
;; selected window by default; `romacs-popup-display-buffer' in
;; `display-buffer-alist' floats it over the frame.  A thread's row says
;; its state (`romacs-agent-ui--state'), what wants the user first, then
;; the newest; the threads it delegated fold under it.  Settling a thread
;; acknowledges it: it moves to the Settled section (M-RET) until its next
;; activity.  The harnesses' own transcripts of a project follow its
;; threads.
;;
;; Rows are plists made by pure functions of the sessions and the board's
;; view (`romacs-agent-board-rows'); the buffer is drawn from them, as wide
;; as its window, again after each batch of session events while the
;; board shows and when a window shows it anew, never while it is hidden.

;;; Code:
(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-store)
(require 'romacs-agent-ui)
(require 'hl-line)
(require 'romacs-key-hint)
(require 'romacs-truncate)
(require 'romacs-agent-history)

(defface romacs-agent-board-row '((t :inherit highlight :extend t))
  "The agent board's row at point."
  :group 'romacs-agent)

(defconst romacs-agent-board--name "*romacs-agents*")
(defvar romacs-agent-board--ticker nil "The timer asking the board's live cells again.")

;;; The view: what the user chose, kept in the board's buffer

(defvar-local romacs-agent-board--settled nil "Non-nil: the Settled section shows.")
(defvar-local romacs-agent-board--words nil "Words every thread shown matches, or nil.")
(defvar-local romacs-agent-board--folds nil "Session id -> `open' or `closed', as the user left it.")
(defvar-local romacs-agent-board--all-history nil "Every project's transcripts show, not its newest few.")
(defvar-local romacs-agent-board--folded nil
  "The projects folded to their heading, a set of their directories, as
the store keeps them (`romacs-agent-store-folded').")

(defun romacs-agent-board--folded ()
  "The board's folded projects, read from the store once a buffer."
  (or romacs-agent-board--folded
      (let ((folded (make-hash-table :test #'equal)))
        (dolist (project (romacs-agent-store-folded)) (puthash project t folded))
        (setq romacs-agent-board--folded folded))))

(defun romacs-agent-board--view ()
  "The board's view, from its buffer's choices, for `romacs-agent-board-rows'."
  (list :settled romacs-agent-board--settled :words romacs-agent-board--words
        :folds (or romacs-agent-board--folds (setq romacs-agent-board--folds (make-hash-table :test #'equal)))
        :projects (romacs-agent-board--folded)
        :all-history romacs-agent-board--all-history
        ;; No transcript listed yet, their first scan under way.
        :reading (and (null romacs-agent-history--listing) (romacs-agent-history-indexing-p))))

;;; Rows

(defvar romacs-agent-board--unloaded nil
  "While rows are made, the sessions shown from their rows alone
\(`romacs-agent-store-unloaded'), by id, and their children: (BY-ID
. CHILDREN), CHILDREN a table from a parent's id to them.")

(defun romacs-agent-board--own-p ()
  "Whether the board shows this editor's own sessions, which its session
commands act on and its registry knows (`romacs-agent-board-source')."
  (romacs-agent-log-source-commands-p romacs-agent-board-source))

(defun romacs-agent-board--parent (session)
  "The known session SESSION was delegated by, or nil."
  (let ((parent (plist-get (romacs-agent-options session) :parent)))
    (or (and (romacs-agent-board--own-p) (romacs-agent-loaded parent))
        (and romacs-agent-board--unloaded (gethash parent (car romacs-agent-board--unloaded))))))

(defun romacs-agent-board--children (session)
  "The sessions SESSION delegated, loaded or not."
  (append (and (romacs-agent-board--own-p) (romacs-agent-children session))
          (and romacs-agent-board--unloaded
               (gethash (romacs-agent-id session) (cdr romacs-agent-board--unloaded)))))

(defvar romacs-agent-board-source romacs-agent-log-local
  "Whose sessions the board shows and opens: a chat's source
\(`romacs-agent-log--source'), this editor's own or another machine's,
one at a time.")

(cl-defgeneric romacs-agent-board-source-sessions (source)
  "Every session of SOURCE's to show, those not loaded known to
`romacs-agent-board--unloaded'.")

(cl-defgeneric romacs-agent-board-source-key (source session)
  "SOURCE's SESSION's state key on the board.")

(cl-defgeneric romacs-agent-board-source-live (source session)
  "SOURCE's SESSION to open, as it is now, or nil when it is gone.")

(cl-defgeneric romacs-agent-board-source-history (_source)
  "The harness transcripts the board lists with SOURCE's sessions, newest
first: none, as of a source here."
  nil)

(cl-defmethod romacs-agent-board-source-history ((_source romacs-agent-log-local-source))
  (romacs-agent-history-items))

(defun romacs-agent-board--sessions ()
  "Every session the board shows (`romacs-agent-board-source')."
  (romacs-agent-board-source-sessions romacs-agent-board-source))

(cl-defmethod romacs-agent-board-source-sessions ((_source romacs-agent-log-local-source))
  "The loaded sessions, then those shown from their rows alone."
  (let ((unloaded (romacs-agent-store-unloaded))
        (by-id (make-hash-table :test #'equal)) (children (make-hash-table :test #'equal)))
    (dolist (session unloaded)
      (puthash (romacs-agent-id session) session by-id)
      (when-let* ((parent (plist-get (romacs-agent-options session) :parent)))
        (push session (gethash parent children))))
    (setq romacs-agent-board--unloaded (cons by-id children))
    (append (hash-table-values romacs-agent--sessions) unloaded)))

(defun romacs-agent-board--key (session)
  "SESSION's state key on the board (`romacs-agent-board-source')."
  (romacs-agent-board-source-key romacs-agent-board-source session))

(cl-defmethod romacs-agent-board-source-key ((_source romacs-agent-log-local-source) session)
  "SESSION's own key, except that a delegated task whose turn ended well
is done here, where its parent's user reads it."
  (let ((key (romacs-agent-ui--state-key session)))
    (if (and (eq key 'idle) (romacs-agent-board--parent session)
             (> (romacs-agent-done-seq session) 0) (not (romacs-agent-failed session)))
        'completed
      key)))

(defun romacs-agent-board--project (cwd)
  "A project's name for CWD, a directory: as every row of it says it."
  (abbreviate-file-name (directory-file-name (expand-file-name (or cwd "~")))))

(defun romacs-agent-board--text (session)
  "What a filter's words match in SESSION: title, project, provider, model."
  (downcase (string-join (delq nil (list (romacs-agent-title session) (romacs-agent-cwd session)
                                         (romacs-agent-provider session) (romacs-agent-model session)))
                         " ")))

(defun romacs-agent-board--matches-p (session words)
  "Whether SESSION's own text has every one of WORDS."
  (or (null words)
      (let ((text (romacs-agent-board--text session)))
        (cl-every (lambda (word) (string-search word text)) words))))

(defun romacs-agent-board--sorted (sessions keys)
  "SESSIONS by rank of their KEYS (a table), then the newest first."
  (sort (copy-sequence sessions)
        (lambda (a b)
          (let ((x (romacs-agent-ui--rank (gethash a keys))) (y (romacs-agent-ui--rank (gethash b keys))))
            (if (= x y) (> (or (romacs-agent-last-time a) 0) (or (romacs-agent-last-time b) 0)) (< x y))))))

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
  (pcase (gethash (romacs-agent-id session) (plist-get view :folds))
    ('open t)
    ('closed nil)
    (_ (or (plist-get view :words)
           (cl-some (lambda (child) (memq (gethash child keys) '(approval question failed working waiting)))
                    children)))))

(defun romacs-agent-board--thread (session depth keys marks view)
  "SESSION's row, DEPTH deep, then those of the threads it delegated when
they show under it (always, when it shows only for them), as VIEW shows
them; KEYS has every session's key, MARKS whether it shows."
  (let* ((children (romacs-agent-board--sorted
                    (seq-filter (lambda (child) (gethash child marks)) (romacs-agent-board--children session))
                    keys))
         (context (eq (gethash session marks) 'context))
         (open (and children (or context (romacs-agent-board--open-p session children keys view)))))
    (cons (list :kind 'thread :id (romacs-agent-id session) :session session :key (gethash session keys)
                :depth depth :open open :context context
                :children (mapcar (lambda (child) (gethash child keys)) children))
          (and open (mapcan (lambda (child) (romacs-agent-board--thread child (1+ depth) keys marks view))
                            children)))))

(defun romacs-agent-board--tree-keys (session keys marks)
  "The keys of SESSION and of every thread under it that shows."
  (cons (gethash session keys)
        (mapcan (lambda (child) (and (gethash child marks) (romacs-agent-board--tree-keys child keys marks)))
                (romacs-agent-board--children session))))

(defun romacs-agent-board--history-rows (cwd items view)
  "Project CWD's transcript ITEMS (newest first) as rows: its newest few,
then a row saying how many more, unless VIEW shows them all."
  (let* ((words (plist-get view :words))
         (items (seq-filter (lambda (item)
                              (let ((title (downcase (or (plist-get item :title) ""))))
                                (cl-every (lambda (word) (string-search word title)) words)))
                            items))
         (shown (if (plist-get view :all-history) items (seq-take items romacs-agent-history-per-project))))
    (append (mapcar (lambda (item) (list :kind 'history :id (plist-get item :path) :item item)) shown)
            (and (> (length items) (length shown))
                 (list (list :kind 'more :id (cons 'more cwd)
                             :count (- (length items) (length shown))))))))

(defun romacs-agent-board-rows (sessions history view)
  "The board's rows for SESSIONS and HISTORY (transcript items, newest
first), as VIEW (`romacs-agent-board--view') shows them: per project with
anything to show, its row, its threads (those wanting the user first,
then the newest, with what each delegated) and, off the Settled section,
its transcripts.  A thread shows in the section it is in, a thread above
it dim for context.  A project VIEW folds (its :projects) shows its row
alone, saying how many threads it has and what they are at, unless a
filter is on; its :first is the row it would show first.  Each row is a
plist of :kind (`project', `thread', `history', `more' or `reading', the
transcripts' first scan under way) and :id, which names it across
redraws."
  (let ((keys (make-hash-table :test #'eq)) (order nil)
        (roots (make-hash-table :test #'equal)) (items (make-hash-table :test #'equal)))
    (dolist (session sessions) (puthash session (romacs-agent-board--key session) keys))
    (let ((marks (romacs-agent-board--marks sessions keys view)))
      (dolist (session (romacs-agent-board--sorted sessions keys))
        (when (and (null (romacs-agent-board--parent session)) (gethash session marks))
          (let ((cwd (romacs-agent-board--project (romacs-agent-cwd session))))
            (unless (gethash cwd roots) (push cwd order))
            (push session (gethash cwd roots)))))
      (unless (plist-get view :settled)
        (dolist (item history)
          (let ((cwd (romacs-agent-board--project (plist-get item :cwd))))
            (unless (or (gethash cwd roots) (gethash cwd items)) (push cwd order))
            (push item (gethash cwd items)))))
      (append
       (mapcan (lambda (cwd)
                (let* ((threads (nreverse (gethash cwd roots)))
                       (rows (append (mapcan (lambda (session) (romacs-agent-board--thread session 0 keys marks view))
                                             threads)
                                     (romacs-agent-board--history-rows cwd (nreverse (gethash cwd items)) view))))
                  (cond ((null rows) nil)
                        ((and (plist-get view :projects) (gethash cwd (plist-get view :projects)) (not (plist-get view :words)))
                         (let ((all (mapcan (lambda (session) (romacs-agent-board--tree-keys session keys marks))
                                            threads)))
                           (list (list :kind 'project :id cwd :cwd cwd :keys all :folded (length all)
                                       :first (car rows)))))
                        (t (cons (list :kind 'project :id cwd :cwd cwd
                                       :keys (mapcar (lambda (s) (gethash s keys)) threads))
                                 rows)))))
              (nreverse order))
       (and (not (plist-get view :settled)) (null history) (plist-get view :reading)
            (list (list :kind 'reading :id 'reading)))))))

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
`romacs-agent-board-rows'), gives its text or nil.  WIDTH is a number of
columns, `fill' for the column taking what the others leave, or a
function giving the number.  A FUNCTION with a non-nil
`romacs-agent-board-live' property is asked again every second for a
thread at work, while the board shows."
  :type '(repeat (cons function (choice integer (const fill) function)))
  :group 'romacs-agent)

(defun romacs-agent-board-state (row)
  "ROW's state: its glyph, or its word."
  (romacs-agent-ui--label (romacs-agent-ui--state-of (plist-get row :key))))

(defun romacs-agent-board-state-width ()
  "The state column's width: a glyph's, or a short word's."
  (if (equal (substring-no-properties (romacs-agent-ui--label (romacs-agent-ui--state-of 'working))) "🤖") 2 9))

(defun romacs-agent-board-title (row)
  "ROW's title: dim when settled or only there for context, bold unread."
  (let ((session (plist-get row :session)))
    (propertize (or (romacs-agent-title session) "")
                'face (cond ((or (plist-get row :context) (eq (plist-get row :key) 'settled)) 'romacs-agent-dim)
                            ((romacs-agent-unseen-p session) 'romacs-agent-unread)
                            (t 'default)))))

(defun romacs-agent-board-doing (row)
  "What ROW's folded threads are doing, else the tool its turn runs."
  (or (and (plist-get row :children) (not (plist-get row :open))
           (romacs-agent-ui--rollup (plist-get row :children)))
      (and (eq (plist-get row :key) 'working)
           (when-let* ((tool (romacs-agent-activity (plist-get row :session))))
             (propertize (romacs-agent-ui--tool-name tool) 'face 'romacs-agent-running)))))

(defun romacs-agent-board-context (row)
  "How full ROW's context is."
  (when-let* ((share (romacs-agent-ui--context-share (plist-get row :session))))
    (propertize share 'face 'romacs-agent-context)))

(defun romacs-agent-board-instance (row)
  "ROW's harness and model; its account on hover."
  (let ((session (plist-get row :session)))
    (propertize (romacs-agent-ui--instance (symbol-name (romacs-agent-harness session)) (romacs-agent-harness session)
                                           (or (romacs-agent-model session)
                                               (plist-get (romacs-agent-options session) :model))
                                           nil)
                'face 'romacs-agent-model 'help-echo (format "account %s" (romacs-agent-provider session)))))

(defun romacs-agent-board--span (seconds)
  "SECONDS, compactly."
  (cond ((< seconds 60) (format "%ds" seconds))
        ((< seconds 3600) (format "%dm" (/ seconds 60)))
        (t (format "%dh" (/ seconds 3600)))))

(defun romacs-agent-board-time (row)
  "How long ROW's turn has run, else how long ago it last did something."
  (let* ((session (plist-get row :session)) (start (romacs-agent-turn-start session)))
    (propertize (if (and (eq (plist-get row :key) 'working) (numberp start))
                    (romacs-agent-board--span (floor (- (float-time) start)))
                  (romacs-agent-ui--ago (romacs-agent-last-time session)))
                'face 'romacs-agent-dim)))

(put 'romacs-agent-board-doing 'romacs-agent-board-live t)
(put 'romacs-agent-board-time 'romacs-agent-board-live t)

;;; Lines

(defun romacs-agent-board--cell (text width)
  "TEXT in exactly WIDTH columns."
  (romacs-truncate (or text "") width ?\s))

(defun romacs-agent-board--widths (width head)
  "The columns' widths in a row WIDTH wide after HEAD columns: `fill'
takes what the others and the gaps between them leave."
  (let* ((fixed (mapcar (lambda (column)
                          (let ((w (cdr column))) (cond ((eq w 'fill) nil) ((functionp w) (funcall w)) (t w))))
                        romacs-agent-board-columns))
         (rest (- width head (apply #'+ (delq nil (copy-sequence fixed))) (1- (length fixed)))))
    (mapcar (lambda (w) (or w (max 8 rest))) fixed)))

(defun romacs-agent-board--column (function width row live)
  "FUNCTION's cell for ROW, WIDTH wide; LIVE, a cell to ask again."
  (let ((cell (romacs-agent-board--cell (funcall function row) width)))
    (if (and live (symbolp function) (get function 'romacs-agent-board-live))
        (propertize cell 'romacs-agent-board-live (list function width))
      cell)))

(defun romacs-agent-board--thread-line (row width)
  "Thread ROW in a line WIDTH columns wide: its fold, then its columns
\(`romacs-agent-board-columns')."
  (let* ((head (concat (make-string (* 2 (plist-get row :depth)) ?\s)
                       (cond ((null (plist-get row :children)) "  ") ((plist-get row :open) "˅ ") (t "› "))))
         (widths (romacs-agent-board--widths width (string-width head))))
    (cl-flet ((line (live)
                (concat head (mapconcat #'identity
                                        (cl-mapcar (lambda (column width) (romacs-agent-board--column (car column) width row live))
                                                   romacs-agent-board-columns widths)
                                        " "))))
      (let ((line (line (eq (plist-get row :key) 'working))))
        ;; A board too narrow for every column loses the last ones, and its
        ;; cells keep still: a cut cell could not keep its width.
        (if (<= (string-width line) width) line (romacs-truncate (line nil) width))))))

(defun romacs-agent-board--line (row width)
  "ROW as a line WIDTH columns wide."
  (pcase (plist-get row :kind)
    ('project
     ;; Its name always shows, then what its threads are at, then how
     ;; many when folded, cut from the end only when nothing else is
     ;; left; the path takes the room after them.
     (let* ((name (romacs-truncate (propertize (file-name-nondirectory (plist-get row :cwd)) 'face 'romacs-agent-section)
                                   width))
            (room (- width (string-width name)))
            (tail (romacs-truncate
                   (concat (if-let* ((rollup (romacs-agent-ui--rollup (plist-get row :keys)))) (concat "  " rollup) "")
                           (if-let* ((count (plist-get row :folded)))
                               (propertize (format "  ▸ %d thread%s" count (if (= count 1) "" "s")) 'face 'romacs-agent-dim)
                             ""))
                   room)))
       (concat name
               (romacs-truncate (propertize (concat "  " (plist-get row :cwd)) 'face 'romacs-agent-dim)
                                (- room (string-width tail)))
               tail)))
    ('thread (romacs-agent-board--thread-line row width))
    ('history (let ((item (plist-get row :item)))
                (propertize (romacs-truncate (concat "  ◇ " (plist-get item :title) "  " (let ((harness (plist-get item :harness))) (if harness (format "%s" harness) ""))
                                                     "  " (romacs-agent-ui--ago (plist-get item :mtime)))
                                             width)
                            'face 'romacs-agent-dim)))
    ('more (romacs-key-hint (format "  ◇ %d more  (\\<romacs-agent-board-mode-map>\\[romacs-agent-board-history] shows all)"
                                    (plist-get row :count))
                            'romacs-agent-dim))
    ('reading (propertize (romacs-truncate "  ◇ Reading harness transcripts…" width) 'face 'romacs-agent-dim))))

(defun romacs-agent-board--title-line (width)
  "The board's first line: which section, and the filter."
  (romacs-truncate
   (concat (propertize (if romacs-agent-board--settled "Settled" "Agents") 'face 'romacs-agent-section)
           (if romacs-agent-board--words
               (propertize (concat "  /" (string-join romacs-agent-board--words " ")) 'face 'romacs-agent-mode)
             ""))
   width))

(defvar romacs-agent-board-mode-map)

(defun romacs-agent-board--keys-text (width)
  "The board's few keys within WIDTH; \\[describe-mode] says every one."
  (romacs-truncate
   (romacs-key-hint (concat "\\<romacs-agent-board-mode-map>\\[romacs-agent-board-open] open  "
                            "\\[romacs-agent-board-fold] fold  \\[romacs-agent-board-settle] settle  "
                            "\\[romacs-agent-board-toggle-settled] settled  \\[describe-mode] keys")
                    'romacs-agent-dim)
   width))

;;; Drawing

(defun romacs-agent-board--row ()
  "The row on point's line, or nil."
  (get-text-property (line-beginning-position) 'romacs-agent-board-row))

(defun romacs-agent-board--find (id)
  "Where the row named ID starts, or nil."
  (when id
    (save-excursion
      (goto-char (point-min))
      (when-let* ((at (text-property-search-forward 'romacs-agent-board-row id
                                                    (lambda (id row) (equal id (plist-get row :id))))))
        (prop-match-beginning at)))))

(defun romacs-agent-board--spot (position)
  "What a redraw keeps of POSITION: the id of its line's row and the line."
  (save-excursion
    (goto-char position)
    (cons (plist-get (romacs-agent-board--row) :id) (line-number-at-pos))))

(defun romacs-agent-board--back (spot)
  "Where SPOT (`romacs-agent-board--spot') is in the board drawn anew: its
row, else the start of its line."
  (or (romacs-agent-board--find (car spot))
      (save-excursion (goto-char (point-min)) (forward-line (1- (cdr spot))) (point))))

(defvar-local romacs-agent-board--width nil "The width the board was last drawn at.")

(defun romacs-agent-board--draw (rows width)
  "Draw ROWS, WIDTH columns wide, in the current buffer: the row point was
on stays under point, else its line."
  (let ((id (plist-get (romacs-agent-board--row) :id))
        (line (line-number-at-pos))
        (inhibit-read-only t))
    (setq romacs-agent-board--width width)
    (erase-buffer)
    (insert (romacs-agent-board--title-line width) "\n")
    (if (null rows)
        (insert (romacs-key-hint (if romacs-agent-board--settled "Nothing is settled."
                                   "No agent threads: \\<romacs-agent-board-mode-map>\\[romacs-agent-board-new] starts one.")
                                 'romacs-agent-dim)
                "\n")
      (dolist (row rows)
        (insert (propertize (romacs-agent-board--line row width) 'romacs-agent-board-row row) "\n")))
    (insert (romacs-agent-board--keys-text width))
    (goto-char (point-min))
    (let ((at (romacs-agent-board--find id)))
      (cond (at (goto-char at))
            ;; Shown anew: on its first thread.
            ((null id) (romacs-agent-board-next (if (cadr rows) 2 1)))
            (t (forward-line (max 1 (1- line)))
               (unless (romacs-agent-board--row) (romacs-agent-board-next 1)))))))

(defun romacs-agent-board--windows (buffer)
  "The windows showing BUFFER on a visible frame: a hidden layer's kept
frame shows nothing."
  (seq-filter (lambda (window) (frame-visible-p (window-frame window)))
              (get-buffer-window-list buffer nil t)))

(defun romacs-agent-board--columns (buffer)
  "The columns BUFFER's board has: its narrowest window's, else the
selected frame's.  Emoji draw a little wider than the two columns they
count: slack, so a row's last column is not cut."
  (- (let ((windows (romacs-agent-board--windows buffer)))
       (if windows (apply #'min (mapcar #'window-body-width windows)) (frame-width)))
     3))

(defun romacs-agent-board--render (buffer &optional columns)
  "Draw BUFFER's board COLUMNS wide, by default as wide as its windows:
each window showing it keeps the row at its point and at its start."
  (with-current-buffer buffer
    (let* ((windows (romacs-agent-board--windows buffer))
           (spots (mapcar (lambda (window)
                            (list window (romacs-agent-board--spot (window-point window))
                                  (romacs-agent-board--spot (window-start window))))
                          windows)))
      (let ((source romacs-agent-board-source))
        ;; Its rows are no one's while they are drawn: a draw cut short
        ;; leaves them so.
        (setq romacs-agent-board--drawn-source nil)
        (romacs-agent-board--draw
         (romacs-agent-board-rows (romacs-agent-board--sessions)
                                  (and (not romacs-agent-board--settled) (romacs-agent-board-source-history source))
                                  (romacs-agent-board--view))
         (or columns (romacs-agent-board--columns buffer)))
        ;; Its rows are SOURCE's once drawn: only then are they acted on as such.
        (setq romacs-agent-board--drawn-source source))
      (pcase-dolist (`(,window ,at ,start) spots)
        (set-window-point window (romacs-agent-board--back at))
        (set-window-start window (save-excursion (goto-char (romacs-agent-board--back start)) (line-beginning-position))
                          t))
      (when hl-line-mode
        (if windows (with-selected-window (car windows) (hl-line-highlight)) (hl-line-highlight)))
      (when (and windows (not romacs-agent-board--ticker)
                 (text-property-not-all (point-min) (point-max) 'romacs-agent-board-live nil))
        (setq romacs-agent-board--ticker (run-at-time 1 1 #'romacs-agent-board--tick))))))

;;; Cells at work, once a second

(defun romacs-agent-board--tick ()
  "Ask again each cell of a thread at work (`romacs-agent-board-columns'),
the same width, while the board shows one; else stop."
  (let ((buffer (romacs-agent-board--shown)) (live nil))
    (when buffer
      (with-current-buffer buffer
        (save-excursion
          (let ((inhibit-read-only t) (at (point-min)))
            (while (setq at (text-property-not-all at (point-max) 'romacs-agent-board-live nil))
              (let* ((end (next-single-property-change at 'romacs-agent-board-live nil (point-max)))
                     (spec (get-text-property at 'romacs-agent-board-live))
                     (row (get-text-property at 'romacs-agent-board-row))
                     (cell (romacs-agent-board--cell (funcall (car spec) row) (cadr spec))))
                (setq live t)
                (unless (equal (buffer-substring-no-properties at end) (substring-no-properties cell))
                  (goto-char at)
                  (delete-region at end)
                  (insert (propertize cell 'romacs-agent-board-live spec 'romacs-agent-board-row row)))
                (setq at (+ at (length cell)))))))))
    (unless live
      (cancel-timer romacs-agent-board--ticker)
      (setq romacs-agent-board--ticker nil))))

(defun romacs-agent-board--shown ()
  "The board's buffer, while it shows."
  (when-let* ((buffer (get-buffer romacs-agent-board--name))
              ((romacs-agent-board--windows buffer)))
    buffer))

(defun romacs-agent-board--refresh ()
  "Draw the board again while it shows: hidden, it is drawn when shown."
  (when-let* ((buffer (romacs-agent-board--shown)))
    (romacs-agent-board--render buffer)))

(add-hook 'romacs-agent-ui-changed-hook #'romacs-agent-board--refresh)
;; The sessions' rows made: those left unloaded show.
(add-hook 'romacs-agent-store-filled-hook #'romacs-agent-board--refresh)
;; The transcripts listed anew, aside: the board shows them.
(add-hook 'romacs-agent-history-indexed-hook #'romacs-agent-board--refresh)

(defun romacs-agent-board--redraw ()
  "Draw the board, from one of its commands."
  (romacs-agent-board--render (current-buffer)))

(defun romacs-agent-board--in (window)
  "WINDOW's buffer when it is a board, else nil: a window that shows
another buffer now runs the board's window hooks too."
  (let ((buffer (window-buffer window)))
    (and (provided-mode-derived-p (buffer-local-value 'major-mode buffer) 'romacs-agent-board-mode) buffer)))

(defun romacs-agent-board--shown-in (window)
  "Draw the board anew as WINDOW shows it: it was not followed hidden."
  (when-let* ((buffer (romacs-agent-board--in window)))
    (romacs-agent-board--render buffer)))

(defun romacs-agent-board--resized (window)
  "Draw the board again when WINDOW, showing it, changed its width."
  (when-let* ((buffer (romacs-agent-board--in window)))
    (unless (eql (romacs-agent-board--columns buffer) (buffer-local-value 'romacs-agent-board--width buffer))
      (romacs-agent-board--render buffer))))

;;; Commands

(defvar-local romacs-agent-board--drawn-source nil
  "The source whose sessions the board shows, drawn whole
\(`romacs-agent-board-source'); nil while it is drawn, or never was.")

(defun romacs-agent-board--acting ()
  "Refuse, unless this editor acts on the sessions the board shows, drawn
whole."
  (unless (and romacs-agent-board--drawn-source (romacs-agent-log-source-commands-p romacs-agent-board--drawn-source))
    (user-error "These sessions are another machine's: shown, not acted on from here")))

(defun romacs-agent-board--session ()
  "The session on this line, to act on."
  (romacs-agent-board--acting)
  (romacs-agent-board--live (or (plist-get (romacs-agent-board--row) :session) (user-error "No thread on this line"))))

(defun romacs-agent-board-next (&optional n)
  "Move to the Nth next row (previous, N negative)."
  (interactive "p" romacs-agent-board-mode)
  (let ((step (if (< (or n 1) 0) -1 1)))
    (dotimes (_ (abs (or n 1)))
      (let ((start (point)))
        (forward-line step)
        (while (and (not (romacs-agent-board--row)) (not (if (> step 0) (eobp) (bobp))))
          (forward-line step))
        (unless (romacs-agent-board--row) (goto-char start))))))

(defun romacs-agent-board-previous (&optional n)
  "Move to the Nth previous row."
  (interactive "p" romacs-agent-board-mode)
  (romacs-agent-board-next (- (or n 1))))

(defconst romacs-agent-board--here '(display-buffer-same-window)
  "Where what the board opens shows: in its window, as dired's files do.")

(defun romacs-agent-board--show-chat (session)
  "Show SESSION's chat in the board's window."
  (romacs-agent-show session romacs-agent-board--here romacs-agent-board-source))

(defun romacs-agent-board-open ()
  "Open the thread on this line, the transcript read-only, or a project's
newest thread; on \"more\", list every transcript."
  (interactive nil romacs-agent-board-mode)
  (let ((row (or (romacs-agent-board--row) (user-error "Nothing on this line"))))
    (when-let* ((first (plist-get row :first)))
      ;; A folded project: the row it would show first.
      (setq row first))
    (when (eq (plist-get row :kind) 'project)
      ;; A project: its first row, what wants the user most, else the newest.
      (romacs-agent-board-next 1)
      (setq row (romacs-agent-board--row))
      (when (memq (plist-get row :kind) '(project nil))
        (romacs-agent-board-previous 1)
        (user-error "Nothing to open in this project")))
    (pcase (plist-get row :kind)
      ('thread (romacs-agent-board--show-chat (romacs-agent-board--live (plist-get row :session))))
      ('history (romacs-agent-history-open (plist-get row :item) romacs-agent-board--here))
      ('more (romacs-agent-board-history)))))

(defun romacs-agent-board--live (session)
  "SESSION as it is now, unless no longer known: then the board is drawn again."
  (or (romacs-agent-board-source-live romacs-agent-board-source session)
      (progn (romacs-agent-board--redraw)
             (user-error "That thread is gone: the board is drawn again"))))

(cl-defmethod romacs-agent-board-source-live ((_source romacs-agent-log-local-source) session)
  "SESSION, loaded first when it was shown from its row alone."
  (or (and (eq (romacs-agent-loaded (romacs-agent-id session)) session) session)
      (and romacs-agent-board--unloaded
           (eq (gethash (romacs-agent-id session) (car romacs-agent-board--unloaded)) session)
           (romacs-agent-get (romacs-agent-id session)))))

(defun romacs-agent-board--fold-project (project folded)
  "Fold PROJECT, a directory, to its heading when FOLDED, else show its
threads; the store keeps it so."
  (if folded (puthash project t (romacs-agent-board--folded)) (remhash project (romacs-agent-board--folded)))
  (romacs-agent-store-fold project folded))

(defun romacs-agent-board--fold (open)
  "Show the threads this one delegated when OPEN is t, hide them when
nil, flip them when `toggle'; on a delegated thread, its parent's; on a
project, its threads."
  (let ((row (romacs-agent-board--row)))
    (cond ((eq (plist-get row :kind) 'project)
           ;; As kept: a filter shows a folded project's threads.
           (romacs-agent-board--fold-project (plist-get row :cwd)
                                             (if (eq open 'toggle)
                                                 (not (gethash (plist-get row :cwd) (romacs-agent-board--folded)))
                                               (not open))))
          (t (unless (plist-get row :children)
               (when-let* ((parent (and (plist-get row :session) (romacs-agent-board--parent (plist-get row :session)))))
                 (setq row (list :id (romacs-agent-id parent) :open t :children t))))
             (unless (plist-get row :children) (user-error "No threads under this one"))
             (puthash (plist-get row :id)
                      (if (if (eq open 'toggle) (not (plist-get row :open)) open) 'open 'closed)
                      (plist-get (romacs-agent-board--view) :folds))))
    (romacs-agent-board--redraw)
    (romacs-agent-board--goto (plist-get row :id))))

(defun romacs-agent-board--project-row ()
  "The row of the project point's line is in, or nil."
  (save-excursion
    (let ((row (romacs-agent-board--row)))
      (while (and row (not (eq (plist-get row :kind) 'project)) (zerop (forward-line -1)))
        (setq row (romacs-agent-board--row)))
      (and (eq (plist-get row :kind) 'project) row))))

(defun romacs-agent-board--goto (id)
  "Move to the row named ID, when it shows."
  (when-let* ((at (romacs-agent-board--find id))) (goto-char at)))

(defun romacs-agent-board-cycle-projects ()
  "Fold every project to its heading, or, when each shown is kept folded,
show them all, as `org-global-cycle' does."
  (interactive nil romacs-agent-board-mode)
  (let* ((projects (seq-filter (lambda (row) (eq (plist-get row :kind) 'project)) (romacs-agent-board--shown-rows)))
         (fold (seq-some (lambda (row) (not (gethash (plist-get row :cwd) (romacs-agent-board--folded)))) projects))
         (project (romacs-agent-board--project-row)))
    (unless projects (user-error "No projects"))
    (dolist (row projects) (romacs-agent-board--fold-project (plist-get row :cwd) fold))
    (romacs-agent-board--redraw)
    ;; Folded, a thread's line goes: its project's heading stays.
    (when (and fold project) (romacs-agent-board--goto (plist-get project :id)))))

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
  (let ((row (romacs-agent-board--row)))
    (if (eq (plist-get row :kind) 'history)
        (romacs-agent-history-resume (plist-get row :item) romacs-agent-board--here)
      (romacs-agent-resume (romacs-agent-board--session)))))

(defun romacs-agent-board-settle ()
  "Settle this thread: it leaves the board for the Settled section until
its next activity.  A settled one, bring back."
  (interactive nil romacs-agent-board-mode)
  (let ((session (romacs-agent-board--session)))
    (romacs-agent-settle session (romacs-agent-settled-p session))
    (romacs-agent-board--redraw)))

(defun romacs-agent-board--shown-rows ()
  "The rows the board shows, in order."
  (let ((rows nil) (at (point-min)))
    (while at
      (when-let* ((row (get-text-property at 'romacs-agent-board-row))) (push row rows))
      (setq at (next-single-property-change at 'romacs-agent-board-row)))
    (nreverse rows)))

(defun romacs-agent-board--finished (rows)
  "The threads of ROWS (top-level and delegated) with nothing going on."
  (cl-loop for row in rows
           when (and (eq (plist-get row :kind) 'thread)
                     (not (plist-get row :context))
                     (not (memq (plist-get row :key) romacs-agent-ui--live-keys))
                     (not (eq (plist-get row :key) 'settled)))
           collect (plist-get row :session)))

(defun romacs-agent-board-settle-finished ()
  "Settle every thread shown with nothing going on."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--acting)
  (let ((finished (romacs-agent-board--finished (romacs-agent-board--shown-rows))))
    (unless finished (user-error "No finished threads"))
    (when (y-or-n-p (format "Settle %d finished thread%s? " (length finished) (if (cdr finished) "s" "")))
      ;; Each by its id, loaded first when shown from its row alone; one
      ;; gone is skipped, and the board drawn once, after.
      (dolist (session finished)
        (when-let* ((live (romacs-agent-get (romacs-agent-id session))))
          (romacs-agent-settle live)))
      (romacs-agent-board--redraw))))

(defun romacs-agent-board-new ()
  "Start a new thread, in this row's project."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-board--acting)
  (let* ((row (romacs-agent-board--row))
         (cwd (cond ((plist-get row :session) (romacs-agent-cwd (plist-get row :session)))
                    ((plist-get row :cwd))
                    ((plist-get row :item) (plist-get (plist-get row :item) :cwd)))))
    (apply #'funcall-interactively #'romacs-agent-start
           (romacs-agent--start-arguments (if cwd (file-name-as-directory cwd) (romacs-agent--project-default))))))

(defun romacs-agent-board-filter ()
  "Show only the threads whose title, project, provider or model have
every word typed, the board following as they are typed."
  (interactive nil romacs-agent-board-mode)
  (let ((board (current-buffer)) (before romacs-agent-board--words))
    (cl-flet ((filter (text)
                (with-current-buffer board
                  (setq romacs-agent-board--words (split-string (downcase text) nil t))
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
  "Show the Settled section, or the board again."
  (interactive nil romacs-agent-board-mode)
  (setq romacs-agent-board--settled (not romacs-agent-board--settled))
  (romacs-agent-board--redraw)
  (goto-char (point-min))
  (romacs-agent-board-next 2))

(defun romacs-agent-board-history ()
  "Show every project's transcripts, or its newest few."
  (interactive nil romacs-agent-board-mode)
  (setq romacs-agent-board--all-history (not romacs-agent-board--all-history))
  (romacs-agent-board--redraw))

(defun romacs-agent-board--revert (&rest _)
  "Draw the board again, the harnesses' transcripts listed anew: its
`revert-buffer-function'."
  (romacs-agent-history-list)
  (romacs-agent-board--redraw))

(defvar-keymap romacs-agent-board-mode-map
  :doc "Keys of the agent board, besides `special-mode''s."
  :suppress t
  :parent special-mode-map
  "n" #'romacs-agent-board-next
  "p" #'romacs-agent-board-previous
  "f" #'romacs-agent-board-unfold
  "b" #'romacs-agent-board-close-fold
  "TAB" #'romacs-agent-board-fold
  "<backtab>" #'romacs-agent-board-cycle-projects
  "S-<tab>" #'romacs-agent-board-cycle-projects
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

(define-derived-mode romacs-agent-board-mode special-mode "Agents"
  "The agent board: every agent thread by project, what wants you first.
\\{romacs-agent-board-mode-map}"
  :interactive nil
  (setq-local truncate-lines t cursor-type nil
              revert-buffer-function #'romacs-agent-board--revert
              hl-line-face 'romacs-agent-board-row)
  (hl-line-mode)
  (add-hook 'window-buffer-change-functions #'romacs-agent-board--shown-in nil t)
  (add-hook 'window-size-change-functions #'romacs-agent-board--resized nil t))

(defun romacs-agent-board--buffer ()
  (or (get-buffer romacs-agent-board--name)
      (with-current-buffer (get-buffer-create romacs-agent-board--name)
        (romacs-agent-board-mode)
        (current-buffer))))

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
    (romacs-agent-board--render buffer)
    (when-let* ((window (display-buffer buffer '(display-buffer-same-window)))
                ;; A frame taking no focus (a floating menu) takes keys of its own.
                ((not (frame-parameter (window-frame window) 'no-accept-focus))))
      (unless (eq (window-frame window) (selected-frame)) (select-frame-set-input-focus (window-frame window)))
      (select-window window))))

;;;###autoload
(defalias 'romacs-agent-switch #'romacs-agents)
;;;###autoload
(defalias 'romacs-agent-switch-chat #'romacs-agents)

(provide 'romacs-agent-board)
;;; romacs-agent-board.el ends here

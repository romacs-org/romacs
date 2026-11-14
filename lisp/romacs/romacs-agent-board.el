;;; romacs-agent-board.el --- Agent threads on a floating board  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The board: every agent thread by project, in a centred layer over the
;; frame (romacs-popup.el's palette) that takes its own keys.  A thread's
;; row says its state (`romacs-agent-ui--state'), what wants the user
;; first, then the newest; the threads it delegated fold under it.
;; Settling a thread acknowledges it: it moves to the Settled section
;; (M-RET) until its next activity.  The harnesses' own transcripts of a
;; project follow its threads.
;;
;; Rows are plists made by pure functions of the sessions and the board's
;; view (`romacs-agent-board-rows'); the buffer is drawn from them, again
;; after each batch of session events while the board shows, never while
;; it is hidden.

;;; Code:
(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-ui)
(require 'romacs-popup)
(require 'romacs-key-hint)
(require 'romacs-truncate)
(require 'romacs-agent-history)

(defcustom romacs-agent-board-width 0.7
  "How much of the frame's width the agent board takes."
  :type 'float :group 'romacs-agent)

(defcustom romacs-agent-board-preview t
  "Whether a card beside the agent board shows the last reply of the
thread at point."
  :type 'boolean :group 'romacs-agent)

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

(defun romacs-agent-board--view ()
  "The board's view, from its buffer's choices, for `romacs-agent-board-rows'."
  (list :settled romacs-agent-board--settled :words romacs-agent-board--words
        :folds (or romacs-agent-board--folds (setq romacs-agent-board--folds (make-hash-table :test #'equal)))
        :all-history romacs-agent-board--all-history))

;;; Rows

(defun romacs-agent-board--parent (session)
  "The known session SESSION was delegated by, or nil."
  (romacs-agent-get (plist-get (romacs-agent-options session) :parent)))

(defun romacs-agent-board--key (session)
  "SESSION's state key on the board: its own, except that a delegated task
whose turn ended well is done here, where its parent's user reads it."
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
                       (dolist (child (romacs-agent-children session))
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
                    (seq-filter (lambda (child) (gethash child marks)) (romacs-agent-children session))
                    keys))
         (context (eq (gethash session marks) 'context))
         (open (and children (or context (romacs-agent-board--open-p session children keys view)))))
    (cons (list :kind 'thread :id (romacs-agent-id session) :session session :key (gethash session keys)
                :depth depth :open open :context context
                :children (mapcar (lambda (child) (gethash child keys)) children))
          (and open (mapcan (lambda (child) (romacs-agent-board--thread child (1+ depth) keys marks view))
                            children)))))

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
it dim for context.  Each row is a plist of :kind (`project', `thread',
`history' or `more') and :id, which names it across redraws."
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
      (mapcan (lambda (cwd)
                (let* ((threads (nreverse (gethash cwd roots)))
                       (rows (append (mapcan (lambda (session) (romacs-agent-board--thread session 0 keys marks view))
                                             threads)
                                     (romacs-agent-board--history-rows cwd (nreverse (gethash cwd items)) view))))
                  (and rows
                       (cons (list :kind 'project :id cwd :cwd cwd :keys (mapcar (lambda (s) (gethash s keys)) threads))
                             rows))))
              (nreverse order)))))

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
  "ROW's provider and model."
  (let ((session (plist-get row :session)))
    (propertize (romacs-agent-ui--instance (romacs-agent-provider session) (romacs-agent-harness session)
                                           (or (romacs-agent-model session)
                                               (plist-get (romacs-agent-options session) :model))
                                           nil)
                'face 'romacs-agent-model)))

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
    ('project (concat (propertize (file-name-nondirectory (plist-get row :cwd)) 'face 'romacs-agent-section)
                      "  " (propertize (plist-get row :cwd) 'face 'romacs-agent-dim)
                      (if-let* ((rollup (romacs-agent-ui--rollup (plist-get row :keys)))) (concat "  " rollup) "")))
    ('thread (romacs-agent-board--thread-line row width))
    ('history (let ((item (plist-get row :item)))
                (propertize (romacs-truncate (concat "  ◇ " (plist-get item :title) "  " (or (plist-get item :provider) "")
                                                     "  " (romacs-agent-ui--ago (plist-get item :mtime)))
                                             width)
                            'face 'romacs-agent-dim)))
    ('more (romacs-key-hint (format "  ◇ %d more  (\\<romacs-agent-board-mode-map>\\[romacs-agent-board-history] shows all)"
                                    (plist-get row :count))
                            'romacs-agent-dim))))

(defun romacs-agent-board--title-line (width)
  "The board's first line: which section, and the filter."
  (romacs-truncate
   (concat (propertize (if romacs-agent-board--settled "Settled" "Agents") 'face 'romacs-agent-section)
           (if romacs-agent-board--words
               (propertize (concat "  /" (string-join romacs-agent-board--words " ")) 'face 'romacs-agent-mode)
             ""))
   width))

(defvar romacs-agent-board-mode-map)
(defvar-local romacs-agent-board--all-keys nil "Every key shows, not the few.")

(defconst romacs-agent-board--key-groups
  '(("Move" (romacs-agent-board-next . "next") (romacs-agent-board-previous . "previous")
     (romacs-agent-board-fold . "fold") (romacs-agent-board-unfold . "unfold") (romacs-agent-board-close-fold . "fold up"))
    ("Thread" (romacs-agent-board-open . "open") (romacs-agent-board-approve . "approve")
     (romacs-agent-board-interrupt . "interrupt") (romacs-agent-board-stop . "stop")
     (romacs-agent-board-resume . "resume") (romacs-agent-board-settle . "settle") (romacs-agent-board-new . "new"))
    ("Board" (romacs-agent-board-settle-finished . "settle finished") (romacs-agent-board-filter . "filter")
     (romacs-agent-board-toggle-settled . "settled") (romacs-agent-board-history . "transcripts")
     (romacs-agent-board-refresh . "refresh") (romacs-agent-board-quit . "close")))
  "The board's keys by group, each command with its word.")

(defun romacs-agent-board--keys-text (width)
  "The board's keys within WIDTH: the few, or every one."
  (if romacs-agent-board--all-keys
      (romacs-popup-key-grid
       (mapcar (lambda (group)
                 (cons (car group)
                       (mapcar (lambda (entry)
                                 (cons (key-description (where-is-internal (car entry) romacs-agent-board-mode-map t))
                                       (cdr entry)))
                               (cdr group))))
               romacs-agent-board--key-groups)
       width)
    (romacs-truncate
     (romacs-key-hint (concat "\\<romacs-agent-board-mode-map>\\[romacs-agent-board-open] open  "
                              "\\[romacs-agent-board-fold] fold  \\[romacs-agent-board-settle] settle  "
                              "\\[romacs-agent-board-toggle-settled] settled  \\[romacs-agent-board-keys] keys")
                      'romacs-agent-dim)
     width)))

;;; Drawing

(defun romacs-agent-board--row ()
  "The row on point's line, or nil."
  (get-text-property (line-beginning-position) 'romacs-agent-board-row))

(defvar-local romacs-agent-board--overlay nil "The highlight of the row at point.")

(defun romacs-agent-board--follow ()
  "Highlight the row at point."
  (unless (overlayp romacs-agent-board--overlay)
    (setq romacs-agent-board--overlay (make-overlay (point) (point)))
    (overlay-put romacs-agent-board--overlay 'face 'romacs-agent-board-row))
  (if (romacs-agent-board--row)
      (move-overlay romacs-agent-board--overlay (line-beginning-position) (line-beginning-position 2))
    (delete-overlay romacs-agent-board--overlay)))

(defun romacs-agent-board--draw (rows width)
  "Draw ROWS, WIDTH columns wide, in the current buffer: the row point was
on stays under point, else its line."
  (let ((id (plist-get (romacs-agent-board--row) :id))
        (line (line-number-at-pos))
        (inhibit-read-only t))
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
    (let ((at (and id (text-property-search-forward 'romacs-agent-board-row id
                                                    (lambda (id row) (equal id (plist-get row :id)))))))
      (cond (at (goto-char (prop-match-beginning at)))
            ;; Shown anew: on its first thread.
            ((null id) (romacs-agent-board-next (if (cadr rows) 2 1)))
            (t (forward-line (max 1 (1- line)))
               (unless (romacs-agent-board--row) (romacs-agent-board-next 1)))))
    (romacs-agent-board--follow)))

(defun romacs-agent-board--columns ()
  "The board's width in columns, of the frame's."
  (max 40 (floor (* (frame-width (romacs-layer--root-frame)) romacs-agent-board-width))))

(defun romacs-agent-board--render (buffer columns)
  "Draw BUFFER's board COLUMNS wide, in its window when it shows (point
moves there)."
  (let ((draw (lambda ()
                (romacs-agent-board--draw
                 (romacs-agent-board-rows (hash-table-values romacs-agent--sessions)
                                          (and (not romacs-agent-board--settled) (romacs-agent-history-items))
                                          (romacs-agent-board--view))
                 ;; Emoji draw a little wider than the two columns they
                 ;; count: slack, so a row's last column is not cut.
                 (- columns 3))))
        (window (get-buffer-window buffer t)))
    (if window (with-selected-window window (funcall draw)) (with-current-buffer buffer (funcall draw)))))

(defvar romacs-agent-board--preview-columns nil "The preview's width, while there is room for it.")

(defun romacs-agent-board--placement (columns lines)
  "Where a board COLUMNS by LINES goes, its top-left in pixels, centred
with its preview beside it when there is room for one
\(`romacs-agent-board--preview-columns' says how wide)."
  (let* ((root (romacs-layer--root-frame))
         (room (- (frame-width root) columns 8))
         (inset (frame-internal-border-width root)))
    (setq romacs-agent-board--preview-columns (and romacs-agent-board-preview (>= room 24) (min 60 room)))
    (cons (+ inset (max 0 (/ (- (frame-text-width root)
                                (* (frame-char-width root) (+ columns (if romacs-agent-board--preview-columns
                                                                          (+ romacs-agent-board--preview-columns 3)
                                                                        0))))
                             2)))
          (+ inset (max 0 (/ (- (frame-text-height root) (* (frame-char-height root) lines)) 2))))))

(defun romacs-agent-board--show (buffer)
  "Draw BUFFER's board and show it, sized to its lines and the frame,
with its preview beside it; its cells at work follow the clock."
  (let* ((columns (romacs-agent-board--columns))
         (most (max 6 (- (frame-text-lines (romacs-layer--root-frame)) 6)))
         (lines (progn (romacs-agent-board--render buffer columns)
                       (min most (with-current-buffer buffer (count-lines (point-min) (point-max)))))))
    (romacs-popup-show 'palette buffer :columns columns :max-lines most :lines lines
                       :anchor (romacs-agent-board--placement columns lines))
    (when-let* ((window (get-buffer-window buffer t)))
      (with-selected-window window (romacs-agent-board--preview)))
    (unless romacs-agent-board--ticker
      (setq romacs-agent-board--ticker (run-at-time 1 1 #'romacs-agent-board--tick)))))

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

;;; The preview: the last reply of the thread at point, beside the board

(defconst romacs-agent-board--preview-name " *romacs-agents-preview*")

(defun romacs-agent-board--preview-place ()
  "Where the preview goes, right of the selected board's frame: (LEFT TOP
COLUMNS LINES), or nil when the board left no room for it."
  (let ((frame (selected-frame)))
    (and romacs-agent-board--preview-columns (frame-parent frame)
         (list (+ (car (frame-position frame)) (frame-outer-width frame) 12) (cdr (frame-position frame))
               romacs-agent-board--preview-columns (frame-text-lines frame)))))

(defun romacs-agent-board--preview-hide ()
  (romacs-popup-hide romacs-agent-board--preview-name))

(defun romacs-agent-board--preview ()
  "Show the last reply of the thread at point beside the board, in its
window; with none, or no room, hide the card."
  (let* ((session (plist-get (romacs-agent-board--row) :session))
         (reply (and romacs-agent-board-preview session (romacs-agent-reply session)))
         (place (and reply (romacs-agent-board--preview-place)))
         ;; Its plain text, no more than the card holds: a reply may be
         ;; long, and formatting Markdown costs tens of milliseconds a key.
         (text (and place (substring reply 0 (min (length reply) (* (nth 2 place) (nth 3 place)))))))
    (if (not (and text (string-match-p "[^[:space:]]" text)))
        (romacs-agent-board--preview-hide)
      (pcase-let ((`(,left ,top ,columns ,lines) place)
                  (buffer (get-buffer-create romacs-agent-board--preview-name)))
        (with-current-buffer buffer
          (let ((inhibit-read-only t))
            (erase-buffer)
            (insert (propertize (romacs-agent-title session) 'face 'romacs-agent-section) "\n" text))
          (goto-char (point-min))
          (setq-local mode-line-format nil cursor-type nil)
          (romacs-layer-wrap-words))
        (romacs-popup-show 'card buffer :anchor (cons left top) :columns columns :lines lines)))))

(defun romacs-agent-board--shown ()
  "The board's buffer, while it shows."
  (when-let* ((buffer (get-buffer romacs-agent-board--name))
              (window (get-buffer-window buffer t))
              ((frame-visible-p (window-frame window))))
    buffer))

(defun romacs-agent-board--refresh ()
  "Draw the board again while it shows: hidden, it is drawn when shown."
  (when-let* ((buffer (romacs-agent-board--shown)))
    (romacs-agent-board--show buffer)))

(add-hook 'romacs-agent-ui-changed-hook #'romacs-agent-board--refresh)

(defun romacs-agent-board--redraw ()
  "Draw the board, from one of its commands."
  (romacs-agent-board--show (current-buffer)))

;;; Commands

(defun romacs-agent-board--session ()
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

(defun romacs-agent-board--show-chat (session)
  "Show SESSION's chat in the window the user was in, once the board hides."
  (romacs-popup-then (lambda ()
                       (let ((display-buffer-overriding-action '(display-buffer-same-window)))
                         (romacs-agent-show session)))))

(defun romacs-agent-board-open ()
  "Open the thread on this line, the transcript read-only, or a project's
newest thread; on \"more\", list every transcript."
  (interactive nil romacs-agent-board-mode)
  (let ((row (or (romacs-agent-board--row) (user-error "Nothing on this line"))))
    (when (eq (plist-get row :kind) 'project)
      ;; A project: its first row, what wants the user most, else the newest.
      (romacs-agent-board-next 1)
      (setq row (romacs-agent-board--row))
      (when (memq (plist-get row :kind) '(project nil))
        (romacs-agent-board-previous 1)
        (user-error "Nothing to open in this project")))
    (pcase (plist-get row :kind)
      ('thread (romacs-agent-board--show-chat (romacs-agent-board--live (plist-get row :session))))
      ('history (let ((item (plist-get row :item))) (romacs-popup-then (lambda () (romacs-agent-history-open item)))))
      ('more (romacs-agent-board-history)))))

(defun romacs-agent-board--live (session)
  "SESSION, unless no longer known: then the board is drawn again."
  (or (and (eq (romacs-agent-get (romacs-agent-id session)) session) session)
      (progn (romacs-agent-board--redraw)
             (user-error "That thread is gone: the board is drawn again"))))

(defun romacs-agent-board--fold (open)
  "Show the threads this one delegated when OPEN is t, hide them when
nil, flip them when `toggle'; on a delegated thread, its parent's."
  (let ((row (romacs-agent-board--row)))
    (unless (plist-get row :children)
      (when-let* ((parent (and (plist-get row :session) (romacs-agent-board--parent (plist-get row :session)))))
        (setq row (list :id (romacs-agent-id parent) :open t :children t))))
    (unless (plist-get row :children) (user-error "No threads under this one"))
    (puthash (plist-get row :id)
             (if (if (eq open 'toggle) (not (plist-get row :open)) open) 'open 'closed)
             (plist-get (romacs-agent-board--view) :folds))
    (romacs-agent-board--redraw)
    (romacs-agent-board--goto (plist-get row :id))))

(defun romacs-agent-board--goto (id)
  "Move to the row named ID, when it shows."
  (let ((at (save-excursion (goto-char (point-min))
                            (text-property-search-forward 'romacs-agent-board-row id
                                                          (lambda (id row) (equal id (plist-get row :id)))))))
    (when at (goto-char (prop-match-beginning at)) (romacs-agent-board--follow))))

(defun romacs-agent-board-fold ()
  "Fold or unfold the threads this one delegated."
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
    (cond (tool (romacs-agent-respond session tool t))
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
        (let ((item (plist-get row :item))) (romacs-popup-then (lambda () (romacs-agent-history-resume item))))
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
  (let ((finished (romacs-agent-board--finished (romacs-agent-board--shown-rows))))
    (unless finished (user-error "No finished threads"))
    (when (y-or-n-p (format "Settle %d finished thread%s? " (length finished) (if (cdr finished) "s" "")))
      (mapc #'romacs-agent-settle finished)
      (romacs-agent-board--redraw))))

(defun romacs-agent-board-new ()
  "Start a new thread, in this row's project."
  (interactive nil romacs-agent-board-mode)
  (let* ((row (romacs-agent-board--row))
         (cwd (cond ((plist-get row :session) (romacs-agent-cwd (plist-get row :session)))
                    ((plist-get row :cwd))
                    ((plist-get row :item) (plist-get (plist-get row :item) :cwd)))))
    (romacs-popup-then (lambda ()
                         (apply #'funcall-interactively #'romacs-agent-start
                                (romacs-agent--start-arguments (if cwd (file-name-as-directory cwd)
                                                                 (romacs-agent--project-default))))))))

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
  (romacs-agent-board-next 2)
  (romacs-agent-board--follow))

(defun romacs-agent-board-history ()
  "Show every project's transcripts, or its newest few."
  (interactive nil romacs-agent-board-mode)
  (setq romacs-agent-board--all-history (not romacs-agent-board--all-history))
  (romacs-agent-board--redraw))

(defun romacs-agent-board-refresh ()
  "Draw the board again, the harnesses' transcripts listed anew."
  (interactive nil romacs-agent-board-mode)
  (romacs-agent-history-list)
  (romacs-agent-board--redraw))

(defun romacs-agent-board-quit ()
  "Close the board."
  (interactive nil romacs-agent-board-mode)
  (romacs-popup-hide (current-buffer))
  (romacs-agent-board--preview-hide))

(defun romacs-agent-board-keys ()
  "Show every key of the board, or the few."
  (interactive nil romacs-agent-board-mode)
  (setq romacs-agent-board--all-keys (not romacs-agent-board--all-keys))
  (romacs-agent-board--redraw))

(defvar-keymap romacs-agent-board-mode-map
  :doc "Keys of the agent board."
  :suppress t
  "n" #'romacs-agent-board-next
  "p" #'romacs-agent-board-previous
  "C-n" #'romacs-agent-board-next
  "C-p" #'romacs-agent-board-previous
  "f" #'romacs-agent-board-unfold
  "b" #'romacs-agent-board-close-fold
  "TAB" #'romacs-agent-board-fold
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
  "h" #'romacs-agent-board-history
  "g" #'romacs-agent-board-refresh
  "?" #'romacs-agent-board-keys
  "q" #'romacs-agent-board-quit
  "C-g" #'romacs-agent-board-quit)

;; The keys the board names, of the two each of these has.
(put 'romacs-agent-board-next :advertised-binding [?n])
(put 'romacs-agent-board-previous :advertised-binding [?p])
(put 'romacs-agent-board-quit :advertised-binding [?q])

(define-derived-mode romacs-agent-board-mode special-mode "Agents"
  "The agent board: every agent thread by project, what wants you first.
\\{romacs-agent-board-mode-map}"
  :interactive nil
  (setq-local truncate-lines t cursor-type nil mode-line-format nil)
  (add-hook 'romacs-popup-key-hook #'romacs-agent-board--follow nil t)
  (add-hook 'romacs-popup-key-hook #'romacs-agent-board--preview nil t))

(defun romacs-agent-board--buffer ()
  (or (get-buffer romacs-agent-board--name)
      (with-current-buffer (get-buffer-create romacs-agent-board--name)
        (romacs-agent-board-mode)
        (current-buffer))))

;;;###autoload
(defun romacs-agents ()
  "Show the agent board over the frame: every agent thread by project,
those that want you first, then the newest, with what each delegated
folded under it.  Its keys act on the thread at point;
\\<romacs-agent-board-mode-map>\\[romacs-agent-board-quit] closes it.
\\{romacs-agent-board-mode-map}"
  (interactive)
  (romacs-agent-restore)
  (romacs-agent-history-list)
  (let ((buffer (romacs-agent-board--buffer)))
    (romacs-agent-board--show buffer)
    (romacs-popup-take-keys buffer #'romacs-agent-board--preview-hide)))

;;;###autoload
(defalias 'romacs-agent-switch #'romacs-agents)
;;;###autoload
(defalias 'romacs-agent-switch-chat #'romacs-agents)

(provide 'romacs-agent-board)
;;; romacs-agent-board.el ends here

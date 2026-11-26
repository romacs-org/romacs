;;; romacs-list.el --- Lists of rows, a line each  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A list: rows drawn as lines in a buffer of `romacs-list-mode' (or a mode
;; derived from it), under a banner (romacs-banner.el) and over a line of
;; key hints.  What a row is belongs to the mode: a plist with at least
;; :id, which names it across redraws, and :kind.  The list owns drawing,
;; point kept on its row across redraws in every window showing it, row
;; motion past rules, columns fitted to the width, cells asked again every
;; second while their row is live, drill-down into a level, and the banner.
;;
;; A mode derived from `romacs-list-mode' sets, buffer-locally:
;;
;;   `romacs-list-redraw-function'  draws the buffer anew, with
;;                                  `romacs-list-render' of its rows
;;   `romacs-list-line-function'    a row as a line WIDTH wide; columns
;;                                  through `romacs-list-cells'
;;   `romacs-list-group-kind'       the :kind of a group's row: a rule
;;                                  before each group but the first
;;   `romacs-list-empty-function'   the text shown with no rows
;;   `romacs-list-hint-function'    the last line, WIDTH wide
;;   `romacs-list-banner'           its banner's pattern, nil for none
;;   `romacs-list-banner-faces'     its colours, 1 to 4 faces
;;   `romacs-list-live-function'    whether ROWS have something live: the
;;                                  banner moves only then
;;   `romacs-list-fresh-function'   a live row as it is now, for its cells
;;   `romacs-list-level-variable'   where its level is kept (drill-down)
;;
;; It draws once per batch: a mode calls its redraw after a batch of
;; changes, never per row.
;;
;; loadup.el loads it before tabulated-list.el, whose mode derives from
;; it, while this directory is not yet on `load-path': what it uses from
;; here loads later, truncation as a list's mode starts, the banner and
;; hl-line as they are first used.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(declare-function romacs-truncate "romacs-truncate" (string width &optional padding))
(declare-function romacs-banner-still "romacs-banner" (pattern cols rows faces &optional epoch moving))
(declare-function romacs-banner-show "romacs-banner" (id buffer start pattern cols rows faces moving &optional epoch))
(declare-function romacs-banner-stop "romacs-banner" (id))
(declare-function romacs-banner-random-epoch "romacs-banner" ())
(declare-function hl-line-highlight "hl-line" ())

(defgroup romacs-list nil
  "Lists of rows, a line each."
  :group 'romacs)

(defface romacs-list-row '((t :inherit hl-line :extend t))
  "A list's row at point: the theme's current line, its background alone,
so each column keeps its own face.")

(defface romacs-list-rule '((t :inherit shadow))
  "The rule before a list's group.")

;;; What a mode sets

(defvar-local romacs-list-redraw-function #'ignore
  "Draw the current buffer's list anew: compute its rows and call
`romacs-list-render'.  Called with no argument; with one non-nil, the
data under the rows changed.")

(defvar-local romacs-list-line-function (lambda (row _width) (format "%S" (plist-get row :id)))
  "A row as a line: called with the row and the width in columns.")

(defvar-local romacs-list-group-kind nil
  "The :kind of a group's row: a rule goes before each such row but the
first.  Nil: no groups.")

(defvar-local romacs-list-rule-face 'romacs-list-rule "The face of the rule before a group.")

(defvar-local romacs-list-empty-function (lambda () "")
  "The text shown when there is no row.")

(defvar-local romacs-list-hint-function nil
  "The list's last line, a function of its width giving it, or nil for none.")

(defvar-local romacs-list-banner nil
  "The pattern of the list's banner, its first line, as
`romacs-banner-still' takes it, `random' for one of them chosen as the
buffer is drawn first, or nil for no banner.  A mode sets its default;
`romacs-list-banners' chooses per mode.")

(defvar-local romacs-list-banner-faces '(romacs-working-running)
  "The banner's colour stops, 1 to 4 faces: their foregrounds.")

(defvar-local romacs-list-live-function #'ignore
  "Whether the list's rows, its argument, have something live: its banner
moves only while they do.")

(defvar-local romacs-list-fresh-function #'identity
  "A row as it is now, from the row drawn: what its live cells are asked
again of, once a second.")

(defvar-local romacs-list-level-variable 'romacs-list-level
  "The buffer-local variable holding the list's level, what
`romacs-list-enter' goes into: nil, the top.")

(defvar-local romacs-list-level nil "The list's level, unless its mode keeps it elsewhere.")

(defcustom romacs-list-banners nil
  "Each list's banner, by its major mode: (MODE . PATTERN), PATTERN one
of `romacs-banner-still''s, `random' for one chosen as each list is
drawn first, or nil for none.  A mode not named keeps its own (the agent
board's is `thread')."
  :type '(alist :key-type (symbol :tag "Major mode")
                :value-type (choice (const :tag "The chat's working thread" thread)
                                    (const wave) (const rain) (const sparkle) (const sweep)
                                    (const :tag "One at random" random)
                                    (const :tag "None" nil)))
  :group 'romacs-list)

(defconst romacs-list--patterns '(thread wave rain sparkle sweep) "The banner's patterns.")

;;; Rows in the buffer

(defun romacs-list-row ()
  "The row on point's line, or nil: anywhere on it, its newline too, as a
mark put over its first columns (`tabulated-list-put-tag') carries none."
  (let ((start (line-beginning-position)))
    (or (get-text-property start 'romacs-list-row)
        (let* ((end (min (point-max) (1+ (line-end-position))))
               (at (next-single-property-change start 'romacs-list-row nil end)))
          (and (< at end) (get-text-property at 'romacs-list-row))))))

(defun romacs-list--find (id)
  "Where the line of the row named ID starts, or nil."
  (when id
    (save-excursion
      (goto-char (point-min))
      (when-let* ((at (text-property-search-forward 'romacs-list-row id
                                                    (lambda (id row) (equal id (plist-get row :id))))))
        (goto-char (prop-match-beginning at))
        (line-beginning-position)))))

(defun romacs-list-goto (id)
  "Move to the row named ID, when it shows; return where it starts."
  (when-let* ((at (romacs-list--find id))) (goto-char at)))

(defun romacs-list-shown-rows ()
  "The rows the list shows, in order: one a line."
  (let ((rows nil))
    (save-excursion
      (goto-char (point-min))
      (while (not (eobp))
        (when-let* ((row (romacs-list-row))) (push row rows))
        (forward-line 1)))
    (nreverse rows)))

(defun romacs-list--spot (position)
  "What a redraw keeps of POSITION: the id of its line's row and the line."
  (save-excursion
    (goto-char position)
    (cons (plist-get (romacs-list-row) :id) (line-number-at-pos))))

(defun romacs-list--back (spot)
  "Where SPOT (`romacs-list--spot') is in the list drawn anew: its row,
else the start of its line."
  (or (romacs-list--find (car spot))
      (save-excursion (goto-char (point-min)) (forward-line (1- (cdr spot))) (point))))

;;; Columns

(defun romacs-list-cell (text width)
  "TEXT in exactly WIDTH columns: padded when it fits, a mark of several
code points kept whole, else cut."
  (let* ((text (or text ""))
         (cut (if (<= (string-width text) width) text (romacs-truncate text width))))
    ;; Cut by its characters' widths, a composed mark can still run over.
    (while (> (string-width cut) width) (setq cut (substring cut 0 -1)))
    (concat cut (make-string (- width (string-width cut)) ?\s))))

(defun romacs-list--widths (columns width head)
  "COLUMNS' widths in a row WIDTH wide after HEAD columns: `fill' takes
what the others and the gaps between them leave."
  (let* ((fixed (mapcar (lambda (column)
                          (let ((w (cdr column))) (cond ((eq w 'fill) nil) ((functionp w) (funcall w)) (t w))))
                        columns))
         (rest (- width head (apply #'+ (delq nil (copy-sequence fixed))) (1- (length fixed)))))
    (mapcar (lambda (w) (or w (max 8 rest))) fixed)))

(defun romacs-list--cell-of (function width row live)
  "FUNCTION's cell for ROW, WIDTH wide; LIVE, a cell to ask again."
  (let ((cell (romacs-list-cell (funcall function row) width)))
    (if (and live (symbolp function) (get function 'romacs-list-live))
        (propertize cell 'romacs-list-live (list function width))
      cell)))

(defun romacs-list-cells (columns row width &optional head live)
  "ROW in a line WIDTH wide: HEAD, then COLUMNS, each (FUNCTION . WIDTH).
FUNCTION, called with ROW, gives its cell's text or nil.  WIDTH is a
number, `fill' for the column taking what the others leave, or a
function giving the number.  While LIVE, a cell whose FUNCTION has a
non-nil `romacs-list-live' property is asked again every second.  A line
too narrow for every column loses the last ones, and keeps still: a cut
cell could not keep its width."
  (let* ((head (or head ""))
         (widths (romacs-list--widths columns width (string-width head))))
    (cl-flet ((line (live)
                (concat head (mapconcat #'identity
                                        (cl-mapcar (lambda (column width) (romacs-list--cell-of (car column) width row live))
                                                   columns widths)
                                        " "))))
      (let ((line (line live)))
        (if (<= (string-width line) width) line (romacs-truncate (line nil) width))))))

;;; The banner

(defvar-local romacs-list--pattern nil "The banner's pattern, chosen as the list is drawn first.")
(defvar-local romacs-list--epoch nil "The seed of the banner's randomness, set once a list.")

(defun romacs-list--banner-pattern ()
  "The list's banner pattern, chosen once a buffer: `romacs-list-banners'
for its mode, else the mode's own; `random', one of them."
  (car (with-memoization romacs-list--pattern
         (let* ((chosen (assq major-mode romacs-list-banners))
                (pattern (if chosen (cdr chosen) romacs-list-banner)))
           (list (if (eq pattern 'random) (seq-random-elt romacs-list--patterns) pattern))))))

(defun romacs-list--banner-id ()
  "The id the buffer's banner shows by."
  (cons 'romacs-list (current-buffer)))

(defun romacs-list--banner (pattern rows width)
  "WIDTH columns of the banner in PATTERN at rest, for ROWS."
  (require 'romacs-banner)
  (romacs-banner-still pattern width 1 romacs-list-banner-faces
                       (with-memoization romacs-list--epoch (romacs-banner-random-epoch))
                       (funcall romacs-list-live-function rows)))

;;; Drawing

(defvar-local romacs-list--width nil "The width the list was last drawn at.")
(defvar-local romacs-list--ticker nil "The timer asking the list's live cells again.")

(defun romacs-list--level ()
  "The list's level."
  (symbol-value romacs-list-level-variable))

(defun romacs-list--insert-rows (rows width)
  "Insert ROWS as lines WIDTH wide, a rule before each group but the first."
  (let ((first t))
    (dolist (row rows)
      (when (and romacs-list-group-kind (eq (plist-get row :kind) romacs-list-group-kind) (not first))
        (insert (propertize (make-string width ?─) 'face romacs-list-rule-face) "\n"))
      (setq first nil)
      (insert (propertize (funcall romacs-list-line-function row width) 'romacs-list-row row) "\n"))))

(defun romacs-list--stop-banner ()
  "Show the list's banner no more: there is none unless one was shown."
  (when (featurep 'romacs-banner) (romacs-banner-stop (romacs-list--banner-id))))

(defun romacs-list-draw (rows width)
  "Draw ROWS, WIDTH columns wide, in the current buffer: the row point was
on stays under point, else its line.  Shown anew, point is on the first
row, in a level past its heading."
  (let ((id (plist-get (romacs-list-row) :id))
        (line (line-number-at-pos))
        (pattern (romacs-list--banner-pattern))
        (inhibit-read-only t))
    (setq romacs-list--width width)
    (erase-buffer)
    (when pattern (insert (romacs-list--banner pattern rows width) "\n"))
    (if rows
        (romacs-list--insert-rows rows width)
      (insert (funcall romacs-list-empty-function) "\n"))
    (when romacs-list-hint-function (insert (funcall romacs-list-hint-function width)))
    (if pattern
        (romacs-banner-show (romacs-list--banner-id) (current-buffer) (point-min) pattern width 1
                            romacs-list-banner-faces (funcall romacs-list-live-function rows) romacs-list--epoch)
      (romacs-list--stop-banner))
    (goto-char (point-min))
    (cond ((romacs-list-goto id))
          ((null id) (unless (romacs-list-row) (romacs-list-next 1))
           (when (and (romacs-list--level) (cadr rows)) (romacs-list-next 1)))
          (t (forward-line (max 1 (1- line)))
             (unless (romacs-list-row) (romacs-list-next 1))))))

(defun romacs-list-windows (buffer)
  "The windows showing BUFFER on a visible frame: a hidden layer's kept
frame shows nothing."
  (seq-filter (lambda (window) (frame-visible-p (window-frame window)))
              (get-buffer-window-list buffer nil t)))

(defun romacs-list-columns (buffer)
  "The columns BUFFER's list has: its narrowest window's, else the
selected frame's.  Emoji draw a little wider than the two columns they
count: slack, so a row's last column is not cut."
  (- (let ((windows (romacs-list-windows buffer)))
       (if windows (apply #'min (mapcar #'window-body-width windows)) (frame-width)))
     3))

(defun romacs-list-highlight ()
  "Highlight the row at point now: hl-line moves it after a command, and a
draw that no command made is followed by none.  Not from a buffer the
selected window does not show while the highlight follows that window
\(`hl-line-sticky-flag' nil): it would take the highlight from it."
  (when (and (bound-and-true-p hl-line-mode)
             (or (bound-and-true-p hl-line-sticky-flag) (eq (window-buffer) (current-buffer))))
    (hl-line-highlight)))

(defun romacs-list-render (buffer rows &optional width)
  "Draw ROWS in BUFFER's list, WIDTH columns wide, by default as wide as
its windows: each window showing it keeps the row at its point and at
its start, or once drawn first, the row the draw put point on."
  (with-current-buffer buffer
    (let* ((windows (romacs-list-windows buffer))
           (spots (mapcar (lambda (window)
                            (list window (romacs-list--spot (window-point window))
                                  (romacs-list--spot (window-start window))))
                          windows))
           (drawn romacs-list--width))
      (romacs-list-draw rows (or width (romacs-list-columns buffer)))
      (pcase-dolist (`(,window ,at ,start) spots)
        (if (not drawn)
            ;; Drawn first: where the draw put point.
            (set-window-point window (point))
          (set-window-point window (romacs-list--back at))
          (set-window-start window (save-excursion (goto-char (romacs-list--back start)) (line-beginning-position))
                            t)))
      (if windows (with-selected-window (car windows) (romacs-list-highlight)) (romacs-list-highlight))
      (when (and windows (not romacs-list--ticker)
                 (text-property-not-all (point-min) (point-max) 'romacs-list-live nil))
        (setq romacs-list--ticker (run-at-time 1 1 #'romacs-list--tick buffer))))))

;;; Cells at work, once a second

(defun romacs-list--tick (buffer)
  "Ask again each live cell of BUFFER's list (`romacs-list-cells'), the
same width, of its row as it is now (`romacs-list-fresh-function'),
while the list shows one; else stop."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (let ((live nil) (now (make-hash-table :test #'eq)))
        (when (romacs-list-windows buffer)
          (save-excursion
            (let ((inhibit-read-only t) (at (point-min)))
              (while (setq at (text-property-not-all at (point-max) 'romacs-list-live nil))
                (let* ((end (next-single-property-change at 'romacs-list-live nil (point-max)))
                       (spec (get-text-property at 'romacs-list-live))
                       (row (get-text-property at 'romacs-list-row))
                       (fresh (with-memoization (gethash row now) (funcall romacs-list-fresh-function row)))
                       (cell (romacs-list-cell (funcall (car spec) fresh) (cadr spec))))
                  (setq live t)
                  (unless (equal (buffer-substring-no-properties at end) (substring-no-properties cell))
                    (goto-char at)
                    (delete-region at end)
                    (insert (propertize cell 'romacs-list-live spec 'romacs-list-row row)))
                  (setq at (+ at (length cell))))))))
        (unless live (romacs-list--stop-ticker))))))

(defun romacs-list--stop-ticker ()
  "Stop asking the list's live cells again."
  (when romacs-list--ticker
    (cancel-timer romacs-list--ticker)
    (setq romacs-list--ticker nil)))

(defun romacs-list--stop ()
  "Stop the list's clock and banner: its buffer goes, or leaves the mode."
  (romacs-list--stop-ticker)
  (romacs-list--stop-banner))

;;; Windows

(defun romacs-list--in (window)
  "WINDOW's buffer when it is a list, else nil: a window that shows
another buffer now runs the list's window hooks too."
  (let ((buffer (window-buffer window)))
    (and (provided-mode-derived-p (buffer-local-value 'major-mode buffer) 'romacs-list-mode) buffer)))

(defun romacs-list--shown-in (window)
  "Draw the list anew as WINDOW shows it: it was not followed hidden."
  (when-let* ((buffer (romacs-list--in window))
              ;; The buffer it showed runs the hook too.
              ((eq buffer (current-buffer))))
    (funcall romacs-list-redraw-function)))

(defun romacs-list--resized (window)
  "Draw the list again when WINDOW, showing it, changed its width."
  (when-let* ((buffer (romacs-list--in window)))
    (unless (eql (romacs-list-columns buffer) (buffer-local-value 'romacs-list--width buffer))
      (with-current-buffer buffer (funcall romacs-list-redraw-function)))))

;;; Commands

(defun romacs-list-next (&optional n)
  "Move to the Nth next row (previous, N negative)."
  (interactive "p" romacs-list-mode)
  (let ((step (if (< (or n 1) 0) -1 1)))
    (dotimes (_ (abs (or n 1)))
      (let ((start (point)))
        (forward-line step)
        (while (and (not (romacs-list-row)) (not (if (> step 0) (eobp) (bobp))))
          (forward-line step))
        (unless (romacs-list-row) (goto-char start))))))

(defun romacs-list-previous (&optional n)
  "Move to the Nth previous row."
  (interactive "p" romacs-list-mode)
  (romacs-list-next (- (or n 1))))

(defun romacs-list-enter (level)
  "Show LEVEL of the list, nil its top; point on its first row, or back
up, on the row of the level it came from."
  (let ((from (romacs-list--level)))
    (set romacs-list-level-variable level)
    (funcall romacs-list-redraw-function)
    (goto-char (point-min))
    (if level (unless (romacs-list-row) (romacs-list-next 1)) (romacs-list-goto from))))

(defun romacs-list-up ()
  "Show the list's top level again."
  (interactive nil romacs-list-mode)
  (unless (romacs-list--level) (user-error "At the top already"))
  (romacs-list-enter nil))

(defun romacs-list--revert (&rest _)
  "Draw the list again, its data read anew: its `revert-buffer-function'."
  (funcall romacs-list-redraw-function t))

(defun romacs-list--highlight ()
  "Highlight the row at point, in the face the mode chose."
  (hl-line-mode 1))

(defvar-keymap romacs-list-mode-map
  :doc "Keys of a list, besides `special-mode''s."
  :suppress t
  :parent special-mode-map
  "n" #'romacs-list-next
  "p" #'romacs-list-previous
  ;; Line motion moves by row: the rules between groups are no stop.
  "<remap> <next-line>" #'romacs-list-next
  "<remap> <previous-line>" #'romacs-list-previous)

(define-derived-mode romacs-list-mode special-mode "List"
  "A list of rows, a line each, under a banner.
\\{romacs-list-mode-map}"
  :interactive nil
  ;; Loaded now, its directory on `load-path': its cells cut through it.
  (require 'romacs-truncate)
  (setq-local truncate-lines t
              revert-buffer-function #'romacs-list--revert
              hl-line-face 'romacs-list-row)
  ;; After every mode's body, before the mode hooks: a derived mode's
  ;; `hl-line-face' is the overlay's, and a hook may still turn it off.
  (add-hook 'change-major-mode-after-body-hook #'romacs-list--highlight nil t)
  (add-hook 'window-buffer-change-functions #'romacs-list--shown-in nil t)
  (add-hook 'window-size-change-functions #'romacs-list--resized nil t)
  (add-hook 'kill-buffer-hook #'romacs-list--stop nil t)
  (add-hook 'change-major-mode-hook #'romacs-list--stop nil t))

(provide 'romacs-list)
;;; romacs-list.el ends here

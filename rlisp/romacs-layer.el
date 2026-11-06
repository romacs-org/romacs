;;; romacs-layer.el --- Buffers shown in a layer over the frame  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A layer is a child frame (GNU's `parent-frame') showing one buffer over
;; the root frame.  Showing, moving or hiding it changes no window of the
;; frame below and no terminal's size (DIVERGENCES.md VM2-CHILD-FRAME).
;; Where a layer goes is policy, here: a full-width strip directly above
;; the minibuffer line (or the frame's bottom while the minibuffer is in
;; its own layer) by default, a centred box, or below point.  A
;; layer's frame is kept for its buffer's next show and deleted once the
;; buffer is killed.
;; `romacs-layers' lists what is shown, for agents and tests.  Romacs's
;; menus over layers are romacs-popup.el's.

;;; Code:

(eval-when-compile (require 'cl-lib))
;; The chrome faces, and where a layer ends (`romacs-chrome-bottom').
(require 'romacs-chrome)

(defgroup romacs-layer nil
  "Buffers shown in a layer over the frame."
  :group 'frames)

(defcustom romacs-layer-position 'bottom
  "Where `romacs-layer-show' puts a layer unless told otherwise.
`bottom': the frame's full width, directly above the minibuffer line.
`center': a box centred on the frame.
`point': below point in the selected window.
A caller may also give `romacs-layer-show' (at WINDOW POS): after POS on
its row in WINDOW; or (window-bottom WINDOW): at the bottom left of its text."
  :type '(choice (const bottom) (const center) (const point)))

(defcustom romacs-layer-animation t
  "How the display animates layers.
t: a layer fades in with a short slide from the edge it is anchored
at, fades out, tweens its height when it grows or shrinks, and its
selection highlight (the `romacs-layer-selection' frame parameter)
glides between rows.  `reduced': the fades only.  nil: no animation.
Animation is the display's alone: Lisp sees each layer's final
geometry at once, and a key ends any animation in flight.  A display
on a software renderer (headless test displays) draws none unless the
environment variable ROMACS_LAYER_ANIMATION is 1."
  :type '(choice (const :tag "Fades, slides and tweens" t)
                 (const :tag "Fades only" reduced)
                 (const :tag "None" nil)))

(defcustom romacs-layer-max-lines 12
  "Most lines a layer shows; its buffer scrolls past them."
  :type 'natnum)

(defcustom romacs-layer-frame-parameters
  '((no-accept-focus . t) (no-focus-on-map . t) (min-width . t) (min-height . t)
    (child-frame-border-width . 1) (undecorated . t) (unsplittable . t)
    (no-other-frame . t) (cursor-type . nil) (desktop-dont-save . t)
    (menu-bar-lines . 0) (tool-bar-lines . 0) (tab-bar-lines . 0)
    (left-fringe . 0) (right-fringe . 0) (vertical-scroll-bars . nil))
  "Frame parameters of a layer's child frame."
  :type '(alist :key-type symbol :value-type sexp))

(defcustom romacs-layer-glass t
  "Whether raised layers (the completion panel, message toasts) are glass:
rounded, padded, on `romacs-layer-glass's background,
casting a soft shadow and glowing in `romacs-layer-glow'.  nil gives
them GNU's square child-frame border."
  :type 'boolean)

(defun romacs-layer-glass-parameters (glass &optional face padding radius)
  "Frame parameters that make a layer glass when GLASS is non-nil, else
the ones that make it GNU's square child frame again.  The host rounds a
glass layer's corners (`romacs-layer-radius', RADIUS pixels, default t,
the style's `:radius-layer' whichever is in force),
lights it (`romacs-layer-shadow'), leaves its border, there PADDING
pixels (default 10), unpainted, and paints it in faces: its background
FACE's (default `romacs-layer-glass'), its glow `romacs-layer-glow''s
foreground."
  (if glass
      `((romacs-layer-radius . ,(or radius t))
        (romacs-layer-shadow . t)
        (romacs-layer-face . ,(or face 'romacs-layer-glass))
        (child-frame-border-width . ,(or padding 10)))
    `((romacs-layer-radius) (romacs-layer-shadow) (romacs-layer-face)
      (child-frame-border-width . ,(alist-get 'child-frame-border-width romacs-layer-frame-parameters)))))

(defvar romacs-layer--frames (make-hash-table :test 'eq)
  "BUFFER -> FRAME: the child frame showing each layer's buffer.  A
buffer's `kill-buffer-hook' deletes its layer and its entry.")

(defun romacs-layer--root-frame ()
  "The frame layers are shown over: the selected frame's top-level ancestor."
  (let ((frame (selected-frame)))
    (while (frame-parent frame) (setq frame (frame-parent frame)))
    frame))

(defun romacs-layer--forget ()
  "Delete the current buffer's layer, as the buffer is killed."
  (let ((frame (gethash (current-buffer) romacs-layer--frames)))
    (remhash (current-buffer) romacs-layer--frames)
    (when (frame-live-p frame) (delete-frame frame))))

;; A major mode change keeps it: the layer is the buffer's until it dies.
(put 'romacs-layer--forget 'permanent-local-hook t)

(defun romacs-layer--frame-of (buffer)
  "The live child frame showing BUFFER's layer, if it has one."
  (let ((frame (gethash buffer romacs-layer--frames)))
    (and (frame-live-p frame) frame)))

(defvar romacs-layer--display-table nil
  "`romacs-layer-display-table', once made.")

(defun romacs-layer-display-table ()
  "The display table Romacs's own text surfaces show: a truncated line
ends in `›' and a continued row in a blank, where a window without
fringes, as every layer is, shows GNU's `$' and `\\'.  The window
keeps one column for that glyph, so it must be one cell in the frame's
font: Iosevka's own `…' is two."
  (or romacs-layer--display-table
      ;; Made as disp-table.el's `make-display-table' makes one, without
      ;; loading that file, which gives `standard-display-table' a value
      ;; and so every window a display table to look through.
      (progn (unless (get 'display-table 'char-table-extra-slots)
               (put 'display-table 'char-table-extra-slots 18))
             (let ((table (make-char-table 'display-table nil)))
               (set-char-table-extra-slot table 0 ?›)
               (set-char-table-extra-slot table 1 ?\s)
               (setq romacs-layer--display-table table)))))

(defun romacs-layer-wrap-words (&optional buffer)
  "Make BUFFER (the current buffer) wrap its lines at words, as prose,
its continued rows unmarked (`romacs-layer-display-table')."
  (with-current-buffer (or buffer (current-buffer))
    (setq-local truncate-lines nil word-wrap t
                buffer-display-table (romacs-layer-display-table))))

(defun romacs-layer--frame (buffer)
  "A live child frame for BUFFER, made once and kept for reuse."
  (or (romacs-layer--frame-of buffer)
      (let ((frame (let ((after-make-frame-functions nil))
                     (make-frame `((parent-frame . ,(romacs-layer--root-frame))
                                   (minibuffer . ,(minibuffer-window))
                                   (visibility . nil)
                                   ,@romacs-layer-frame-parameters)))))
        (puthash buffer frame romacs-layer--frames)
        (with-current-buffer buffer
          (add-hook 'kill-buffer-hook #'romacs-layer--forget nil t)
          ;; A layer has no fringes: its line ends show Romacs's glyphs.
          (unless buffer-display-table
            (setq-local buffer-display-table (romacs-layer-display-table))))
        (let ((window (frame-root-window frame)))
          (set-window-buffer window buffer)
          (set-window-dedicated-p window t)
          ;; Ordinary window cycling skips it (window.el `other-window').
          (set-window-parameter window 'no-other-window t)
          (set-window-parameter window 'no-delete-other-windows t))
        frame)))

(defun romacs-layer--text-size (buffer)
  "BUFFER's text in columns and lines, as (COLUMNS . LINES)."
  (with-current-buffer buffer
    (save-excursion
      (goto-char (point-min))
      (let ((columns 0) (lines 0))
        (while (not (eobp))
          (setq columns (max columns (string-width (buffer-substring (point) (line-end-position))))
                lines (1+ lines))
          (forward-line 1))
        (cons (max columns 1) (max lines 1))))))

(declare-function romacs-chrome-bottom "romacs-chrome" (&optional frame except))
(declare-function romacs-chrome-top "romacs-chrome" (&optional frame))

(defun romacs-layer--bottom (root)
  "Where a `bottom' layer of ROOT ends: above the status bar and the
minibuffer line, or the layer hosting the minibuffer window
\(`romacs-chrome-bottom')."
  (romacs-chrome-bottom root))

(defun romacs-layer--placement (position columns lines border)
  "Where POSITION puts a layer of COLUMNS by LINES with BORDER pixels:
\(LEFT TOP WIDTH HEIGHT), its outer position in the root frame and its
text size, in pixels."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root)) (ch (frame-char-height root))
         ;; Child frames are placed from the native origin, outside the
         ;; internal border round the windows.
         (inset (frame-internal-border-width root))
         (frame-width (- (frame-text-width root) (* 2 border)))
         (width (min (* columns cw) frame-width))
         (height (* lines ch))
         (outer-height (+ height (* 2 border))))
    (pcase position
      ('bottom
       (let ((top (- (romacs-layer--bottom root) outer-height)))
         (list inset (max inset top) frame-width height)))
      ('center
       (let ((width (min width (- frame-width (* 4 cw)))))
         (list (+ inset (/ (- frame-width width) 2)) (+ inset (/ (- (frame-text-height root) outer-height) 2)) width height)))
      (`(at ,window ,pos)
       (romacs-layer--after window pos width height border))
      (`(pixel ,left ,top)
       ;; At LEFT TOP of the root frame, outer: a key-hint card's corner.
       (list left top width height))
      (`(window-bottom ,window)
       ;; At the bottom left of WINDOW's text, a margin inside it.
       (pcase-let ((`(,left ,_ ,right ,bottom) (window-edges window t nil t)))
         (let ((width (min width (- right left 24 (* 2 border)))))
           (list (+ left 12) (- bottom 12 outer-height) (max cw width) height))))
      ('point
       (let* ((edges (window-edges nil t nil t))
              (posn (posn-x-y (posn-at-point)))
              (x (+ (nth 0 edges) (or (car posn) 0)))
              (y (+ (nth 1 edges) (or (cdr posn) 0) ch)))
         (list (min x (max 0 (- frame-width width)))
               (if (> (+ y outer-height) (frame-text-height root)) (max 0 (- y ch outer-height)) y)
               width height)))
      (_ (error "Unknown layer position %S" position)))))

(defun romacs-layer--after (window pos width height border)
  "Where a layer of WIDTH by HEIGHT text pixels inside BORDER goes after
POS in WINDOW: one column after POS's glyph, its text on POS's row; on
the row below, flush with the window's right edge, when it would cross
that edge; on the row above when the row below is past the window's
bottom.  As (LEFT TOP WIDTH HEIGHT), or nil when POS is not shown."
  (when-let* ((posn (posn-at-point pos window))
              (xy (posn-x-y posn)))
    (let* ((edges (window-edges window t nil t))
           (left (nth 0 edges)) (top (nth 1 edges)) (right (nth 2 edges)) (bottom (nth 3 edges))
           (ch (frame-char-height (window-frame window)))
           (outer (+ width (* 2 border)))
           (x (+ left (car xy) (frame-char-width (window-frame window))))
           (y (+ top (cdr xy))))
      (cond
       ((<= (+ x outer) right))
       ((<= (+ y ch height) bottom) (setq x (max left (- right outer)) y (+ y ch)))
       (t (setq x (max left (- right outer)) y (max top (- y height)))))
      (list x (- y border) width height))))

(cl-defun romacs-layer-show (buffer &key position columns lines max-lines)
  "Show BUFFER in a layer over the frame and return the layer's frame.
POSITION is `bottom', `center', `point' or (at WINDOW POS),
`romacs-layer-position' by default; nil, showing nothing, when POS is
not shown in WINDOW.  COLUMNS and LINES size it, by default to BUFFER's
text, at most MAX-LINES lines, by default `romacs-layer-max-lines'; a
`bottom' layer is as wide as the frame."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer))
         (text (romacs-layer--text-size buffer))
         (border (or (frame-parameter frame 'child-frame-border-width) 0)))
    (pcase (romacs-layer--placement (or position romacs-layer-position)
                                    (or columns (car text))
                                    (min (or lines (cdr text)) (or max-lines romacs-layer-max-lines))
                                    border)
      (`(,left ,top ,width ,height)
       (set-frame-size frame width height t)
       (set-frame-position frame left top)
       ;; The edge it slides in from and grows away from.
       (set-frame-parameter frame 'romacs-layer-edge
                            (if (eq (or position romacs-layer-position) 'bottom) 'bottom 'top))
       ;; A layer shown anew starts at its text's top; one re-placed while
       ;; shown keeps its viewport (a scrolled popup refitting).
       (unless (frame-visible-p frame)
         (set-window-start (frame-root-window frame) (with-current-buffer buffer (point-min))))
       (make-frame-visible frame)
       frame)
      (_ (romacs-layer-hide buffer) nil))))

(defun romacs-layer-hide (buffer)
  "Hide the layer showing BUFFER, keeping its frame for the next show."
  (let ((frame (and (get-buffer buffer) (romacs-layer--frame-of (get-buffer buffer)))))
    (when (and frame (frame-visible-p frame)) (make-frame-invisible frame))))

(defun romacs-layers ()
  "The child frames shown over the frame, oldest first: for each, a plist
of its buffer, its outer position in its parent, and its text size in
columns and lines."
  (cl-loop for frame in (visible-frame-list)
           when (frame-parent frame)
           collect (list :frame frame
                         :buffer (window-buffer (frame-root-window frame))
                         :position (frame-position frame)
                         :size (cons (frame-text-cols frame) (frame-text-lines frame))
                         :minibuffer (eq frame (romacs--minibuffer-host)))))

;; Messages as toasts: a stack from the top-right corner of the selected
;; window's text (or the frame's, `romacs-layer-toast-position'), which
;; covers no status bar and no minibuffer.  The echo area's semantics are
;; GNU's (`message', `current-message', *Messages*); only where a message
;; shows is ours (DIVERGENCES.md VM2-ECHO-LAYER).

(defcustom romacs-layer-toast-timeout 2.5
  "Seconds a message toast shows before it fades, unless the next command
clears the echo area first."
  :type 'number)

(defcustom romacs-layer-toast-max 1
  "Most message toasts shown at once; the oldest goes first.  At 1, a new
message replaces the shown one in place."
  :type 'natnum)

(defcustom romacs-layer-toast-ignore '("\\`Quit\\'")
  "Regexps of messages never shown as toasts; *Messages* still logs them."
  :type '(repeat regexp))

(defcustom romacs-layer-toast-width 420
  "Widest a toast is, in pixels; never wider than its pane less 24."
  :type 'natnum)

(defcustom romacs-layer-toast-lines 2
  "Most rows a toast shows; longer text ends in `…' (*Messages* has it all)."
  :type 'natnum)

(defcustom romacs-layer-toast-position 'window
  "Where message toasts stack.
`window': from the top-right corner of the selected window's text, inside
its pane, following the selection and the window's size.
`frame': from the frame's top-right corner, below `romacs-chrome-top'."
  :type '(choice (const window) (const frame)))

(defvar romacs-layer-echo-claim-functions nil
  "Functions offered the echo area's new message before it shows as a
toast, each called with the text; one returning non-nil has shown it
otherwise (an eval chip, romacs-eval-chip.el), and no toast shows.")

(defconst romacs-layer--toast-tab-width 8
  "The tab stops of a toast's text: its buffer's `tab-width', and its
shaping's, whatever the buffer the message came from.")

(defvar romacs-layer--toasts nil
  "The toasts shown, newest first: each (BUFFER SHOWN TIMER TEXT COLUMNS),
SHOWN the text as it fits (`romacs-layer--toast-text'), TEXT the
message, COLUMNS SHOWN's widest line as painted, measured once.")

(defun romacs-layer--toast-buffer ()
  "A toast buffer no shown toast uses."
  (let ((used (mapcar #'car romacs-layer--toasts)) (n 0) buffer)
    (while (memq (setq buffer (get-buffer-create (format " *romacs-toast-%d*" n))) used)
      (setq n (1+ n)))
    (with-current-buffer buffer
      (setq-local mode-line-format nil header-line-format nil cursor-type nil
                  tab-width romacs-layer--toast-tab-width)
      (romacs-layer-wrap-words))
    buffer))

(defconst romacs-layer--toast-path-regexp
  "\\(?:\\`\\|[[:space:]\"'`(]\\)\\(\\(?:/\\|~[^/[:space:]\"'`)]*/\\)[^[:space:]\"'`)]*\\)"
  "A file name in a message: an absolute or home-relative one (`~/',
`~USER/'), after a space, a quote or a parenthesis.")

(defun romacs-layer--toast-width (string)
  "STRING's width in the root frame's columns as the display paints it:
a glyph the font draws wider than a cell (Iosevka's `…') counts as
painted, which `string-width' does not know; ASCII, and anything where
the display cannot measure, by `string-width'."
  (or (and (string-match-p "[^[:ascii:]]" string) (display-graphic-p) (fboundp 'string-pixel-width)
           (ignore-errors (ceiling (string-pixel-width string) (float (frame-char-width (romacs-layer--root-frame))))))
      (string-width string)))

(defun romacs-layer--toast-shorten (name room)
  "NAME, a file name, at most ROOM columns where it can be: its
directories dropped from the front behind `…/', then, for a basename
wider than that, the basename's middle, keeping its end."
  (if (<= (romacs-layer--toast-width name) room) name
    (let* ((head (if (string-match "\\`\\(?:/\\|~[^/]*/\\)" name) (match-string 0 name) ""))
           (base (file-name-nondirectory name))
           (dirs (split-string (substring (or (file-name-directory name) head) (length head)) "/" t))
           (fixed (+ (romacs-layer--toast-width head) (romacs-layer--toast-width "…/") (romacs-layer--toast-width base)))
           (widths (mapcar (lambda (d) (1+ (romacs-layer--toast-width d))) dirs))
           (total (apply #'+ widths)))
      ;; Drop directories from the front, each measured once.
      (while (and dirs (> (+ fixed total) room))
        (setq total (- total (pop widths)) dirs (cdr dirs)))
      (if (<= (+ fixed total) room)
          (concat head "…/" (mapconcat (lambda (d) (concat d "/")) dirs "") base)
        (let* ((lead (concat head "…/"))
               (left (- room (romacs-layer--toast-width lead) (romacs-layer--toast-width "…")))
               (suffix (romacs-layer--toast-tail base (max 1 (/ (* 2 left) 3))))
               (front (truncate-string-to-width base (max 0 (- left (romacs-layer--toast-width suffix))))))
          (concat lead front "…" suffix))))))

(defun romacs-layer--toast-tail (string columns)
  "The longest end of STRING at most COLUMNS wide as painted (one
character at least)."
  (let ((i (length string)) (used 0))
    (while (and (> i 0) (<= (+ used (char-width (aref string (1- i)))) columns))
      (setq used (+ used (char-width (aref string (1- i)))) i (1- i)))
    (substring string (min i (max 0 (1- (length string)))))))

(defun romacs-layer--untab (line)
  "LINE with each tab as the spaces to its next stop
\(`romacs-layer--toast-tab-width'), as the toast's window shows it."
  (if (not (string-search "\t" line)) line
    (let ((column 0) (width romacs-layer--toast-tab-width))
      (mapconcat (lambda (c)
                   (if (eq c ?\t)
                       (let ((n (- width (% column width)))) (setq column (+ column n)) (make-string n ?\s))
                     (setq column (+ column (char-width c))) (string c)))
                 line ""))))

(defun romacs-layer--toast-path-word-p (word)
  "Whether WORD, a word of a message, is a file name (a quote or a
parenthesis before it allowed)."
  (string-match-p "\\`[\"'`(]?\\(?:/\\|~[^/[:space:]]*/\\)" word))

(defun romacs-layer--toast-rows (line columns)
  "LINE's rows as a window COLUMNS wide wraps it at words: a list of
strings.  A file name too wide for a row is shortened to fit
\(`romacs-layer--toast-shorten'); another word too wide is broken across
rows, as the window breaks it."
  (let ((words (split-string line " ")) rows (row nil) (used 0))
    ;; A line one file name's directories make too wide for a row: cut
    ;; them there, so the line keeps one row with its basename whole.
    (let* ((widths (mapcar #'romacs-layer--toast-width words))
           (total (+ (apply #'+ widths) (1- (length words))))
           (path (seq-reduce (lambda (best i) (if (and (romacs-layer--toast-path-word-p (nth i words))
                                                       (or (null best) (> (nth i widths) (nth best widths))))
                                                  i best))
                             (number-sequence 0 (1- (length words))) nil)))
      (when (and path (> total columns))
        (let* ((word (nth path words))
               (room (- columns (- total (nth path widths))))
               (lead (if (string-match "\\`[\"'`(]" word) (match-string 0 word) ""))
               (base (file-name-nondirectory (substring word (length lead)))))
          (when (>= room (+ (length lead) (romacs-layer--toast-width "…/") (romacs-layer--toast-width base)))
            (setf (nth path words) (concat lead (romacs-layer--toast-shorten (substring word (length lead)) (- room (length lead)))))))))
    (dolist (word words)
      (let* ((word (if (and (> (romacs-layer--toast-width word) columns) (romacs-layer--toast-path-word-p word))
                       (let ((lead (if (string-match "\\`[\"'`(]" word) (match-string 0 word) "")))
                         (concat lead (romacs-layer--toast-shorten (substring word (length lead)) (- columns (length lead)))))
                     word))
             (width (romacs-layer--toast-width word)))
        (cond
         ((and row (<= (+ used 1 width) columns))
          (setq row (concat row " " word) used (+ used 1 width)))
         (t (when row (push row rows))
            (while (> (romacs-layer--toast-width word) columns)
              (let ((piece (truncate-string-to-width word columns)))
                (push piece rows)
                (setq word (substring word (length piece)))))
            (setq row word used (romacs-layer--toast-width word))))))
    (nreverse (if row (cons row rows) rows))))

(defun romacs-layer--toast-text (text columns)
  "TEXT as a toast COLUMNS wide shows it: its file names abbreviated
\(`abbreviate-file-name'), its lines wrapped at words into rows as the
window wraps them, a file name too wide for a row cut in its
directories, and at most `romacs-layer-toast-lines' rows, the last ending
in `…' where more follow, never inside a file name.  TEXT itself when it
fits unchanged; else the rows, one per line."
  (let* ((abbreviated (replace-regexp-in-string
                       romacs-layer--toast-path-regexp
                       (lambda (match) (let ((name (match-string 1 match)))
                                         (replace-match (if (file-name-absolute-p name) (abbreviate-file-name name) name)
                                                        t t match 1)))
                       text t t))
         (lines (mapcar #'romacs-layer--untab (split-string abbreviated "\n")))
         (rows (mapcan (lambda (line) (or (romacs-layer--toast-rows line columns) (list ""))) lines))
         (budget (max 1 romacs-layer-toast-lines)))
    (cond
     ((and (equal abbreviated text) (<= (length rows) budget)
           (seq-every-p (lambda (line) (<= (romacs-layer--toast-width line) columns)) lines))
      text)
     ((<= (length rows) budget) (string-join rows "\n"))
     (t
      (let* ((kept (seq-take rows budget))
             (last (car (last kept)))
             (ellipsis (romacs-layer--toast-width "…"))
             (words (split-string last " ")))
        ;; The last row, its words dropped from the end until `…' fits;
        ;; a file name is dropped whole, never cut.
        (while (and (cdr words) (> (+ (romacs-layer--toast-width (string-join words " ")) 1 ellipsis) columns))
          (setq words (butlast words)))
        (let* ((row (string-join words " "))
               (row (if (> (+ (romacs-layer--toast-width row) 1 ellipsis) columns)
                        (if (romacs-layer--toast-path-word-p row)
                            (romacs-layer--toast-shorten row (- columns 1 ellipsis))
                          (truncate-string-to-width row (- columns 1 ellipsis)))
                      row)))
          (string-join (append (butlast kept) (list (concat row " …"))) "\n")))))))

(defun romacs-layer--toast-columns (text columns)
  "The columns a toast of TEXT takes, at most COLUMNS: its widest line's,
as painted."
  (min columns (max 1 (apply #'max (mapcar #'romacs-layer--toast-width (split-string text "\n"))))))

(defun romacs-layer--toast-lines (frame text columns)
  "The rows TEXT takes in toast FRAME as its window wraps it, at most
`romacs-layer-toast-lines';
without a host's layout to measure, its lines in rows of COLUMNS."
  (let* ((ch (frame-char-height frame))
         (rows (condition-case nil
                   (ceiling (cdr (window-text-pixel-size (frame-root-window frame))) (float ch))
                 (error (apply #'+ (mapcar (lambda (line) (max 1 (ceiling (string-width line) (float columns))))
                                           (split-string text "\n")))))))
    (max 1 (min (max 1 romacs-layer-toast-lines) rows))))

(defun romacs-layer--toast-window ()
  "The window toasts show in: the root frame's selected window, or the
window a prompt was entered from while the minibuffer is selected."
  (let ((window (frame-selected-window (romacs-layer--root-frame))))
    (if (window-minibuffer-p window)
        (or (minibuffer-selected-window) (get-mru-window (romacs-layer--root-frame) nil t))
      window)))

(defcustom romacs-layer-toast-min-columns 16
  "Fewest text columns a toast in the selected window has; a narrower
window's toasts stack in the frame's corner."
  :type 'natnum)

(defconst romacs-layer--toast-padding 10
  "A glass toast's padding: its `child-frame-border-width'.")

(defvar romacs-layer--toast-placed nil
  "What the toasts were last stacked for: the area and the toasts.")

(defun romacs-layer--toast-area ()
  "Where toasts stack, in the root frame's native pixels: (LEFT TOP RIGHT
BOTTOM COLUMNS), COLUMNS the most a toast's text is wide.  The selected
window's text area for `window' (`romacs-layer-toast-position'), else
the frame's, below the chrome above it and above the chrome below it.
A window too narrow for a toast of `romacs-layer-toast-min-columns' with
its padding and margin inside it takes the frame's area instead."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root))
         (window (and (eq romacs-layer-toast-position 'window) (romacs-layer--toast-window)))
         ;; The columns a toast PIXELS wide holds for its text: less the
         ;; glass padding either side and the column a window's last keeps
         ;; for its continuation glyph.
         (columns (lambda (pixels) (1- (/ (- pixels (* 2 romacs-layer--toast-padding)) cw))))
         (widest (funcall columns romacs-layer-toast-width)))
    (or (and (window-live-p window)
             (pcase-let ((`(,left ,top ,right ,bottom) (window-edges window t nil t)))
               (let ((room (funcall columns (- right left 24))))
                 (and (>= room romacs-layer-toast-min-columns)
                      (list left top right (min bottom (romacs-chrome-bottom root)) (min widest room))))))
        (list 0 (romacs-chrome-top root) (- (frame-native-width root) (frame-internal-border-width root))
              (romacs-chrome-bottom root)
              (min widest (funcall columns (- (frame-native-width root) 24)))))))

(defun romacs-layer--toast-follow (frame)
  "Restack the toasts after a change to FRAME's windows (the selection, a
size), when FRAME is the root frame and where they go moved."
  (when (and romacs-layer--toasts (eq frame (romacs-layer--root-frame))
             (not (equal romacs-layer--toast-placed (cons (romacs-layer--toast-area) romacs-layer--toasts))))
    (romacs-layer--toast-stack)))

(defun romacs-layer--toast-hooks (on)
  "Follow window changes while toasts show (ON), else not."
  (dolist (hook '(window-selection-change-functions window-size-change-functions))
    (if on (add-hook hook #'romacs-layer--toast-follow) (remove-hook hook #'romacs-layer--toast-follow))))

(defun romacs-layer--toast-stack ()
  "Place the shown toasts from the top right of `romacs-layer--toast-area'
downward, newest on top; a toast that would pass its bottom waits hidden."
  (pcase-let* ((root (romacs-layer--root-frame))
               (cw (frame-char-width root)) (ch (frame-char-height root))
               (`(,_ ,area-top ,right ,bottom ,most) (romacs-layer--toast-area))
               (margin 12) (gap 8) (top (+ area-top margin)))
    (romacs-layer--toast-hooks romacs-layer--toasts)
    (setq romacs-layer--toast-placed (cons (romacs-layer--toast-area) romacs-layer--toasts))
    (dolist (toast romacs-layer--toasts)
      (let* ((frame (romacs-layer--frame (car toast)))
             (border (or (frame-parameter frame 'child-frame-border-width) 0))
             (columns (max 1 (min most (nth 4 toast))))
             ;; One column more: a window's last holds its continuation glyph.
             (width (* (1+ columns) cw)) height)
        ;; Its rows as its width wraps them at words.
        (set-frame-size frame width (frame-text-height frame) t)
        (setq height (* (romacs-layer--toast-lines frame (nth 1 toast) columns) ch))
        (set-frame-size frame width height t)
        ;; Never over the minibuffer's layer (the command panel): below it
        ;; where they would meet.
        (when-let* ((host (and (fboundp 'romacs--minibuffer-host) (romacs--minibuffer-host)))
                    ((frame-visible-p host)))
          (pcase-let ((`(,hx . ,hy) (frame-position host))
                      (x (- right width (* 2 border) margin)))
            (when (and (< x (+ hx (frame-pixel-width host))) (< hx (+ x width (* 2 border)))
                       (< top (+ hy (frame-pixel-height host))) (< hy (+ top height (* 2 border))))
              (setq top (+ hy (frame-pixel-height host) gap)))))
        (set-frame-position frame (- right width (* 2 border) margin) top)
        (set-frame-parameter frame 'romacs-layer-edge 'top)
        (if (> (+ top height (* 2 border)) bottom)
            (when (frame-visible-p frame) (make-frame-invisible frame))
          ;; A new toast shows above every other layer, the panel too.
          (unless (frame-visible-p frame) (make-frame-visible frame) (raise-frame frame)))
        (setq top (+ top height (* 2 border) gap))))))

(defun romacs-layer--toast-expire (buffer)
  "Hide the toast showing BUFFER and close up the stack."
  (setq romacs-layer--toasts (seq-remove (lambda (toast) (eq (car toast) buffer)) romacs-layer--toasts))
  (romacs-layer-hide buffer)
  (romacs-layer--toast-stack))

(defun romacs-layer-toast-clear ()
  "Hide every toast, which fades, as the echo area clears.  Nothing to do
when none shows: the echo area clears as every command starts."
  (when romacs-layer--toasts
    (let ((toasts romacs-layer--toasts))
      (setq romacs-layer--toasts nil romacs-layer--toast-placed nil)
      (romacs-layer--toast-hooks nil)
      (dolist (toast toasts)
        (cancel-timer (nth 2 toast))
        (romacs-layer-hide (car toast))))))

(defun romacs-layer-toast (text &optional kind)
  "Show TEXT as a toast for `romacs-layer-toast-timeout' seconds.
KIND `error' draws it with `romacs-layer-toast-error''s glow.  The same
TEXT as the newest toast shows that one longer instead.  With the most
toasts shown, the oldest's layer shows TEXT in its place."
  (if (equal text (nth 3 (car romacs-layer--toasts)))
      (timer-set-time (nth 2 (car romacs-layer--toasts))
                      (timer-relative-time nil romacs-layer-toast-timeout))
    (let ((reused nil))
      (while (>= (length romacs-layer--toasts) (max 1 romacs-layer-toast-max))
        (let ((oldest (car (last romacs-layer--toasts))))
          (cancel-timer (nth 2 oldest))
          (setq romacs-layer--toasts (butlast romacs-layer--toasts))
          (if (or reused (> (length romacs-layer--toasts) (1- (max 1 romacs-layer-toast-max))))
              (romacs-layer-hide (car oldest))
            (setq reused (car oldest)))))
      (let* ((buffer (or reused (romacs-layer--toast-buffer)))
             (glyph (and (eq kind 'error) (fboundp 'romacs-icon) (romacs-icon 'error 'romacs-layer-toast-error)))
             (glyph (and glyph (not (string-empty-p glyph)) (concat glyph " ")))
             (columns (nth 4 (romacs-layer--toast-area)))
             ;; An error leads with the error glyph, shaped with its text in
             ;; one budget; the text keeps the toast's foreground.
             (shown (romacs-layer--toast-text (if glyph (concat glyph text) text) columns)))
        (with-current-buffer buffer
          (let ((inhibit-read-only t)) (erase-buffer) (insert shown)))
        (let ((frame (romacs-layer--frame buffer)))
          (modify-frame-parameters frame `(,@(romacs-layer-glass-parameters romacs-layer-glass 'romacs-layer-toast)
                                           (romacs-layer-glow-face . ,(and (eq kind 'error) 'romacs-layer-toast-error))
                                           (romacs-layer-outline-face . ,(and (eq kind 'error) 'romacs-layer-toast-error))))
          (set-window-start (frame-root-window frame) 1))
        (push (list buffer shown (run-at-time romacs-layer-toast-timeout nil #'romacs-layer--toast-expire buffer) text
                    (romacs-layer--toast-columns shown columns))
              romacs-layer--toasts)
        (romacs-layer--toast-stack)))))

;; A command's error, reported by the command loop, and an undefined key's
;; message show as error toasts (`romacs-complete--message').

(defvar romacs-layer--command-error nil
  "Non-nil from the command loop's report of an error (not a quit) until
the echo area's next message or clearing.")

(defun romacs-layer--note-command-error (data &rest _)
  "Before the command loop reports DATA: its message is an error's."
  (unless (memq 'quit (get (car data) 'error-conditions))
    (setq romacs-layer--command-error t)))

(defun romacs-layer--note-undefined (&rest _)
  "Before `undefined' reports a key bound to nothing: an error's message."
  (setq romacs-layer--command-error t))

(defun romacs-layer-toast-kind ()
  "The kind of toast the echo area's new message takes, and forget it:
`error' after a command error or an undefined key, else nil."
  (prog1 (and romacs-layer--command-error 'error)
    (setq romacs-layer--command-error nil)))

;; The minibuffer and the echo area wrap at words too: minibuffers as each
;; is entered, the echo area at its first message (DIVERGENCES.md
;; VM2-WRAP-WORDS).

(defun romacs-layer--wrap-echo-area (&optional _text)
  "Make the echo area's buffer wrap at words, once."
  (let ((echo (get-buffer " *Minibuf-0*")))
    (when (and echo (not (buffer-local-value 'word-wrap echo)))
      (romacs-layer-wrap-words echo))))

(add-hook 'minibuffer-setup-hook #'romacs-layer-wrap-words)
(add-hook 'romacs-echo-area-functions #'romacs-layer--wrap-echo-area)

(provide 'romacs-layer)
;;; romacs-layer.el ends here

;;; romacs-grid.el --- Blocks of lines laid out in columns and pages that fit  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A key menu's layout, as a pure function of its blocks and its room:
;; `romacs-grid-pages'.  A block is a list of lines, a group of keys
;; already rendered (its title first); a page is a list of lines no
;; wider than the room's columns and, when the room has rows, no taller
;; than them.  Every line of every block is on exactly one page, in
;; order: nothing is clipped, a menu too big for its room pages.
;;
;; Its shape, the first that fits:
;; - the author's bands (keymap-popup's `:row's), each block a column,
;;   one width per column position across bands;
;; - else a flow: the blocks column-major in their order, in the fewest
;;   balanced columns whose height is at most half the room's rows (a
;;   menu should leave the text it acts on in view), or failing that the
;;   most columns the width holds;
;; - else pages of such flows.
;; A block taller than a page goes in chunks; a line wider than the room
;; is cut at a word.  Padding between columns is spaces displayed as an
;; `:align-to' stretch, so a glyph painted wider or narrower than its
;; `string-width' (an emoji, an icon, `…', a bold key) cannot push a
;; column over; a block marks its own padding (a key column's) with the
;; text property `romacs-grid-align', its column within the block, and
;; gets the same.

;;; Code:

(eval-when-compile (require 'cl-lib))
(require 'seq)
(require 'subr-x)

(defconst romacs-grid-gap 3
  "Columns between two columns of blocks.")

(defun romacs-grid--width (lines)
  "The widest of LINES, in columns."
  (seq-reduce (lambda (w line) (max w (string-width line))) lines 0))

(defvar romacs-grid--block-widths nil
  "While `romacs-grid-pages' lays blocks out: each block's width, by the
block, measured once however many layouts it is tried in.")

(defun romacs-grid--block-width (block)
  "BLOCK's width (`romacs-grid--width'), from `romacs-grid--block-widths'
when a layout is in progress."
  (if romacs-grid--block-widths
      (or (gethash block romacs-grid--block-widths)
          (puthash block (romacs-grid--width block) romacs-grid--block-widths))
    (romacs-grid--width block)))

(defun romacs-grid--column-width (column)
  "COLUMN's width: its widest block's."
  (apply #'max 0 (mapcar #'romacs-grid--block-width column)))

(defun romacs-grid-cut (line width)
  "LINE in at most WIDTH columns: whole when it fits, else cut at the
last word boundary that leaves room for `truncate-string-ellipsis' (at
a column when no boundary is past half of WIDTH; no ellipsis in a room
narrower than it), its properties kept."
  (cond
   ((<= (string-width line) width) line)
   ((< width (string-width (truncate-string-ellipsis))) (truncate-string-to-width line width))
   (t
    (let* ((ellipsis (truncate-string-ellipsis))
           (room (max 0 (- width (string-width ellipsis))))
           (head (truncate-string-to-width line room))
           (space (string-match-p "[[:space:]]+[^[:space:]]*\\'" head))
           (head (if (and space (> (string-width (substring head 0 space)) (/ width 2)))
                     (substring head 0 space)
                   head)))
      (concat (string-trim-right head)
              (propertize ellipsis 'face (get-text-property (max 0 (1- (length head))) 'face line)))))))

(defun romacs-grid--fit-block (block columns)
  "BLOCK's lines cut to COLUMNS."
  (mapcar (lambda (line) (romacs-grid-cut line columns)) block))

(defun romacs-grid-chunks (block rows &optional key-of width-at)
  "BLOCK, a list of lines (or of items), in chunks of at most ROWS (one
chunk when ROWS is nil): as few as ROWS allows, each break within two
items of where even chunks would put it: the most even breaks, and of
those the ones whose columns take the least width in all.
A column's width is its widest item drawn beside the column's widest
key: KEY-OF gives an item's key width, WIDTH-AT an item's width drawn
beside a key column K wide (a label cut to the room left); by default
an item is a line and its width.  Each break has five places to fall,
so the search is linear in the items, times ROWS and the number of
distinct key widths."
  (let ((n (length block)))
    (if (or (null rows) (<= n rows)) (list block)
      (let* ((rows (max 1 rows))
             (count (ceiling n (float rows)))
             ;; Each break within SLACK of where even chunks would put it,
             ;; so the search stays linear and the columns near even.
             (slack 2)
             (key-of (or key-of (lambda (_) 0)))
             (width-at (or width-at (lambda (line _) (string-width line))))
             (keys (vconcat (mapcar key-of block)))
             ;; The distinct key widths, and each item's width beside each.
             (ks (vconcat (sort (seq-uniq (append keys nil)) #'<)))
             (at (vconcat (mapcar (lambda (item) (vconcat (mapcar (lambda (k) (funcall width-at item k)) ks))) block)))
             ;; BEST maps a break position to (WIDTH SQUARES . BREAKS): the
             ;; total width of the chunks before it, the sum of their
             ;; lengths squared (least when they are even), and the
             ;; breaks, latest first.
             (best (make-hash-table)))
        (puthash 0 (list 0 0) best)
        (dotimes (c count)
          (let* ((next (make-hash-table))
                 (even (round (* (1+ c) n) count))
                 (low (if (= c (1- count)) n (max (1+ c) (- even slack))))
                 (high (if (= c (1- count)) n (min (- n (- count c 1)) (+ even slack)))))
            (cl-loop for p from low to high
                     do (let ((key 0) (widest (make-vector (length ks) 0)))
                          ;; Chunks ending at P, from each break before it:
                          ;; the widest key, and the widest item beside each
                          ;; key width, kept as Q goes back.
                          (cl-loop for q from (1- p) downto (max 0 (- p rows))
                                   do (setq key (max key (aref keys q)))
                                   (dotimes (j (length ks))
                                     (aset widest j (max (aref widest j) (aref (aref at q) j))))
                                   (when-let* ((from (gethash q best)))
                                     (let ((cand (cons (+ (car from) (aref widest (seq-position ks key)))
                                                       (cons (+ (cadr from) (* (- p q) (- p q))) (cons p (cddr from)))))
                                           (have (gethash p next)))
                                       ;; The most even first, then the narrowest.
                                       (when (or (null have) (< (cadr cand) (cadr have))
                                                 (and (= (cadr cand) (cadr have)) (< (car cand) (car have))))
                                         (puthash p cand next)))))))
            (setq best next)))
        (let ((rest block) (start 0))
          (mapcar (lambda (end) (prog1 (seq-take rest (- end start))
                                  (setq rest (nthcdr (- end start) rest) start end)))
                  (reverse (cddr (gethash n best)))))))))

;;;; Columns

(defun romacs-grid--fill (heights target)
  "HEIGHTS (of blocks, in order) filled greedily into columns no taller
than TARGET, a blank line between two blocks of a column: a list of
the number of blocks in each column."
  (let ((counts nil) (count 0) (used 0))
    (dolist (h heights)
      (if (and (> count 0) (> (+ used 1 h) target))
          (progn (push count counts) (setq count 1 used h))
        (setq used (if (> count 0) (+ used 1 h) h) count (1+ count))))
    (nreverse (if (> count 0) (cons count counts) counts))))

(defun romacs-grid--balance (blocks k)
  "BLOCKS in at most K columns, the tallest as short as the order allows:
a list of columns, each a list of blocks."
  (let* ((heights (mapcar #'length blocks))
         (low (apply #'max 1 heights))
         (high (+ (apply #'+ heights) (max 0 (1- (length heights))))))
    ;; The least target height whose greedy fill takes at most K columns.
    (while (< low high)
      (let ((mid (/ (+ low high) 2)))
        (if (<= (length (romacs-grid--fill heights mid)) k) (setq high mid) (setq low (1+ mid)))))
    (let ((rest blocks))
      (mapcar (lambda (n) (prog1 (seq-take rest n) (setq rest (nthcdr n rest))))
              (romacs-grid--fill heights low)))))

(defun romacs-grid--column-height (column)
  "COLUMN's height: its blocks' lines and a blank line between two."
  (+ (apply #'+ (mapcar #'length column)) (max 0 (1- (length column)))))

(defun romacs-grid--band-width (widths)
  "The columns a band of columns WIDTHS wide takes, gaps included."
  (+ (apply #'+ widths) (* romacs-grid-gap (max 0 (1- (length widths))))))

(defun romacs-grid--flow (blocks height columns)
  "BLOCKS as one band of balanced columns within COLUMNS: the fewest
columns whose height is at most HEIGHT, else (or when HEIGHT is nil) the
shortest band of those that fit COLUMNS (one column when none do).  A
list of columns, each a list of blocks."
  ;; Every k up to the most columns the narrowest blocks could fill: a
  ;; balanced band's width need not grow with k (its columns regroup),
  ;; so one too wide says nothing of the next.
  (let* ((narrowest (apply #'min (mapcar #'romacs-grid--block-width blocks)))
         (most (min (length blocks) (max 1 (/ (+ columns romacs-grid-gap) (+ (max 1 narrowest) romacs-grid-gap)))))
         (candidates (cl-loop for k from 1 to most
                              for band = (romacs-grid--balance blocks k)
                              when (<= (romacs-grid--band-width
                                        (mapcar #'romacs-grid--column-width band))
                                       columns)
                              collect band)))
    (or (and height (seq-find (lambda (band) (<= (romacs-grid--band-height band) height)) candidates))
        (car (sort (copy-sequence candidates)
                   (lambda (a b) (< (romacs-grid--band-height a) (romacs-grid--band-height b)))))
        (list blocks))))

(defun romacs-grid--band-height (band)
  "BAND's height: its tallest column's."
  (apply #'max 0 (mapcar #'romacs-grid--column-height band)))

;;;; Pages

(defun romacs-grid--fill-pages (blocks budget columns)
  "BLOCKS set like a newspaper's: down a column while it stays within
BUDGET rows (a blank line between two blocks), then the next column,
then the next page once a column would pass COLUMNS.  A list of pages,
each a list of columns, each a list of blocks.  Linear in the blocks."
  (let (pages page column (height 0) (width 0) (used 0))
    (dolist (block blocks)
      (let ((h (length block)) (w (romacs-grid--block-width block)))
        (when (and column (> (+ height 1 h) budget))
          ;; The column is full: it joins the page.
          (setq used (+ used (if page romacs-grid-gap 0) width)
                page (cons (nreverse column) page) column nil height 0 width 0))
        (when (and page (> (+ used romacs-grid-gap (max width w)) columns))
          ;; Its column would pass the width: the page is full.
          (push (nreverse page) pages)
          (setq page nil used 0))
        (setq height (if column (+ height 1 h) h)
              width (max width w)
              column (cons block column))))
    (when column (push (nreverse column) page))
    (when page (push (nreverse page) pages))
    (nreverse pages)))

(defun romacs-grid--flow-pages (blocks rows columns)
  "BLOCKS (each at most ROWS lines) in pages of one band each, at most
ROWS high: as few pages as filling them allows, balanced, at the least
budget of rows that still needs no more pages, so a last page is not a
remnant.  Each page is a list of one band, its shortest flow."
  ;; Upward from the tallest block, not by halving: a fill's page count
  ;; need not fall as its budget grows (a block it now takes can push a
  ;; wide pair apart).  Each try is linear, so the whole is ROWS times.
  (let* ((count (length (romacs-grid--fill-pages blocks rows columns)))
         (pages (cl-loop for budget from (apply #'max 1 (mapcar #'length blocks)) to rows
                         for fill = (romacs-grid--fill-pages blocks budget columns)
                         when (<= (length fill) count) return fill)))
    ;; The fill breaks the pages; each page then takes its shortest flow
    ;; (side by side where the width holds), the fill's own columns where
    ;; that would be taller than the budget the fill kept.
    (mapcar (lambda (band)
              (let ((flow (romacs-grid--flow (apply #'append band) nil columns)))
                (list (if (<= (romacs-grid--band-height flow) (romacs-grid--band-height band)) flow band))))
            (or pages (romacs-grid--fill-pages blocks rows columns)))))

(defun romacs-grid--declared (blocks bands)
  "BLOCKS split into BANDS (how many blocks each band has), each block
its own column: a list of bands."
  (let ((rest blocks))
    (mapcar (lambda (n) (prog1 (mapcar #'list (seq-take rest n)) (setq rest (nthcdr n rest))))
            bands)))

(defun romacs-grid--position-widths (page)
  "One width per column position of PAGE's bands: the widest column at
that position in any band."
  (let ((widths (make-vector (apply #'max 0 (mapcar #'length page)) 0)))
    (dolist (band page (append widths nil))
      (seq-do-indexed (lambda (column i)
                        (aset widths i (max (aref widths i) (romacs-grid--column-width column))))
                      band))))

(defun romacs-grid--page-height (page)
  "PAGE's height: its bands' and a blank line between two."
  (+ (apply #'+ (mapcar #'romacs-grid--band-height page)) (max 0 (1- (length page)))))

;;;; Lines

(defun romacs-grid--anchor (text x)
  "TEXT, set at column X: each run of it carrying `romacs-grid-align' N
displayed as a stretch to column X+N, so a block's own columns (a key
column's padding) hold whatever its glyphs paint."
  (let ((pos 0) (text (copy-sequence text)))
    (while (setq pos (text-property-not-all pos (length text) 'romacs-grid-align nil text))
      (let ((end (next-single-property-change pos 'romacs-grid-align text (length text))))
        (put-text-property pos end 'display `(space :align-to ,(+ x (get-text-property pos 'romacs-grid-align text))) text)
        (setq pos end)))
    text))

(defun romacs-grid--pad (text start target)
  "TEXT, which starts at column START, padded to column TARGET with
spaces displayed as one stretch to TARGET."
  (let ((n (- target start (string-width text))))
    (if (<= n 0) text
      (concat text (propertize (make-string n ?\s) 'display `(space :align-to ,target))))))

(defun romacs-grid--spaced (lists)
  "LISTS (of lines) appended, a blank line between two."
  (let ((out nil))
    (dolist (lines lists (nreverse out))
      (when out (push "" out))
      (dolist (line lines) (push line out)))))

(defun romacs-grid--band-lines (band widths)
  "BAND's lines, its columns WIDTHS wide, `romacs-grid-gap' apart."
  (let* ((columns (mapcar (lambda (column)
                            (vconcat (romacs-grid--spaced column)))
                          band))
         (height (apply #'max 0 (mapcar #'length columns))))
    (cl-loop for row below height
             collect (let ((x 0) (line ""))
                       (cl-loop for column in columns
                                for width in widths
                                for first = t then nil
                                do (let ((text (romacs-grid--anchor (if (< row (length column)) (aref column row) "") x)))
                                     (setq line (if first text (concat (romacs-grid--pad line 0 x) text))
                                           x (+ x width romacs-grid-gap))))
                       (string-trim-right line)))))

(defun romacs-grid--page-lines (page widths)
  "PAGE's lines: its bands, each laid out on WIDTHS, a blank line apart."
  (romacs-grid--spaced (mapcar (lambda (band) (romacs-grid--band-lines band widths)) page)))

;;;; Entry

(defun romacs-grid-pages (blocks rows columns &optional bands height)
  "BLOCKS laid out in pages: a list of pages, each a list of lines.
A block is a list of lines (a group: its title, then its keys).  Each
page is at most COLUMNS columns wide and, when ROWS is non-nil, at most
ROWS lines tall; every line of BLOCKS is on one page, in order, a line
wider than COLUMNS cut at a word (`romacs-grid-cut').  BANDS, how many
blocks each of the author's rows holds, is the layout kept while it
fits; otherwise the blocks flow in balanced columns (see the
Commentary), on one page the fewest columns at most HEIGHT rows tall
\(by default half of ROWS: a menu leaves the text it acts on in view).
No blocks is one empty page."
  (let* ((romacs-grid--block-widths (make-hash-table :test 'eq))
         (columns (max 1 columns))
         (blocks (mapcar (lambda (block) (romacs-grid--fit-block block columns))
                         (seq-remove #'null blocks)))
         (declared (and bands (= (apply #'+ bands) (length blocks))
                        (romacs-grid--declared blocks (remq 0 bands))))
         (declared-widths (and declared (romacs-grid--position-widths declared))))
    (cond
     ((null blocks) (list nil))
     ((and declared
           (seq-every-p (lambda (band) (<= (romacs-grid--band-width (seq-take declared-widths (length band))) columns))
                        declared)
           (or (null rows) (<= (romacs-grid--page-height declared) rows)))
      (list (romacs-grid--page-lines declared declared-widths)))
     (t
      (let ((chunks (mapcan (lambda (block) (romacs-grid-chunks block rows)) blocks)))
        (let* ((height (or height (and rows (/ rows 2))))
               (pages (if (and rows (> (romacs-grid--band-height (romacs-grid--flow chunks nil columns)) rows))
                          (romacs-grid--flow-pages chunks rows columns)
                        (list (list (romacs-grid--flow chunks height columns)))))
               ;; One column start per position on every page, each page
               ;; taking those of its own columns where they fit: turning a
               ;; page moves nothing.
               (shared (romacs-grid--position-widths (apply #'append pages))))
          (mapcar (lambda (page)
                    (let ((own (seq-take shared (length (romacs-grid--position-widths page)))))
                      (romacs-grid--page-lines page (if (<= (romacs-grid--band-width own) columns) own
                                                      (romacs-grid--position-widths page)))))
                  pages)))))))

(defun romacs-grid-width (pages)
  "The widest line of PAGES, in columns."
  (apply #'max 0 (mapcar #'romacs-grid--width pages)))

(provide 'romacs-grid)
;;; romacs-grid.el ends here

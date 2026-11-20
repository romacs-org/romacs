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

(defun romacs-grid--chunks (block rows)
  "BLOCK in chunks of at most ROWS lines (one chunk when ROWS is nil)."
  (if (or (null rows) (<= (length block) rows)) (list block)
    (seq-partition block (max 1 rows))))

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

(defun romacs-grid--flow (blocks rows columns)
  "BLOCKS as one band of balanced columns within COLUMNS: the fewest
columns whose height is at most half of ROWS, else the most that fit
COLUMNS (one when none do), the shortest of those.  A list of columns,
each a list of blocks."
  ;; Every k: a balanced band's width need not grow with k (its columns
  ;; regroup), so one too wide says nothing of the next.
  (let ((candidates (cl-loop for k from 1 to (length blocks)
                             for band = (romacs-grid--balance blocks k)
                             when (<= (romacs-grid--band-width
                                       (mapcar (lambda (c) (romacs-grid--width (apply #'append c))) band))
                                      columns)
                             collect band)))
    (or (and rows (seq-find (lambda (band) (<= (apply #'max (mapcar #'romacs-grid--column-height band)) (/ rows 2)))
                            candidates))
        (car (sort (copy-sequence candidates)
                   (lambda (a b) (< (romacs-grid--band-height a) (romacs-grid--band-height b)))))
        (list blocks))))

(defun romacs-grid--band-height (band)
  "BAND's height: its tallest column's."
  (apply #'max 0 (mapcar #'romacs-grid--column-height band)))

;;;; Pages

(defun romacs-grid--fill-pages (blocks budget columns)
  "BLOCKS in pages, each the most blocks, in order, whose shortest flow
within COLUMNS is at most BUDGET rows: a list of pages, each a list of
blocks."
  (let (pages)
    (while blocks
      (let ((n 1))
        (while (and (< n (length blocks))
                    (<= (romacs-grid--band-height (romacs-grid--flow (seq-take blocks (1+ n)) nil columns)) budget))
          (setq n (1+ n)))
        (push (seq-take blocks n) pages)
        (setq blocks (nthcdr n blocks))))
    (nreverse pages)))

(defun romacs-grid--flow-pages (blocks rows columns)
  "BLOCKS (each at most ROWS lines) in pages of one band each, at most
ROWS high: as few pages as filling them allows, balanced, the least
budget of rows that still needs no more pages (as `romacs-grid--balance'
balances columns), so a last page is not a remnant.  A paged menu fills
its room, so each page's flow is its shortest (the most columns the
width holds), not one kept to half the rows."
  ;; Upward from the tallest block, not by halving: a greedy fill's page
  ;; count need not fall as its budget grows (a block it now takes can
  ;; push a wide pair apart).
  (let* ((count (length (romacs-grid--fill-pages blocks rows columns)))
         (pages (cl-loop for budget from (apply #'max 1 (mapcar #'length blocks)) to rows
                         for fill = (romacs-grid--fill-pages blocks budget columns)
                         when (<= (length fill) count) return fill)))
    (mapcar (lambda (page) (list (romacs-grid--flow page nil columns)))
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
                        (aset widths i (max (aref widths i) (romacs-grid--width (apply #'append column)))))
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

(defun romacs-grid-pages (blocks rows columns &optional bands)
  "BLOCKS laid out in pages: a list of pages, each a list of lines.
A block is a list of lines (a group: its title, then its keys).  Each
page is at most COLUMNS columns wide and, when ROWS is non-nil, at most
ROWS lines tall; every line of BLOCKS is on one page, in order, a line
wider than COLUMNS cut at a word (`romacs-grid-cut').  BANDS, how many
blocks each of the author's rows holds, is the layout kept while it
fits; otherwise the blocks flow in balanced columns (see the
Commentary).  No blocks is one empty page."
  (let* ((columns (max 1 columns))
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
      (let ((chunks (mapcan (lambda (block) (romacs-grid--chunks block rows)) blocks)))
        (let* ((pages (if (and rows (> (romacs-grid--band-height (romacs-grid--flow chunks rows columns)) rows))
                          (romacs-grid--flow-pages chunks rows columns)
                        (list (list (romacs-grid--flow chunks rows columns)))))
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

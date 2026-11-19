;;; romacs-grid-tests.el --- Key menus laid out to fit -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-grid)

(defun romacs-grid-tests--block (title n width)
  "A group TITLE of N keys whose labels are WIDTH columns wide."
  (cons title (cl-loop for i below n collect (format "  %c  %s" (+ ?a i) (make-string width ?x)))))

(defun romacs-grid-tests--menu ()
  "The agent chat's C-g menu's shape: seven groups of uneven sizes."
  (list (romacs-grid-tests--block "Approval" 3 22) (romacs-grid-tests--block "Turn" 5 30)
        (romacs-grid-tests--block "Runs on" 7 34) (romacs-grid-tests--block "Session" 6 20)
        (romacs-grid-tests--block "Workers" 11 44) (romacs-grid-tests--block "Compose" 5 18)
        (romacs-grid-tests--block "View" 4 28)))

(defun romacs-grid-tests--text (line)
  "LINE's text without its padding stretches' layout: what a reader sees."
  (substring-no-properties line))

(ert-deftest romacs-grid-fits-every-room-and-loses-nothing ()
  "Every page is within the room, and every line of every block is on
exactly one page, in order: nothing is clipped, whatever the room."
  (let ((blocks (romacs-grid-tests--menu)))
    (dolist (room '((nil . 300) (nil . 90) (40 . 200) (40 . 120) (24 . 160) (14 . 100) (10 . 60) (6 . 40) (3 . 30)))
      (let* ((rows (car room)) (columns (cdr room))
             (pages (romacs-grid-pages blocks rows columns '(1 4 2)))
             (seen (mapcan (lambda (page) (mapcan (lambda (line) (split-string (romacs-grid-tests--text line) "   +" t "[[:space:]]+"))
                                                  (copy-sequence page)))
                           (copy-sequence pages))))
        (dolist (page pages)
          (when rows (should (<= (length page) rows)))
          (dolist (line page) (should (<= (string-width line) columns))))
        ;; Every title is there once.
        (dolist (block blocks)
          (should (= 1 (cl-count (car block) seen :test #'equal))))
        ;; And every key, in each block's order.
        (should (= (length seen) (+ (length blocks) (apply #'+ (mapcar (lambda (b) (length (cdr b))) blocks)))))))))

(ert-deftest romacs-grid-keeps-the-authors-rows-while-they-fit ()
  "Declared bands lay out as written, one width per column position,
a blank line between bands."
  (let* ((blocks (list (romacs-grid-tests--block "A" 2 4) (romacs-grid-tests--block "B" 1 4)
                       (romacs-grid-tests--block "Longer title" 1 4) (romacs-grid-tests--block "D" 1 4)))
         (pages (romacs-grid-pages blocks nil 200 '(2 2)))
         (lines (mapcar #'romacs-grid-tests--text (car pages))))
    (should (= (length pages) 1))
    ;; The first position is as wide as its widest block, "Longer title".
    (should (= (string-match "B" (nth 0 lines)) (+ 12 romacs-grid-gap)))
    (should (equal (nth 3 lines) ""))
    (should (= (string-match "B" (nth 0 lines)) (string-match "D" (nth 4 lines))))))

(ert-deftest romacs-grid-flows-a-band-too-wide-into-aligned-columns ()
  "A band wider than the room flows its blocks column-major, in order;
each column starts at one place on every row it has text."
  (let* ((blocks (list (romacs-grid-tests--block "Alpha" 2 8) (romacs-grid-tests--block "Beta" 2 8)
                       (romacs-grid-tests--block "Gamma" 2 8) (romacs-grid-tests--block "Delta" 2 8)))
         (page (mapcar #'romacs-grid-tests--text (car (romacs-grid-pages blocks nil 40 '(4))))))
    ;; Two columns of two blocks: Alpha over Beta, Gamma over Delta.
    (should (string-prefix-p "Alpha" (nth 0 page)))
    (should (string-prefix-p "Beta" (nth 4 page)))
    (should (= (string-match "Gamma" (nth 0 page)) (string-match "Delta" (nth 4 page))))
    (should (= (string-match "  a  x" (nth 1 page) 3) (string-match "Gamma" (nth 0 page))))))

(ert-deftest romacs-grid-finds-a-narrower-band-past-a-wider-one ()
  "More columns can be narrower, the blocks regrouping: a band too wide
at two columns does not end the search for one that fits at three."
  (let* ((blocks (mapcar (lambda (w) (list "T" (make-string w ?x))) '(1 1 20 20 1)))
         (pages (romacs-grid-pages blocks 10 30)))
    (should (= (length pages) 1))
    (should (<= (length (car pages)) 5))))

(ert-deftest romacs-grid-leaves-half-the-room-to-the-text ()
  "With rows to spare, a flow takes the fewest columns at most half the
room tall, not the frame's whole height."
  (let* ((blocks (romacs-grid-tests--menu))
         (page (car (romacs-grid-pages blocks 60 200))))
    (should (<= (length page) 30))))

(ert-deftest romacs-grid-pages-what-will-not-fit ()
  "A menu taller than its room pages; a block taller than a page goes in
chunks, its title on the first."
  (let* ((pages (romacs-grid-pages (list (romacs-grid-tests--block "Tall" 20 6)) 8 20)))
    (should (= (length pages) 3))
    (should (equal (romacs-grid-tests--text (car (car pages))) "Tall"))
    (should (equal (mapcar #'length pages) '(8 8 5)))))

(ert-deftest romacs-grid-pages-use-the-width ()
  "A paged menu's last page lays its groups out side by side where they
fit, not down one column of a card sized for the first page."
  ;; A and B fill page 1; D and E, stacked, would be half the room tall
  ;; (the unpaged rule's limit) but go side by side.
  (let* ((blocks (list (romacs-grid-tests--block "A" 20 10) (romacs-grid-tests--block "B" 20 10)
                       (romacs-grid-tests--block "D" 4 10) (romacs-grid-tests--block "E" 4 10)))
         (pages (romacs-grid-pages blocks 22 40)))
    (should (= (length pages) 2))
    ;; D and E share a row on the last page.
    (should (string-match-p "D +E" (substring-no-properties (car (car (last pages))))))))

(ert-deftest romacs-grid-cuts-at-a-word ()
  "A line wider than the room is cut at a word, with the ellipsis; a
word longer than half the room is cut where it must be."
  (let ((truncate-string-ellipsis "..."))
    (should (equal (romacs-grid-cut "  u  Roll back to before this turn (C-u: the last)" 40)
                   "  u  Roll back to before this turn..."))
    (should (equal (romacs-grid-cut "abcdefghijklmnopqrstuvwxyz" 10) "abcdefg..."))
    (should (equal (romacs-grid-cut "short" 10) "short"))
    ;; A room narrower than the ellipsis gets none.
    (should (equal (romacs-grid-cut "abcdef" 2) "ab"))))

(ert-deftest romacs-grid-pads-with-stretches ()
  "The padding before a column is one stretch to the column's start, so
a glyph painted wider than its width cannot move the column."
  (let* ((blocks (list (list "😀 A" "  a  x") (list "B" "  b  y")))
         (line (car (car (romacs-grid-pages blocks nil 80 '(2)))))
         (at (string-match " +" line 3)))
    (should (equal (get-text-property at 'display line) `(space :align-to ,(+ 6 romacs-grid-gap))))))

(ert-deftest romacs-grid-empty ()
  (should (equal (romacs-grid-pages nil 10 10) '(nil))))

;;; romacs-grid-tests.el ends here

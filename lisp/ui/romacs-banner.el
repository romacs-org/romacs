;;; romacs-banner.el --- A banner the display moves by itself  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A banner: a block of cells at the top of a list (the agent board, and
;; every `romacs-list' later) that the render thread moves by itself
;; (romacs-animate.el), with no Lisp per frame.  It moves only while its
;; list says something is live, so an idle list costs nothing.  Its
;; patterns:
;;
;;   thread   the chat's working thread (romacs-working.el)
;;   wave     a braille sine travelling across
;;   rain     braille drops falling
;;   sparkle  cells lighting and fading at random
;;   sweep    a band of block shades moving along
;;
;; Its colours are 1 to 4 faces' foregrounds, read as each frame is
;; drawn, so a theme switch recolours it.  A module shows one over COLS
;; by ROWS characters of its own text:
;;
;;   (insert (romacs-banner-still PATTERN COLS ROWS FACES EPOCH))
;;   (romacs-banner-show ID BUFFER START PATTERN COLS ROWS FACES MOVING EPOCH)
;;   (romacs-banner-stop ID)
;;
;; The still frame is the display's first frame, so motion starts from
;; what the text shows; with motion off (`romacs-animation') the text
;; alone shows.  The thread pattern is one row, its first face its
;; colour while moving.

;;; Code:

(require 'romacs-animate)
(require 'romacs-working)

(declare-function romacs--banner-frame "crates/romacs-vm/src/prims/logo.rs"
                  (pattern cols rows faces &optional epoch time))

;; The display draws at most this many columns and rows of a banner.
(defconst romacs-banner--max-cols 400)
(defconst romacs-banner--max-rows 200)

(defconst romacs-banner-fps 30 "Frames a second of a moving banner.")

(defvar romacs-banner--banners (make-hash-table :test #'equal)
  "Each banner moving, by its id: a plist of :start, :pattern, :cols,
:rows, :faces and :epoch, what the display is told of it.")

(defun romacs-banner-random-epoch ()
  "A fresh epoch: the seed of what is random in a banner."
  (random most-positive-fixnum))

(defun romacs-banner--foregrounds (faces)
  "FACES as the display paints them over a banner's cells: their
foregrounds alone, in the text's own face otherwise."
  (mapcar (lambda (face) (list :foreground (face-foreground face nil 'default))) faces))

(defun romacs-banner-still (pattern cols rows faces &optional epoch moving)
  "The still frame of a banner in PATTERN, COLS by ROWS characters (rows
joined by newlines; at most 400 by 200), its colour stops FACES' foregrounds,
seeded by EPOCH; for the thread, one row whose look says whether it is MOVING."
  (let ((cols (min cols romacs-banner--max-cols)))
    (if (eq pattern 'thread)
        (romacs-working-still cols (if moving 'running 'idle) (car faces))
      (romacs--banner-frame pattern cols (min rows romacs-banner--max-rows)
                            (romacs-banner--foregrounds faces) epoch))))

(defun romacs-banner--element (id)
  "What the display draws of banner ID."
  (when-let* ((banner (gethash id romacs-banner--banners)))
    (list :start (plist-get banner :start) :cols (plist-get banner :cols) :rows (plist-get banner :rows)
          :kind (list 'banner (plist-get banner :pattern) (plist-get banner :epoch) (plist-get banner :faces)))))

(defun romacs-banner-show (id buffer start pattern cols rows faces moving &optional epoch)
  "Show banner ID over the COLS by ROWS characters from START (a line's
start in BUFFER, a position or a marker), where the caller put
`romacs-banner-still' of the same arguments.  It is PATTERN in FACES,
seeded by EPOCH, and moves while MOVING; at rest nothing is registered
with the display.  Showing ID again while it moves changes it in place,
its motion going on across a list's redraws."
  (cond ((eq pattern 'thread)
         (romacs-banner-stop id)
         (romacs-working-show id buffer start (min cols romacs-banner--max-cols)
                              (if moving 'running 'idle) (car faces)))
        ((not moving) (romacs-banner-stop id))
        (t
         (romacs-working-stop id)
         (puthash id (list :start start :pattern pattern :cols (min cols romacs-banner--max-cols)
                           :rows (min rows romacs-banner--max-rows) :faces faces :epoch (or epoch 0))
                  romacs-banner--banners)
         (if (eq (romacs-animate-buffer id) buffer)
             (romacs-animate-refresh id)
           (romacs-animate-start id buffer romacs-banner-fps #'ignore
                                 (lambda (_window) (romacs-banner--element id))))))
  id)

(defun romacs-banner-stop (id)
  "Show banner ID no more."
  (remhash id romacs-banner--banners)
  (romacs-working-stop id)
  (romacs-animate-stop id))

(provide 'romacs-banner)
;;; romacs-banner.el ends here

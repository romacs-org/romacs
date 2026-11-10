;;; evaporating-overlays.el --- scaling probe  -*- lexical-binding: t -*-

;; N edits of a buffer holding N evaporating overlays (as a notmuch
;; search holds one per result): each edit asks only the overlays at the
;; span it leaves whether they emptied.  Half the edits empty an overlay,
;; which is deleted, as GNU's are; they delete its span by the overlay's
;; own positions, so no text is read between edits.
(scaling-probe evaporating-overlays :n 2000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-evaporate" n)
           (goto-char (point-min))
           (let (overlays)
             (dotimes (i n)
               (let ((o (make-overlay (point) (line-end-position))))
                 (overlay-put o 'evaporate t)
                 (when (cl-evenp i) (push o overlays)))
               (forward-line 1))
             (cons (current-buffer) overlays)))
  :body (with-current-buffer (car it)
          (goto-char (point-max))
          (dotimes (_ (/ n 2))
            (insert-before-markers "xy")
            (delete-region (- (point) 2) (point)))
          (dolist (o (cdr it))
            (delete-region (overlay-start o) (overlay-end o)))))

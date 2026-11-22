;;; buffer-order-peak.el --- scaling probe  -*- lexical-binding: t -*-

;; Collections after a peak of N live buffers, all of them killed since:
;; the live set is the session's own few, whatever N was.  The buffer
;; order's arena keeps a node for each of the N (buffer_order.rs), freed,
;; and the collector reaches the order's buffers by its links, so the
;; freed nodes cost a collection nothing.  The body makes young garbage
;; until eight minors have run.  Held by its slope: instructions the eight
;; minors grow by per buffer of the earlier peak.  Measured 2026-11-21 on
;; a 242.6M body: 7.0, 2.7 and 1.2 in three runs, which is noise (7 a
;; buffer is 42k over the 6,000 more, 0.02% of the body); neither the
;; killed buffers' freed table entries nor the names they leave interned
;; (romacs#359) cost a minor anything measurable.  Walking the arena
;; instead, freed nodes included, measured 474, 492 and 468: about 60
;; a freed node a minor.  A budget of 60 holds the noise eight times over
;; and fails that walk by eight.
(scaling-probe buffer-order-peak :n 2000 :k 4 :slope 60
  :setup (progn
           (mapc #'kill-buffer (mapcar #'get-buffer-create (scaling--names "scaling-peak" n)))
           (garbage-collect)
           nil)
  :body (let ((until (+ gcs-done 8)))
          (while (< gcs-done until) (make-list 1000 nil))))

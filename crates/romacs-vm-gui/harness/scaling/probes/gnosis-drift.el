;;; gnosis-drift.el --- scaling probe  -*- lexical-binding: t -*-

;; Gnosis's collection-wide drift check over N reviewed themata, two
;; reviews and one voided review each: the whole history read in two
;; queries and each review recomputed once.
(scaling-probe gnosis-drift :n 100
  :setup (progn
           (require 'gnosis-scheduler)
           (setq gnosis-dir (file-name-as-directory (make-temp-file "scaling-gnosis-" t))
                 gnosis-db nil)
           (let ((db (gnosis--ensure-db)))
             (dotimes (i n)
               (let ((id (1+ i)))
                 (gnosis-sqlite-execute
                  db "INSERT INTO themata (id, type, keimenon, hypothesis, answer, source_guid)
                      VALUES (?, 'basic', 'Q', ?, ?, NULL)" (list id '("") '("A")))
                 (gnosis-sqlite-execute db "INSERT INTO scheduler_baseline VALUES (?, 20260801, 0, 0)"
                                        (list id))
                 (gnosis-sqlite-execute
                  db "INSERT INTO scheduler_state VALUES (?, 1, NULL, NULL, NULL, NULL, 20260801, 0, 0, 0)"
                  (list id))
                 (dotimes (k 3)
                   (gnosis-scheduler-accept-review
                    (format "%032x%032x" id (1+ k)) id (if (= k 1) 'failure 'success)
                    (+ (* id 10000000) (* k 1000000)) (+ 20260801 (* 2 k))))
                 (gnosis-scheduler-void-review (format "%032x%032x" id 9) (format "%032x%032x" id 3))))
             ;; Warm the check's code so the body counts only its work.
             (gnosis-scheduler-drift db)
             db))
  :body (gnosis-scheduler-drift it))

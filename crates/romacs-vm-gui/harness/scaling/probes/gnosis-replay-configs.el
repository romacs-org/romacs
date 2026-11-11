;;; gnosis-replay-configs.el --- scaling probe  -*- lexical-binding: t -*-

;; A Gnosis thema's replay with N review events while the database holds N
;; scheduler configurations: retention changes and exam plans each add
;; one, so both grow.  Each event finds its configuration by identity.
(scaling-probe gnosis-replay-configs :n 200
  :setup (progn
           (require 'gnosis-scheduler)
           (let ((state (list 1 nil nil nil nil 20260830 0 0 0))
                 (day 20260830)
                 events)
             (dotimes (i n)
               (let ((result (gnosis-scheduler--compute-result
                              (format "%064x" (1+ i)) 1 (if (zerop (% i 5)) 'failure 'success)
                              (* (1+ i) 1000000) day state 0.9)))
                 (push (gnosis-scheduler--evidence result) events)
                 (setq state (list 1 (plist-get result :stability) (plist-get result :difficulty)
                                   (plist-get result :reviewed-at-us) day
                                   (plist-get result :due-day) (plist-get result :reps-after)
                                   (plist-get result :lapses-after) 0)
                       day (plist-get result :due-day))))
             (list (nreverse events)
                   (cl-loop for id from 1 to n collect (list id 0.9 nil)))))
  :body (gnosis-scheduler-replay '(1 20260830 0 0) (car it)
                                 (gnosis-scheduler--config-table (cadr it)) 0))

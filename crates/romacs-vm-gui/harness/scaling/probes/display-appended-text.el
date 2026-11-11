;;; display-appended-text.el --- scaling probe  -*- lexical-binding: t -*-

;; Output arriving at the end of a large displayed buffer, as a process
;; filter inserts it, redisplayed after each insertion: what the display
;; reads of the text costs what it shows, not the buffer's size.
(scaling-probe display-appended-text :gui t :n 20000 :k 8 :steps 20 :budget 0.1
  :setup (switch-to-buffer (scaling--lines-buffer "scaling-appended" n))
  :body (with-current-buffer it
          (save-excursion (goto-char (point-max)) (insert "an appended line\n"))
          ;; Point moves, so each step paints.
          (forward-char 1)))

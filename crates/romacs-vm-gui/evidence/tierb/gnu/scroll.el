;; GNU 32.0.50 auto-scroll oracle probe (tty, -Q, defaults). Writes to $OUT.
(defun probe-out (fmt &rest args)
  (append-to-file (concat (apply #'format fmt args) "\n") nil (getenv "OUT")))
;; Pin geometry: "one" is 13 lines (12 text); the halves split a 23-line window.
(delete-other-windows)
(split-window nil 13)
(switch-to-buffer (get-buffer-create "long"))
(dotimes (i 200) (insert (format "line %d\n" (1+ i))))
(defun probe-case (label target)
  (set-window-start nil 1)
  (redisplay t)
  (goto-char (point-min)) (forward-line (1- target))
  (redisplay t)
  (probe-out "%s body=%S target=%S start-line=%S row=%S end-line=%S %%p=%S %%l=%S"
             label (window-body-height) target
             (line-number-at-pos (window-start))
             (- target (line-number-at-pos (window-start)))
             (line-number-at-pos (window-end nil t))
             (format-mode-line "%p") (format-mode-line "%l")))
(dolist (k '(22 23 24 30 100 200 201)) (probe-case "one" k))
;; scrolling up past the top
(set-window-start nil (save-excursion (goto-char (point-min)) (forward-line 99) (point)))
(goto-char (save-excursion (goto-char (point-min)) (forward-line 105) (point)))
(redisplay t)
(probe-out "start100 start-line=%S" (line-number-at-pos (window-start)))
(goto-char (save-excursion (goto-char (point-min)) (forward-line 98) (point)))
(redisplay t)
(probe-out "up-to-99 start-line=%S row=%S" (line-number-at-pos (window-start)) (- 99 (line-number-at-pos (window-start))))
;; one line past the bottom by C-n style
(set-window-start nil 1) (goto-char (point-min)) (forward-line 21) (redisplay t)
(probe-out "at-22 start-line=%S" (line-number-at-pos (window-start)))
(forward-line 1) (redisplay t)
(probe-out "next-to-23 start-line=%S row=%S" (line-number-at-pos (window-start)) (- 23 (line-number-at-pos (window-start))))
;; force start with point outside: point moves
(set-window-start nil (save-excursion (goto-char (point-min)) (forward-line 49) (point)))
(goto-char (point-min)) (redisplay t)
(probe-out "force start-line=%S point-line=%S" (line-number-at-pos (window-start)) (line-number-at-pos (point)))
;; split: smaller window
(delete-other-windows) (split-window nil 23) (split-window-below)
(dolist (k '(5 11 12 13 40)) (probe-case "half" k))
(other-window 1)
(dolist (k '(5 11 12 40)) (probe-case "lower" k))
(delete-other-windows)
(kill-emacs 0)

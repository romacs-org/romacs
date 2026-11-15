;;; redisplay-far-jump.el --- scaling probe  -*- lexical-binding: t -*-

;; Point jumps N lines down from the window's top and back, under
;; `scroll-conservatively' 101, each step painted.  Choosing the start
;; that shows point lays out the rows around point and GNU's search
;; below the window (try_scrolling's SCROLL_LIMIT, 100 lines), not the
;; lines the jump passed over: a step costs the same however far point
;; went.  Held by its slope, instructions per line jumped a step: 3.6
;; (6.8 M a step at N and at 8N); 9,764 when the jump laid out every line
;; it passed over.
(scaling-probe redisplay-far-jump :gui t :n 2000 :k 8 :steps 8 :slope 10
  :setup (let ((buffer (scaling--lines-buffer "scaling-far-jump" (+ n 200))))
           (switch-to-buffer buffer)
           (setq scroll-conservatively 101)
           (set-window-start nil 1)
           (cons buffer (with-current-buffer buffer (goto-char (point-min)) (forward-line n) (prog1 (point) (goto-char 1)))))
  :body (with-current-buffer (car it)
          (goto-char (if (cl-evenp i) (cdr it) 1))))

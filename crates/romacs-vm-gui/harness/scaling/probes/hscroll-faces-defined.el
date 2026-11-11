;;; hscroll-faces-defined.el --- scaling probe  -*- lexical-binding: t -*-

;; Cursor motion in a truncated window, which places point's line for its
;; hscroll each redisplay: N faces defined, then N distinct faces on
;; point's line.  Setup returns the buffer: a step's timer runs in whatever
;; buffer is current then.
(scaling-probe hscroll-faces-defined :gui t :n 500 :steps 40
  :setup (progn (dotimes (i n) (custom-declare-face (intern (format "scaling-hs-%d" i)) '((t :weight bold)) "x" :group 'scaling))
                (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (insert (make-string 400 ?x) "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))

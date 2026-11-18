;;; window-font-width.el --- scaling probe  -*- lexical-binding: t -*-

;; `window-font-width' N times in a step, as per-command callers ask it
;; (`window-max-chars-per-line', a line-number width): each call opens the
;; default face's font by name (`font-info'), whose family the display
;; lists once until its font cache is cleared, not on every call.  Held by
;; its slope, instructions per call: about 61 thousand, where listing the
;; family again each call cost about 2 million.
(scaling-probe window-font-width :gui t :idle t :n 100 :k 4 :steps 1 :slope 80000
  :setup (switch-to-buffer (get-buffer-create "scaling-font-width"))
  :body (dotimes (_ n) (window-font-width)))

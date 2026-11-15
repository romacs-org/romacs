;;; render_stages.el --- scenarios only the render thread's bars need  -*- lexical-binding: t -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; render_stages_live.py loads this after display_stages.el: its own
;; scenarios, in that file's form.  They are kept from display_stages.el
;; because nothing they show is the evaluator's to hold: three windows of
;; which one changes, and a window scrolled by a line, cost redisplay what
;; one window typed in costs it, and differ in what the render thread has
;; to draw again.

;;; Code:

(require 'display-stages)

;; Three windows on three buffers; a character typed in the selected one.
(display-stages-scenario typed-three-windows :paints t
  :setup (let ((a (scaling--lines-buffer "stages-a" 20000))
               (b (scaling--lines-buffer "stages-b" 20000)))
           (delete-other-windows)
           (switch-to-buffer a) (split-window-below) (other-window 1)
           (switch-to-buffer b) (split-window-right) (other-window 1)
           (display-stages--show (scaling--lines-buffer "stages" 20000)))
  :body (insert "a"))
;; The window scrolls by one line: every row moves, one enters.
(display-stages-scenario scroll-line :paints t
  :setup (display-stages--show (scaling--lines-buffer "stages" 20000))
  :body (scroll-up 1))

(provide 'render-stages)
;;; render_stages.el ends here

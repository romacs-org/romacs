;;; mode-line-contexts-saturated.el --- scaling probe  -*- lexical-binding: t -*-

;; N side windows whose own lines are each wider than the bar, so every
;; one is dropped when the bar fits (VM2-STATUS-BAR): fitting and folding
;; stay linear in what they drop.
(scaling-probe mode-line-contexts-saturated :gui t :n 3 :steps 20
  :setup (progn
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-saturated" 20)))
             (switch-to-buffer main)
             (dotimes (i n)
               (let ((buffer (scaling--lines-buffer (format "scaling-saturated-%d" i) 2)))
                 (with-current-buffer buffer (setq-local mode-line-format (list (make-string 200 (+ ?a (% i 26))))))
                 (display-buffer-in-side-window buffer `((side . ,(if (cl-oddp i) 'left 'right)) (slot . ,i) (window-width . 10)))))
             main))
  :body (with-current-buffer it (insert "x")))

;;; header-line-stretches.el --- scaling probe  -*- lexical-binding: t -*-

;; A header line of N columns, each its own face run then a `(space
;; :align-to ...)' (VM2-MODE-LINE-STRETCH), redisplayed after each key in
;; the buffer under it: the band's stretches resolve with the window's
;; geometry read once, not once per stretch, and the face runs once each.
(scaling-probe header-line-stretches :gui t :n 40 :steps 20
  :setup (let ((buffer (scaling--lines-buffer "scaling-header" 50)))
           (switch-to-buffer buffer)
           (setq header-line-format
                 (cl-loop for i below n append
                          (list (propertize (format "c%d" i) 'face (if (cl-oddp i) 'bold 'italic))
                                (propertize " " 'display `(space :align-to ,(* 3 (1+ i)))))))
           buffer)
  :body (with-current-buffer it (insert "x")))

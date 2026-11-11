;;; vertical-motion-up-hidden.el --- scaling probe  -*- lexical-binding: t -*-

;; The same over long lines a display property hides, one row each.
(scaling-probe vertical-motion-up-hidden :gui t :n 100 :k 8 :steps 1
  :setup (with-current-buffer (get-buffer-create "scaling-motion-hidden")
           (erase-buffer)
           (dotimes (_ (* 2 n))
             (let ((start (point)))
               (insert (make-string 400 ?x))
               (put-text-property (1+ start) (point) 'display "")
               (insert "\n")))
           (switch-to-buffer (current-buffer)))
  :body (with-current-buffer it
          (goto-char (point-max))
          (vertical-motion (- n))))

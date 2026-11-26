;;; tabulated-list-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; A tabulated list of N entries, drawn as a list (romacs-list.el), printed
;; again updating with point on an entry past its middle: each shown
;; entry's line is kept by id from one table, so the print is linear.
(scaling-probe tabulated-list-rows :n 2500
  :setup (let ((buffer (generate-new-buffer " *scaling-tabulated*")))
           (with-current-buffer buffer
             (tabulated-list-mode)
             (setq tabulated-list-format [("Name" 12 t) ("Size" 8 t)]
                   tabulated-list-sort-key '("Name")
                   tabulated-list-entries
                   (cl-loop for i below n
                            collect (list i (vector (format "entry %06d" i) (number-to-string (* 7 i))))))
             (tabulated-list-init-header)
             (tabulated-list-print)
             (forward-line (/ (* 2 n) 3)))
           buffer)
  :body (with-current-buffer it (tabulated-list-print t t)))

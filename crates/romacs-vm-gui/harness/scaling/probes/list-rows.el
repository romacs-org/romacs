;;; list-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; A list (romacs-list.el) of N rows in groups of ten, with columns and a
;; live cell on every row, drawn again with point on a row past its middle:
;; each row's line is made once and its spot found in one pass, so the
;; redraw is linear in the rows shown.
(defvar-local scaling--list-rows nil "The rows the probe's list draws.")

(scaling-probe list-rows :n 2500
  :setup (progn (require 'romacs-list)
                (put 'scaling--list-name 'romacs-list-live t)
                (defalias 'scaling--list-name (lambda (row) (format "row %d" (plist-get row :id))))
                (let ((buffer (generate-new-buffer " *scaling-list*"))
                      (columns '((scaling--list-name . fill) (ignore . 6))))
                  (with-current-buffer buffer
                    (romacs-list-mode)
                    (setq-local romacs-list-group-kind 'group
                                romacs-list-line-function
                                (lambda (row width) (romacs-list-cells columns row width "  " t))
                                romacs-list-banner 'thread
                                scaling--list-rows
                                (cl-loop for i below n
                                         collect (list :kind (if (zerop (% i 10)) 'group 'item) :id i)))
                    (romacs-list-draw scaling--list-rows 120)
                    (romacs-list-goto (/ (* 2 n) 3)))
                  buffer))
  :body (with-current-buffer it (romacs-list-draw scaling--list-rows 120)))

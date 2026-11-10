;;; syntax-summaries-refill.el --- scaling probe  -*- lexical-binding: t -*-

;; Erased and filled again, then edited: cut into leaves anew and served.
(scaling-probe syntax-summaries-refill :n 2000
  :setup (with-current-buffer (scaling--syntax-buffer " scaling-syntax-refill" n)
           (cons (current-buffer) (buffer-string)))
  :body (with-current-buffer (car it)
          (erase-buffer)
          (insert (cdr it))
          (parse-partial-sexp (point-min) (point-max))
          (goto-char 100)
          (insert "x")
          (parse-partial-sexp (point-min) (point-max))))

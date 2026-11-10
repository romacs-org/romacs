;;; syntax-summaries-starter-runs.el --- scaling probe  -*- lexical-binding: t -*-

;; Runs of `/', each a comment starter's first and second character, longer
;; than a leaf: their leaf ends move or stay unsafe in one bounded search
;; each, never a character at a time over the text.
(scaling-probe syntax-summaries-starter-runs :n 400
  :setup (scaling--syntax-buffer " scaling-syntax-runs" n (concat "a = b;\n" (make-string 3000 ?/) "\n"))
  :body (with-current-buffer it
          (dotimes (_ 5)
            (goto-char 3)
            (insert "x")
            (parse-partial-sexp (point-min) (point-max)))))

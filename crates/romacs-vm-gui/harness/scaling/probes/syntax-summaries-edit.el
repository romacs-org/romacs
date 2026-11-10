;;; syntax-summaries-edit.el --- scaling probe  -*- lexical-binding: t -*-

;; An edit near the top, then the state at the end: the edited leaf
;; rescanned and the rest composed, nothing in N.
(scaling-probe syntax-summaries-edit :n 4000 :budget 0.5
  :setup (scaling--syntax-buffer " scaling-syntax-edit" n)
  :body (with-current-buffer it
          (dotimes (_ 20)
            (goto-char 100)
            (insert "x")
            (parse-partial-sexp (point-min) (point-max)))))

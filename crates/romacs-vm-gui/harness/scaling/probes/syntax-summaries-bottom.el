;;; syntax-summaries-bottom.el --- scaling probe  -*- lexical-binding: t -*-

;; Lines that each close a list no line opened, as prose numbered "1)"
;; does: every leaf closes past the bottom level, where its summary is the
;; one for the count of levels it is entered with. An edit near the top,
;; then the state at the end: the edited leaf rescanned and the rest
;; composed, nothing in N.
(scaling-probe syntax-summaries-bottom :n 4000 :budget 0.5
  :setup (scaling--syntax-buffer " scaling-syntax-bottom" n "1) item of prose text here, forty chars.\n")
  :body (with-current-buffer it
          (dotimes (_ 20)
            (goto-char 100)
            (insert "x")
            (parse-partial-sexp (point-min) (point-max)))))

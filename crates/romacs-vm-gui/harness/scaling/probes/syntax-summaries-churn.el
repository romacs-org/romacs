;;; syntax-summaries-churn.el --- scaling probe  -*- lexical-binding: t -*-

;; Nested comment starts typed near the top, each a new key for every node
;; below it: each parse rescans the rest once (as the scanner would), and
;; what a node keeps stays at four keys, whatever the history.
(scaling-probe syntax-summaries-churn :n 2000
  :setup (scaling--syntax-buffer " scaling-syntax-churn" n)
  :body (with-current-buffer it
          (dotimes (_ 10)
            (goto-char 50)
            (insert "#|")
            (parse-partial-sexp (point-min) (point-max)))
          (let ((bytes (plist-get (romacs-syntax-summaries-stats) :bytes)))
            (when (> bytes (* 2 (buffer-size)))
              (error "Syntax summaries hold %d bytes for %d characters" bytes (buffer-size))))))

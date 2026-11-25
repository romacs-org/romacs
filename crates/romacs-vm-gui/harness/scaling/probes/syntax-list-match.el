;;; syntax-list-match.el --- scaling probe  -*- lexical-binding: t -*-

;; A forward match from the opener of one list that holds the whole
;; buffer, N lines of about 64 characters (64K to 4M at k 64), as
;; electric-pair and cc-mode ask.  Warm, the scan folds the summaries
;; between: nothing in N.
(scaling-probe syntax-list-match-warm :n 1000 :k 64 :budget 0.5
  :setup (with-current-buffer (scaling--syntax-buffer " scaling-list-match-warm" n)
           (goto-char (point-min))
           (insert "{\n")
           (goto-char (point-max))
           (insert "}\n")
           (scan-lists 1 1 0)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ 20)
            (scan-lists 1 1 0))))

;; The same after an edit near the opener that leaves the propertize
;; frontier at the edit, as `syntax-ppss-flush-cache' does: the scan
;; stops for `internal--syntax-propertize' there, which has nothing to
;; propertize (no `syntax-propertize-function') and moves the frontier to
;; the end, and folds the rest: nothing in N.
(scaling-probe syntax-list-match-edit :n 1000 :k 64 :budget 0.5
  :setup (with-current-buffer (scaling--syntax-buffer " scaling-list-match-edit" n)
           (goto-char (point-min))
           (insert "{\n")
           (goto-char (point-max))
           (insert "}\n")
           (setq-local parse-sexp-lookup-properties t)
           (setq-local syntax-propertize-function nil)
           (setq syntax-propertize--done (point-max))
           (scan-lists 1 1 0)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ 20)
            (goto-char 100)
            (insert "x")
            (setq syntax-propertize--done 100)
            (scan-lists 1 1 0))))

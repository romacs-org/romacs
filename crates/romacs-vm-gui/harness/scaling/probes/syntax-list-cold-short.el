;;; syntax-list-cold-short.el --- scaling probe  -*- lexical-binding: t -*-

;; The first scans of fresh buffers, each a list of three leaves followed by
;; an untouched tail of escapes N lines long (64K to 4M characters at k 64):
;; the index cuts its partition, and checks its cuts, only as far as a scan
;; reaches, so nothing grows with the tail.  Escapes make every cut in the
;; tail one a check would have to move.
(scaling-probe syntax-list-cold-short :n 1000 :k 64 :budget 0.5
  :setup (let ((buffers nil))
           (dotimes (i 10)
             (with-current-buffer (get-buffer-create (format " scaling-cold-short-%d" i))
               (erase-buffer)
               (insert "(" (make-string 3000 ?x) ")\n")
               (dotimes (_ n) (insert (make-string 63 ?\\) "\n"))
               (push (current-buffer) buffers)))
           buffers)
  :body (dolist (buffer it)
          (with-current-buffer buffer
            (scan-sexps 1 1))))

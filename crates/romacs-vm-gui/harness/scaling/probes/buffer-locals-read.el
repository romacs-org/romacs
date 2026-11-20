;;; buffer-locals-read.el --- scaling probe  -*- lexical-binding: t -*-

;; Reading a buffer's local among N locals the buffer holds: a lookup in
;; the buffer's own store, a binary search, so N costs a comparison per
;; doubling and the work stays flat.  GNU's `assq' over `local_var_alist'
;; is linear here.
(defvar-local scaling--read-local-target nil)
(scaling-probe buffer-locals-read :n 500 :k 4 :budget 0.1
  :setup (with-current-buffer (get-buffer-create " scaling-read-locals")
           (dolist (s (mapcar #'intern (scaling--names "scaling--read-local" n)))
             (set (make-local-variable s) 0))
           (setq scaling--read-local-target 1)
           (current-buffer))
  :body (with-current-buffer it
          (let ((x nil))
            (dotimes (_ 20000) (setq x scaling--read-local-target))
            x)))

;;; lambda-lists.el --- scaling probe  -*- lexical-binding: t -*-

;; N `(lambda ...)' lists, each called five times: one compile per list,
;; then hits (`Vm::lambda_list_closure'); the cache holds what was called
;; since the last collection.
(scaling-probe lambda-lists :n 200
  :setup (let (functions)
           (dotimes (i n) (push (list 'lambda '(x) (list '+ 'x i)) functions))
           functions)
  :body (dotimes (_ 5) (dolist (f it) (funcall f 1))))

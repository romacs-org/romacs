;;; gc-lambda-lists.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-lambda-lists :n 20000 :budget 1.1
  :setup (let ((lists nil))
           (dotimes (i n)
             (let ((f (list 'lambda '(x) (list '+ 'x i (list 'length (list 'quote (make-list 8 i)))))))
               (funcall f 1)
               (push f lists)))
           (setq scaling--lambda-lists lists)
           nil)
  :body (dotimes (_ 3) (garbage-collect)))

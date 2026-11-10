;;; gc-mark.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-mark :n 100000
  ;; Collections over N live items of every kind the collector's threads
  ;; trace, kept by a list.  Mark and sweep are linear in the tables, on
  ;; however many threads; the harness counts them on one (scaling_live.py).
  :setup (let ((all nil))
           (dotimes (i n)
             (let ((x i)) (push (list x (vector x x) (make-string 8 ?x) (lambda () (setq x (1+ x)))) all)))
           (setq scaling--live all)
           nil)
  :body (dotimes (_ 3) (garbage-collect)))

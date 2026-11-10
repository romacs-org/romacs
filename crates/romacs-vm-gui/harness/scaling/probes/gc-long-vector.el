;;; gc-long-vector.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-long-vector :n 1000000
  ;; One vector of N conses, too long for one thread's turn: it goes out in
  ;; parts, and the parts cost what the slots do.
  :setup (let ((v (make-vector n nil)))
           (dotimes (i n) (aset v i (cons i nil)))
           (setq scaling--live v)
           nil)
  :body (dotimes (_ 3) (garbage-collect)))

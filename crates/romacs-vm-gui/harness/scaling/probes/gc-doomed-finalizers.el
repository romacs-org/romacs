;;; gc-doomed-finalizers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-doomed-finalizers :n 20000
  ;; N finalizers made unreachable together, in the counted body, whose
  ;; collection dooms them all and whose run drains them in order.  A queue
  ;; shifted at each removal grows as the square of N.  The last one made may
  ;; stay in a register.
  :setup nil
  :body (progn
          (setq scaling--finalized 0)
          (dotimes (_ n)
            (make-finalizer (lambda () (setq scaling--finalized (1+ scaling--finalized)))))
          (garbage-collect)
          (unless (>= scaling--finalized (1- n))
            (error "%d of %d finalizers ran" scaling--finalized n))))

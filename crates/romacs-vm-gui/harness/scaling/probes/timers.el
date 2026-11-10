;;; timers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe timers :n 200
  :known "ticket timers: timer--activate walks the sorted timer-list, ~12k instructions a step"
  :setup nil
  :body (let (timers)
          (dotimes (i n) (push (run-with-timer (+ 3600 i) nil #'ignore) timers))
          (dotimes (_ 20) (sit-for 0))
          (mapc #'cancel-timer timers)))

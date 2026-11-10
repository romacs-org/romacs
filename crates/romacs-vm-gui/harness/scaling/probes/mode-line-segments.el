;;; mode-line-segments.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe mode-line-segments :n 500
  ;; Segments defined while the mode is on, each on a shared hook and on
  ;; a hook of its own, so both registries grow; then both hooks a
  ;; refresh runs on.
  :setup (progn (require 'romacs-mode-line) (romacs-mode-line-mode 1) nil)
  :body (progn
          (dotimes (i n)
            (let ((own (intern (format "scaling-mode-line-hook-%d" i))))
              (set own nil)
              (eval `(romacs-mode-line-define-segment ,(intern (format "scaling-%d" i)) "A probe."
                       :compute (lambda () nil) :hooks (scaling-mode-line-hook ,own))
                    t)))
          (run-hooks 'scaling-mode-line-hook)
          (run-hooks 'after-change-major-mode-hook)))

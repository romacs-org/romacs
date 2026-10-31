;;; perfbench-gui.el --- perfbench.py's GUI measurements, the same Lisp in GNU and Romacs  -*- lexical-binding: t -*-

;;; Commentary:
;; Loaded by the GUI init perfbench.py writes for both engines.  Each
;; command's latency runs from `pre-command-hook' to the next idle moment,
;; which comes after the command's redisplay: a repeating zero-second idle
;; timer fires once per idle period.  The harness reads
;; `perfbench-gui-latencies' through the parity probe.

;;; Code:

(defvar perfbench-gui--started nil "When the running command began.")
(defvar perfbench-gui-latencies nil "Seconds from each command's start to the next idle moment.")

(defun perfbench-gui-reset ()
  (setq perfbench-gui-latencies nil perfbench-gui--started nil)
  t)

(defun perfbench-gui-stamp (file)
  "Write the time to FILE."
  (with-temp-file file (insert (number-to-string (float-time)))))

(defun perfbench-gui-pause (seconds)
  "Stop the parity probe's polling for SECONDS, so a measured phase counts none of it."
  (let ((timer (seq-find (lambda (timer) (eq (timer--function timer) 'parity-probe--poll)) timer-list)))
    (when timer
      (cancel-timer timer)
      (run-with-timer seconds nil #'timer-activate timer)))
  t)

(add-hook 'pre-command-hook (lambda () (setq perfbench-gui--started (float-time))))
(run-with-idle-timer 0 t (lambda ()
                           (when perfbench-gui--started
                             (push (- (float-time) perfbench-gui--started) perfbench-gui-latencies)
                             (setq perfbench-gui--started nil))))

;;; perfbench-gui.el ends here

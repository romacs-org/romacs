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
(defvar perfbench-gui-commands 0
  "How many commands ran.  Keys that arrive together share an idle moment, so
under load this, not `perfbench-gui-latencies', says how many keys ran.")

(load (expand-file-name "perfbench-loaded.el" (file-name-directory load-file-name)) nil t t)

(defun perfbench-gui-write-loaded (file)
  "Write `perfbench-loaded' to FILE."
  (with-temp-file file (prin1 (perfbench-loaded) (current-buffer))))

(defun perfbench-gui-reset ()
  (setq perfbench-gui-latencies nil perfbench-gui--started nil perfbench-gui-commands 0)
  t)

(defun perfbench-gui-stamp (file)
  "Write the time to FILE."
  (with-temp-file file (insert (number-to-string (float-time)))))

(defvar perfbench-gui--paused nil "The probe's timer while paused, and the timer that restores it.")

(defun perfbench-gui-pause (seconds)
  "Stop the parity probe's polling for SECONDS at most, so a measured phase
counts none of it; `perfbench-gui-resume' ends that sooner."
  (let ((timer (seq-find (lambda (timer) (eq (timer--function timer) 'parity-probe--poll)) timer-list)))
    (when timer
      (cancel-timer timer)
      (setq perfbench-gui--paused (cons timer (run-with-timer seconds nil #'perfbench-gui-resume)))))
  t)

(defun perfbench-gui-resume ()
  "Restart the parity probe's polling, if it is paused."
  (when perfbench-gui--paused
    (cancel-timer (cdr perfbench-gui--paused))
    (timer-activate (car perfbench-gui--paused))
    (setq perfbench-gui--paused nil)))

(add-hook 'pre-command-hook (lambda () (setq perfbench-gui--started (float-time)
                                             perfbench-gui-commands (1+ perfbench-gui-commands))))
(run-with-idle-timer 0 t (lambda ()
                           (when perfbench-gui--started
                             (push (- (float-time) perfbench-gui--started) perfbench-gui-latencies)
                             (setq perfbench-gui--started nil))))

(defun perfbench-gui-visit (file ts where)
  "Visit FILE afresh in the selected window alone, C in `c-ts-mode' when TS,
point at WHERE: `top', or `middle' (`perfbench-middle').  Its major mode."
  (let ((old (find-buffer-visiting file)))
    (when old
      (with-current-buffer old (set-buffer-modified-p nil))
      (kill-buffer old)))
  (delete-other-windows)
  (let ((major-mode-remap-alist (if ts (cons '(c-mode . c-ts-mode) major-mode-remap-alist) major-mode-remap-alist)))
    (find-file file))
  (if (eq where 'middle) (perfbench-middle) (goto-char (point-min)))
  (recenter)
  (symbol-name major-mode))

;;; perfbench-gui.el ends here

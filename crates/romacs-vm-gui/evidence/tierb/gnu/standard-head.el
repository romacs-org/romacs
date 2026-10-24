;; GNU oracle: the shape of bindings.el's standard-mode-line-format, whose
;; head is "%e", with its variables void (as before bindings.el defines them),
;; and "%e" while `memory-full' is non-nil.
;; Run: OUT=file emacs -Q -nw -l standard-head.el   (tty; writes, then exits)
(defun ml-out (fmt &rest args)
  (append-to-file (concat (apply #'format fmt args) "\n") nil (getenv "OUT")))
(switch-to-buffer (get-buffer-create "probe"))
(fundamental-mode)
(redisplay t)
(ml-out "memory-full=%S" memory-full)
(dolist (fmt '(("%e" "%b")
               ("%e" romacs-void-front-space romacs-void-mule-info "%b" "   "
                (romacs-void-project romacs-void-project-format) "  " "|")
               "%3e|"))
  (ml-out "%S => %S" fmt (substring-no-properties (format-mode-line fmt))))
(let ((memory-full t))
  (ml-out "memory-full=t: %S => %S" "%e|" (substring-no-properties (format-mode-line "%e|"))))
(kill-emacs 0)

;; GNU oracle: mode-line constructs whose values hold function objects, as
;; bindings.el's mode-line-mule-info and mode-line-modes do (keymaps with
;; command closures inside :propertize plists).
;; Run: OUT=file emacs -Q -nw -l object-values.el   (tty; writes, then exits)
(defun ml-out (fmt &rest args)
  (append-to-file (concat (apply #'format fmt args) "\n") nil (getenv "OUT")))
(switch-to-buffer (get-buffer-create "probe"))
(fundamental-mode)
(redisplay t)
(defvar ml-probe-propertized
  (list (list :propertize "%b" 'local-map
              (list 'keymap (cons 'down-mouse-1 (lambda () (interactive) nil)))
              'help-echo (lambda (&rest _) "help"))))
(put 'ml-probe-propertized 'risky-local-variable t)
(defvar ml-probe-object (list "<" (lambda () 1) ">"))
(put 'ml-probe-object 'risky-local-variable t)
(dolist (fmt '(("" ml-probe-propertized "|")
               ("" ml-probe-object "|")))
  (ml-out "%S => %S" fmt (substring-no-properties (format-mode-line fmt))))
(kill-emacs 0)

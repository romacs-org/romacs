;;; startup-trace.el --- Trace shutdown-prone startup paths -*- lexical-binding: t; -*-

(defvar romacs-startup-trace-file
  (or (getenv "ROMACS_STARTUP_TRACE_FILE")
      "/tmp/romacs-startup-trace.log")
  "File that receives startup trace lines.")

(defvar romacs-startup-trace-enabled t
  "Non-nil enables startup tracing.")

(defun romacs-startup-trace--write (fmt &rest args)
  "Append one formatted trace line to `romacs-startup-trace-file'."
  (when romacs-startup-trace-enabled
    (let ((line (concat (apply #'format fmt args) "\n")))
      (with-temp-buffer
        (insert line)
        (append-to-file (point-min) (point-max) romacs-startup-trace-file)))))

(defun romacs-startup-trace--safe (fn)
  "Call FN and turn any error into a printable marker."
  (condition-case err
      (funcall fn)
    (error (format "<error %S>" err))))

(defun romacs-startup-trace--frame-snapshot ()
  "Return a compact snapshot of frame state for trace logging."
  (list
   :selected (romacs-startup-trace--safe #'selected-frame)
   :live-selected
   (romacs-startup-trace--safe
    (lambda ()
      (let ((frame (selected-frame)))
        (frame-live-p frame))))
   :frames (romacs-startup-trace--safe #'frame-list)
   :visible (romacs-startup-trace--safe #'visible-frame-list)
   :window-system window-system
   :initial-window-system initial-window-system
   :args-left command-line-args-left))

(defun romacs-startup-trace--around-kill-emacs (orig &rest args)
  "Log kill-emacs calls before delegating to ORIG."
  (romacs-startup-trace--write
   "kill-emacs args=%S snapshot=%S"
   args
   (romacs-startup-trace--frame-snapshot))
  (apply orig args))

(defun romacs-startup-trace--around-delete-frame (orig &rest args)
  "Log delete-frame calls before delegating to ORIG."
  (romacs-startup-trace--write
   "delete-frame args=%S snapshot=%S"
   args
   (romacs-startup-trace--frame-snapshot))
  (prog1
      (apply orig args)
    (romacs-startup-trace--write
     "delete-frame return snapshot=%S"
     (romacs-startup-trace--frame-snapshot))))

(defun romacs-startup-trace--around-frame-live-p (orig object)
  "Log calls where `frame-live-p' returns nil."
  (let ((result (funcall orig object)))
    (when (null result)
      (romacs-startup-trace--write
       "frame-live-p nil object=%S snapshot=%S"
       object
       (romacs-startup-trace--frame-snapshot)))
    result))

(defun romacs-startup-trace--mark (label)
  "Log LABEL with the current startup snapshot."
  (romacs-startup-trace--write
   "%s snapshot=%S"
   label
   (romacs-startup-trace--frame-snapshot)))

(defun romacs-startup-trace--around-suspect (name orig &rest args)
  "Log startup-sensitive calls to NAME around ORIG with ARGS."
  (romacs-startup-trace--write
   "enter %s args=%S snapshot=%S"
   name
   args
   (romacs-startup-trace--frame-snapshot))
  (condition-case err
      (let ((result (apply orig args)))
        (romacs-startup-trace--write
         "leave %s result=%S snapshot=%S"
         name
         result
         (romacs-startup-trace--frame-snapshot))
        result)
    (error
     (romacs-startup-trace--write
      "error %s err=%S snapshot=%S"
      name
      err
      (romacs-startup-trace--frame-snapshot))
     (signal (car err) (cdr err)))))

(advice-add 'kill-emacs :around #'romacs-startup-trace--around-kill-emacs)
(advice-add 'delete-frame :around #'romacs-startup-trace--around-delete-frame)
(advice-add 'frame-live-p :around #'romacs-startup-trace--around-frame-live-p)
(dolist (fn '(frame-set-background-mode
              frame--current-background-mode
              frame-parameter
              display-color-p
              xw-display-color-p
              x-display-grayscale-p))
  (when (fboundp fn)
    (advice-add fn :around
                (apply-partially #'romacs-startup-trace--around-suspect fn))))

(add-hook 'emacs-startup-hook
          (lambda ()
            (romacs-startup-trace--mark "emacs-startup-hook")))
(add-hook 'window-setup-hook
          (lambda ()
            (romacs-startup-trace--mark "window-setup-hook")))

(romacs-startup-trace--mark "startup-trace-loaded")

(provide 'startup-trace)

;;; startup-trace.el ends here

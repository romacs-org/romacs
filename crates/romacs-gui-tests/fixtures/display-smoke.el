;;; display-smoke.el --- Native editor smoke -*- lexical-binding: t; -*-

(defun display-smoke-fail (failure)
  (write-region (format "%S\n" failure) nil "/dev/stderr")
  (kill-emacs 1))

(defun display-smoke-resized (before deadline)
  (cond
   ((>= (frame-pixel-width) (+ before 160))
    (write-region "inserted, pasted, resized\n" nil "display-smoke-result")
    (kill-emacs 0))
   ((< (float-time) deadline)
    (run-at-time 0.05 nil #'display-smoke-resized before deadline))
   (t (display-smoke-fail (list 'resize before (frame-pixel-width))))))

(defun display-smoke-clipboard (text before deadline)
  (condition-case failure
      (progn
        (romacs--clipboard-set (buffer-string))
        (let ((pasted (romacs--clipboard-text)))
          (cond
           ((equal pasted text)
            (erase-buffer)
            (insert pasted)
            (write-region (format "%s\n" before) nil "display-smoke-ready")
            (run-at-time 0.05 nil #'display-smoke-resized before (+ (float-time) 10)))
           ((< (float-time) deadline)
            (run-at-time 0.05 nil #'display-smoke-clipboard text before deadline))
           (t (error "Clipboard round-trip returned %S" pasted)))))
    (error (display-smoke-fail failure))))

(defun display-smoke-run ()
  (condition-case failure
      (let ((before (frame-pixel-width))
            (text "Romacs clipboard ✓"))
        (unless (and (eq window-system 'pgtk)
                     (equal (pgtk-backend-display-class) (getenv "ROMACS_SMOKE_CLASS"))
                     (equal (terminal-name) (getenv "ROMACS_SMOKE_DISPLAY")))
          (error "Unexpected display %S %S %S" window-system
                 (pgtk-backend-display-class) (terminal-name)))
        (erase-buffer)
        (insert text)
        ;; A Wayland selection requires a serial from the focused seat.
        (when (equal (pgtk-backend-display-class) "GdkWaylandDisplay")
          (unless (eql 0 (call-process (getenv "ROMACS_SMOKE_WTYPE")
                                       nil nil nil "-k" "Shift_L"))
            (error "wtype could not seed the virtual seat")))
        (run-at-time 0.05 nil #'display-smoke-clipboard text before (+ (float-time) 10)))
    (error (display-smoke-fail failure))))

(run-at-time 0 nil #'display-smoke-run)

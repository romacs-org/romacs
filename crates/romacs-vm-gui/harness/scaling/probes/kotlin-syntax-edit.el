;;; kotlin-syntax-edit.el --- scaling probe -*- lexical-binding: t; -*-

;; Four edits beside a raw-string opener with a growing tail of interpolated
;; strings. Measured 0.73 extra instructions per row per step; 20 covers
;; the residual growth and catches a scan of the tail.
(scaling-probe kotlin-syntax-edit :n 22000 :k 4 :slope 20
  :setup (progn
           (require 'kotlin-ts-mode)
           (let ((buffer (get-buffer-create " scaling-kotlin-syntax-edit")))
             (with-current-buffer buffer
               (erase-buffer)
               (insert "val draft = 0\n")
               (dotimes (_ n)
                 (insert "val text = \"value ${if (true) { 1 } else { 0 }}\"\n"))
               (kotlin-ts-mode)
               (syntax-ppss (point-max))
               (goto-char (1+ (length "val draft = ")))
               (delete-char 1)
               (insert "\"\"\"")
               (unless (nth 3 (syntax-ppss))
                 (error "Unterminated raw literal is not open")))
             buffer))
  :body (with-current-buffer it
          (dotimes (_ 4)
            (insert "x")
            (unless (nth 3 (syntax-ppss))
              (error "Edit left the raw literal")))))

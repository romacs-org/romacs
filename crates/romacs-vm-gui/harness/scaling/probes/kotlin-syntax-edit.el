;;; kotlin-syntax-edit.el --- scaling probe -*- lexical-binding: t; -*-

;; Four edits beside a raw-string opener with a growing tail of interpolated
;; strings. Measured 0.49 extra instructions per row per step. 20 covers
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

(defun scaling--kotlin-syntax-at-tail (name prefix row n suffix)
  "Prepare NAME for an edit after N copies of ROW between PREFIX and SUFFIX."
  (require 'kotlin-ts-mode)
  (let ((buffer (get-buffer-create name)))
    (with-current-buffer buffer
      (erase-buffer)
      (insert prefix)
      (dotimes (_ n) (insert row))
      (insert suffix)
      (kotlin-ts-mode)
      (syntax-ppss (point-max))
      (goto-char (- (point-max) (length suffix))))
    buffer))

;; The same local edit as the distance from its raw opener grows. Measured
;; 1.26 extra instructions per row per step. 20 leaves room for indexing.
(scaling-probe kotlin-syntax-deep-edit :n 22000 :k 4 :slope 20
  :setup (scaling--kotlin-syntax-at-tail
          " scaling-kotlin-syntax-deep" "val raw = \"\"\"\n"
          "Raw content with ${name} and \"quotes\".\n" n "edit here\n\"\"\"\n")
  :body (with-current-buffer it
          (dotimes (_ 4)
            (insert "x")
            (unless (nth 3 (syntax-ppss))
              (error "Edit left the raw literal")))))

;; Repeated edits at the lower bound of the same small restriction. Measured
;; 0.60 extra instructions per row per step. 20 leaves room for indexing.
(scaling-probe kotlin-syntax-narrow-edit :n 22000 :k 4 :slope 20
  :setup (let ((buffer (scaling--kotlin-syntax-at-tail
                        " scaling-kotlin-syntax-narrow" ""
                        "val text = \"value ${name}\"\n" n "val edit = 0\n")))
           (with-current-buffer buffer
             (narrow-to-region (point) (point-max)))
           buffer)
  :body (with-current-buffer it
          (dotimes (_ 4)
            (goto-char (point-min))
            (insert "x")
            (when (nth 8 (syntax-ppss))
              (error "Edit entered a string or comment")))))

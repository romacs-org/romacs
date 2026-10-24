;;; External reader oracle only; VM2 never uses this for input conversion.
(require 'json)
(require 'lisp-mode)
(let* ((input (getenv "VM2_CORPUS_INPUT"))
       (sources (with-temp-buffer
                  (insert-file-contents input)
                  (json-parse-buffer :array-type 'list)))
       (answers
        (mapcar
         (lambda (source)
           (with-temp-buffer
             (set-syntax-table emacs-lisp-mode-syntax-table)
             (insert source)
             (goto-char (point-min))
             (let ((forms 0))
               (condition-case e
                   (progn
                     (while (progn (forward-comment (point-max))
                                   (< (point) (point-max)))
                       (read (current-buffer))
                       (setq forms (1+ forms)))
                     (vector forms t ""))
                 (error (vector forms :false (error-message-string e)))))))
         sources)))
  (princ (json-serialize (vconcat answers))))

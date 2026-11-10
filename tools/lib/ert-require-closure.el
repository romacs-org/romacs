;;; ert-require-closure.el --- Read static dependencies -*- lexical-binding: t; -*-
;;; Derive require edges with the pinned GNU reader, without evaluating them.
(require 'json)
(require 'cl-lib)
(let* ((root (file-name-as-directory (pop command-line-args-left)))
       (mode (pop command-line-args-left))
       (load-path (append (list root)
                          (let (out) (dolist (p (directory-files root t "^[^.]"))
                                       (when (file-directory-p p) (push p out))) out)
                          load-path))
       (todo (or command-line-args-left '("ert"))) seen rows edges ordinal form-line)
  (setq command-line-args-left nil)
  (cl-labels
      ((walk (x context)
         (when (consp x)
           (cond
            ((eq (car x) 'require)
             (let ((f (cadr x)))
               (when (and (consp f) (eq (car f) 'quote) (symbolp (cadr f)))
                 (push `((feature . ,(symbol-name (cadr f))) (context . ,context)
                         (form . ,ordinal) (line . ,form-line)) edges))))
            ((eq (car x) 'quote) nil)
            (t (let ((child (cond
                             ((member (car x) '(defun defsubst defmacro cl-defun cl-defmacro cl-defmethod function lambda)) "deferred")
                             ((eq (car x) 'eval-when-compile) "compile-only")
                             (t context))))
                 (while (consp x) (walk (car x) child) (setq x (cdr x)))))))))
    (while todo
      (let* ((feature (pop todo)) (path (locate-file (concat feature ".el") load-path)) err)
        (unless path (error "Unresolved dependency: %s" feature))
        (unless (member feature seen)
          (push feature seen)
          (setq edges nil ordinal 0 form-line 1)
          (when path
            (with-temp-buffer
              (emacs-lisp-mode)
              (insert-file-contents path)
              (condition-case e
                  (while (progn (forward-comment (point-max)) (not (eobp)))
                    (setq form-line (line-number-at-pos))
                    (let ((x (read (current-buffer))))
                      (setq ordinal (1+ ordinal)) (walk x "load-time")))
                (error (error "Cannot read %s at form %s, line %s: %S"
                              path (1+ ordinal) form-line e)))))
          (push `((feature . ,feature) (file . ,path) (forms . ,ordinal)
                  (edges . ,(vconcat (reverse edges))) (error . ,err)) rows)
          (dolist (edge edges)
            (when (or (equal mode "all") (not (equal (cdr (assoc 'context edge)) "deferred")))
              (push (cdr (assoc 'feature edge)) todo)))))))
  (princ (json-encode (vconcat (nreverse rows)))))

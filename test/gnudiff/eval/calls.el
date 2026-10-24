;;; calls.el --- E1b recorded call scenarios -*- lexical-binding: t; -*-
(require 'bytecomp)
(defun e1b-record (name value)
  (princ (format "(%s . " name)) (prin1 value) (princ ")\n"))
(defun e1b-rec () (e1b-rec))
(e1b-record 'overflow-funcall
  (let ((max-lisp-eval-depth 100))
    (condition-case e (e1b-rec) (error e))))
(byte-compile 'e1b-rec)
(e1b-record 'overflow-bytecode
  (let ((max-lisp-eval-depth 100))
    (condition-case e (e1b-rec) (error e))))
(defun e1b-debug () (backtrace-debug 1 t) 1)
(e1b-record 'debugger-replaces-value
  (let ((debugger (lambda (&rest _args) 42))) (e1b-debug)))
(e1b-record 'argdesc-rest
  ((lambda (a &optional b c &rest r) (list a b c r)) 1 nil nil 4 5))
(defun e1b-bt (&optional a b)
  (let ((f (backtrace-frame 0 'e1b-bt))) (list (eq (cadr f) 'e1b-bt) (length (cddr f)))))
(e1b-record 'backtrace-original-callable-argc (e1b-bt 7 8))
(e1b-record 'debugger-exit-throw
  (list (let ((debugger (lambda (&rest _args) (throw 'done 9))))
          (catch 'done (e1b-debug)))
        (funcall (lambda () 7))))
(dolist (fun (list nil 'e1b-void 'e1b-alias))
  (fset 'e1b-alias 'e1b-void)
  (e1b-record (intern (format "void-%s" fun))
    (condition-case e (funcall fun) (error e))))
(let ((lambda-fun '(lambda (a &optional b c) a)))
  (dolist (case (list (cons 'lambda lambda-fun)
                     (cons 'bytecode (byte-compile lambda-fun))
                     (cons 'subr (symbol-function 'cons))))
    (dolist (args '(() (1 2 3 4)))
      (e1b-record (intern (format "arity-%s-%s" (car case) (length args)))
        (condition-case e (apply (cdr case) args) (error e))))))
(e1b-record 'subr-many (funcall #'list 1 2 3))
(e1b-record 'subr-unevalled
  (condition-case e (funcall (symbol-function 'if)) (error e)))

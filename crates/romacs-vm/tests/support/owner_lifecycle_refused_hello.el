;;; owner_lifecycle_refused_hello.el --- Observe the actual refusal and retry paths -*- lexical-binding: t; -*-

(setq isolate-zygotes t isolate-pool-size 4
      zygotes 0 resubmits 0 callbacks 0 refusal-seen nil
      original-make (symbol-function 'romacs--isolate-zygote-make)
      original-resubmit (symbol-function 'romacs--isolate-resubmit))
(fset 'romacs--isolate-zygote-make
      (lambda (require)
        (setq zygotes (1+ zygotes))
        (funcall original-make require)))
(fset 'romacs--isolate-resubmit
      (lambda (future)
        (setq resubmits (1+ resubmits))
        (funcall original-resubmit future)))
(setq isolate-error-functions
      (list (lambda (isolate condition _data)
              (when (and (eq isolate first-zygote) (eq condition 'isolate-refused))
                (setq refusal-seen t)))))
(setq future (isolate-spawn '+ 1 2 :require '(subr-x))
      first-zygote (aref (gethash '(subr-x) romacs--isolate-zygotes) 1))
(isolate-on-done future (lambda (_future) (setq callbacks (1+ callbacks))))

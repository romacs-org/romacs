;;; isolate-zygote-check.el --- scaling probe  -*- lexical-binding: t -*-

;; A zygote's validity check before each dispatch (lisp/romacs/vm/isolate.el,
;; worker-vms design §2.4): the inherited variables compared, then the
;; files it loaded and the `load-path' directories stamped.  Linear in
;; each, and in nothing else.
(scaling-probe isolate-zygote-check :n 200
  :setup (let* ((environment (scaling--names "SCALING_VARIABLE=value" (* 10 n)))
                (directories (scaling--names "/nonexistent/scaling-lisp" n))
                (files (scaling--names "/nonexistent/scaling-library.el" n))
                (inputs (let ((process-environment environment) (load-path directories))
                          (romacs--isolate-inputs)))
                (expanded (let ((load-path directories)) (romacs--isolate-load-directories))))
           (list environment directories
                 (record 'isolate-zygote nil nil 'ready nil inputs files (romacs--isolate-stamp files)
                         expanded (romacs--isolate-stamp expanded) 0 nil nil isolate-job-memory-headroom nil)))
  :body (let ((process-environment (nth 0 it)) (load-path (nth 1 it)))
          (dotimes (_ 10)
            (or (romacs--isolate-zygote-current-p (nth 2 it)) (error "Zygote check failed")))))

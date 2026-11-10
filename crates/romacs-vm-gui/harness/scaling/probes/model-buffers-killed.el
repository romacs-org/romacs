;;; model-buffers-killed.el --- scaling probe  -*- lexical-binding: t -*-

;; N buffers each show a native model; each one's kill hook destroys its
;; own model without looking at the others' (the buffers stay, so the
;; probe measures the models, not `kill-buffer').  Batch has no
;; renderer, so the primitives are stand-ins that accept every request.
(scaling-probe model-buffers-killed :n 400
  :setup (progn (require 'romacs-model)
                (let ((next 0))
                  (fset 'romacs-model--available-p (lambda () t))
                  (fset 'romacs-model--create (lambda (&rest _) (setq next (1+ next))))
                  (fset 'romacs-surface-destroy #'ignore))
                (cl-loop for i below n
                         collect (with-current-buffer (generate-new-buffer (format " scaling-model-%d" i))
                                   (romacs-model-insert
                                    (romacs-model-create :meshes '(("a" "/m/a.obj")) :callback #'ignore))
                                   (current-buffer))))
  :body (dolist (buffer it)
          (with-current-buffer buffer (romacs-model--kill-buffer))))

;;; model-events.el --- scaling probe  -*- lexical-binding: t -*-

;; Native model events find their model by its surface id: each of N live
;; models' events costs the same however many models live.  Batch has no
;; renderer, so the primitives are stand-ins that accept every request.
(scaling-probe model-events :n 500
  :setup (progn (require 'romacs-model)
                (let ((next 0))
                  (fset 'romacs-model--available-p (lambda () t))
                  (fset 'romacs-model--create (lambda (&rest _) (setq next (1+ next))))
                  (fset 'romacs-surface-destroy #'ignore))
                (cl-loop repeat n collect (romacs-model-create :meshes '(("a" "/m/a.obj")) :callback #'ignore)))
  :body (dolist (model it)
          (romacs-model--handle-event
           (list 'romacs-model (romacs-model--id model) 'readiness (list :current t :rev 1 :req 0)))))

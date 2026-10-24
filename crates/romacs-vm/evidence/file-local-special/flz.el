;;; flz.el --- -*- lexical-binding: t -*-
(defun flz-get () (if (boundp 'fla-v) fla-v 'unbound))
(setq flz-1 (let ((fla-v 8)) (flz-get)))
(setq flz-2 (let ((fla-v 8)) (flz-get)))

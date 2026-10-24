;;; flb.el --- -*- lexical-binding: t -*-
(defun flb-late () (let ((fla-v 6)) (fla-get)))
(setq flb-a fla-same)
(setq fla-same (let ((fla-v 7)) (fla-get)))
(setq flb-r (list (flb-late) flb-a fla-same))

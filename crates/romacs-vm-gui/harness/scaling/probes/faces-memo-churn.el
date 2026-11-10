;;; faces-memo-churn.el --- scaling probe  -*- lexical-binding: t -*-

;; Recalculating N faces whose memos exist: a first pass records them, then
;; the counted pass hits. Fresh faces each size insert into a populated
;; table, and a frame coming and going leaves nothing behind.
(scaling-probe faces-memo-churn :gui t :n 200 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-memo" n)))
               (frame (selected-frame)))
           (dolist (face faces)
             (custom-declare-face face '((((background dark)) :foreground "#aabbcc") (t :foreground "#112233")) "A scaling face." :group 'scaling))
           (dolist (face faces) (face-spec-recalc face frame))
           (cons (switch-to-buffer (get-buffer-create "scaling-memo")) faces))
  :body (let ((frame (selected-frame)) (churn (intern (format "scaling-memo-new-%s" (float-time)))))
          (custom-declare-face churn '((t :foreground "#102030")) "A scaling face." :group 'scaling)
          (dolist (face (cdr it)) (face-spec-recalc face frame))
          (with-current-buffer (car it) (insert "x"))))

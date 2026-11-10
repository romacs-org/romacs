;;; faces-memo-frames.el --- scaling probe  -*- lexical-binding: t -*-

;; N faces memoised on an owned frame beside others, with frames going and
;; a collection between passes: a stale table must not be scanned per face.
(scaling-probe faces-memo-frames :gui t :n 100 :steps 1
  :setup (let* ((faces (mapcar #'intern (scaling--names "scaling-memo-frames" n)))
                (frames (progn
                          (unless (fboundp 'pgtk-register-dnd-targets) (defalias 'pgtk-register-dnd-targets #'ignore))
                          (mapcar (lambda (_) (make-frame `((parent-frame . ,(selected-frame)) (visibility . nil) (minibuffer . nil))))
                                  (number-sequence 1 8)))))
           (dolist (face faces)
             (custom-declare-face face '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling))
           (dolist (frame frames)
             (set-face-attribute (car faces) frame :weight 'bold)
             (dolist (face faces) (face-spec-recalc face frame)))
           (list (switch-to-buffer (get-buffer-create "scaling-memo-frames")) frames faces))
  :body (let ((extra (make-frame `((parent-frame . ,(selected-frame)) (visibility . nil) (minibuffer . nil)))))
          (set-face-attribute (car (nth 2 it)) extra :weight 'bold)
          (delete-frame extra)
          (garbage-collect)
          (dolist (frame (nth 1 it))
            (dolist (face (nth 2 it)) (face-spec-recalc face frame)))
          (with-current-buffer (car it) (insert "x"))))

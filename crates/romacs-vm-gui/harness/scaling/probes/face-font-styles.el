;;; face-font-styles.el --- scaling probe  -*- lexical-binding: t -*-

;; `face-font' for N faces each asking a distinct font (size, weight,
;; slant), then the same N again: the display's per-request font caches
;; grow by one entry per request and answer the repeats from the cache.
(scaling-probe face-font-styles :gui t :n 20 :steps 1
  :setup (let (faces)
           (dotimes (i n)
             (let ((face (intern (format "scaling-font-%d" i))))
               (make-face face)
               (set-face-attribute face nil :height (+ 100 (* 2 (/ i 4))) :weight (if (cl-oddp i) 'bold 'normal)
                                   :slant (if (zerop (% (/ i 2) 2)) 'normal 'italic))
               (push face faces)))
           (cons (switch-to-buffer (get-buffer-create "scaling-font")) faces))
  ;; The step paints (a character typed in the shown buffer), as a GUI
  ;; probe's must.
  :body (progn (dotimes (_ 2) (dolist (face (cdr it)) (face-font face)))
               (with-current-buffer (car it) (insert "x"))))

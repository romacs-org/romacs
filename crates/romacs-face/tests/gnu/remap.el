;;; remap.el --- GNU oracle for face remapping and anonymous faces  -*- lexical-binding: t -*-
;;
;; emacs -Q --batch -l remap.el --eval '(s86-remap-dump OUT)'
;; Each case defines faces with display-independent `((t ...))' specs, binds
;; `face-remapping-alist', and records GNU's C merge of a face reference via
;; `face-attributes-as-vector' (merge_face_ref with no window, from all
;; attributes unspecified; the default face is not merged in).

(defvar s86-remap-cases
  '((text-scale
     ((s86-a ((t :inherit s86-b :height 1.2)))
      (s86-b ((t :foreground "red" :height 1.5))))
     ((default :height 2.0 default) (s86-b (:foreground "blue") s86-b))
     s86-a)
    (remapped-parent-reaches-child
     ((s86-p ((t :weight bold :foreground "black")))
      (s86-child ((t :inherit s86-p :slant italic))))
     ((s86-p :foreground "green" s86-p))
     s86-child)
    (remap-list-precedence
     ((s86-x ((t :foreground "x")))
      (s86-p ((t :foreground "p" :background "pb")))
      (s86-q ((t :foreground "q" :background "qb" :underline t))))
     ((s86-x s86-p s86-q))
     s86-x)
    (remap-to-other-face-only
     ((s86-x ((t :foreground "x" :weight bold)))
      (s86-q ((t :background "qb"))))
     ((s86-x . s86-q))
     s86-x)
    (anonymous-inherit-list
     ((s86-p ((t :foreground "p")))
      (s86-q ((t :foreground "q" :slant italic))))
     nil
     (:height 1.25 :inherit (s86-p s86-q)))
    (anonymous-list-of-refs
     ((s86-p ((t :foreground "p")))
      (s86-q ((t :background "q"))))
     nil
     ((:foreground "anon") s86-p s86-q (foreground-color . "old")))
    (filtered-without-window
     ((s86-p ((t :foreground "p"))))
     ((s86-p (:filtered (:window s86-param t) (:foreground "filtered")) s86-p))
     s86-p)
    ;; Deliberate divergences, recorded in GNU-DIVERGENCES.md:
    (anonymous-attribute-before-inherit
     ((s86-blue ((t :foreground "blue"))))
     nil
     (:foreground "red" :inherit s86-blue))
    (remap-loop-through-inheritance
     ((s86-a ((t :inherit s86-b :weight bold)))
      (s86-b ((t :foreground "red"))))
     ((s86-b s86-a))
     s86-a)))

(defun s86--vector-plist (v)
  (let ((keys '(:family :foundry :width :height :weight :slant :underline
                :inverse-video :foreground :background :stipple :overline
                :strike-through :box))
        out)
    (dotimes (i (length keys))
      (let ((val (aref v (1+ i))))
        (unless (eq val 'unspecified)
          (setq out (append out (list (nth i keys) val))))))
    (let ((dfg (aref v 18)) (ext (aref v 19)))
      (unless (eq dfg 'unspecified) (setq out (append out (list :distant-foreground dfg))))
      (unless (eq ext 'unspecified) (setq out (append out (list :extend ext)))))
    out))

;; Height arithmetic: relative heights over an absolute default, through
;; inheritance, as `face-attribute ... 'default' computes them (truncation).
(defvar s86-height-faces
  '((s86-h1 ((t :height 1.2)))
    (s86-h2 ((t :inherit s86-h1 :height 1.1)))
    (s86-h3 ((t :inherit (s86-h2 s86-h1) :height 0.9)))
    (s86-h4 ((t :inherit s86-h3)))
    (s86-h5 ((t :inherit s86-h1 :height 140)))
    (s86-h6 ((t :inherit s86-h5 :height 1.15)))))

(defun s86-height-dump ()
  (dolist (f s86-height-faces)
    (face-spec-set (car f) (cadr f) 'face-defface-spec))
  (prog1
      (mapcar (lambda (dh)
                (set-face-attribute 'default nil :height dh)
                (cons dh (mapcar (lambda (f) (face-attribute (car f) :height nil 'default))
                                 s86-height-faces)))
              '(100 113 97 1))
    (set-face-attribute 'default nil :height 1)))

(defun s86-remap-dump (out)
  (let ((print-length nil) (print-level nil) (print-escape-newlines t) results)
    (dolist (c s86-remap-cases)
      (pcase-let ((`(,name ,faces ,remap ,ref) c))
        (dolist (f faces)
          (face-spec-set (car f) (cadr f) 'face-defface-spec))
        (let ((face-remapping-alist remap))
          (push (list name faces remap ref
                      (s86--vector-plist (face-attributes-as-vector ref)))
                results))
        (dolist (f faces)
          (face-spec-set (car f) nil 'face-defface-spec))))
    (setq results
          (cons (list 'dark-p
                      (mapcar (lambda (c)
                                (cons c (and (color-dark-p
                                              (mapcar (lambda (v) (/ v 65535.0))
                                                      (color-values-from-color-spec c)))
                                             t)))
                              (append
                               (mapcar (lambda (i) (format "#%02x%02x%02x" i i i))
                                       (number-sequence 0 255 4))
                               '("#9a9a9a" "#9b9b9b" "#9c9c9c" "#ff0000" "#00ff00" "#0000ff"
                                 "#808000" "#008080" "#a0a000" "#00a0a0" "#fff" "#000"
                                 "#123" "#cccccc" "#999999999999" "#8000ff"))))
                results))
    (setq results (cons (list 'heights s86-height-faces (s86-height-dump)) results))
    (with-temp-file out
      (insert "(")
      (dolist (r (nreverse results))
        (insert "\n ") (prin1 r (current-buffer)))
      (insert ")\n"))))

;;; fontify-hidden-text.el --- scaling probe  -*- lexical-binding: t -*-

;; The first paint of a folded buffer: forty shown lines, a window of them
;; as a folded outline shows, each over a hidden stretch of N/40 characters
;; of Emacs Lisp, longer than a `jit-lock' chunk.  Redisplay asks for the
;; text it shows and the first character of each hidden stretch, as GNU's
;; display iterator does, never the rest of the stretch: the paint costs
;; what is shown, not what is folded away.  The fontification worker is
;; off, so the main evaluator fontifies whatever is asked, and the
;; highlighting is by keywords only: a syntactic pass parses from the
;; buffer's start to each chunk it fontifies, hidden text or not, in GNU
;; as here.
(defvar romacs-fontify-isolate)

(scaling-probe fontify-hidden-text :gui t :n 80000 :k 8 :steps 4 :budget 0.1
  :setup (progn
           (setq romacs-fontify-isolate nil)
           (let (buffers)
             (dotimes (b 4)
               (with-current-buffer (get-buffer-create (format "scaling-folded-%d.el" b))
                 ;; Its name shown, so that each step's paint differs.
                 (insert (format ";;; %s\n" (buffer-name)))
                 (dotimes (s 40)
                   (insert (format ";;; Section %d\n" s))
                   (let ((start (point)) (i 0))
                     (while (< (- (point) start) (/ n 40))
                       (prin1 (scaling--defun-form (+ (* s n) i)) (current-buffer))
                       (insert "\n")
                       (setq i (1+ i)))
                     (overlay-put (make-overlay start (point)) 'invisible t)))
                 (setq-local font-lock-defaults '((("(\\(defun\\|let\\|setq\\|if\\|list\\)\\_>" 1 font-lock-keyword-face)) t))
                 (font-lock-mode 1)
                 (goto-char (point-min))
                 (push (current-buffer) buffers)))
             (nreverse buffers)))
  :body (switch-to-buffer (nth i it)))

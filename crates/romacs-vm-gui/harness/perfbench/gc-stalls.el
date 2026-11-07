;;; gc-stalls.el --- The stalls a running program sees  -*- lexical-binding: t -*-
;; Gaps between loop iterations while it allocates, at one live heap per process,
;; in any Emacs, byte-compiled for GNU:
;;   TAG=name SCEN=small|mid|big|real [FIX=gnu-tree] [PHASES=1] emacs -Q --batch -l gc-stalls.elc
;; SCEN is the live heap: 10k, 500k or 2M (cons + cons + string) items, or `real':
;; common libraries and three large files of FIX, whose setup it reports, so the
;; two sides can be seen to do the same work. A stall is a gap over 2 ms; it need
;; not be a collection. PHASES prints where Romacs's collections spent their time.
;; Reading memory allocates, so nothing here does while the clock runs: `anon' is
;; the end of the run, and peak-anon.py samples a run's peak from outside it.
;; `explicit' is the median of 21 collections. Pin both sides to the same physical
;; cores of one chiplet (taskset), off SMT siblings, for a spread under 3%.
(require 'cl-lib)
(defvar gap-log nil)
(defun gap-note () (when (fboundp 'romacs--gc-phases) (push (funcall 'romacs--gc-phases) gap-log)))
(defun gap-phases (title)
  (when gap-log
    (princ (format "  %s: %d collections\n" title (length gap-log)))
    (dolist (n (mapcar #'car (cl-remove-if-not (lambda (x) (floatp (cdr x))) (car gap-log))))
      (let* ((v (sort (mapcar (lambda (row) (* 1000 (cdr (assq n row)))) gap-log) #'<)) (k (length v)))
        (when (> (car (last v)) 0.05)
          (princ (format "    %-8s median %8.2f  max %8.2f ms\n" n (nth (/ k 2) v) (car (last v)))))))
    (princ (format "    last marked %S of %S\n" (cdr (assq 'marked (car gap-log))) (cdr (assq 'entries (car gap-log)))))
    (setq gap-log nil)))
(defun gap-live (n) (let (l) (dotimes (i n) (push (cons i (make-string 16 ?x)) l)) l))
(defun gap-explicit (k)
  (let (times)
    (dotimes (_ k)
      (let ((t0 (float-time))) (garbage-collect) (push (- (float-time) t0) times)))
    (setq times (sort times #'<))
    (list (* 1000 (nth (/ k 2) times)) (* 1000 (car times)) (* 1000 (car (last times))))))
(defun gap-churn (n)
  (let ((last (float-time)) (gaps nil) (t0 (float-time)))
    (dotimes (i n)
      (let ((x (make-vector 8 i)) (s (format "%d" i)) (c (list i i i))) (ignore x s c))
      (when (= 0 (% i 16))
        (let ((now (float-time)))
          (when (> (- now last) 0.002) (push (- now last) gaps))
          (setq last now))))
    (let* ((p (sort gaps #'<)) (k (length p)))
      (list (- (float-time) t0) k (* 1000 (if p (nth (/ k 2) p) 0)) (* 1000 (if p (car (last p)) 0)) (apply #'+ p)))))
(defun gap-status (field)
  "FIELD of /proc/self/status, in MB."
  (with-temp-buffer
    (insert-file-contents "/proc/self/status")
    (goto-char (point-min))
    (re-search-forward (concat field ":[ \t]*\\([0-9]+\\)"))
    (/ (string-to-number (match-string 1)) 1024)))
(defun gap-real (tree)
  (dolist (library '(org dired eshell gnus calc cc-mode)) (require library))
  (dolist (file '("src/xdisp.c" "lisp/subr.el" "etc/ORG-NEWS"))
    (find-file (expand-file-name file tree))
    (let ((failed (condition-case e (progn (font-lock-ensure) nil) (error e))) (runs 0) (at (point-min)))
      (while (setq at (next-single-property-change at 'face)) (setq runs (1+ runs)))
      (princ (format "  setup %s: %s, %d chars, %d face runs, error %S\n" file major-mode (buffer-size) runs failed)))))
(let* ((scen (getenv "SCEN"))
       (keep (pcase scen
               ("small" (gap-live 10000))
               ("mid" (gap-live 500000))
               ("big" (gap-live 2000000))
               ("real" (gap-real (getenv "FIX")) nil)
               (_ (error "SCEN is one of small, mid, big, real"))))
       (elapsed gc-elapsed) (done gcs-done)
       (first (let ((t0 (float-time))) (garbage-collect) (* 1000 (- (float-time) t0)))))
  (when (getenv "PHASES") (add-hook 'post-gc-hook #'gap-note))
  (let* ((explicit (prog1 (gap-explicit 21) (gap-phases "explicit")))
         (churn (prog1 (gap-churn 6000000) (gap-phases "churn")))
         (after (gap-explicit 21)))
    (princ (format "%s %s first %.2f after-churn %.2f explicit %.2f (%.2f-%.2f) ms | wall %.2f s stalls %d median %.2f ms max %.2f ms total %.2f s | gcs %d gc-elapsed %.3f anon %d hwm %d MB\n"
                   (getenv "TAG") scen first (car after) (nth 0 explicit) (nth 1 explicit) (nth 2 explicit)
                   (nth 0 churn) (nth 1 churn) (nth 2 churn) (nth 3 churn) (nth 4 churn)
                   (- gcs-done done) (- gc-elapsed elapsed) (gap-status "RssAnon") (gap-status "VmHWM")))
    (ignore keep)))

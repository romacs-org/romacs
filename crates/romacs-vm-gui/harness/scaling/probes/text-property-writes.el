;;; text-property-writes.el --- scaling probe  -*- lexical-binding: t -*-

;; N property writes into a buffer of N lines that already holds N runs of
;; `face' and N/20 unrelated properties, as a mode refontifying text other
;; packages have marked, with undo off as `jit-lock' writes: a write that
;; changes a run, one that changes nothing, and one that adds a key no
;; other text has, each line its own; then each of those N keys is
;; removed, in the order they came.  A write costs the runs it touches and
;; a key's lookup, whatever the runs and keys around it: adding and
;; dropping a key costs the registry of keys its logarithm.
(scaling-probe text-property-writes :n 4000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-property-writes" n)
           (let ((others (max 1 (/ n 20))) (keys (make-vector n nil)))
             (dotimes (i n)
               (put-text-property (point) (+ (point) 4) 'face (if (cl-evenp i) 'bold 'italic))
               (put-text-property (+ (point) 2) (+ (point) 9) (intern (format "scaling-mark-%d" (% i others))) i)
               (aset keys i (intern (format "scaling-own-%d" i)))
               (forward-line 1))
             (cons (current-buffer) keys)))
  :body (with-current-buffer (car it)
          (let ((buffer-undo-list t) (inhibit-modification-hooks t) (keys (cdr it)))
            (goto-char (point-min))
            (dotimes (i n)
              (put-text-property (point) (+ (point) 6) 'face 'underline)
              (put-text-property (point) (+ (point) 4) 'face 'underline)
              (put-text-property (+ (point) 10) (+ (point) 12) (aref keys i) t)
              (forward-line 1))
            (goto-char (point-min))
            (dotimes (i n)
              (remove-text-properties (+ (point) 10) (+ (point) 12) (list (aref keys i) nil))
              (forward-line 1)))))

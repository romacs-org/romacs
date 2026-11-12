;;; text-property-writes.el --- scaling probe  -*- lexical-binding: t -*-

;; N property writes into a buffer of N lines that already holds N runs of
;; `face' and 40 unrelated properties, each on every fortieth fifth line,
;; as a mode refontifying text other packages have marked: a write that
;; changes a run, one that changes nothing, one of two properties, a
;; removal and a whole replacement.  Each costs the runs it touches,
;; whatever the number of runs, with undo off (as `jit-lock' writes) and
;; on.  The per-key store makes a whole-plist step (`set-text-properties',
;; undo's interval walk) cost the text's keys, so their number is held.
(scaling-probe text-property-writes :n 4000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-property-writes" n)
           (dotimes (i n)
             (put-text-property (point) (+ (point) 4) 'face (if (cl-evenp i) 'bold 'italic))
             (when (zerop (% i 5))
               (put-text-property (+ (point) 2) (+ (point) 9) (intern (format "scaling-mark-%d" (% i 40))) i))
             (forward-line 1))
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (dolist (undo '(t nil))
            (let ((buffer-undo-list undo) (inhibit-modification-hooks t))
              (goto-char (point-min))
              (dotimes (i n)
                (pcase (% i 5)
                  (0 (put-text-property (point) (+ (point) 6) 'face 'underline))
                  (1 (put-text-property (point) (+ (point) 4) 'face (get-text-property (point) 'face)))
                  (2 (add-text-properties (+ (point) 1) (+ (point) 8) '(face shadow scaling-written t)))
                  (3 (remove-text-properties (point) (+ (point) 12) '(face nil scaling-absent nil)))
                  (4 (set-text-properties (+ (point) 3) (+ (point) 7) '(face bold))))
                (forward-line 1))))))

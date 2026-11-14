;;; text-property-plists.el --- scaling probe  -*- lexical-binding: t -*-

;; The whole-plist steps of N property writes into a buffer whose N lines
;; carry N/20 unrelated properties: `set-text-properties', which drops every
;; key of its range, and a write a buffer keeping undo records interval by
;; interval, whose boundaries are every key's.  Each key keeps its own runs,
;; so each step asks every key the text has: N/20 keys make them quadratic.
(scaling-probe text-property-plists :n 4000
  :known "romacs#113: whole-plist steps ask every key's runs; interned plist runs would bound them by the keys in range"
  :setup (with-current-buffer (scaling--lines-buffer " scaling-property-plists" n)
           (let ((others (max 1 (/ n 20))))
             (dotimes (i n)
               (put-text-property (+ (point) 2) (+ (point) 9) (intern (format "scaling-plist-%d" (% i others))) i)
               (forward-line 1)))
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (let ((inhibit-modification-hooks t))
            (goto-char (point-min))
            (dotimes (_ n)
              (set-text-properties (+ (point) 3) (+ (point) 7) '(face bold))
              (put-text-property (point) (+ (point) 12) 'face 'italic)
              (forward-line 1)))))

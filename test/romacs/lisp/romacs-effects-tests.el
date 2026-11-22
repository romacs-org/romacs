;;; romacs-effects-tests.el --- A window-animation slot's options  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; lisp/ui/romacs-effects.el's slots, as `romacs-effects--defslot' makes
;; them from `romacs-effects--slot-options'.  The renderer side, with a
;; window system: crates/romacs-vm-gui/tests/effects.rs.

(require 'ert)
(require 'romacs-effects)

(defconst romacs-effects-tests--slots
  '((window-open (enabled t) (kind 'easing) (duration 0.15) (easing 'ease-out-expo) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800))
    (window-close (enabled nil) (kind 'easing) (duration 0.15) (easing 'ease-out-quad) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800))
    (window-resize (enabled t) (kind 'spring) (duration 0.15) (easing 'ease-out-quad) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800))
    (window-movement (enabled t) (kind 'spring) (duration 0.15) (easing 'ease-out-quad) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800))
    (child-frame-open (enabled t) (kind 'easing) (duration 0.14) (easing 'ease-out-cubic) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800) (slide-pixels 8.0) (scale-from 1.0))
    (child-frame-close (enabled t) (kind 'easing) (duration 0.09) (easing 'ease-out-cubic) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800) (slide-pixels 0.0) (scale-from 1.0))
    (child-frame-movement (enabled nil) (kind 'spring) (duration 0.15) (easing 'ease-out-quad) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800) (slide-pixels 0.0) (scale-from 1.0))
    (child-frame-resize (enabled t) (kind 'easing) (duration 0.08) (easing 'ease-out-cubic) (bezier-x1 0.0) (bezier-y1 0.0) (bezier-x2 1.0) (bezier-y2 1.0) (damping-ratio 1.0) (stiffness 800) (slide-pixels 0.0) (scale-from 1.0)))
  "Each slot's options, in order, and their defaults: what the forty
written-out options of the slots were before the table.")

(defun romacs-effects-tests--expansion (call)
  "CALL's options as (PROPERTY DEFAULT), its slot's name dropped."
  (let ((prefix (format "romacs-%s-" (nth 1 call))))
    (mapcar (lambda (option) (list (intern (string-remove-prefix prefix (symbol-name (nth 1 option))))
                                   (nth 2 option)))
            (cdr (macroexpand-1 call)))))

(ert-deftest romacs-effects-each-slot-keeps-its-options-and-defaults ()
  (dolist (slot romacs-effects-tests--slots)
    (let ((name (car slot)))
      (should (equal (mapcar (lambda (option) (symbol-value (intern (format "romacs-%s-%s" name (car option)))))
                             (cdr slot))
                     (mapcar (lambda (option) (eval (nth 1 option) t)) (cdr slot))))
      (dolist (extra '(slide-pixels scale-from))
        (unless (assq extra (cdr slot))
          (should-not (boundp (intern (format "romacs-%s-%s" name extra)))))))))

(ert-deftest romacs-effects-an-option-takes-its-override-and-its-condition ()
  (let ((options (romacs-effects-tests--expansion
                  '(romacs-effects--defslot probe "a probe moving" "it jumps" :kind 'easing :slide 3.0))))
    (should (equal (assq 'kind options) '(kind 'easing)))
    (should (equal (assq 'duration options) '(duration 0.15)))
    (should (equal (assq 'slide-pixels options) '(slide-pixels 3.0)))
    (should-not (assq 'scale-from options))
    (should (equal (mapcar #'car options)
                   '(enabled kind duration easing bezier-x1 bezier-y1 bezier-x2 bezier-y2 damping-ratio stiffness
                             slide-pixels))))
  (should (equal (romacs-effects--slot-option '(:property scale-from :key :scale :type number :only-with :scale)
                                              '("s" "o" "probe" "p") 'g nil)
                 nil)))

(ert-deftest romacs-effects-an-option-s-docstring-is-written-from-its-slot ()
  (let ((option (cadr (macroexpand-1 '(romacs-effects--defslot probe "a probe moving" "it jumps"
                                                               :group 'romacs-child-frame :master 'probe-off)))))
    (should (eq (nth 1 option) 'romacs-probe-enabled))
    (should (equal (nth 3 option) "Whether a probe moving is animated.
When nil, it jumps happens immediately.

Turning every window-animation slot off also stops the compositor from keeping
the previous frame in a texture, which it must do to animate from it — see
`probe-off' for the cheaper way to say that."))
    (should (equal (last option 2) '(:group 'romacs-child-frame))))
  (should (equal (get 'romacs-window-movement-bezier-y2 'variable-documentation)
                 "The second control point's value of the Bezier curve for a window that moves without changing size.
Used only when `romacs-window-movement-easing' is `cubic-bezier'.  These are the same four
numbers a niri configuration writes as `cubic-bezier(x1, y1, x2, y2)'.

Time is clamped to 0.0-1.0 so the curve stays solvable; value is deliberately
not, because overshooting past 1.0 is the point of a curve like
`cubic-bezier(0.34, 1.56, 0.64, 1.0)'.")))

;;; romacs-effects-tests.el ends here

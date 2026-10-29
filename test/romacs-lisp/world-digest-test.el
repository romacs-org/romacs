;;; world-digest-test.el --- The world differential's digest  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(load (expand-file-name "../../gnu-tests/tools/world-digest.el"
                        (file-name-directory (or load-file-name buffer-file-name)))
      nil t)

(defun world-digest-test--timer (high low usecs psecs function args)
  "A timer as timer.el makes it, due at HIGH LOW USECS PSECS."
  (vector nil high low usecs nil function args nil psecs nil))

(romacs-deftest world-digest-compares-a-timer-but-for-its-due-time ()
  (let ((now (world-digest-test--timer 1 2 3 4 'undo-auto--boundary-timer nil))
        (later (world-digest-test--timer 9 8 7 6 'undo-auto--boundary-timer nil))
        (other (world-digest-test--timer 1 2 3 4 'ignore nil))
        (text (lambda (value) (world-digest--text (world-digest--timeless value)))))
    ;; Due at another clock reading: the same state.
    (romacs-should (equal (funcall text now) (funcall text later)))
    (romacs-should (equal (funcall text (list now 'x)) (funcall text (list later 'x))))
    ;; Another callback, or a timer gone: another state.
    (romacs-should (not (equal (funcall text now) (funcall text other))))
    (romacs-should (not (equal (funcall text (list now)) (funcall text (list now later)))))
    (romacs-should (not (equal (funcall text (list now)) (funcall text nil))))
    ;; The timer itself is left alone.
    (romacs-should (eq (aref now 1) 1))
    ;; A vector of another shape is data, compared whole.
    (romacs-should (not (equal (funcall text [nil 1 2 3 nil f nil nil 4]) (funcall text [nil 9 2 3 nil f nil nil 4]))))))

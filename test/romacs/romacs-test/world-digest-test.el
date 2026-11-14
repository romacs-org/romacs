;;; world-digest-test.el --- The world differential's digest  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(load (expand-file-name "../../../test/gnu/tools/world-digest.el"
                        (file-name-directory (or load-file-name buffer-file-name)))
      nil t)

(defun world-digest-test--timer (high low usecs psecs function args &optional idle)
  "A timer as timer.el makes it, due at HIGH LOW USECS PSECS; with IDLE,
an idle timer, whose time fields are that delay."
  (vector nil high low usecs nil function args idle psecs nil))

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

(romacs-deftest world-digest-compares-an-idle-timer-with-its-delay ()
  ;; timer-set-idle-time: an idle timer of 1 s and one of 60 s differ
  ;; in their time fields, which hold the delay.
  (let ((second (world-digest-test--timer 0 1 0 0 'jit-lock-stealth-fontify nil t))
        (minute (world-digest-test--timer 0 60 0 0 'jit-lock-stealth-fontify nil t))
        (text (lambda (value) (world-digest--text (world-digest--timeless value)))))
    (romacs-should (not (equal (funcall text second) (funcall text minute))))
    (romacs-should (not (equal (funcall text (list second)) (funcall text (list minute)))))
    (romacs-should (equal (funcall text (list second))
                          (funcall text (list (world-digest-test--timer 0 1 0 0 'jit-lock-stealth-fontify nil t)))))))

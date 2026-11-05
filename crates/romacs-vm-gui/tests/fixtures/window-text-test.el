;;; window-text-test.el --- window-text-width and -height  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; ERT tests tests/window_text.rs runs in Romacs's batch session and in
;; pinned GNU's: window.c's window-text-width and window-text-height on
;; batch's terminal frame, split.

(require 'ert)

(ert-deftest window-text-sizes-are-the-text-area ()
  (delete-other-windows)
  (should (= (window-text-width) (window-body-width)))
  (should (= (window-text-height) (window-body-height)))
  (should (= (window-text-width nil t) (window-body-width nil t)))
  (should (= (window-text-height nil t) (window-body-height nil t))))

(ert-deftest window-text-width-keeps-the-border-column ()
  ;; A terminal frame's window left of another loses a column to the
  ;; border in its body width (window_body_width), not its text width
  ;; (window_box_width); the rightmost window loses none.
  (delete-other-windows)
  (let ((left (selected-window)) (right (split-window-right)))
    (split-window-below)
    (unwind-protect
        (progn
          (should (= (- (window-text-width left) (window-body-width left)) 1))
          (should (= (window-text-width right) (window-body-width right)))
          (should (= (window-text-height left) (window-body-height left))))
      (delete-other-windows))))

(ert-deftest window-text-width-signals-for-a-dead-window ()
  (should-error (window-text-width 'nope) :type 'wrong-type-argument))

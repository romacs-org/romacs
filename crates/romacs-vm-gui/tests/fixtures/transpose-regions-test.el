;;; transpose-regions-test.el --- transpose-regions in a full session  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; ERT tests tests/transpose_regions.rs runs in Romacs's batch session and
;; in pinned GNU's: editfns.c's transpose-regions with undo, an indirect
;; buffer sharing the text, its interactive form and a window's point.

(require 'ert)

(ert-deftest transpose-regions-undoes-as-one-change ()
  (dolist (span '((1 3 7 11) (1 4 7 10) (1 4 4 7) (2 3 5 9)))
    (with-temp-buffer
      (buffer-enable-undo)
      (insert (propertize "abc" 'face 'bold) "XYZ" (propertize "defg" 'p 1))
      (undo-boundary)
      (let ((before (buffer-string)))
        (apply #'transpose-regions span)
        (should-not (equal-including-properties (buffer-string) before))
        (primitive-undo 1 buffer-undo-list)
        (should (equal-including-properties (buffer-string) before))))))

(ert-deftest transpose-regions-in-a-shared-text ()
  (let* ((base (generate-new-buffer "transpose-base"))
         (other (progn (with-current-buffer base (insert "abcXYZdefg"))
                       (make-indirect-buffer base "transpose-indirect"))))
    (unwind-protect
        (progn
          (with-current-buffer other (goto-char 2))
          (with-current-buffer base (transpose-regions 1 3 7 11))
          ;; Its point and narrowing are markers of the shared text
          ;; (pt_marker, begv_marker, zv_marker): they move as markers do.
          (with-current-buffer other
            (should (equal (list (point) (point-min) (point-max)) '(10 9 11)))
            (widen)
            (should (equal (buffer-string) "defgcXYZab"))))
      (kill-buffer other)
      (kill-buffer base))))

(ert-deftest transpose-regions-moves-a-window-point ()
  (with-temp-buffer
    (insert "abcXYZdefg")
    (let ((window (selected-window)) (old (window-buffer)))
      (set-window-buffer window (current-buffer))
      (unwind-protect
          (progn
            (set-window-point window 9)
            (goto-char 9)
            (transpose-regions 1 3 7 11 nil)
            (should (= (window-point window) (point))))
        (set-window-buffer window old)))))

(ert-deftest transpose-regions-interactively-takes-the-mark-ring ()
  (with-temp-buffer
    (insert "abcXYZdefg")
    (goto-char 1) (push-mark 3 t)
    (push-mark 7 t) (push-mark 11 t)
    (goto-char 1) (set-mark 3)
    (should (eq (car (cadr (interactive-form 'transpose-regions))) 'if))
    (let ((ring (copy-sequence mark-ring)))
      (call-interactively #'transpose-regions)
      (should (equal (mapcar #'marker-position ring) '(7 1)))
      (should (equal (buffer-string) "cXYZabdefg")))))

(ert-deftest transpose-regions-undoes-unibyte-properties ()
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (buffer-enable-undo)
    (insert "abcd")
    (put-text-property 1 3 'p 1)
    (undo-boundary)
    (transpose-regions 1 3 3 5)
    (should (equal (list (get-text-property 1 'p) (get-text-property 3 'p)) '(nil 1)))
    (primitive-undo 1 buffer-undo-list)
    (should (equal (buffer-substring-no-properties 1 5) "abcd"))
    (should (equal (mapcar (lambda (p) (get-text-property p 'p)) '(1 2 3 4)) '(1 1 nil nil)))
    ;; Typing after the undo inherits the property before point.
    (goto-char 3)
    (let ((last-command-event ?z)) (self-insert-command 1))
    (should (equal (buffer-substring-no-properties 1 6) "abzcd"))
    (should (equal (get-text-property 3 'p) 1))))

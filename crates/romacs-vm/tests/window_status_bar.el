;;; window_status_bar.el --- window.el's fitting under the status bar  -*- lexical-binding: t -*-
;; Evaluated as window_dividers.el is (tests/window_text_pixel_size.rs
;; against window_status_bar.gnu).  Romacs shows its status bar
;; (VM2-STATUS-BAR) on the 1216x876 frame, and the bar is every window's
;; mode line, so no window has a band; pinned GNU, which has no bar, runs
;; on a frame shorter by the bar's line, 1216x851, with `mode-line-format'
;; nil, so its windows have none either and the root window is the same
;; height on both.  A bottom side window fitted to its buffer, with
;; dividers and border 6, gives the same answers.

(defun probe-settle (w h) (dotimes (_ 100) (unless (and (= (frame-native-width) w) (= (frame-native-height) h)) (sit-for 0.02))) (redisplay t) (list (frame-native-width) (frame-native-height)))
(progn (setq window-resize-pixelwise t) (when (fboundp 'x-create-frame) (message nil) (setq frame-inhibit-implied-resize t frame-resize-pixelwise t) (menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1) (blink-cursor-mode -1) (set-frame-font (font-spec :family "Iosevka" :size 19) nil t) (modify-frame-parameters nil '((internal-border-width . 0) (left-fringe . 8) (right-fringe . 8))) (set-frame-size nil 1200 851 t) (probe-settle 1216 851)) (dolist (face '(mode-line mode-line-active mode-line-inactive)) (set-face-attribute face nil :box nil :inherit nil :height 'unspecified :family 'unspecified)) (when (fboundp 'x-create-frame) (setq-default mode-line-format nil) (dolist (b (buffer-list)) (with-current-buffer b (kill-local-variable 'mode-line-format)))) (list (frame-char-width) (frame-char-height)))
;; window_el_fits_a_bottom_side_window_under_the_status_bar
(progn (modify-frame-parameters nil '((right-divider-width . 6) (bottom-divider-width . 6) (internal-border-width . 6))) (unless (fboundp 'x-create-frame) (modify-frame-parameters nil '((romacs-status-bar-lines . 1)))) (when (fboundp 'x-create-frame) (probe-settle 1216 851)) (list (frame-right-divider-width) (frame-bottom-divider-width) (frame-internal-border-width) (window-pixel-height (frame-root-window))))
(let ((w (display-buffer-in-side-window (get-buffer-create "*side*") '((side . bottom))))) (with-current-buffer "*side*" (erase-buffer) (insert "a\nb\nc")) (fit-window-to-buffer w) (list (window-pixel-height w) (window-body-height w t) (window-bottom-divider-width w) (window-pixel-top w) (window-pixel-height (frame-root-window)) (window-text-pixel-size w nil t nil 800 t)))
(let ((w (get-buffer-window "*side*"))) (with-current-buffer "*side*" (erase-buffer) (dotimes (i 40) (insert (format "line %d\n" i)))) (fit-window-to-buffer w) (list (window-pixel-height w) (window-pixel-top w) (progn (fit-window-to-buffer w 8 3) (window-pixel-height w)) (progn (with-current-buffer "*side*" (erase-buffer) (insert "x")) (fit-window-to-buffer w) (window-pixel-height w))))
(let ((w (get-buffer-window "*side*"))) (with-current-buffer "*side*" (erase-buffer) (insert (make-string 200 ?w) "\nshort\nend")) (fit-window-to-buffer w) (list (window-pixel-height w) (window-body-height w) (count-screen-lines nil nil nil w) (mapcar (lambda (w) (list (window-pixel-top w) (window-pixel-height w))) (window-list nil 'never))))

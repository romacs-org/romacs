(progn
;;; window-scroll.el --- VM2: recenter, scrolling and move-to-window-line  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM by prims/window_scroll.rs, the first time one of
;; its commands runs.  GNU window.c's line-based paths: Frecenter on a
;; terminal frame, window_scroll_line_based under scroll_command, and
;; Fmove_to_window_line with displayed_window_lines, moving by screen
;; lines through `vertical-motion', which follows the host's layout once a
;; host lays the frame out.  On a window-system frame GNU takes their
;; pixel-based twins, which agree where a window's rows share the frame's
;; line height (DIVERGENCES.md VM2-SCROLL1).

;;; Code:

(defvar romacs--scroll-preserve-vpos -1
  "window.c window_scroll_preserve_vpos: point's screen line when a run
of scroll commands began, for `scroll-preserve-screen-position'.")

(defvar romacs--scroll-preserve-hpos 0
  "window.c window_scroll_preserve_hpos: point's column then.")

(defun romacs--window-internal-height (window)
  "WINDOW's text lines: window.c window_internal_height, its lines less
its mode and header lines, which on a window-system frame's pixel-based
paths is the text area's height (window_box_height), its bottom divider
left out."
  (window-body-height window))

(defun romacs--window-scroll-margin (window)
  "window.c window_scroll_margin in lines: `scroll-margin' within a
quarter of WINDOW's text lines, or `maximum-scroll-margin' of them."
  (if (> scroll-margin 0)
      (let* ((lines (window-body-height window))
             (ratio (if (floatp maximum-scroll-margin)
                        (min (max maximum-scroll-margin 0.0) 0.5)
                      0.25))
             (most (min (/ (1- lines) 2) (truncate (* lines ratio)))))
        (max 0 (min scroll-margin most)))
    0))

(defun romacs--screen-position (window from to)
  "compute_motion from FROM, a screen line start, to TO in WINDOW:
TO's column on its screen line and the screen lines between, (HPOS . VPOS)."
  (save-excursion
    (goto-char from)
    (let ((vpos 0))
      (while (and (< (point) to) (= (vertical-motion 1 window) 1) (<= (point) to))
        (setq vpos (1+ vpos)))
      (goto-char to)
      (let ((column (current-column)))
        (vertical-motion 0 window)
        (cons (- column (current-column)) vpos)))))

(defun romacs--scroll-command (window n direction)
  "window.c scroll_command: scroll WINDOW (nil for the selected one) by N
lines, by a screen-full when N is nil or `-', in DIRECTION (1 up, -1 down)."
  (let* ((window (or window (selected-window)))
         (other (not (eq window (selected-window)))))
    (if (or other (not (eq (window-buffer window) (current-buffer))))
        (save-excursion
          (set-buffer (window-buffer window))
          (when other (goto-char (window-point window)))
          (romacs--scroll-by window n direction)
          (when other (set-window-point window (point))))
      (romacs--scroll-by window n direction)))
  nil)

(defun romacs--scroll-by (window n direction)
  (cond ((null n) (romacs--window-scroll window direction t))
        ((eq n '-) (romacs--window-scroll window (- direction) t))
        (t (romacs--window-scroll window (* (prefix-numeric-value n) direction) nil))))

(defun romacs--preserved-line (window pos limit margin)
  "Point at the preserved screen line from POS, within MARGIN of LIMIT lines."
  (let ((lines romacs--scroll-preserve-vpos))
    (goto-char pos)
    (cond ((< lines margin) (setq lines margin))
          ((>= lines (- limit margin)) (setq lines (- limit margin 1))))
    (vertical-motion (cons romacs--scroll-preserve-hpos lines) window)))

(defun romacs--point-visible-p (window lines)
  "Whether point shows in WINDOW, as `pos-visible-in-window-p' tells; where
the host's layout cannot measure that, whether it lies within WINDOW's
LINES screen lines from its start, as on a terminal frame."
  (condition-case err
      (pos-visible-in-window-p (point) window)
    (error
     (unless (and (stringp (car (cdr err))) (string-match "\\`VM2 window-text-pixel-size " (car (cdr err))))
       (signal (car err) (cdr err)))
     ;; Where the motion stops short of LINES rows, at the end of the
     ;; buffer, that end shows too.
     (let* ((start (window-start window)) (pt (point))
            (end (save-excursion (goto-char start) (cons (vertical-motion lines window) (point)))))
       (and (>= pt start)
            (if (< (car end) lines) (<= pt (cdr end)) (< pt (cdr end))))))))

(defun romacs--window-scroll (window n whole)
  "window.c window_scroll_line_based: scroll WINDOW N lines, or N
screen-fulls when WHOLE, keeping point in view."
  (let* ((opoint (point-marker))
         (ht (romacs--window-internal-height window))
         (start (window-start window))
         original pos lose)
    (when whole (setq n (* n (max 1 (- ht next-screen-context-lines)))))
    (when scroll-preserve-screen-position
      (when (or (<= romacs--scroll-preserve-vpos 0)
                (not (symbolp last-command))
                (not (get last-command 'scroll-command)))
        (let ((at (romacs--screen-position window start (point))))
          (setq romacs--scroll-preserve-vpos (cdr at)
                romacs--scroll-preserve-hpos (+ (car at) (window-hscroll window)))))
      (setq original (cons romacs--scroll-preserve-hpos romacs--scroll-preserve-vpos)))
    (unless (romacs--point-visible-p window ht)
      (vertical-motion (- (/ ht 2)) window)
      (setq start (point)))
    (goto-char start)
    (setq lose (and (< n 0) (= (point) (point-min))))
    (vertical-motion n window)
    (setq pos (point))
    (goto-char opoint)
    (when lose (signal 'beginning-of-buffer nil))
    (unless (< pos (point-max)) (signal 'end-of-buffer nil))
    (let ((margin (romacs--window-scroll-margin window)))
      (set-window-start window pos)
      (cond
       ((and scroll-preserve-screen-position (= margin 0)
             (or whole (not (eq scroll-preserve-screen-position t))))
        (goto-char pos)
        (vertical-motion original window))
       ;; Scrolled forward: point enough lines down to clear the margin.
       ((> n 0)
        (let ((top (if (> margin 0)
                       (save-excursion (goto-char pos) (vertical-motion margin window) (point))
                     pos)))
          (cond ((<= top opoint) (goto-char opoint))
                (scroll-preserve-screen-position
                 (romacs--preserved-line window pos (window-total-height window) margin))
                (t (goto-char top)))))
       ;; Scrolled back: point near the end of the window, clear of the margin.
       ((< n 0)
        (goto-char pos)
        (let* ((moved (vertical-motion (- ht margin) window))
               (bottom (if (= moved (- ht margin)) (point) (1+ (point)))))
          (cond ((> bottom opoint) (goto-char opoint))
                (scroll-preserve-screen-position
                 (romacs--preserved-line window pos ht margin))
                (t (vertical-motion -1 window)))))))
    (set-marker opoint nil)
    nil))

(defun romacs--recenter (arg redisplay)
  "window.c Frecenter on a terminal frame: put point's screen line ARG
lines from the top (from the bottom when negative, in the middle when
nil or a list) of the selected window, clear of `scroll-margin'."
  (let ((window (selected-window)) center iarg)
    (unless (eq (window-buffer window) (current-buffer))
      (error "%s" (format-message "`recenter'ing a window that does not display current-buffer")))
    (cond ((null arg)
           ;; A window-system frame is no tty, so `tty' redraws nothing.
           (when (and redisplay recenter-redisplay (not (eq recenter-redisplay 'tty)))
             (redraw-display))
           (setq center t))
          ((consp arg) (setq center t))
          (t (setq iarg (prefix-numeric-value arg))))
    (let* ((margin (romacs--window-scroll-margin window))
           (ht (romacs--window-internal-height window))
           (line (cond (center (/ ht 2)) ((< iarg 0) (+ iarg ht)) (t iarg)))
           (line (max margin (min line (- ht margin 1))))
           ;; Short of lines, the iterator ends at BEGV, shown or not.
           (start (save-excursion (if (< (- (vertical-motion (- line) window)) line) (point-min) (point)))))
      ;; optional_new_start: redisplay keeps it while point shows.
      (set-window-start window start t)
      ;; Zero in lines or pixels alike; VM2 keeps it in pixels.
      (set-window-vscroll window 0 t)))
  nil)

(defun romacs--displayed-window-lines (window)
  "window.c displayed_window_lines: the screen line WINDOW's text area
reaches from its start, counting empty lines below the end of the text."
  (let* ((unit (frame-char-height (window-frame window)))
         (height (window-body-height window t))
         (rows (/ height unit))
         (moved (save-excursion (goto-char (window-start window)) (vertical-motion rows window))))
    (if (>= moved rows)
        rows
      (let ((bottom (* (1+ moved) unit)))
        (cond ((< bottom height) (+ moved (/ (+ (- height bottom) unit -1) unit)))
              ((= bottom height) (1+ moved))
              (t moved))))))

(defun romacs--move-to-window-line (arg)
  "window.c Fmove_to_window_line: point to screen line ARG of the selected
window (from the bottom when negative, the middle when nil); the line."
  (let* ((window (selected-window))
         (start (window-start window)))
    (unless (eq (window-buffer window) (current-buffer))
      (error "move-to-window-line called from unrelated buffer"))
    (if (or (< start (point-min)) (> start (point-max)))
        (progn (vertical-motion (- (/ (romacs--window-internal-height window) 2)) window)
               (set-window-start window (point)))
      (goto-char start))
    (let* ((lines (romacs--displayed-window-lines window))
           (line (if (null arg)
                     (/ lines 2)
                   (let ((n (prefix-numeric-value arg))) (if (< n 0) (+ n lines) n)))))
      ;; Skip past a partially visible first line.
      (when (/= (window-vscroll window t) 0) (setq line (1+ line)))
      (vertical-motion line window))))

)
;;; window-scroll.el ends here

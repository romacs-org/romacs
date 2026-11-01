;;; romacs-armillary.el --- The *Romacs* landing's armillary sphere  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The ASCII armillary sphere at the top of the *Romacs* startup screen,
;; ported from thanosapollo.org ascii-home.js, used with the author's permission (renderer in
;; crates/romacs-armillary).  Lisp draws its still frame into the buffer
;; (`romacs--armillary-frame') and registers it with rlisp/romacs-animate.el
;; as a display element: the render thread turns it, frame after frame,
;; while *Romacs* is shown in the focused frame, with no Lisp per frame.
;; Where motion is off (`romacs-animate-motion-p') it stands still.  As on
;; the site, the arrows turn it, + and - zoom, SPC pauses and Home resets;
;; the other keys stay the startup screen's.

;;; Code:

(require 'romacs-animate)

(defgroup romacs-armillary nil
  "The *Romacs* landing's armillary sphere."
  :group 'environment)

(defface romacs-armillary-shade-1
  '((((background light)) :foreground "#a59a85") (t :foreground "#756b59"))
  "The sphere's dimmest cells: the palette's muted foreground.")

(defface romacs-armillary-shade-2
  '((((background light)) :foreground "#a8650f") (t :foreground "#d48a20"))
  "The sphere's half-lit cells: the palette's accent.")

(defface romacs-armillary-shade-3
  '((((background light)) :foreground "#4a4030") (t :foreground "#d4c5a0"))
  "The sphere's lit cells: the palette's foreground.")

(defface romacs-armillary-shade-4
  '((((background light)) :foreground "#000000") (t :foreground "#f4e7c3"))
  "The sphere's brightest cells and stars: the palette's bright foreground.")

(defconst romacs-armillary--faces
  [romacs-armillary-shade-1 romacs-armillary-shade-2 romacs-armillary-shade-3 romacs-armillary-shade-4]
  "The sphere's faces, dark to light.")

(defconst romacs-armillary-controls
  "arrows turn   + - zoom   SPC pause   Home reset   q close"
  "The line under the sphere.")

(defvar romacs-armillary-landing-hook nil
  "Run in *Romacs* once its sphere and controls are in place, with point
after them: where further landing content goes (a newcomer's choice of
key bindings, say).")

(defvar-local romacs-armillary--view nil
  "The sphere's view: a plist of :pitch, :yaw, :zoom and :phase.")

(defvar-local romacs-armillary--drawn nil
  "The still frame's size and cell aspect: (COLS ROWS ASPECT).")

(defvar-local romacs-armillary--epoch 0
  "How many times the sphere was reset; the display restarts its motion
at each.")

(defvar-local romacs-armillary--end nil
  "The end of the sphere's text, which starts the buffer.")

(defun romacs-armillary--home ()
  (list :pitch -0.25 :yaw 0.4 :zoom 1.0 :phase 0.0))

(defun romacs-armillary--advance (view dt)
  "VIEW DT seconds on: the site's yaw of 0.16 and phase of 0.2 a second."
  (let ((view (copy-sequence view)))
    (plist-put view :yaw (+ (plist-get view :yaw) (* dt 0.16)))
    (plist-put view :phase (+ (plist-get view :phase) (* dt 0.2)))))

(defun romacs-armillary--text (view cols rows aspect)
  "VIEW's frame of COLS by ROWS cells ASPECT wide per unit of height."
  (let ((phase (plist-get view :phase)))
    (romacs--armillary-frame cols rows
                             (+ (plist-get view :pitch) (* (sin (* phase 0.47)) 0.12))
                             (plist-get view :yaw)
                             (+ -0.15 (* (sin (* phase 0.3)) 0.1))
                             (plist-get view :zoom) aspect t romacs-armillary--faces)))

(defun romacs-armillary--replace (text)
  "Make TEXT the sphere's, keeping point and the window's start where
they were relative to the text after it."
  (let* ((inhibit-read-only t)
         (inhibit-modification-hooks t)
         (window (get-buffer-window (current-buffer) (selected-frame)))
         (end (marker-position romacs-armillary--end))
         (delta (- (length text) (- end (point-min))))
         (point (point))
         (start (and window (window-start window))))
    (save-excursion
      (goto-char (point-min))
      (delete-region (point-min) end)
      (insert text))
    (goto-char (if (> point end) (+ point delta) (point-min)))
    (when (and start (> start end))
      (set-window-start window (+ start delta) t))
    (set-buffer-modified-p nil)))

(defun romacs-armillary--frame (dt)
  "Draw the sphere DT seconds on, sized to its window (romacs-animate)."
  (let ((window (get-buffer-window (current-buffer) (selected-frame))))
    (when window
      (setq romacs-armillary--view (romacs-armillary--advance romacs-armillary--view dt))
      (let ((cols (max 1 (min 200 (1- (window-body-width window)))))
            ;; Whole lines only: the controls line must show in full.
            (rows (max 1 (min 100 (- (floor (window-body-height window t) (default-line-height)) 2))))
            (aspect (max 0.05 (min 2.0 (/ (float (frame-char-width)) (frame-char-height))))))
        (setq romacs-armillary--drawn (list cols rows aspect))
        (romacs-armillary--replace (romacs-armillary--text romacs-armillary--view cols rows aspect))))))

(defun romacs-armillary--element (window)
  "What the display turns in WINDOW (romacs-animate): the still frame's
block, in as many shades as it has faces, and the view; nil once WINDOW
starts below the sphere."
  (let ((start (window-start window))
        (view romacs-armillary--view))
    (when (and romacs-armillary--drawn (< start romacs-armillary--end))
      (list :top (- (count-lines (point-min) start))
            :cols (nth 0 romacs-armillary--drawn) :rows (nth 1 romacs-armillary--drawn)
            :shades (length romacs-armillary--faces)
            :kind (list 'armillary (plist-get view :pitch) (plist-get view :yaw) (plist-get view :zoom)
                        (nth 2 romacs-armillary--drawn) romacs-armillary--epoch)))))

(defun romacs-armillary--turn (key value)
  "Set the view's KEY to VALUE and redraw."
  (setq romacs-armillary--view (plist-put (copy-sequence romacs-armillary--view) key value))
  (romacs-animate-redraw 'romacs-armillary))

(defun romacs-armillary-left ()
  "Turn the sphere left."
  (interactive)
  (romacs-armillary--turn :yaw (- (plist-get romacs-armillary--view :yaw) 0.12)))

(defun romacs-armillary-right ()
  "Turn the sphere right."
  (interactive)
  (romacs-armillary--turn :yaw (+ (plist-get romacs-armillary--view :yaw) 0.12)))

(defun romacs-armillary-up ()
  "Tip the sphere toward you."
  (interactive)
  (romacs-armillary--turn :pitch (- (plist-get romacs-armillary--view :pitch) 0.12)))

(defun romacs-armillary-down ()
  "Tip the sphere away."
  (interactive)
  (romacs-armillary--turn :pitch (+ (plist-get romacs-armillary--view :pitch) 0.12)))

(defun romacs-armillary-zoom-in ()
  "Bring the sphere closer, up to 1.3."
  (interactive)
  (romacs-armillary--turn :zoom (min 1.3 (+ (plist-get romacs-armillary--view :zoom) 0.05))))

(defun romacs-armillary-zoom-out ()
  "Move the sphere away, down to 0.75."
  (interactive)
  (romacs-armillary--turn :zoom (max 0.75 (- (plist-get romacs-armillary--view :zoom) 0.05))))

(defun romacs-armillary-pause ()
  "Stop or restart the sphere's turning."
  (interactive)
  (if (romacs-animate-paused-p 'romacs-armillary)
      (romacs-animate-resume 'romacs-armillary)
    (romacs-animate-pause 'romacs-armillary)))

(defun romacs-armillary-reset ()
  "Put the sphere back as it started, turning."
  (interactive)
  (setq romacs-armillary--view (romacs-armillary--home)
        romacs-armillary--epoch (1+ romacs-armillary--epoch))
  (romacs-animate-resume 'romacs-armillary)
  (romacs-animate-redraw 'romacs-armillary))

(defvar romacs-armillary-map
  (let ((map (make-sparse-keymap)))
    (define-key map [left] #'romacs-armillary-left)
    (define-key map [right] #'romacs-armillary-right)
    (define-key map [up] #'romacs-armillary-up)
    (define-key map [down] #'romacs-armillary-down)
    (define-key map "+" #'romacs-armillary-zoom-in)
    (define-key map "=" #'romacs-armillary-zoom-in)
    (define-key map "-" #'romacs-armillary-zoom-out)
    (define-key map " " #'romacs-armillary-pause)
    (define-key map [home] #'romacs-armillary-reset)
    map)
  "The sphere's keys in *Romacs*, over the startup screen's.")

(defun romacs-armillary-landing ()
  "Put the turning sphere and its controls at the top of the current
buffer, the startup screen, whose keymap becomes the parent of
`romacs-armillary-map'; then run `romacs-armillary-landing-hook'."
  (buffer-disable-undo)
  (let ((inhibit-read-only t))
    (goto-char (point-min))
    (insert "\n\n" (propertize romacs-armillary-controls 'face 'shadow) "\n\n")
    ;; Empty until the first frame, which goes before it.
    (setq romacs-armillary--end (copy-marker (point-min) t))
    (set-buffer-modified-p nil))
  (setq romacs-armillary--view (romacs-armillary--home))
  (setq cursor-type nil)
  (use-local-map (make-composed-keymap romacs-armillary-map (current-local-map)))
  (romacs-animate-start 'romacs-armillary (current-buffer) 25 #'romacs-armillary--frame
                        #'romacs-armillary--element)
  (run-hooks 'romacs-armillary-landing-hook))

(provide 'romacs-armillary)
;;; romacs-armillary.el ends here

;;; romacs-landing.el --- The *Romacs* landing's logo  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The Romacs mark, its red R and blue crescent, over the wordmark, at the
;; top of the *Romacs* startup screen, drawn in braille cells (renderer in
;; crates/romacs-logo).  Lisp draws a still frame into the buffer
;; (`romacs--logo-frame') and registers it with lisp/romacs/romacs-animate.el as
;; a display element in every window of the frame that shows it.  On a GPU
;; the display draws the element as a scene over those cells
;; (render_thread/page_scenes in romacs-display-runtime): braille dots rain
;; into the mark, swell into a lit, extruded R that turns slowly in a warm
;; rim light, a band of light crosses it, and the wordmark rises in as a
;; vector.  Elsewhere the render thread sweeps light across the braille
;; letters.  Either way no Lisp runs per frame.  Where motion is off
;; (`romacs-animation', `romacs-animate-motion-p') it stands still, on a
;; GPU in its posed, lit still frame.  SPC pauses and Home restarts it;
;; the other keys stay the startup screen's.

;;; Code:

(require 'romacs-animate)


(defvar romacs-landing-hook nil
  "Run in *Romacs* once its logo and controls are in place, with point
after them: where further landing content goes (a newcomer's choice of
key bindings, say).")

(defvar-local romacs-landing--drawn nil
  "The still frame's size and cell aspect: (COLS ROWS ASPECT).")

(defvar-local romacs-landing--epoch 0
  "How many times the logo was restarted; the display restarts its motion
at each.")

(defvar-local romacs-landing--end nil
  "The end of the logo's text, which starts the buffer.")

(defun romacs-landing--replace (text)
  "Make TEXT the logo's, keeping point and the window's start where they
were relative to the text after it."
  (let* ((inhibit-read-only t)
         (inhibit-modification-hooks t)
         (window (get-buffer-window (current-buffer) (selected-frame)))
         (end (marker-position romacs-landing--end))
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

(defun romacs-landing--frame (_dt)
  "Draw the logo's still frame, sized to fit every window of the selected
frame that shows it (romacs-animate)."
  (let ((windows (get-buffer-window-list (current-buffer) 'nomini (selected-frame))))
    (when windows
      (let ((cols (max 1 (min 400 (1- (apply #'min (mapcar #'window-body-width windows))))))
            ;; Whole lines only: the controls line must show in full.
            (rows (max 1 (min 200 (- (apply #'min (mapcar (lambda (w) (floor (window-body-height w t) (default-line-height)))
                                                          windows))
                                     2))))
            (aspect (max 0.05 (min 2.0 (/ (float (frame-char-width)) (frame-char-height))))))
        (setq romacs-landing--drawn (list cols rows aspect))
        (romacs-landing--replace (romacs--logo-frame cols rows aspect))))))

(defun romacs-landing--element (_window)
  "What the display draws in a window that shows the landing
\(romacs-animate): the still frame's block, which starts the buffer, and
the logo's motion.  The display places it wherever the window scrolls."
  (when romacs-landing--drawn
    (list :start (point-min)
          :cols (nth 0 romacs-landing--drawn) :rows (nth 1 romacs-landing--drawn)
          :kind (list 'logo (nth 2 romacs-landing--drawn) romacs-landing--epoch))))

(defun romacs-landing-pause ()
  "Stop or restart the logo's motion."
  (interactive)
  (if (romacs-animate-paused-p 'romacs-landing)
      (romacs-animate-resume 'romacs-landing)
    (romacs-animate-pause 'romacs-landing)))

(defun romacs-landing-restart ()
  "Start the logo's motion over."
  (interactive)
  (setq romacs-landing--epoch (1+ romacs-landing--epoch))
  (romacs-animate-resume 'romacs-landing)
  (romacs-animate-redraw 'romacs-landing))

(defvar romacs-landing-map
  (let ((map (make-sparse-keymap)))
    (define-key map " " #'romacs-landing-pause)
    (define-key map [home] #'romacs-landing-restart)
    map)
  "The logo's keys in *Romacs*, over the startup screen's.")

(defun romacs-landing--command-p (_symbol buffer)
  "Whether BUFFER shows the logo these commands act on."
  (buffer-local-value 'romacs-landing--end buffer))

(put 'romacs-landing-pause 'completion-predicate #'romacs-landing--command-p)
(put 'romacs-landing-restart 'completion-predicate #'romacs-landing--command-p)

(defun romacs-landing ()
  "Put the logo and its controls at the top of the current buffer, the
startup screen, whose keymap becomes the parent of `romacs-landing-map';
then run `romacs-landing-hook'."
  (buffer-disable-undo)
  (let ((inhibit-read-only t))
    (goto-char (point-min))
    (insert "\n\n" (romacs-key-hint "\\<romacs-landing-map>\\[romacs-landing-pause] pause   \\[romacs-landing-restart] restart   \\`q' close"
                                     'shadow)
            "\n\n")
    ;; Empty until the first frame, which goes before it.
    (setq romacs-landing--end (copy-marker (point-min) t))
    (set-buffer-modified-p nil))
  (setq cursor-type nil)
  (use-local-map (make-composed-keymap romacs-landing-map (current-local-map)))
  (romacs-animate-start 'romacs-landing (current-buffer) 25 #'romacs-landing--frame
                        #'romacs-landing--element)
  (run-hooks 'romacs-landing-hook))

(declare-function romacs-gui-loadup-startup-buffer "romacs-gui-loadup")

(defun romacs-landing-show ()
  "Show the *Romacs* landing in the selected window.
Reuse the buffer, or build it again as startup does: q kills it."
  (interactive)
  (switch-to-buffer (romacs-gui-loadup-startup-buffer "*Romacs*")))

(defun romacs-landing--empty-session-p ()
  "Whether the session holds nothing yet: only what startup leaves.
Hidden buffers, *Messages*, the landing and an untouched *scratch*."
  (seq-every-p (lambda (buffer)
                 (let ((name (buffer-name buffer)))
                   (or (string-prefix-p " " name)
                       (member name '("*Messages*" "*Romacs*"))
                       (and (equal name "*scratch*") (not (buffer-modified-p buffer))))))
               (buffer-list)))

(defun romacs-landing-new-client ()
  "Show the landing to a client with nothing to visit, on an empty session.
Where startup would: not with `initial-buffer-choice' or
`inhibit-startup-screen' set; and not over a prompt or a search under way."
  (when (and (not initial-buffer-choice) (not inhibit-startup-screen)
             (not isearch-mode) (not (active-minibuffer-window))
             (romacs-landing--empty-session-p))
    (romacs-landing-show)))

(provide 'romacs-landing)
;;; romacs-landing.el ends here

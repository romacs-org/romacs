;;; romacs-chrome.el --- Pane chrome: gaps, rounded panes, focus edge, dimming  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The root frame's windows are drawn as panes: apart by a gap, with
;; rounded corners, a hairline round each, the selected one's edge in an
;; accent, and the others dimmed toward the frame background.
;;
;; The gap is GNU's: the `right-divider-width' and `bottom-divider-width'
;; window dividers between the windows and the `internal-border-width'
;; round them, so every window edge, body, minimum size, split, balance
;; and fit is the answer GNU gives for a frame with those parameters.
;; Corners, hairline, focus edge and dimming are paint only, inside each
;; window's box; they change no number any primitive returns.  Every
;; value reaches the display as a frame parameter of the root frame.
;;
;; `romacs-classic-chrome' sets every value to 0: GNU's look, with the
;; geometry of a GNU frame without dividers or border.  DIVERGENCES.md
;; VM2-PANE-CHROME.

;;; Code:

(defgroup romacs-chrome nil
  "How the root frame's windows are drawn as panes."
  :group 'frames
  :prefix "romacs-pane-")

(defface romacs-pane-focus '((t :inherit romacs-layer-glow))
  "The selected pane's edge: its foreground.")

(defface romacs-pane-border '((t))
  "The hairline round every other pane: its foreground.  Unless a theme
sets it, the default background lifted 15% toward the foreground
\(`romacs-chrome-derived-faces').")

(defun romacs-chrome--set (option value)
  "Set OPTION to VALUE and apply the chrome to the root frame."
  (set-default-toplevel-value option value)
  (romacs-chrome-apply))

(defcustom romacs-pane-gap 6
  "Pixels between the panes and round them.
They are the root frame's `right-divider-width', `bottom-divider-width'
and `internal-border-width'."
  :type 'natnum
  :initialize #'custom-initialize-default
  :set #'romacs-chrome--set)

(defcustom romacs-pane-radius 6
  "Pixels of each pane's rounded corners."
  :type 'natnum
  :initialize #'custom-initialize-default
  :set #'romacs-chrome--set)

(defcustom romacs-pane-dim 0.12
  "How far the panes other than the selected one are blended toward the
frame background, from 0 to 0.3."
  :type 'float
  :initialize #'custom-initialize-default
  :set (lambda (option value)
         (unless (and (numberp value) (<= 0 value 0.3))
           (user-error "`%s' must be a number from 0 to 0.3" option))
         (romacs-chrome--set option value)))

(defun romacs-chrome--root-frame ()
  "The root frame: the selected frame's outermost parent."
  (let ((frame (selected-frame)))
    (while (frame-parent frame) (setq frame (frame-parent frame)))
    frame))

(defun romacs-chrome--hex (color)
  "COLOR, a colour name, as #RRGGBB, or nil when it is unknown."
  (when-let* ((values (and color (color-values color))))
    (apply #'format "#%02x%02x%02x" (mapcar (lambda (v) (ash v -8)) values))))

;; Chrome colours and themes.  Every chrome colour is a face: the focus
;; edge (`romacs-pane-focus', by default `romacs-layer-glow'), the
;; hairline (`romacs-pane-border'), a layer's glow (`romacs-layer-glow'),
;; the completion panel's selected row and matches
;; (`romacs-complete-current', and `romacs-complete-match', which
;; inherits the glow; its prompt inherits `shadow').  A theme that sets one of
;; them is obeyed.  For the ones it leaves unset the faces' default
;; specs are derived from faces every theme sets, again whenever a theme
;; is enabled or disabled; with no theme enabled they are the built-in
;; palette's.  A glass layer's background and the dimming tint are not
;; faces: they are the default background, the first lifted 8% toward
;; the foreground (`romacs-layer-glass-parameters').

(defconst romacs-chrome-builtin-accent "#d48a20"
  "The built-in palette's accent (rlisp/romacs-startup.el), Omarchy Rum's:
the focus edge, glow and match colour while no theme is enabled.")

(defun romacs-chrome-mix (amount)
  "The default background moved AMOUNT, from 0 to 1, toward the default
foreground, as #RRGGBB, or nil when either is unknown."
  (let ((bg (color-values (face-background 'default nil t)))
        (fg (color-values (face-foreground 'default nil t))))
    (when (and bg fg)
      (apply #'format "#%02x%02x%02x"
             (seq-mapn (lambda (b f) (round (/ (+ (* (- 1 amount) b) (* amount f)) 257.0))) bg fg)))))

(defun romacs-chrome--chromatic (color)
  "COLOR as #RRGGBB when it has a clear hue, nil when it is a grey."
  (when-let* ((values (and color (color-values color))))
    (and (>= (- (apply #'max values) (apply #'min values)) (* 0.2 65535))
         (romacs-chrome--hex color))))

(defun romacs-chrome--accent ()
  "The enabled theme's accent: the cursor's colour, which marks where
input goes as the focus edge does; when that is a grey (black or white
cursors are common), the link colour, the colour themes give what can
be acted on; else the prompt's; else the cursor's grey."
  (let ((cursor (face-background 'cursor nil t)))
    (or (seq-some #'romacs-chrome--chromatic
                  (list cursor (face-foreground 'link nil t) (face-foreground 'minibuffer-prompt nil t)))
        (romacs-chrome--hex cursor))))

(defun romacs-chrome-derived-faces ()
  "The chrome faces' default specs, each (FACE ATTRIBUTE VALUE...):
derived from `default', `cursor', `link' and `minibuffer-prompt', which
every theme sets.  The accent is the
built-in one while no theme is enabled.  A face whose colour cannot be
derived is left out."
  (let ((accent (if custom-enabled-themes (romacs-chrome--accent) romacs-chrome-builtin-accent)))
    (seq-filter
     (lambda (spec) (nth 2 spec))
     `((romacs-layer-glow :foreground ,accent)
       (romacs-pane-border :foreground ,(romacs-chrome-mix 0.15))
       (romacs-complete-current :background ,(romacs-chrome-mix 0.2) :extend t)))))

(defun romacs-chrome--set-faces ()
  "Make `romacs-chrome-derived-faces' the faces' default specs.  A
theme's spec for a face still wins (faces.el `face-spec-recalc')."
  (pcase-dolist (`(,face . ,attributes) (romacs-chrome-derived-faces))
    (let ((spec `((t ,@attributes))))
      (when (and (facep face) (not (equal spec (get face 'face-defface-spec))))
        (face-spec-set face spec 'face-defface-spec)))))

(defun romacs-chrome--parameters (classic)
  "The root frame's chrome parameters, GNU's look when CLASSIC."
  (let ((gap (if classic 0 romacs-pane-gap)))
    `((right-divider-width . ,gap)
      (bottom-divider-width . ,gap)
      (internal-border-width . ,gap)
      (romacs-pane-radius . ,(if classic 0 romacs-pane-radius))
      (romacs-pane-dim . ,(if classic 0.0 (float romacs-pane-dim)))
      (romacs-pane-focus . ,(and (not classic) (romacs-chrome--hex (face-foreground 'romacs-pane-focus nil t))))
      (romacs-pane-border . ,(and (not classic) (romacs-chrome--hex (face-foreground 'romacs-pane-border nil t)))))))

(defvar romacs-classic-chrome)

(defun romacs-chrome-apply (&rest _)
  "Apply the chrome options to the root frame, on a window system only."
  (let ((frame (romacs-chrome--root-frame)))
    (when (window-system frame)
      (modify-frame-parameters frame (romacs-chrome--parameters (bound-and-true-p romacs-classic-chrome))))))

(defun romacs-chrome-refresh (&rest _)
  "Derive the chrome faces a theme leaves unset, then show the colours:
on the root frame and on every glass layer.  Run from
`enable-theme-functions', `disable-theme-functions' and
`after-init-hook', so the chrome follows a theme and the faces an init
sets."
  (romacs-chrome--set-faces)
  (romacs-chrome-apply)
  (when (fboundp 'romacs-layer-refresh) (romacs-layer-refresh)))

(define-minor-mode romacs-classic-chrome
  "Draw the root frame's windows as GNU does: no gaps, corners, hairlines,
focus edge or dimming, and `window-divider-mode' off.  Off, the
`romacs-pane-' options apply again."
  :global t
  :group 'romacs-chrome
  (when (and romacs-classic-chrome (bound-and-true-p window-divider-mode))
    (window-divider-mode -1))
  (romacs-chrome-apply))

(defun romacs-chrome-state (&optional frame)
  "The chrome FRAME (default the root frame) shows, as a plist."
  (let ((frame (or frame (romacs-chrome--root-frame))))
    (list :classic (and (bound-and-true-p romacs-classic-chrome) t)
          :gap romacs-pane-gap :radius romacs-pane-radius :dim romacs-pane-dim
          :right-divider (frame-right-divider-width frame)
          :bottom-divider (frame-bottom-divider-width frame)
          :internal-border (frame-internal-border-width frame)
          :pane-radius (or (frame-parameter frame 'romacs-pane-radius) 0)
          :pane-dim (or (frame-parameter frame 'romacs-pane-dim) 0.0)
          :focus (frame-parameter frame 'romacs-pane-focus)
          :border (frame-parameter frame 'romacs-pane-border))))

(add-hook 'enable-theme-functions #'romacs-chrome-refresh)
(add-hook 'disable-theme-functions #'romacs-chrome-refresh)
(add-hook 'after-init-hook #'romacs-chrome-refresh)

(provide 'romacs-chrome)
;;; romacs-chrome.el ends here

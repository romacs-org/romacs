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
;; window's box; they change no number any primitive returns.  The
;; sizes reach the display as frame parameters of the root frame; the
;; colours are faces, which the display reads as it paints.
;;
;; `romacs-status-bar-mode' puts a status bar between the windows and
;; the minibuffer line: the mode line of the window the selected one is
;; (`romacs-status-bar-window'), while each window's own mode-line band
;; shows a short label (`romacs-pane-label').  Layers and toasts end
;; above it (`romacs-chrome-bottom').
;;
;; `romacs-classic-chrome' sets every value to 0 and shows no bar: GNU's
;; look, with the geometry of a GNU frame without dividers or border.
;; DIVERGENCES.md VM2-PANE-CHROME and VM2-STATUS-BAR.

;;; Code:

(require 'romacs-faces)

(defgroup romacs-chrome nil
  "How the root frame's windows are drawn as panes."
  :group 'frames
  :prefix "romacs-pane-")

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
;; inherits the glow; its prompt inherits `shadow'), a glass layer's
;; background (`romacs-layer-glass').  A theme that sets one of them is
;; obeyed.  For the ones it leaves unset the faces' default specs are
;; derived from faces every theme sets, again whenever a theme is enabled
;; or disabled; with no theme enabled they are the built-in palette's.
;; The display reads the faces as it paints, so a theme, `custom-set-faces'
;; or `set-face-attribute' shows on the next frame.  The dimming tint is
;; not a face: it is the default background.

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
       (romacs-layer-glass :background ,(romacs-chrome-mix 0.08))
       (romacs-complete-current :background ,(romacs-chrome-mix 0.2) :extend t)))))

(defun romacs-chrome--set-faces (&rest _)
  "Make `romacs-chrome-derived-faces' the faces' default specs.  A
theme's spec for a face still wins (faces.el `face-spec-recalc').
Run from `enable-theme-functions' and `disable-theme-functions'."
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
      (romacs-pane-chrome . ,(not classic)))))

(defvar romacs-classic-chrome)
(defvar romacs-status-bar-mode)
(defvar romacs-mode-line-mode)

(defvar romacs-chrome--mode-line nil
  "Whether `romacs-mode-line-mode' was on when classic chrome turned it off.")

(defun romacs-chrome--classic-mode-line (classic)
  "Turn Romacs's mode line off for CLASSIC chrome, and on again after it
when classic chrome turned it off."
  (when (fboundp 'romacs-mode-line-mode)
    (cond ((and classic (bound-and-true-p romacs-mode-line-mode))
           (setq romacs-chrome--mode-line t)
           (romacs-mode-line-mode -1))
          ((and (not classic) romacs-chrome--mode-line)
           (setq romacs-chrome--mode-line nil)
           (romacs-mode-line-mode 1)))))

(defcustom romacs-pane-label 'name
  "What each window's own mode-line band shows while the status bar
shows the selected window's mode line: `name', the window's label
\(`romacs-pane-label-format') in `mode-line-inactive'; `full', its
mode line as in GNU; nil, no band, so its text takes that line too.
A window GNU draws no mode line for has none under any value."
  :type '(choice (const name) (const full) (const :tag "No band" nil))
  :initialize #'custom-initialize-default
  :set #'romacs-chrome--set)

(defvar romacs-pane-label-format '("%b")
  "The mode-line format of a window's band under the pane label `name'.")
(put 'romacs-pane-label-format 'risky-local-variable t)

(defun romacs-chrome--status-bar-parameters (classic)
  "The root frame's status-bar parameters, none when CLASSIC."
  (let ((bar (and (not classic) (bound-and-true-p romacs-status-bar-mode))))
    `((romacs-status-bar-lines . ,(if bar 1 0))
      (romacs-pane-label . ,(if bar romacs-pane-label 'full)))))

(defun romacs-chrome-apply (&rest _)
  "Apply the chrome options to the root frame, on a window system only."
  (let ((frame (romacs-chrome--root-frame)))
    (when (window-system frame)
      (let ((classic (bound-and-true-p romacs-classic-chrome)))
        (modify-frame-parameters frame (append (romacs-chrome--parameters classic)
                                               (romacs-chrome--status-bar-parameters classic)))))))

(defun romacs-chrome-refresh (&rest _)
  "Derive the chrome faces a theme leaves unset and apply the chrome to
the root frame: at startup and from `after-init-hook', so the chrome
follows the faces an init sets."
  (romacs-chrome--set-faces)
  (romacs-chrome-apply))

(define-minor-mode romacs-classic-chrome
  "Draw the root frame's windows as GNU does: no gaps, corners, hairlines,
focus edge or dimming, no status bar, and `window-divider-mode' off.
Off, the `romacs-pane-' options and `romacs-status-bar-mode' apply
again."
  :global t
  :group 'romacs-chrome
  (when (and romacs-classic-chrome (bound-and-true-p window-divider-mode))
    (window-divider-mode -1))
  (romacs-chrome--classic-mode-line romacs-classic-chrome)
  (romacs-chrome-apply))

(define-minor-mode romacs-status-bar-mode
  "Show the selected window's mode line in a bar above the minibuffer
line, and `romacs-pane-label' in each window's own mode-line band.
Off, every window shows its own mode line, as in GNU."
  :global t
  :group 'romacs-chrome
  (romacs-chrome-apply))

(defun romacs-status-bar-string (&optional _frame)
  "The text the status bar shows, nil while it shows none."
  (romacs--status-bar-text))

(defun romacs-chrome-bottom (&optional frame except)
  "Where a layer of FRAME (default the root frame) anchored at its bottom
ends, in the native pixels child frames are placed in: the top of the
status bar, else of the minibuffer line, else the frame's bottom inside
its internal border; or the top of the layer hosting the minibuffer,
unless it is EXCEPT, where that layer reaches below it."
  (let* ((root (or frame (romacs-chrome--root-frame)))
         (bottom (nth 3 (window-edges (frame-root-window root) nil nil t)))
         (host (romacs--minibuffer-host)))
    (when (and host (not (eq host except)) (eq (frame-parent host) root) (frame-visible-p host))
      (let ((top (cdr (frame-position host))))
        (when (> (+ top (frame-pixel-height host)) bottom)
          (setq bottom (min bottom top)))))
    bottom))

(defun romacs--status-bar-click (column type)
  "Run the binding of the status bar's mode line for a mouse event of
TYPE at COLUMN of its text: the `local-map' or `keymap' there of the
shown window's mode line, else the global map's, under `mode-line'.
The event's position is that window's mode line."
  (let* ((window (romacs-status-bar-window))
         (format (or (window-parameter window 'mode-line-format)
                     (buffer-local-value 'mode-line-format (window-buffer window))))
         (string (format-mode-line format nil window))
         (column (min column (max 0 (1- (length string)))))
         (map (and (< column (length string))
                   (or (get-text-property column 'local-map string)
                       (get-text-property column 'keymap string))))
         (key (vector 'mode-line type))
         (command (or (and (keymapp map) (lookup-key map key))
                      (key-binding key t)))
         (event (list type (list window 'mode-line (cons (* column (frame-char-width)) 0)
                                 0 (cons string column) nil (cons column 0) nil (cons 0 0)
                                 (cons (frame-char-width) (frame-char-height))))))
    (when (commandp command)
      (setq last-input-event event
            last-nonmenu-event event
            last-command-event event
            this-command command)
      (call-interactively command nil (vector 'mode-line event))
      (setq last-command command))))

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
          :focus (and (frame-parameter frame 'romacs-pane-chrome) (face-foreground 'romacs-pane-focus frame t))
          :border (and (frame-parameter frame 'romacs-pane-chrome) (face-foreground 'romacs-pane-border frame t))
          :status-bar (or (frame-parameter frame 'romacs-status-bar-lines) 0)
          :status-bar-height (romacs-status-bar-height)
          :label (if (assq 'romacs-pane-label (frame-parameters frame))
                     (frame-parameter frame 'romacs-pane-label)
                   'full))))

(add-hook 'enable-theme-functions #'romacs-chrome--set-faces)
(add-hook 'disable-theme-functions #'romacs-chrome--set-faces)
(add-hook 'after-init-hook #'romacs-chrome-refresh)

(provide 'romacs-chrome)
;;; romacs-chrome.el ends here

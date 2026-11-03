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

(defgroup romacs-chrome nil
  "How the root frame's windows are drawn as panes."
  :group 'frames
  :prefix "romacs-pane-")

(defface romacs-pane-focus '((t :inherit romacs-layer-glow))
  "The selected pane's edge: its foreground.")

(defface romacs-pane-border '((t))
  "The hairline round every other pane: its foreground, or when that is
unspecified the default background lifted 15% toward the foreground.")

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

(defun romacs-chrome--border ()
  "The hairline's colour: `romacs-pane-border''s foreground, else the
default background lifted 15% toward the default foreground."
  (or (romacs-chrome--hex (face-foreground 'romacs-pane-border nil t))
      (let ((bg (color-values (face-background 'default nil t)))
            (fg (color-values (face-foreground 'default nil t)))
            (mixed nil))
        (when (and bg fg)
          (while bg
            (push (round (+ (* 0.85 (ash (pop bg) -8)) (* 0.15 (ash (pop fg) -8)))) mixed))
          (apply #'format "#%02x%02x%02x" (nreverse mixed))))))

(defun romacs-chrome--parameters (classic)
  "The root frame's chrome parameters, GNU's look when CLASSIC."
  (let ((gap (if classic 0 romacs-pane-gap)))
    `((right-divider-width . ,gap)
      (bottom-divider-width . ,gap)
      (internal-border-width . ,gap)
      (romacs-pane-radius . ,(if classic 0 romacs-pane-radius))
      (romacs-pane-dim . ,(if classic 0.0 (float romacs-pane-dim)))
      (romacs-pane-focus . ,(and (not classic) (romacs-chrome--hex (face-foreground 'romacs-pane-focus nil t))))
      (romacs-pane-border . ,(and (not classic) (romacs-chrome--border))))))

(defvar romacs-classic-chrome)
(defvar romacs-status-bar-mode)
(defvar romacs-mode-line-mode)

(defvar romacs-chrome--classic-off nil
  "The modes classic chrome turned off, to turn on again after it.")

(defvar romacs-chrome--toggling nil
  "Non-nil while classic chrome turns a mode off or on itself.")

(defun romacs-chrome--classic-modes (classic)
  "Turn Romacs's status bar and mode line off for CLASSIC chrome, and on
again after it those classic chrome turned off."
  (let ((romacs-chrome--toggling t))
    (dolist (mode '(romacs-status-bar-mode romacs-mode-line-mode))
      (when (fboundp mode)
        (cond ((and classic (symbol-value mode))
               (push mode romacs-chrome--classic-off)
               (funcall mode -1))
              ((and (not classic) (memq mode romacs-chrome--classic-off))
               (setq romacs-chrome--classic-off (delq mode romacs-chrome--classic-off))
               (funcall mode 1)))))))

(defun romacs-chrome--chosen (mode)
  "MODE was turned on or off by its user, not by classic chrome: classic
chrome's end leaves it as chosen."
  (unless romacs-chrome--toggling
    (setq romacs-chrome--classic-off (delq mode romacs-chrome--classic-off))))

(add-hook 'romacs-status-bar-mode-hook (lambda () (romacs-chrome--chosen 'romacs-status-bar-mode)))
(add-hook 'romacs-mode-line-mode-hook (lambda () (romacs-chrome--chosen 'romacs-mode-line-mode)))

(defcustom romacs-pane-label 'name
  "What each window's own mode-line band shows while the status bar
shows the selected window's mode line: `name', the window's label
\(`romacs-pane-label-format') in `mode-line-inactive'; `full', its
mode line as in GNU; nil, no band, so its text takes that line too.
A window GNU draws no mode line for has none under any value."
  :type '(choice (const name) (const full) (const :tag "No band" nil))
  :initialize #'custom-initialize-default
  :set #'romacs-chrome--set)

(defvar romacs-pane-label-format '("" romacs-mode-line-mode-icon "%b")
  "The mode-line format of a window's band under the pane label `name'.")
(put 'romacs-pane-label-format 'risky-local-variable t)

(defun romacs-chrome--status-bar-parameters (classic)
  "The root frame's status-bar parameters, none when CLASSIC."
  (let ((bar (and (not classic) (bound-and-true-p romacs-status-bar-mode))))
    `((romacs-status-bar-lines . ,(if bar 1 0))
      (romacs-pane-label . ,(if bar romacs-pane-label 'full)))))

(defun romacs-chrome-apply (&rest _)
  "Apply the chrome options to the root frame, on a window system only.
Also run from `enable-theme-functions' and `after-init-hook', so the
colours follow a theme and the faces an init sets."
  (let ((frame (romacs-chrome--root-frame)))
    (when (window-system frame)
      (let ((classic (bound-and-true-p romacs-classic-chrome)))
        (modify-frame-parameters frame (append (romacs-chrome--parameters classic)
                                               (romacs-chrome--status-bar-parameters classic)))))))

(define-minor-mode romacs-classic-chrome
  "Draw the root frame's windows as GNU does: no gaps, corners, hairlines,
focus edge or dimming, no status bar, and `window-divider-mode' off.
Off, the `romacs-pane-' options and `romacs-status-bar-mode' apply
again."
  :global t
  :group 'romacs-chrome
  (when (and romacs-classic-chrome (bound-and-true-p window-divider-mode))
    (window-divider-mode -1))
  (romacs-chrome--classic-modes romacs-classic-chrome)
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
          :focus (frame-parameter frame 'romacs-pane-focus)
          :border (frame-parameter frame 'romacs-pane-border)
          :status-bar (or (frame-parameter frame 'romacs-status-bar-lines) 0)
          :status-bar-height (romacs-status-bar-height)
          :label (if (assq 'romacs-pane-label (frame-parameters frame))
                     (frame-parameter frame 'romacs-pane-label)
                   'full))))

(add-hook 'enable-theme-functions #'romacs-chrome-apply)
(add-hook 'after-init-hook #'romacs-chrome-apply)

(provide 'romacs-chrome)
;;; romacs-chrome.el ends here

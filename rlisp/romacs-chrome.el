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
;; The status bar between the windows and the minibuffer line is the
;; mode line: the mode line of the window the selected one is
;; (`romacs-status-bar-window'), with the lines of other visible windows
;; that set their own folded in; no window draws a band of its own.  It
;; is at the bottom, or under the tab bar (`romacs-status-bar-position');
;; layers and toasts keep clear of it (`romacs-chrome-bottom',
;; `romacs-chrome-top').
;;
;; `romacs-classic-chrome' sets every pane value to 0: GNU's look, with
;; the geometry of a GNU frame without dividers or border, and the bar.
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

(defcustom romacs-status-bar-position 'bottom
  "Where the status bar, the mode line, is: `bottom', between the windows
and the minibuffer line, or `top', under the tab bar and above the
windows.  Setting it moves the bar at once."
  :type '(choice (const bottom) (const top))
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

(defconst romacs-style-flat
  '(:radius-pane 6 :radius-layer 8 :radius-band 4
    :shadow-layer (18 6 0.55) :glow-layer (14 0.35) :dim-inactive 0.12)
  "The `flat' style's tokens, which every style starts from: corners in
pixels, a layer's shadow as (REACH DROP OPACITY) and its glow as (REACH
OPACITY), and how far an unselected pane is blended into the frame.")

(defconst romacs-style-modern nil
  "What the `modern' style changes of `romacs-style-flat'.")

(defcustom romacs-style-overrides nil
  "Style tokens, a property list, over the enabled theme's style."
  :type 'plist
  :initialize #'custom-initialize-default
  :set #'romacs-chrome--set)

(defvar romacs-style--resolved romacs-style-flat
  "The tokens last published (`romacs-chrome--style-sync').")

(defun romacs-style--scheme ()
  "The enabled colour scheme of highest precedence, or nil."
  (let ((found nil))
    (when (fboundp 'romacs-theme--colour-scheme-p)
      (dolist (theme (and (boundp 'custom-enabled-themes) custom-enabled-themes))
        (when (and (null found) (not (eq theme 'user)) (romacs-theme--colour-scheme-p theme))
          (setq found theme))))
    found))

(defun romacs-style--tokens (style)
  "The tokens STYLE, a theme's declared `romacs-style', sets over flat's: a
style's name or a property list; a theme that declares none is `modern'."
  (cond ((memq style '(nil modern)) romacs-style-modern)
        ((eq style 'flat) nil)
        (t style)))

(defun romacs-style--over (&rest styles)
  "STYLES, property lists, each later one's tokens over the earlier ones'."
  (let ((merged nil))
    (dolist (style styles merged)
      (setq merged (copy-sequence merged))
      (while style
        (setq merged (plist-put merged (car style) (car (cdr style)))
              style (cdr (cdr style)))))))

(defun romacs-style-resolve ()
  "The style in force: `romacs-style-flat', under the enabled colour
scheme's own, under `romacs-style-overrides'."
  (let ((scheme (romacs-style--scheme)))
    (romacs-style--over romacs-style-flat
                        (romacs-style--tokens (and scheme (get scheme 'romacs-style)))
                        romacs-style-overrides)))

(defun romacs-style--wire (style)
  "STYLE as the host reads it from the `romacs-style' frame parameter."
  (vconcat (list (plist-get style :radius-layer) (plist-get style :radius-band))
           (plist-get style :shadow-layer) (plist-get style :glow-layer)))

(defun romacs-style--option (option token style)
  "OPTION's value where the user changed it from flat's TOKEN, else STYLE's."
  (let ((value (symbol-value option)))
    (if (equal value (plist-get romacs-style-flat token)) (plist-get style token) value)))

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
       (romacs-complete-current :background ,(romacs-chrome-mix 0.2) :extend t)
       (romacs-mode-line-mark :foreground ,(romacs-chrome--mark-color))))))

(defconst romacs-chrome-mark-red "#ff5f59"
  "The Romacs mark's red (assets/branding.md: Modus Vivendi's red).")

(defun romacs-chrome--mark-color ()
  "The colour Romacs's R draws in on the status bar: the mark's red where
it reaches the 3:1 contrast WCAG asks of graphics on the bar's background,
else the bar's foreground."
  (let* ((bg (face-background 'mode-line-active nil t))
         ;; A named colour (tango's "grey75") as #rrggbb, which the contrast takes.
         (hex (and (stringp bg) (color-defined-p bg)
                   (apply #'format "#%02x%02x%02x" (mapcar (lambda (c) (/ c 256)) (color-values bg))))))
    (if (and hex (fboundp 'romacs-color-contrast)
             (>= (romacs-color-contrast romacs-chrome-mark-red hex) 3.0))
        romacs-chrome-mark-red
      (face-foreground 'mode-line-active nil t))))

(defun romacs-chrome--set-faces (&rest _)
  "Make `romacs-chrome-derived-faces' the faces' default specs.  A
theme's spec for a face still wins (faces.el `face-spec-recalc').
Run from `enable-theme-functions' and `disable-theme-functions'."
  (pcase-dolist (`(,face . ,attributes) (romacs-chrome-derived-faces))
    (let ((spec `((t ,@attributes))))
      (when (and (facep face) (not (equal spec (get face 'face-defface-spec))))
        (face-spec-set face spec 'face-defface-spec)))))

(defun romacs-chrome--parameters (classic)
  "The root frame's chrome parameters, GNU's look when CLASSIC, which no
theme's or user's tokens change."
  (let ((gap (if classic 0 romacs-pane-gap)) (style (if classic romacs-style-flat romacs-style--resolved)))
    `((right-divider-width . ,gap)
      (bottom-divider-width . ,gap)
      (internal-border-width . ,gap)
      (romacs-pane-radius . ,(if classic 0 (romacs-style--option 'romacs-pane-radius :radius-pane style)))
      (romacs-pane-dim . ,(if classic 0.0 (float (romacs-style--option 'romacs-pane-dim :dim-inactive style))))
      (romacs-pane-chrome . ,(not classic))
      (romacs-style . ,(romacs-style--wire style)))))

(defvar romacs-classic-chrome)

(defun romacs-chrome-apply (&rest _)
  "Apply the chrome options and the style in force to the root frame, on a
window system only."
  (setq romacs-style--resolved (romacs-style-resolve))
  (let ((frame (romacs-chrome--root-frame)))
    (when (window-system frame)
      (let ((classic (bound-and-true-p romacs-classic-chrome)))
        ;; The bar is the mode line, in every look (VM2-STATUS-BAR).
        (modify-frame-parameters frame `((romacs-status-bar-lines . 1)
                                         (romacs-status-bar-position . ,romacs-status-bar-position)
                                         ,@(romacs-chrome--parameters classic)))
        ;; Toasts shown keep clear of where the bar is now.
        (when (and (bound-and-true-p romacs-layer--toasts) (fboundp 'romacs-layer--toast-stack))
          (romacs-layer--toast-stack))))))

(defun romacs-chrome--style-sync (&rest _)
  "Publish the style in force, if it changed: from
`enable-theme-functions' and `disable-theme-functions'."
  (unless (equal (romacs-style-resolve) romacs-style--resolved)
    (romacs-chrome-apply)))

(defun romacs-chrome-refresh (&rest _)
  "Derive the chrome faces a theme leaves unset and apply the chrome to
the root frame: at startup and from `after-init-hook', so the chrome
follows the faces an init sets."
  (romacs-chrome--set-faces)
  (romacs-chrome-apply))

(define-minor-mode romacs-classic-chrome
  "Draw the root frame's windows as GNU does: no gaps, corners, hairlines,
focus edge or dimming, and `window-divider-mode' off; the status bar
stays the mode line.  Off, the `romacs-pane-' options apply again."
  :global t
  :group 'romacs-chrome
  (when (and romacs-classic-chrome (bound-and-true-p window-divider-mode))
    (window-divider-mode -1))
  (romacs-chrome-apply))

(defun romacs-status-bar-string (&optional _frame)
  "The text the status bar shows, nil while it shows none."
  (romacs--status-bar-text))

(defun romacs-chrome-top (&optional frame)
  "Where a layer of FRAME (default the root frame) anchored at its top
starts, in the native pixels child frames are placed in: below the tab
bar and a status bar at the top, inside the internal border."
  (let ((root (or frame (romacs-chrome--root-frame))))
    (nth 1 (window-edges (frame-root-window root) nil nil t))))

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

(defun romacs--mode-line-click (window string column type)
  "Run the binding of WINDOW's mode line for a mouse event of TYPE at
COLUMN of STRING, the text the status bar shows with the properties a
click reads: the `local-map' or `keymap' there, else the global map's,
under `mode-line'.  The event's position is the `mode-line' area of
WINDOW with STRING and COLUMN as its object, as GNU's are."
  (let* ((map (and (< column (length string))
                   (or (get-text-property column 'local-map string)
                       (get-text-property column 'keymap string))))
         (key (vector 'mode-line type))
         (command (or (and (keymapp map) (lookup-key map key))
                      (key-binding key t)))
         (event (list type (list window 'mode-line (cons (* column (frame-char-width)) 0)
                                 0 (cons string column) nil (cons column 0) nil (cons 0 0)
                                 (cons (frame-char-width) (frame-char-height))))))
    (when (and (window-live-p window) (commandp command))
      (setq last-input-event event
            last-nonmenu-event event
            last-command-event event
            this-command command)
      (call-interactively command nil (vector 'mode-line event))
      (setq last-command command))))

(defun romacs--status-bar-click (column type)
  "Run the binding of the status bar's mode line for a mouse event of
TYPE at the bar's shown character COLUMN: `romacs--mode-line-click' of
the window whose line it is, at that column of the bar's text
\(`romacs--status-bar-target'); a separator runs nothing."
  (when-let* ((target (romacs--status-bar-target column)))
    (romacs--mode-line-click (nth 0 target) (nth 1 target) (nth 2 target) type)))

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
          :status-bar-position (if (eq (frame-parameter frame 'romacs-status-bar-position) 'top) 'top 'bottom))))

(add-hook 'enable-theme-functions #'romacs-chrome--set-faces)
(add-hook 'disable-theme-functions #'romacs-chrome--set-faces)
(add-hook 'enable-theme-functions #'romacs-chrome--style-sync)
(add-hook 'disable-theme-functions #'romacs-chrome--style-sync)
(add-hook 'after-init-hook #'romacs-chrome-refresh)

(provide 'romacs-chrome)
;;; romacs-chrome.el ends here

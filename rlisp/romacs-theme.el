;;; romacs-theme.el --- Themes from one palette: colours, slots, faces  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Portions Copyright (C) 2010-2026 Free Software Foundation, Inc., from
;; GNU Emacs lisp/color.el (the sRGB, CIE XYZ and Oklab conversions), kept
;; as the GPL requires.

;;; Commentary:
;; A Romacs theme is one palette: an alist of base colours in the
;; vocabulary of Omarchy's colors.toml, `((SLOT VALUE) ...)', VALUE a
;; "#rrggbb" string or another slot's name.  Every other colour a theme
;; needs (borders, glass, bands, highlights, search and diff tints, the
;; terminal's sixteen, the font-lock roles) is derived from the base
;; slots in Oklab by the rules of `romacs-theme-rules', and a palette may
;; pin any of them by naming it.  `romacs-theme-faces' turns a resolved
;; palette into the face list, and `romacs-theme-define' makes a theme
;; of it: GNU's `deftheme' with those faces where Custom is loaded, the
;; faces' own specs where it is not (a launch without GNU's Lisp).
;;
;; Everything here is plain Elisp over numbers and strings, without
;; color.el, cl-lib or seq: it also runs in the launches whose only Lisp
;; is the machine's own prelude.

;;; Code:

;;;; Colours

(defun romacs-theme--second (list)
  "LIST's second element (the machine's own prelude has no `cadr')."
  (car (cdr list)))

(defun romacs-color--channels (hex)
  "HEX, \"#rrggbb\", as (R G B), each from 0.0 to 1.0."
  (list (/ (string-to-number (substring hex 1 3) 16) 255.0)
        (/ (string-to-number (substring hex 3 5) 16) 255.0)
        (/ (string-to-number (substring hex 5 7) 16) 255.0)))

(defun romacs-color--hex (rgb)
  "RGB, three numbers from 0.0 to 1.0 (clamped), as \"#rrggbb\"."
  (apply #'format "#%02x%02x%02x"
         (mapcar (lambda (c) (round (* 255 (min 1.0 (max 0.0 c))))) rgb)))

(defun romacs-color--linear (c)
  "sRGB channel C made linear, as color.el's `color-srgb-to-xyz'."
  (if (<= c 0.04045) (/ c 12.95) (expt (/ (+ c 0.055) 1.055) 2.4)))

(defun romacs-color--gamma (c)
  "Linear channel C as sRGB, as color.el's `color-xyz-to-srgb'."
  (if (<= c 0.0031308) (* 12.92 c) (- (* 1.055 (expt c (/ 2.4))) 0.055)))

(defun romacs-color--cbrt (f)
  "The real cube root of F."
  (if (< f 0) (- (expt (- f) (/ 1.0 3.0))) (expt f (/ 1.0 3.0))))

(defun romacs-color-oklab (hex)
  "HEX as Oklab (L A B), through CIE XYZ as color.el's `color-srgb-to-oklab'."
  (let* ((rgb (mapcar #'romacs-color--linear (romacs-color--channels hex)))
         (r (nth 0 rgb)) (g (nth 1 rgb)) (b (nth 2 rgb))
         (x (+ (* 0.4124564 r) (* 0.3575761 g) (* 0.1804375 b)))
         (y (+ (* 0.21266729 r) (* 0.7151522 g) (* 0.0721750 b)))
         (z (+ (* 0.0193339 r) (* 0.1191920 g) (* 0.9503041 b)))
         (l (romacs-color--cbrt (+ (* x 0.8189330101) (* y 0.3618667424) (* z -0.1288597137))))
         (m (romacs-color--cbrt (+ (* x 0.0329845436) (* y 0.9293118715) (* z 0.0361456387))))
         (s (romacs-color--cbrt (+ (* x 0.0482003018) (* y 0.2643662691) (* z 0.6338517070)))))
    (list (+ (* l 0.2104542553) (* m 0.7936177850) (* s -0.0040720468))
          (+ (* l 1.9779984951) (* m -2.4285922050) (* s 0.4505937099))
          (+ (* l 0.0259040371) (* m 0.7827717662) (* s -0.8086757660)))))

(defun romacs-color--oklab-linear (lab)
  "Oklab LAB as linear sRGB (R G B), not clamped, as color.el's
`color-oklab-to-xyz' and `color-xyz-to-srgb' before the transfer curve."
  (let* ((L (nth 0 lab)) (a (nth 1 lab)) (b (nth 2 lab))
         (l (expt (+ (* 1.0 L) (* 0.39633779 a) (* 0.21580376 b)) 3))
         (m (expt (+ (* 1.00000001 L) (* -0.10556134 a) (* -0.06385417 b)) 3))
         (s (expt (+ (* 1.00000005 L) (* -0.08948418 a) (* -1.29148554 b)) 3))
         (x (+ (* l 1.22701385) (* m -0.55779998) (* s 0.28125615)))
         (y (+ (* l -0.04058018) (* m 1.11225687) (* s -0.07167668)))
         (z (+ (* l -0.07638128) (* m -0.42148198) (* s 1.58616322))))
    (list (+ (* 3.2404542 x) (* -1.5371385 y) (* -0.4985314 z))
          (+ (* -0.9692660 x) (* 1.8760108 y) (* 0.0415560 z))
          (+ (* 0.0556434 x) (* -0.2040259 y) (* 1.0572252 z)))))

(defun romacs-color-from-oklab (lab)
  "Oklab LAB as \"#rrggbb\", each channel clamped into sRGB."
  (romacs-color--hex (mapcar (lambda (c) (romacs-color--gamma (min 1.0 (max 0.0 c))))
                             (romacs-color--oklab-linear lab))))

(defun romacs-color--in-gamut (lab)
  "Whether Oklab LAB lies inside sRGB."
  (let ((inside t))
    (dolist (c (romacs-color--oklab-linear lab) inside)
      (when (or (< c -0.0001) (> c 1.0001)) (setq inside nil)))))

(defun romacs-color--oklch (L chroma hue)
  "The Oklab colour of lightness L, CHROMA and HUE (radians), brought into
sRGB by reducing its chroma, hue and lightness kept."
  (let ((lab (list L (* chroma (cos hue)) (* chroma (sin hue)))))
    (while (and (> chroma 0.0001) (null (romacs-color--in-gamut lab)))
      (setq chroma (* chroma 0.95)
            lab (list L (* chroma (cos hue)) (* chroma (sin hue)))))
    lab))

(defun romacs-color-mix (a b amount)
  "The colour AMOUNT (0 to 1) of the way from A to B, mixed in Oklab."
  (let ((la (romacs-color-oklab a)) (lb (romacs-color-oklab b)))
    (romacs-color-from-oklab
     (list (+ (nth 0 la) (* amount (- (nth 0 lb) (nth 0 la))))
           (+ (nth 1 la) (* amount (- (nth 1 lb) (nth 1 la))))
           (+ (nth 2 la) (* amount (- (nth 2 lb) (nth 2 la))))))))

(defun romacs-color--lch (hex)
  "HEX as (L CHROMA HUE), Oklab's polar form."
  (let ((lab (romacs-color-oklab hex)))
    (list (nth 0 lab) (sqrt (+ (* (nth 1 lab) (nth 1 lab)) (* (nth 2 lab) (nth 2 lab))))
          (atan (nth 2 lab) (nth 1 lab)))))

(defun romacs-color-lift (hex delta)
  "HEX with its Oklab lightness moved by DELTA, hue and chroma kept as far
as sRGB allows."
  (let ((lch (romacs-color--lch hex)))
    (romacs-color-from-oklab
     (romacs-color--oklch (min 1.0 (max 0.0 (+ (nth 0 lch) delta))) (nth 1 lch) (nth 2 lch)))))

(defun romacs-color-chroma (hex factor)
  "HEX with its Oklab chroma scaled by FACTOR."
  (let ((lch (romacs-color--lch hex)))
    (romacs-color-from-oklab (romacs-color--oklch (nth 0 lch) (* factor (nth 1 lch)) (nth 2 lch)))))

(defun romacs-color-luminance (hex)
  "HEX's relative luminance, as WCAG 2 defines it."
  (let ((lin (mapcar (lambda (c) (if (<= c 0.03928) (/ c 12.92) (expt (/ (+ c 0.055) 1.055) 2.4)))
                     (romacs-color--channels hex))))
    (+ (* 0.2126 (nth 0 lin)) (* 0.7152 (nth 1 lin)) (* 0.0722 (nth 2 lin)))))

(defun romacs-color-contrast (a b)
  "The WCAG 2 contrast ratio of colours A and B, from 1 to 21."
  (let ((x (romacs-color-luminance a)) (y (romacs-color-luminance b)))
    (/ (+ (max x y) 0.05) (+ (min x y) 0.05))))

(defun romacs-color--worst (fg backgrounds)
  "FG's lowest contrast ratio on any of BACKGROUNDS."
  (let ((worst nil))
    (dolist (bg backgrounds worst)
      (let ((ratio (romacs-color-contrast fg bg)))
        (when (or (null worst) (< ratio worst)) (setq worst ratio))))))

(defun romacs-color--toward (fg backgrounds minimum step)
  "FG moved in Oklab lightness by STEP at a time, endpoint included, until
it shows on every one of BACKGROUNDS at the contrast MINIMUM: the first
colour that does, else the endpoint."
  (let* ((lch (romacs-color--lch fg)) (L (nth 0 lch)) (moved fg))
    (while (and (< (romacs-color--worst moved backgrounds) minimum)
                (if (> step 0) (< L 1.0) (> L 0.0)))
      (setq L (min 1.0 (max 0.0 (+ L step)))
            moved (romacs-color-from-oklab (romacs-color--oklch L (nth 1 lch) (nth 2 lch)))))
    moved))

(defun romacs-color-floor-on (fg backgrounds minimum)
  "FG, or when it shows below the contrast MINIMUM on any of BACKGROUNDS,
the colours it is drawn over, FG moved in Oklab lightness until it
reaches it on all of them: away from the one it shows worst on first,
then the other way; when neither reaches it, the end that comes closest.
Only the foreground moves."
  (if (>= (romacs-color--worst fg backgrounds) minimum) fg
    (let* ((worst (car backgrounds)))
      (dolist (bg (cdr backgrounds))
        (when (< (romacs-color-contrast fg bg) (romacs-color-contrast fg worst)) (setq worst bg)))
      (let* ((Lf (nth 0 (romacs-color-oklab fg))) (Lb (nth 0 (romacs-color-oklab worst)))
             (up (if (= Lf Lb) (< Lb 0.5) (> Lf Lb)))
             (first (romacs-color--toward fg backgrounds minimum (if up 0.01 -0.01))))
        (if (>= (romacs-color--worst first backgrounds) minimum) first
          (let ((second (romacs-color--toward fg backgrounds minimum (if up -0.01 0.01))))
            (if (>= (romacs-color--worst second backgrounds) minimum) second
              (romacs-color--scan fg backgrounds minimum))))))))

(defun romacs-color--scan (fg backgrounds minimum)
  "FG at the Oklab lightness, in steps of 0.002 over the whole range, that
shows on every one of BACKGROUNDS at the contrast MINIMUM nearest FG's
own, else the one whose worst ratio is highest: the colours that fit can
be a band too narrow for `romacs-color--toward's steps."
  (let* ((lch (romacs-color--lch fg)) (own (nth 0 lch))
         (best nil) (best-worst -1) (best-distance 2.0) (i 0))
    (while (<= i 500)
      (let* ((L (/ i 500.0))
             (colour (romacs-color-from-oklab (romacs-color--oklch L (nth 1 lch) (nth 2 lch))))
             (worst (romacs-color--worst colour backgrounds))
             (distance (abs (- L own))))
        (cond ((and (>= worst minimum) (or (< best-worst minimum) (< distance best-distance)))
               (setq best colour best-worst worst best-distance distance))
              ((and (< best-worst minimum) (> worst best-worst))
               (setq best colour best-worst worst best-distance distance))))
      (setq i (1+ i)))
    (if (>= best-worst minimum) best
      ;; Two samples either side: a band between samples can begin a step
      ;; beyond the one the best sample lies next to.
      (romacs-color--refine lch (- (nth 0 (romacs-color--lch best)) 0.004) (+ (nth 0 (romacs-color--lch best)) 0.004)
                            backgrounds best))))

(defun romacs-color--refine (lch low high backgrounds best)
  "The colour of LCH's chroma and hue whose lightness, between LOW and
HIGH in steps of 0.00002, shows best on its worst of BACKGROUNDS, or BEST
when none beats it.  The steps are finer than a change of 8-bit colour,
so no colour between LOW and HIGH is skipped: the worst ratio is a
staircase, which no bisection can climb."
  (let ((L (max 0.0 low)) (high (min 1.0 high)) (score (romacs-color--worst best backgrounds)))
    (while (<= L high)
      (let* ((colour (romacs-color-from-oklab (romacs-color--oklch L (nth 1 lch) (nth 2 lch))))
             (worst (romacs-color--worst colour backgrounds)))
        (when (> worst score) (setq best colour score worst)))
      (setq L (+ L 0.00002)))
    best))

(defun romacs-color-floor (fg bg minimum)
  "FG floored at the contrast MINIMUM on BG (`romacs-color-floor-on')."
  (romacs-color-floor-on fg (list bg) minimum))

;;;; The palette

(defconst romacs-theme-base-slots
  '(mode bg bg-dark bg-darker bg-alt fg fg-dim fg-light fg-bright accent selection muted
    red yellow orange green cyan blue magenta brown
    bright-red bright-yellow bright-green bright-cyan bright-blue bright-magenta)
  "The base slots of a palette, Omarchy colors.toml's keys: `mode' (`dark'
or `light') and colours.  `orange' and `brown' may be left out.")

(defun romacs-theme--accent-alt (get)
  "The secondary accent: `blue', or `magenta' when `accent' is a blue
(their Oklab hues within 40 degrees).  GET reads a slot."
  (let ((da (- (nth 2 (romacs-color--lch (funcall get 'accent)))
               (nth 2 (romacs-color--lch (funcall get 'blue))))))
    (if (< (abs (atan (sin da) (cos da))) (* 40 (/ 3.141592653589793 180))) (funcall get 'magenta) (funcall get 'blue))))

(defconst romacs-theme-rules
  '((cursor . fg-bright)
    (accent-alt . romacs-theme--accent-alt)
    (orange mix yellow red 0.5)
    (brown mix fg-dim red 0.5)
    ;; Chrome.
    (border mix bg fg 0.15)
    (glass mix bg fg 0.08)
    (hover lift-away bg-alt 0.06)
    (glow . accent)
    (glow-soft chroma accent 0.6)
    ;; Text that must stay readable.
    (fg-muted floor muted bg 3.0)
    (fg-dim* floor fg-dim bg 4.5)
    (prompt . accent)
    ;; A key in help, popups and the completion layer: a cool blue, as
    ;; Modus keys are, floored on each of their backgrounds
    ;; (`help-key-binding').
    (keybind . blue)
    ;; Bands, highlights, search, diff.
    (bg-mode-line . bg-alt)
    (bg-mode-line-inactive . bg-dark)
    (fg-mode-line-inactive . fg-muted)
    (bg-region . selection)
    (bg-highlight mix bg selection 0.5)
    (bg-search mix bg yellow 0.35)
    (bg-search-current mix bg accent 0.5)
    (bg-match mix bg accent 0.3)
    (bg-paren mix bg magenta 0.5)
    (bg-block . bg-dark)
    (bg-block-delimiter . bg-alt)
    (diff-add mix bg green 0.2)
    (diff-remove mix bg red 0.2)
    (diff-change mix bg yellow 0.2)
    (diff-add-refine mix bg green 0.35)
    (diff-remove-refine mix bg red 0.35)
    (diff-change-refine mix bg yellow 0.35)
    ;; Meanings.
    (error . red)
    (warning . yellow)
    (success . green)
    (info . blue)
    (link . accent-alt)
    ;; Font-lock roles.
    (keyword . magenta)
    (function . bright-magenta)
    (variable . cyan)
    (type . bright-cyan)
    (constant . bright-blue)
    (string . green)
    (comment . fg-muted)
    (builtin . bright-magenta)
    (doc . fg-light)
    (preprocessor . orange)
    (negation . red)
    (fl-warning . yellow)
    (regexp . accent)
    ;; The terminal's sixteen, Omarchy's own mapping.
    (ansi-0 . bg) (ansi-1 . red) (ansi-2 . green) (ansi-3 . yellow)
    (ansi-4 . blue) (ansi-5 . magenta) (ansi-6 . cyan) (ansi-7 . fg)
    (ansi-8 . muted) (ansi-9 . bright-red) (ansi-10 . bright-green) (ansi-11 . bright-yellow)
    (ansi-12 . bright-blue) (ansi-13 . bright-magenta) (ansi-14 . bright-cyan) (ansi-15 . fg-bright))
  "How each slot a palette leaves out is derived: (SLOT . OTHER) takes
OTHER's colour; (SLOT . FUNCTION) calls FUNCTION with a slot reader;
\(SLOT OP ARGS...) applies OP to ARGS, slot names or numbers: `mix' A B
AMOUNT (`romacs-color-mix'), `floor' FG BG MINIMUM (`romacs-color-floor'),
`chroma' C FACTOR, and `lift-away' C DELTA (lighter on a dark palette,
darker on a light one).")

(defconst romacs-theme-slots
  (append romacs-theme-base-slots (mapcar #'car romacs-theme-rules))
  "Every slot a resolved palette holds.")

(defun romacs-theme-resolve (palette)
  "PALETTE with every slot of `romacs-theme-slots', as an alist
\(SLOT . VALUE): a slot PALETTE names is its value (a slot name in its
place is that slot's; `mode' is a symbol), every other one is derived by
`romacs-theme-rules'."
  (let ((done nil) (busy nil) (get nil))
    (setq get (lambda (slot)
                (let ((known (assq slot done)))
                  (if known (cdr known)
                    (when (memq slot busy) (error "Palette slot `%s' refers to itself" slot))
                    (push slot busy)
                    (let* ((given (assq slot palette))
                           (rule (cdr (assq slot romacs-theme-rules)))
                           (value (cond ((and given (eq slot 'mode)) (romacs-theme--second given))
                                        (given (let ((v (romacs-theme--second given))) (if (symbolp v) (funcall get v) v)))
                                        ((null rule) (error "Palette slot `%s' is missing" slot))
                                        ((symbolp rule) (if (memq rule romacs-theme-slots) (funcall get rule) (funcall rule get)))
                                        (t (romacs-theme--apply rule get)))))
                      (push (cons slot value) done)
                      value)))))
    (mapcar (lambda (slot) (cons slot (funcall get slot))) romacs-theme-slots)))

(defun romacs-theme--apply (rule get)
  "Apply RULE, (OP ARGS...) of `romacs-theme-rules', reading slots by GET."
  (let ((args (mapcar (lambda (a) (if (symbolp a) (funcall get a) a)) (cdr rule))))
    (cond ((eq (car rule) 'mix) (apply #'romacs-color-mix args))
          ((eq (car rule) 'floor) (apply #'romacs-color-floor args))
          ((eq (car rule) 'chroma) (apply #'romacs-color-chroma args))
          ((eq (car rule) 'lift-away)
           (romacs-color-lift (nth 0 args) (if (eq (funcall get 'mode) 'light) (- (nth 1 args)) (nth 1 args))))
          (t (error "Unknown palette rule %S" rule)))))

;;;; Faces

(defun romacs-theme--face (face &rest attributes)
  "FACE's row: ATTRIBUTES under every display."
  (list face (list (cons t attributes))))

(defun romacs-theme-faces (palette)
  "The faces of resolved PALETTE (`romacs-theme-resolve'), as
`custom-theme-set-faces' takes them: ((FACE ((t ATTRIBUTES))) ...).
Each text colour is floored against the backgrounds it shows on: 4.5:1
for text, 3:1 for de-emphasised text.  A row sets colours; where GNU's
own spec gives a face its font by inheritance (`fixed-pitch',
`variable-pitch', the mode line's), the row keeps it, since a theme's
spec replaces the face's own."
  (let* ((v (lambda (slot) (cdr (assq slot palette))))
         (bg (funcall v 'bg))
         (text (lambda (slot &optional on)
                 (romacs-color-floor-on (funcall v slot) (if (listp on) (or on (list bg)) (list on)) 4.5)))
         (dim (lambda (slot &optional on) (romacs-color-floor (funcall v slot) (or on bg) 3.0)))
         (face #'romacs-theme--face))
    (append
     ;; Core.
     (list
      (funcall face 'default :foreground (funcall text 'fg) :background bg)
      (funcall face 'cursor :background (funcall v 'cursor))
      (funcall face 'region :background (funcall v 'bg-region) :extend t)
      (funcall face 'secondary-selection :background (funcall v 'bg-highlight) :extend t)
      (funcall face 'highlight :background (funcall v 'bg-highlight))
      (funcall face 'hl-line :background (funcall v 'bg-highlight) :extend t)
      (funcall face 'fringe :foreground bg :background bg)
      (funcall face 'window-divider :foreground bg :background bg)
      (funcall face 'window-divider-first-pixel :foreground bg :background bg)
      (funcall face 'window-divider-last-pixel :foreground bg :background bg)
      (funcall face 'vertical-border :foreground (funcall v 'border))
      (funcall face 'internal-border :background bg)
      (funcall face 'child-frame-border :background (funcall v 'border))
      (funcall face 'border :background (funcall v 'border))
      (funcall face 'minibuffer-prompt :foreground (funcall text 'prompt))
      (funcall face 'mode-line :foreground (funcall text 'fg (funcall v 'bg-mode-line)) :background (funcall v 'bg-mode-line))
      (funcall face 'mode-line-active :inherit 'mode-line)
      (funcall face 'mode-line-inactive :foreground (funcall dim 'fg-mode-line-inactive (funcall v 'bg-mode-line-inactive))
               :background (funcall v 'bg-mode-line-inactive))
      (funcall face 'mode-line-buffer-id :foreground (funcall text 'fg-bright (funcall v 'bg-mode-line)) :weight 'bold)
      (funcall face 'mode-line-emphasis :foreground (funcall text 'accent (funcall v 'bg-mode-line)))
      (funcall face 'mode-line-highlight :background (funcall v 'hover))
      (funcall face 'header-line :inherit 'mode-line :foreground (funcall text 'fg (funcall v 'bg-alt)) :background (funcall v 'bg-alt))
      (funcall face 'tab-bar :inherit 'variable-pitch :foreground (funcall text 'fg (funcall v 'bg-dark)) :background (funcall v 'bg-dark))
      (funcall face 'tab-line :inherit 'variable-pitch :foreground (funcall text 'fg (funcall v 'bg-dark)) :background (funcall v 'bg-dark))
      (funcall face 'line-number :inherit 'default :foreground (funcall dim 'fg-muted))
      ;; The current line's number in the bright cyan, bold; no band.
      (funcall face 'line-number-current-line :inherit '(bold default) :foreground (funcall text 'cyan))
      (funcall face 'show-paren-match :background (funcall v 'bg-paren))
      (funcall face 'show-paren-mismatch :foreground (funcall v 'bg) :background (funcall v 'error))
      (funcall face 'isearch :foreground (funcall text 'fg-bright (funcall v 'bg-search-current)) :background (funcall v 'bg-search-current))
      (funcall face 'isearch-fail :foreground (funcall text 'error))
      (funcall face 'lazy-highlight :foreground (funcall text 'fg-bright (funcall v 'bg-search)) :background (funcall v 'bg-search))
      (funcall face 'match :background (funcall v 'bg-match))
      (funcall face 'error :foreground (funcall text 'error))
      (funcall face 'warning :foreground (funcall text 'warning))
      (funcall face 'success :foreground (funcall text 'success))
      (funcall face 'link :foreground (funcall text 'link) :underline t)
      (funcall face 'link-visited :foreground (funcall text 'magenta) :underline t)
      (funcall face 'button :inherit 'link)
      (funcall face 'shadow :foreground (funcall dim 'fg-muted))
      (funcall face 'trailing-whitespace :background (funcall v 'red))
      (funcall face 'escape-glyph :foreground (funcall text 'orange))
      (funcall face 'homoglyph :foreground (funcall text 'orange))
      (funcall face 'nobreak-space :foreground (funcall text 'orange) :underline t)
      (funcall face 'tooltip :inherit 'variable-pitch :foreground (funcall text 'fg (funcall v 'glass)) :background (funcall v 'glass))
      ;; Keys stand out by weight and colour, on no box or band of their own.
      (funcall face 'help-key-binding :inherit '(bold fixed-pitch)
               :foreground (funcall text 'keybind (list bg (funcall v 'bg-alt) (funcall v 'glass))))
      (funcall face 'completions-first-difference :foreground (funcall text 'fg-bright))
      (funcall face 'completions-annotations :foreground (funcall dim 'fg-muted))
      (funcall face 'completions-highlight :background (funcall v 'selection)))
     ;; Font-lock: no bold or italic constructs.
     (mapcar (lambda (pair) (funcall face (car pair) :foreground (funcall text (cdr pair))))
             '((font-lock-keyword-face . keyword) (font-lock-function-name-face . function)
               (font-lock-function-call-face . function) (font-lock-variable-name-face . variable)
               (font-lock-variable-use-face . variable) (font-lock-type-face . type)
               (font-lock-constant-face . constant) (font-lock-string-face . string)
               (font-lock-builtin-face . builtin) (font-lock-doc-face . doc)
               (font-lock-preprocessor-face . preprocessor) (font-lock-negation-char-face . negation)
               (font-lock-warning-face . fl-warning) (font-lock-regexp-grouping-backslash . regexp)
               (font-lock-regexp-grouping-construct . regexp) (font-lock-number-face . constant)))
     (list (funcall face 'font-lock-comment-face :foreground (funcall dim 'comment))
           (funcall face 'font-lock-comment-delimiter-face :inherit 'font-lock-comment-face))
     ;; Romacs chrome.
     (list
      (funcall face 'romacs-pane-focus :foreground (funcall v 'glow))
      (funcall face 'romacs-pane-border :foreground (funcall v 'border))
      (funcall face 'romacs-layer-glow :foreground (funcall v 'glow))
      (funcall face 'romacs-layer-glass :background (funcall v 'glass))
      (funcall face 'romacs-layer-toast :foreground (funcall text 'fg (funcall v 'glass)) :background (funcall v 'glass))
      (funcall face 'romacs-layer-selection :background (funcall v 'selection))
      (funcall face 'romacs-landing-mark :foreground (funcall v 'glow))
      (funcall face 'romacs-landing-word :foreground (funcall v 'fg-bright))
      (funcall face 'romacs-complete-current :background (funcall v 'selection) :extend t)
      ;; Matches show on the panel and on its selected row.
      (funcall face 'romacs-complete-match :foreground (funcall text 'accent (list (funcall v 'glass) (funcall v 'selection))))
      (funcall face 'romacs-complete-prompt :foreground (funcall text 'fg-dim* (funcall v 'glass)))
      (funcall face 'completions-common-part :foreground (funcall text 'accent (list bg (funcall v 'selection))))
      (funcall face 'romacs-comment :foreground (funcall dim 'comment))
      (funcall face 'romacs-code :foreground (funcall text 'string))
      (funcall face 'romacs-result :foreground (funcall text 'constant))
      (funcall face 'orderless-match-face-0 :foreground (funcall text 'accent))
      (funcall face 'orderless-match-face-1 :foreground (funcall text 'accent-alt))
      (funcall face 'orderless-match-face-2 :foreground (funcall text 'magenta))
      (funcall face 'orderless-match-face-3 :foreground (funcall text 'cyan)))
     ;; The mode line's own faces, on its band.
     (let ((band (funcall v 'bg-mode-line)))
       (list
        ;; Their own specs' inheritance stays: a theme sets colours.
        (funcall face 'romacs-mode-line-dim :inherit 'shadow :foreground (funcall text 'fg-dim* band))
        (funcall face 'romacs-mode-line-bright :inherit 'mode-line-buffer-id :foreground (funcall text 'fg-bright band))
        (funcall face 'romacs-mode-line-state :inherit '(romacs-pane-focus bold) :foreground (funcall text 'accent band))
        (funcall face 'romacs-mode-line-error :inherit 'error :foreground (funcall text 'error band))
        (funcall face 'romacs-mode-line-warning :inherit 'warning :foreground (funcall text 'warning band))
        (funcall face 'romacs-mode-line-ok :inherit 'success :foreground (funcall text 'success band))))
     ;; Headings: Rum's heights and weights (its Modus `headings': 1
     ;; ultrabold 1.35, 2 semibold 1.2, the rest Modus's bold at 1.15), on
     ;; outline and Org headings alike.
     (let ((rows nil) (level 1))
       (dolist (slot '(red yellow blue magenta green orange cyan fg-dim*))
         (let ((colour (funcall text slot))
               (height (cond ((= level 1) 1.35) ((= level 2) 1.2) (t 1.15)))
               (weight (cond ((= level 1) 'ultrabold) ((= level 2) 'semibold) (t 'bold))))
           (dolist (name (list (format "outline-%d" level) (format "org-level-%d" level)))
             (push (funcall face (intern name) :foreground colour :height height :weight weight) rows)))
         (setq level (1+ level)))
       (nreverse rows))
     ;; The terminal.
     (let ((names '(black red green yellow blue magenta cyan white)) (rows nil) (i 0))
       (dolist (name names)
         (push (funcall face (intern (format "ansi-color-%s" name))
                        :foreground (funcall v (intern (format "ansi-%d" i))) :background (funcall v (intern (format "ansi-%d" i))))
               rows)
         (push (funcall face (intern (format "ansi-color-bright-%s" name))
                        :foreground (funcall v (intern (format "ansi-%d" (+ i 8)))) :background (funcall v (intern (format "ansi-%d" (+ i 8)))))
               rows)
         (setq i (1+ i)))
       (nreverse rows))
     ;; Packages in Romacs's Lisp and daily use.
     (list
      (funcall face 'diff-added :background (funcall v 'diff-add) :extend t)
      (funcall face 'diff-removed :background (funcall v 'diff-remove) :extend t)
      (funcall face 'diff-changed :background (funcall v 'diff-change) :extend t)
      (funcall face 'diff-refine-added :background (funcall v 'diff-add-refine))
      (funcall face 'diff-refine-removed :background (funcall v 'diff-remove-refine))
      (funcall face 'diff-refine-changed :background (funcall v 'diff-change-refine))
      (funcall face 'diff-header :background (funcall v 'bg-alt) :extend t)
      (funcall face 'diff-file-header :foreground (funcall text 'fg-bright (funcall v 'bg-alt)) :background (funcall v 'bg-alt) :extend t)
      (funcall face 'diff-hunk-header :foreground (funcall text 'accent-alt (funcall v 'bg-alt)) :background (funcall v 'bg-alt) :extend t)
      (funcall face 'dired-directory :foreground (funcall text 'accent-alt))
      (funcall face 'dired-symlink :foreground (funcall text 'cyan))
      (funcall face 'dired-marked :foreground (funcall text 'yellow))
      (funcall face 'dired-flagged :foreground (funcall text 'red))
      (funcall face 'dired-header :foreground (funcall text 'accent))
      (funcall face 'org-block :background (funcall v 'bg-block) :extend t)
      (funcall face 'org-block-begin-line :background (funcall v 'bg-block-delimiter) :extend t)
      (funcall face 'org-block-end-line :inherit 'org-block-begin-line)
      (funcall face 'org-todo :foreground (funcall text 'bright-green))
      (funcall face 'org-done :foreground (funcall dim 'fg-muted))
      (funcall face 'org-date :foreground (funcall text 'cyan) :underline t)
      (funcall face 'org-link :inherit 'link)
      (funcall face 'org-table :foreground (funcall text 'fg-light))
      (funcall face 'org-code :foreground (funcall text 'string))
      (funcall face 'org-verbatim :foreground (funcall text 'constant))
      (funcall face 'compilation-error :inherit 'error)
      (funcall face 'compilation-warning :inherit 'warning)
      (funcall face 'compilation-info :inherit 'success)
      (funcall face 'flymake-error :underline (list :style 'wave :color (funcall v 'error)))
      (funcall face 'flymake-warning :underline (list :style 'wave :color (funcall v 'warning)))
      (funcall face 'flymake-note :underline (list :style 'wave :color (funcall v 'info)))
      (funcall face 'which-key-key-face :foreground (funcall text 'accent))
      (funcall face 'which-key-command-description-face :foreground (funcall text 'fg))
      (funcall face 'eshell-prompt :foreground (funcall text 'accent))
      (funcall face 'comint-highlight-prompt :foreground (funcall text 'accent))))))

;;;; Themes

;; Begin: generated by scripts/import-omarchy-palettes.
(defconst romacs-theme-bundled
  '((default dark default)
    (omarchy-catppuccin dark omarchy-catppuccin)
    (omarchy-catppuccin-latte light omarchy-catppuccin)
    (omarchy-ethereal dark omarchy-ethereal)
    (omarchy-everforest dark omarchy-everforest)
    (omarchy-flexoki-light light omarchy-flexoki-light)
    (omarchy-gruvbox dark omarchy-gruvbox)
    (omarchy-hackerman dark omarchy-hackerman)
    (omarchy-jade-horse dark omarchy-jade-horse)
    (omarchy-kanagawa dark omarchy-kanagawa)
    (omarchy-last-horizon dark omarchy-last-horizon)
    (omarchy-lumon dark omarchy-lumon)
    (omarchy-lupine light omarchy-lupine)
    (omarchy-matte-black dark omarchy-matte-black)
    (omarchy-miasma dark omarchy-miasma)
    (omarchy-nord dark omarchy-nord)
    (omarchy-retro-82 dark omarchy-retro-82)
    (omarchy-ristretto dark omarchy-ristretto)
    (omarchy-rose-pine light omarchy-rose-pine)
    (omarchy-solitude dark omarchy-solitude)
    (omarchy-starship dark omarchy-starship)
    (omarchy-tokyo-night dark omarchy-tokyo-night)
    (omarchy-vantablack dark omarchy-vantablack)
    (omarchy-white light omarchy-white))
  "The themes Romacs ships, as (NAME MODE FAMILY).")
;; End: generated by scripts/import-omarchy-palettes.

;; GNU's `theme-list-variants' reads a theme's family from its symbol, which
;; a theme not loaded yet would not have.
(dolist (entry romacs-theme-bundled)
  (put (car entry) 'theme-properties
       (list :kind 'color-scheme :background-mode (nth 1 entry) :family (nth 2 entry))))

(defun romacs-theme-define (name doc &rest properties)
  "Define theme NAME, documented by DOC, from PROPERTIES :palette (base
and pinned slots, `romacs-theme-resolve'), :mode and :family.  Where
Custom is loaded it is GNU's theme, enabled and disabled as any other;
elsewhere (a launch without GNU's Lisp) its faces are declared as the
faces' own specs.  Defining it changes no face that shows."
  (let* ((palette (let ((given (plist-get properties :palette)))
                    (if (assq 'mode given) given (cons (list 'mode (plist-get properties :mode)) given))))
         (resolved (romacs-theme-resolve palette))
         (faces (romacs-theme-faces resolved)))
    (put name 'romacs-palette palette)
    (put name 'romacs-palette-resolved resolved)
    (if (fboundp 'custom-theme-set-faces)
        (progn
          (custom-declare-theme name (intern (concat (symbol-name name) "-theme")) doc
                                (list :kind 'color-scheme :background-mode (plist-get properties :mode)
                                      :family (plist-get properties :family)))
          (apply #'custom-theme-set-faces name faces)
          (provide-theme name))
      (dolist (row faces)
        (eval (list 'defface (car row) (list 'quote (romacs-theme--with-font (car row) (romacs-theme--second row)))
                    "From the theme.")
              t)))
    name))

(defun romacs-theme--own-attributes (face)
  "The attributes FACE's own spec (`face-defface-spec') gives every display."
  (let* ((own (assq t (get face 'face-defface-spec))) (rest (cdr own)))
    (if (and rest (consp (car rest)) (null (cdr rest))) (car rest) rest)))

(defun romacs-theme--with-font (face spec)
  "SPEC as FACE's own spec where there is no Custom: for `default', after
the attributes its spec already has, the launch font, which SPEC's
colours join."
  (let ((attributes (and (eq face 'default) (romacs-theme--own-attributes face))))
    (if attributes (list (cons t (append attributes (cdr (car spec))))) spec)))

(defconst romacs-theme--font-attributes '(:family :foundry :height :weight :slant :width)
  "The attributes of the `default' face that name its font.")

(defun romacs-theme--font ()
  "The `default' face's font attributes on the selected frame now, as a
property list."
  (let ((font nil))
    (dolist (attribute romacs-theme--font-attributes font)
      (setq font (append font (list attribute (face-attribute 'default attribute)))))))

(defun romacs-theme-palette (&optional theme)
  "THEME's resolved palette (default the first enabled theme that has one)."
  (let ((theme (or theme (let ((found nil))
                           (dolist (each (and (boundp 'custom-enabled-themes) custom-enabled-themes) found)
                             (when (and (null found) (get each 'romacs-palette-resolved)) (setq found each)))))))
    (and theme (get theme 'romacs-palette-resolved))))

;;;; Palette files

(defconst romacs-theme-file-keys
  '((accent . accent) (selection . selection) (muted . muted) (background . bg)
    (dark_background . bg-dark) (darker_background . bg-darker) (lighter_background . bg-alt)
    (foreground . fg) (dark_foreground . fg-dim) (light_foreground . fg-light)
    (bright_foreground . fg-bright) (red . red) (yellow . yellow) (orange . orange)
    (green . green) (cyan . cyan) (blue . blue) (magenta . magenta) (brown . brown)
    (bright_red . bright-red) (bright_yellow . bright-yellow) (bright_green . bright-green)
    (bright_cyan . bright-cyan) (bright_blue . bright-blue) (bright_magenta . bright-magenta))
  "A palette file's keys, Omarchy colors.toml's, and the slots they name.")

(defun romacs-theme-read-palette (file)
  "The palette of FILE, KEY = \"#rrggbb\" lines in Omarchy colors.toml's
vocabulary (`romacs-theme-file-keys') and a `mode' of dark or light.
Other keys are ignored; a known key that is not #rrggbb, or another
mode, signals."
  (let ((palette nil) (mode nil))
    (with-temp-buffer
      (insert-file-contents file)
      (goto-char (point-min))
      (while (re-search-forward "^[ \t]*\\([a-z_]+\\)[ \t]*=[ \t]*\"\\([^\"]*\\)\"[ \t]*$" nil t)
        (let ((key (intern (match-string 1))) (value (match-string 2)))
          (cond ((eq key 'mode)
                 (unless (member value '("dark" "light")) (error "%s: mode %S is not dark or light" file value))
                 (setq mode (intern value)))
                ((assq key romacs-theme-file-keys)
                 (unless (string-match-p "\\`#[0-9a-fA-F]\\{6\\}\\'" value)
                   (error "%s: %s %S is not #rrggbb" file key value))
                 (push (list (cdr (assq key romacs-theme-file-keys)) (downcase value)) palette))))))
    (unless mode (error "%s: no mode" file))
    (cons (list 'mode mode) (nreverse palette))))

;; A user option where Custom is loaded (`defcustom''s expansion); the
;; launch without GNU's Lisp has no Custom and no `system' theme.
(if (fboundp 'custom-declare-variable)
    (custom-declare-variable
     'romacs-theme-system-file nil
     "A palette file behind the theme `system', or nil.
The file holds KEY = \"#rrggbb\" lines in the Omarchy colors.toml
vocabulary (see etc/themes/README.md), for example
~/.config/omarchy/current/theme/colors.toml."
     :type '(choice (const nil) file) :group 'faces)
  (defvar romacs-theme-system-file nil))

;;;; The stock theme

(defvar romacs-theme-fallback 'default
  "The theme a session starts in, which the first colour scheme enabled
over it replaces; nil for GNU's stacking.")

(defvar romacs-theme--stock nil
  "Non-nil while `romacs-theme-fallback' is enabled as the session's
stock theme rather than by choice.")

(defun romacs-theme--replace-stock (theme)
  "Run from `enable-theme-functions': THEME, a colour scheme enabled over
the stock theme, replaces it, once.  Enabling the fallback itself is a
choice and keeps it."
  (cond ((eq theme 'user))
        ((eq theme romacs-theme-fallback) (setq romacs-theme--stock nil))
        ((and romacs-theme--stock romacs-theme-fallback
              (memq romacs-theme-fallback custom-enabled-themes)
              (memq (plist-get (get theme 'theme-properties) :kind) '(nil color-scheme)))
         (setq romacs-theme--stock nil)
         (disable-theme romacs-theme-fallback))))

(defun romacs-theme-start ()
  "Enable `romacs-theme-fallback', a bundled theme, as the session's stock
theme.  Its file is loaded from `data-directory' itself rather than by
`load-theme', whose search reads the user's theme directory: this runs
in the chain a world holds, which must not keep a per-process value."
  (when romacs-theme-fallback
    (load (expand-file-name (format "themes/%s-theme.el" romacs-theme-fallback) data-directory) nil t t)
    ;; A theme sets colours.  On the frame without a window system this
    ;; chain runs on, faces.el's reset before a theme spec clears the
    ;; default face's font, which a window-system frame keeps through it
    ;; (faces.el `face-spec-reset-face'): it is put back as it was.
    (let ((font (romacs-theme--font)) (frame (selected-frame)))
      (enable-theme romacs-theme-fallback)
      (while font
        (unless (equal (face-attribute 'default (car font) frame) (romacs-theme--second font))
          (set-face-attribute 'default frame (car font) (romacs-theme--second font)))
        (setq font (cdr (cdr font)))))
    (setq romacs-theme--stock t)))

(when (boundp 'enable-theme-functions)
  (add-hook 'enable-theme-functions #'romacs-theme--replace-stock))

(provide 'romacs-theme)
;;; romacs-theme.el ends here

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

(defun romacs-color-rotate (hex target step)
  "HEX with its Oklab hue turned by at most STEP degrees toward the hue
TARGET, in degrees."
  (let* ((lch (romacs-color--lch hex))
         (radians (/ 3.141592653589793 180))
         (gap (- (* target radians) (nth 2 lch)))
         (shortest (atan (sin gap) (cos gap)))
         (limit (* step radians)))
    (romacs-color-from-oklab
     (romacs-color--oklch (nth 0 lch) (nth 1 lch) (+ (nth 2 lch) (max (- limit) (min limit shortest)))))))

(defun romacs-theme-lighten (color amount)
  "COLOR with its Oklab lightness raised by AMOUNT (0 to 1)."
  (romacs-color-lift color amount))

(defun romacs-theme-darken (color amount)
  "COLOR with its Oklab lightness lowered by AMOUNT (0 to 1)."
  (romacs-color-lift color (- amount)))

(defun romacs-theme-blend (a b amount)
  "The colour AMOUNT (0 to 1) of the way from A to B, in Oklab."
  (romacs-color-mix a b amount))

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
or `light') and colours.  Only `bg', `fg' and the six hues `red' `green'
`yellow' `blue' `magenta' `cyan' are required; `romacs-theme-rules'
derives the others.")

(defun romacs-theme--accent-alt (get)
  "The secondary accent: `blue', or `magenta' when `accent' is a blue
(their Oklab hues within 40 degrees).  GET reads a slot."
  (let ((da (- (nth 2 (romacs-color--lch (funcall get 'accent)))
               (nth 2 (romacs-color--lch (funcall get 'blue))))))
    (if (< (abs (atan (sin da) (cos da))) (* 40 (/ 3.141592653589793 180))) (funcall get 'magenta) (funcall get 'blue))))

(defconst romacs-theme-rules
  '(;; The base slots a short palette leaves out.
    (bg-dark lift-away bg -0.025)
    (bg-darker lift-away bg -0.045)
    (bg-alt lift-away bg 0.05)
    (fg-dim mix fg bg 0.45)
    (fg-light mix fg bg 0.12)
    (fg-bright lift-away fg 0.06)
    (accent . blue)
    (selection mix bg accent 0.3)
    (muted mix bg fg 0.35)
    (bright-red lift-away red 0.06)
    (bright-yellow lift-away yellow 0.06)
    (bright-green lift-away green 0.06)
    (bright-cyan lift-away cyan 0.06)
    (bright-blue lift-away blue 0.06)
    (bright-magenta lift-away magenta 0.06)
    (cursor . fg-bright)
    (accent-alt call romacs-theme--accent-alt)
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
OTHER's colour; (SLOT OP ARGS...) applies OP to ARGS, slot names or
numbers: `mix' A B AMOUNT (`romacs-color-mix'), `floor' FG BG MINIMUM
\(`romacs-color-floor'), `chroma' C FACTOR, `rotate' C TARGET STEP
\(`romacs-color-rotate'), `lift-away' C DELTA (lighter on a dark palette,
darker on a light one), and `call' FUNCTION, called with a slot reader.")

(defconst romacs-theme-slots
  (let ((slots (reverse romacs-theme-base-slots)))
    (dolist (rule romacs-theme-rules (reverse slots))
      (unless (memq (car rule) slots) (setq slots (cons (car rule) slots)))))
  "Every slot a resolved palette holds.")

(defun romacs-theme--reader (palette rule-of)
  "A function reading a slot of PALETTE, (SLOT VALUE) rows, the first of a
slot winning: a row's value (a slot name in its place is that slot's;
`mode' is a symbol), else the rule RULE-OF gives for the slot.  Each slot
is computed once; `unspecified' reads as itself."
  (let ((given (make-hash-table :test 'eq)) (done (make-hash-table :test 'eq)) (get nil))
    (dolist (row (reverse palette)) (puthash (car row) row given))
    (setq get (lambda (slot)
                (let ((known (gethash slot done 'none)))
                  (cond ((eq slot 'unspecified) slot)
                        ((eq known 'busy) (error "Palette slot `%s' refers to itself" slot))
                        ((not (eq known 'none)) known)
                        (t (puthash slot 'busy done)
                           (let* ((row (gethash slot given))
                                  (rule (funcall rule-of slot))
                                  (value (cond ((and row (eq slot 'mode)) (romacs-theme--second row))
                                               (row (let ((v (romacs-theme--second row)))
                                                      (if (symbolp v) (funcall get v) v)))
                                               ((null rule) (error "Palette slot `%s' is missing" slot))
                                               ((symbolp rule) (funcall get rule))
                                               (t (romacs-theme--apply rule get)))))
                             (puthash slot value done)
                             value))))))
    get))

(defun romacs-theme-resolve (palette)
  "PALETTE with every slot of `romacs-theme-slots', as an alist
\(SLOT . VALUE): a slot PALETTE names is its value, every other one is
derived by `romacs-theme-rules'."
  (let ((get (romacs-theme--reader palette (lambda (slot) (cdr (assq slot romacs-theme-rules))))))
    (mapcar (lambda (slot) (cons slot (funcall get slot))) romacs-theme-slots)))

(defun romacs-theme--apply (rule get)
  "Apply RULE, (OP ARGS...) of `romacs-theme-rules', reading slots by GET."
  (if (eq (car rule) 'call)
      (funcall (nth 1 rule) get)
    (let ((args (mapcar (lambda (a) (if (symbolp a) (funcall get a) a)) (cdr rule))))
      (cond ((eq (car rule) 'mix) (apply #'romacs-color-mix args))
            ((eq (car rule) 'floor) (apply #'romacs-color-floor args))
            ((eq (car rule) 'chroma) (apply #'romacs-color-chroma args))
            ((eq (car rule) 'rotate) (apply #'romacs-color-rotate args))
            ((eq (car rule) 'lift-away)
             (romacs-color-lift (nth 0 args) (if (eq (funcall get 'mode) 'light) (- (nth 1 args)) (nth 1 args))))
            (t (error "Unknown palette rule %S" rule))))))

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
      ;; Windows are panes apart (romacs-chrome.el): fringes, dividers and
      ;; borders are the window background, so no strip shows beside a
      ;; window, as the reference session's invisible dividers do; an
      ;; indicator with a face of its own (flymake's, diff-hl's) shows.
      (funcall face 'fringe :foreground bg :background bg)
      (funcall face 'window-divider :foreground bg :background bg)
      (funcall face 'window-divider-first-pixel :foreground bg :background bg)
      (funcall face 'window-divider-last-pixel :foreground bg :background bg)
      (funcall face 'vertical-border :foreground bg :background bg)
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
    (rum dark rum)
    (acario-dark dark acario)
    (acario-light light acario)
    (ayu-dark dark ayu)
    (ayu-light light ayu)
    (ayu-mirage dark ayu)
    (badger dark badger)
    (bluloco-dark dark bluloco)
    (bluloco-light light bluloco)
    (catppuccin-latte light catppuccin)
    (catppuccin-mocha dark catppuccin)
    (challenger-deep dark challenger-deep)
    (city-lights dark city-lights)
    (dark-plus dark dark-plus)
    (dracula dark dracula)
    (earl-grey light earl-grey)
    (ephemeral dark ephemeral)
    (everforest-dark dark everforest)
    (everforest-light light everforest)
    (fairy-floss dark fairy-floss)
    (feather-dark dark feather)
    (feather-light light feather)
    (flatwhite light flatwhite)
    (gruvbox-dark dark gruvbox)
    (gruvbox-light light gruvbox)
    (henna dark henna)
    (homage-black dark homage)
    (homage-white light homage)
    (horizon dark horizon)
    (iosvkem dark iosvkem)
    (ir-black dark ir-black)
    (kanagawa-lotus light kanagawa)
    (kanagawa-wave dark kanagawa)
    (lantern dark lantern)
    (laserwave dark laserwave)
    (leet dark leet)
    (manegarm dark manegarm)
    (material dark material)
    (miramare dark miramare)
    (molokai dark molokai)
    (monokai-classic dark monokai-classic)
    (moonlight dark moonlight)
    (nord-aurora dark nord-aurora)
    (nord-light light nord)
    (nord dark nord)
    (nova dark nova)
    (oceanic-next dark oceanic-next)
    (oksolar-dark dark oksolar)
    (oksolar-light light oksolar)
    (old-hope dark old-hope)
    (one-dark dark one)
    (one-light light one)
    (opera-light light opera-light)
    (outrun-electric dark outrun-electric)
    (peacock dark peacock)
    (pine dark pine)
    (plain-dark dark plain)
    (plain light plain)
    (rose-pine-dawn light rose-pine)
    (rose-pine-main dark rose-pine)
    (rouge dark rouge)
    (shades-of-purple dark shades-of-purple)
    (snazzy dark snazzy)
    (solarized-dark-high-contrast dark solarized)
    (solarized-dark dark solarized)
    (solarized-light light solarized)
    (sourcerer dark sourcerer)
    (spacegrey dark spacegrey)
    (tokyonight-day light tokyonight)
    (tokyonight-night dark tokyonight)
    (tomorrow-day light tomorrow)
    (tomorrow-night dark tomorrow)
    (vibrant dark vibrant)
    (wilmersdorf dark wilmersdorf)
    (winter-is-coming-dark-blue dark winter-is-coming)
    (winter-is-coming-light light winter-is-coming)
    (xcode dark xcode)
    (zenburn dark zenburn)
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
(defun romacs-theme--register (rows)
  "Give each theme of ROWS, (NAME MODE FAMILY), the properties GNU's
variant lookup reads."
  (dolist (entry rows)
    (put (car entry) 'theme-properties
         (list :kind 'color-scheme :background-mode (nth 1 entry) :family (nth 2 entry)))))

(romacs-theme--register romacs-theme-bundled)

(defun romacs-theme--modus-palette (modus)
  "The palette MODUS describes, Modus's every entry (overrides first, as
Modus merges them): MODUS is a plist of :base, :preference, :core (a
`romacs-theme-modus-' core palette's symbol), :overrides and :user."
  (append (plist-get modus :overrides) romacs-theme-common-palette-overrides
          (plist-get modus :user)
          (romacs-theme-modus-generate-palette (plist-get modus :base) (plist-get modus :preference)
                                               (symbol-value (plist-get modus :core)))))

(defconst romacs-theme--modus-constants
  '((fg-table-dim "gray50") (term-dim "#595959") (term-light "#a6a6a6"))
  "The literal colours of Modus's table, under the names
scripts/adapt-modus gives them, as Modus has them: a theme with its own
roles sets these names, one on Modus's palette gets Modus's values.")

(defun romacs-theme--modus-bindings (palette)
  "PALETTE's names, each with its colour, as (NAME . VALUE): the first row
of a name wins, a name in its place is followed, and a value that is
neither gives `unspecified', as `romacs-theme-modus-retrieve-palette-value'
does."
  (let ((index (make-hash-table :test 'eq)) (names nil))
    (dolist (row palette)
      (when (eq (gethash (car row) index 'absent) 'absent)
        (puthash (car row) (romacs-theme--second row) index)
        (setq names (cons (car row) names))))
    (mapcar (lambda (name)
              (let ((value (gethash name index)) (steps 0))
                (while (and value (symbolp value) (not (eq value 'unspecified)))
                  (when (> (setq steps (1+ steps)) 64) (error "Palette name `%s' refers to itself" name))
                  (setq value (gethash value index)))
                (cons name (if (stringp value) value 'unspecified))))
            (reverse names))))

(defun romacs-theme--table-settings (bindings mode)
  "The faces and variables Modus's table gives BINDINGS, (NAME . VALUE),
for a theme of MODE, as (FACES . VARIABLES): `modus-themes-theme''s
evaluation.  VARIABLES are (VARIABLE VALUE), evaluated."
  ;; Modus writes its specs for a colour display of 256 colours or more;
  ;; Romacs draws only on one, and builds its world on a frame without a
  ;; window system, so the specs apply to every display.
  (let ((settings
         (eval `(let* ((c t)
                       ,@(mapcar (lambda (binding) (list (car binding) (list 'quote (cdr binding)))) bindings))
                  (cons (list ,@romacs-theme-modus-faces)
                        (list ,@romacs-theme-modus-custom-variables '(frame-background-mode ',mode))))
               t)))
    (cons (car settings)
          (mapcar (lambda (row) (list (car row) (eval (romacs-theme--second row) t))) (cdr settings)))))

(defun romacs-theme--modus-settings (modus mode)
  "The faces and variables Modus's table gives the palette MODUS
describes, for a theme of MODE, as (FACES . VARIABLES)."
  (romacs-theme--table-settings
   (romacs-theme--modus-bindings (append (romacs-theme--modus-palette modus) romacs-theme--modus-constants))
   mode))

(defun romacs-theme--romacs-face-p (face)
  "Whether FACE is one Romacs's engine colours over a Modus table: Romacs's
own faces, the fringes and dividers a pane is separated without, and
`font-lock-comment-delimiter-face', which keeps GNU's defface relation
(it inherits `font-lock-comment-face', as GNU's own tests and every
change to the comment face expect) where Modus gives it the comment's
colour directly."
  (or (string-prefix-p "romacs-" (symbol-name face))
      (memq face '(fringe window-divider window-divider-first-pixel window-divider-last-pixel vertical-border
                   font-lock-comment-delimiter-face))))

(defun romacs-theme--clause-attributes (clause)
  "The attributes of face spec CLAUSE, a property list."
  (if (consp (car (cdr clause))) (car (cdr clause)) (cdr clause)))

(defun romacs-theme--with-attributes (spec attributes)
  "Face SPEC with ATTRIBUTES, a property list, set in every clause a
display can select: a `default' clause only underlies those.  A spec with
no such clause for all displays gains one."
  (let ((merge (lambda (own)
                 (let ((merged (copy-sequence own)) (rest attributes))
                   (while rest
                     (setq merged (plist-put merged (car rest) (romacs-theme--second rest))
                           rest (cdr (cdr rest))))
                   merged))))
    (append (mapcar (lambda (clause)
                      (if (eq (car clause) 'default)
                          clause
                        (cons (car clause) (funcall merge (romacs-theme--clause-attributes clause)))))
                    spec)
            (and (null (assq t spec)) (list (cons t (funcall merge nil)))))))

(defun romacs-theme--merge-rows (rows over)
  "ROWS, (FACE SPEC), with OVER's rows in their place, each face once."
  (let ((replaced (make-hash-table :test 'eq)))
    (dolist (row over) (puthash (car row) t replaced))
    (append (seq-remove (lambda (row) (gethash (car row) replaced)) rows) over)))

(defun romacs-theme--apply-delta (rows delta)
  "ROWS with DELTA's attributes, (FACE . ATTRIBUTES), merged over them, a
face with no row gaining one."
  (let ((index (make-hash-table :test 'eq)) (faces nil) (present (make-hash-table :test 'eq)))
    (dolist (entry delta)
      (let ((known (gethash (car entry) index)))
        (puthash (car entry) (append known (cdr entry)) index)
        (unless known (setq faces (cons (car entry) faces)))))
    (dolist (row rows) (puthash (car row) t present))
    (append (mapcar (lambda (row)
                      (let ((attributes (gethash (car row) index)))
                        (if attributes
                            (list (car row) (romacs-theme--with-attributes (romacs-theme--second row) attributes))
                          row)))
                    rows)
            (delq nil (mapcar (lambda (face)
                                (and (not (gethash face present))
                                     (list face (list (cons t (gethash face index))))))
                              (reverse faces))))))

(defconst romacs-theme--colour-attributes
  '(:foreground :background :distant-foreground :underline :overline :strike-through :box)
  "The face attributes whose values may name a palette slot.")

(defun romacs-theme--face-value (value env)
  "VALUE of a colour attribute under ENV, (NAME . VALUE): a slot's name
reads the colour, a `romacs-theme-' form evaluates, a list maps."
  (cond ((and (symbolp value) (stringp (cdr (assq value env)))) (cdr (assq value env)))
        ((and (consp value) (symbolp (car value)) (string-prefix-p "romacs-theme-" (symbol-name (car value))))
         (eval value env))
        ((consp value) (mapcar (lambda (each) (romacs-theme--face-value each env)) value))
        (t value)))

(defun romacs-theme--override (row mode env)
  "ROW of `def-romacs-theme''s FACES as (FACE . ATTRIBUTES) for a theme of
MODE, or nil where it has no branch for MODE."
  (let ((attributes (if (memq (nth 1 row) '(&dark &light))
                        (plist-get (cdr row) (if (eq mode 'light) '&light '&dark))
                      (cdr row)))
        (out nil))
    (while attributes
      (let ((key (car attributes)) (value (nth 1 attributes)))
        (setq out (append out (list key (if (memq key romacs-theme--colour-attributes)
                                            (romacs-theme--face-value value env)
                                          value)))
              attributes (cdr (cdr attributes)))))
    (and out (cons (car row) out))))

(defun romacs-theme--delta (floors faces mode env)
  "The attributes FLOORS' moves and FACES, `def-romacs-theme''s rows, set for
a theme of MODE, as (FACE . ATTRIBUTES), the rows last so they win."
  (append (mapcar (lambda (move) (list (car move) :foreground (nth 2 move))) floors)
          (delq nil (mapcar (lambda (row) (romacs-theme--override row mode env)) faces))))

(defun romacs-theme--infer-mode (palette)
  "`light' if PALETTE's `bg' is, else `dark'."
  (if (> (nth 0 (romacs-color-oklab (romacs-theme--second (assq 'bg palette)))) 0.6) 'light 'dark))

(defun romacs-theme--custom-define (name doc properties mode rows)
  "Declare theme NAME as GNU's theme over ROWS (`romacs-theme-rows'): the
adapter from that value to Custom."
  (custom-declare-theme name (intern (concat (symbol-name name) "-theme")) doc
                        (list :kind 'color-scheme :background-mode mode :family (plist-get properties :family)))
  (apply #'custom-theme-set-faces name (plist-get rows :faces))
  (let ((variables (plist-get rows :variables)))
    (when variables
      (apply #'custom-theme-set-variables name
             (mapcar (lambda (row) (list (car row) (list 'quote (romacs-theme--second row)))) variables))))
  (provide-theme name))

(defun romacs-theme--table-rows (properties palette)
  "`romacs-theme-rows''s value for a theme of PROPERTIES with its own
roles, on PALETTE (rlisp/romacs-theme-roles.el)."
  (require 'romacs-theme-roles)
  (romacs-theme-roles-rows palette (plist-get properties :faces) (plist-get properties :variables)))

(defun romacs-theme--plain-rows (properties resolved)
  "`romacs-theme-rows''s value for a theme of PROPERTIES without roles of
its own: with :modus, Modus's table with Romacs's own faces over it and
:floor's floors, else RESOLVED's faces; :faces on top of either."
  (let* ((faces (romacs-theme-faces resolved))
         (modus (plist-get properties :modus))
         (mode (plist-get properties :mode))
         (own (plist-get properties :faces)))
    (if (null modus)
        (let ((delta (romacs-theme--delta nil own mode resolved)))
          (list :palette resolved :faces (romacs-theme--apply-delta faces delta) :delta delta))
      (require 'romacs-theme-modus)
      (let* ((settings (romacs-theme--modus-settings modus mode))
             (baseline (romacs-theme--merge-rows
                        (car settings) (seq-filter (lambda (row) (romacs-theme--romacs-face-p (car row))) faces)))
             (floors (when (plist-get properties :floor)
                       (require 'romacs-theme-roles)
                       (romacs-theme-roles-floors baseline resolved)))
             (delta (romacs-theme--delta floors own mode resolved)))
        (list :palette resolved :faces (romacs-theme--apply-delta baseline delta)
              :delta delta :floors floors :variables (cdr settings))))))

(defun romacs-theme-define (name doc &rest properties)
  "Define theme NAME, documented by DOC, from PROPERTIES :palette (base
and pinned slots, `romacs-theme-resolve'), :mode and :family.  Where
Custom is loaded it is GNU's theme, enabled and disabled as any other;
elsewhere (a launch without GNU's Lisp) its faces are declared as the
faces' own specs.  With :modus (`romacs-theme--modus-palette') and
Custom, its faces are Modus's table over that palette, Romacs's own faces
over them from :palette, then :floor's floors and :faces on top.  With
:table and Custom, the palette makes every role of that table itself
\(rlisp/romacs-theme-roles.el), floors apply, and :faces and :variables,
as `def-romacs-theme' takes them, go on top.
:style is kept as the theme's `romacs-style' property.  Defining it
changes no face that shows."
  (let* ((given (plist-get properties :palette))
         (mode (or (plist-get properties :mode) (romacs-theme--second (assq 'mode given))
                   (romacs-theme--infer-mode given)))
         (properties (plist-put (copy-sequence properties) :mode mode))
         (palette (if (assq 'mode given) given (cons (list 'mode mode) given)))
         (custom (fboundp 'custom-theme-set-faces))
         (rows (if (and custom (plist-get properties :table))
                   (romacs-theme--table-rows properties palette)
                 (romacs-theme--plain-rows (if custom properties (plist-put properties :modus nil))
                                           (romacs-theme-resolve palette)))))
    (put name 'romacs-palette palette)
    (put name 'romacs-palette-resolved (plist-get rows :palette))
    (put name 'romacs-theme-rows rows)
    (put name 'romacs-style (plist-get properties :style))
    (if custom
        (romacs-theme--custom-define name doc properties mode rows)
      (dolist (row (plist-get rows :faces))
        (eval (list 'defface (car row) (list 'quote (romacs-theme--with-font (car row) (romacs-theme--second row)))
                    "From the theme.")
              t)))
    name))

(defun romacs-theme-rows (theme)
  "THEME's definition as data, a property list: :palette its effective
resolved palette (SLOT . VALUE), :roles the Modus names its table read
\(NAME . VALUE), :faces its rows (FACE SPEC) with floors and overrides
applied, each face once, :delta the attributes floors and the theme's own
overrides set (FACE . ATTRIBUTES), :floors the moves, and :variables its
rows (VARIABLE VALUE).  Custom's settings are made from this value; it
owes nothing to how a theme is applied."
  (get theme 'romacs-theme-rows))

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

(defvar romacs-theme--reading nil
  "The slots a theme being defined has so far, as (SLOT . VALUE).")

(defun romacs-theme-color (slot &optional theme)
  "SLOT's colour: of the theme being defined, else of THEME (default the
first enabled theme that has a palette)."
  (cdr (assq slot (or romacs-theme--reading (romacs-theme-palette theme)))))

(defun romacs-theme--eval-palette (rows)
  "ROWS, `def-romacs-theme''s PALETTE, as a palette: a row's form is
evaluated with the slots before it bound."
  (let ((env nil) (palette nil))
    (dolist (row rows (reverse palette))
      (let* ((given (romacs-theme--second row))
             (value (if (consp given)
                        (let ((romacs-theme--reading env)) (eval given env))
                      given)))
        (setq palette (cons (list (car row) value) palette)
              env (cons (cons (car row) (if (symbolp value) (cdr (assq value env)) value)) env))))))

(defun romacs-theme--check-style (style)
  "STYLE, or signal: `flat', `modern', a property list, or nil."
  (unless (or (memq style '(nil flat modern)) (and (consp style) (keywordp (car style))))
    (error "Theme style %S is not `flat', `modern' or a property list" style))
  style)

(defmacro def-romacs-theme (name doc &rest args)
  "Define theme NAME, documented by DOC, from a short named palette.
ARGS are [:mode dark|light] [:family F] [:style S] PALETTE [FACES]
\[VARIABLES].

PALETTE rows are (SLOT VALUE): `bg', `fg' and the six hues at least, and
any slot of `romacs-theme-slots' or role of Modus's table.  VALUE is a
colour, a slot's name, or a form over the slots before it, such as
\(romacs-theme-blend bg red 0.2).  The rest is derived.

FACES rows are (FACE :ATTRIBUTE VALUE ...), or (FACE &dark (...) &light
\(...)) for one list shared by a theme's variants.  Their attributes
merge over what the palette gives FACE.  A colour attribute's VALUE may
name a slot or be a `romacs-theme-' form.  VARIABLES rows are (VARIABLE
FORM), the slots bound.

The result is an ordinary theme, a plain `deftheme': `load-theme',
`enable-theme' and `disable-theme' treat it as any other."
  (declare (indent 1) (doc-string 2))
  (let ((keys nil))
    (while (keywordp (car args))
      (setq keys (append keys (list (car args) (list 'quote (romacs-theme--second args))))
            args (cdr (cdr args))))
    `(romacs-theme-define ',name ,doc :table t :mode ,(plist-get keys :mode)
                          :family ,(or (plist-get keys :family) (list 'quote name))
                          :style (romacs-theme--check-style ,(plist-get keys :style))
                          :palette (romacs-theme--eval-palette ',(nth 0 args))
                          :faces ',(nth 1 args)
                          :variables ',(nth 2 args))))

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
vocabulary (`romacs-theme-file-keys') and a `mode' of dark or light,
read from the background when left out.  Other keys are ignored; a known
key that is not #rrggbb, or another mode, signals."
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
    (cons (list 'mode (or mode (romacs-theme--infer-mode palette))) (nreverse palette))))

;; A user option where Custom is loaded (`defcustom''s expansion); the
;; launch without GNU's Lisp has no Custom and no `system' theme.
(if (fboundp 'custom-declare-variable)
    (custom-declare-variable
     'romacs-theme-system-file nil
     "A palette file behind the theme `system', or nil.
The file holds KEY = \"#rrggbb\" lines in the Omarchy colors.toml
vocabulary (see etc/themes/README.md), for example
~/.local/state/omarchy/current/theme/colors.toml."
     :type '(choice (const nil) file) :group 'faces)
  (defvar romacs-theme-system-file nil))

;;;; The stock theme

(defvar romacs-theme-fallback 'default
  "The theme a session starts in, which the first colour scheme enabled
over it replaces; nil for GNU's stacking.")

(defvar romacs-theme--stock nil
  "Non-nil while `romacs-theme-fallback' is enabled as the session's
stock theme rather than by choice.")

(defun romacs-theme--colour-scheme-p (theme)
  "Whether THEME is a colour scheme: one that says so (`:kind'
`color-scheme', as GNU's and Modus's themes do), or one that says nothing
and sets the `default' face, as most third-party themes do.  A theme
packages use for their own settings, such as `use-package''s for
`:custom-face', is neither."
  (let ((kind (plist-get (get theme 'theme-properties) :kind)))
    (or (eq kind 'color-scheme)
        (and (null kind)
             (let ((found nil))
               (dolist (setting (get theme 'theme-settings) found)
                 (when (and (eq (car setting) 'theme-face) (eq (nth 1 setting) 'default))
                   (setq found t))))))))

(defun romacs-theme--replace-stock (theme)
  "Run from `enable-theme-functions': THEME, a colour scheme enabled over
the stock theme, replaces it, once.  Romacs's own colour schemes (those
`romacs-theme-define' made) are one look each: enabling one disables the
others, so `rum' and `default' toggle.  Other themes keep stacking as
GNU's do."
  (cond ((eq theme 'user))
        ((get theme 'romacs-palette)
         (setq romacs-theme--stock nil)
         (dolist (other (copy-sequence custom-enabled-themes))
           (when (and (not (eq other theme)) (get other 'romacs-palette))
             (disable-theme other))))
        ((and romacs-theme--stock romacs-theme-fallback
              (memq romacs-theme-fallback custom-enabled-themes)
              (romacs-theme--colour-scheme-p theme))
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

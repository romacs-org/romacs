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
;; A theme sets colours and styles, never the font: text size and family
;; are the user's.  Under Custom every theme's faces are one map, Modus's
;; table (lisp/ui/romacs-theme-modus.el), whose sizes and families follow
;; the typography options alone (`romacs-theme-headings' and the like), so
;; every theme shows the same; a theme's own rows lose any font attribute
;; (`romacs-theme-typography-attributes').
;;
;; Everything here is plain Elisp over numbers and strings, without
;; color.el, cl-lib or seq: it also runs in the launches whose only Lisp
;; is the machine's own prelude.  The machine does the colour arithmetic
;; (`romacs-color-oklab', `romacs-color-from-oklab', `romacs-color--oklch',
;; `romacs-color-luminance'); the rules are here.

;;; Code:

;;;; Colours

(defun romacs-theme--second (list)
  "LIST's second element (the machine's own prelude has no `cadr')."
  (car (cdr list)))

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

(defvar romacs-color--memo (make-hash-table :test 'equal)
  "What the pure colour functions below answered, by their arguments: a
theme's floors ask the same questions many times over.")

(defun romacs-color--remember (key value)
  "Keep VALUE under KEY in `romacs-color--memo', which stays small, and
answer VALUE."
  (when (> (hash-table-count romacs-color--memo) 20000) (clrhash romacs-color--memo))
  (puthash key value romacs-color--memo))

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
  (let ((key (list fg backgrounds minimum)))
    (or (gethash key romacs-color--memo)
        (romacs-color--remember key (romacs-color--floor-on fg backgrounds minimum)))))

(defun romacs-color--floor-on (fg backgrounds minimum)
  "`romacs-color-floor-on', computed."
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

(defun romacs-color-hex (color)
  "COLOR as #rrggbb: a #rrggbb as it is, a name as the frame resolves it;
nil when it is no colour the frame knows, or in a launch with no frame
colours (no `color-values')."
  (cond ((not (stringp color)) nil)
        ((string-match-p "\\`#[[:xdigit:]]\\{6\\}\\'" color) color)
        ((and (fboundp 'color-values) (color-values color))
         (apply #'format "#%02x%02x%02x" (mapcar (lambda (v) (ash v -8)) (color-values color))))))

(defun romacs-color-band (bg fg &optional contrast)
  "BG, #rrggbb, mixed toward FG until it stands CONTRAST (1.15 by default)
apart from BG: a surface or a band that shows on a black background as
on a white one.  Nil when no band does that and keeps FG readable on it
\(3:1), a theme whose text barely stands off its background."
  (let ((contrast (or contrast 1.15)) (amount 0.05) band)
    (while (progn (setq band (romacs-color-mix bg fg (min amount 1.0)))
                  (and (< (romacs-color-contrast band bg) contrast) (< amount 1.0)))
      (setq amount (+ amount 0.02)))
    ;; The least mix that stands apart, between the last step and this:
    ;; the most room left for the text.
    (let ((low (if (= amount 0.05) 0.0 (- amount 0.02))) (high (min amount 1.0)))
      (when (>= (romacs-color-contrast band bg) contrast)
        (dotimes (_ 8)
          (let ((mid (/ (+ low high) 2)))
            (if (>= (romacs-color-contrast (romacs-color-mix bg fg mid) bg) contrast) (setq high mid) (setq low mid))))
        ;; A mix is rounded to whole channels, its contrasts not quite
        ;; even along the way: the first nearby one that meets both.
        (let ((at (max 0.0 (- high 0.01))) (found nil))
          (while (and (not found) (<= at (min 1.0 (+ high 0.02))))
            (let ((try (romacs-color-mix bg fg at)))
              (when (and (>= (romacs-color-contrast try bg) contrast) (>= (romacs-color-contrast fg try) 3.0))
                (setq found try)))
            (setq at (+ at 0.001)))
          found)))))

(defun romacs-color-legible (color &optional background contrast)
  "COLOR moved in lightness until it reads on BACKGROUND (the default
face's, by default) at CONTRAST (WCAG's 4.5:1 for text, by default), its
hue kept, as #rrggbb; nil when either is no colour the frame knows.
Either may be named in any form the frame resolves."
  (let ((fg (romacs-color-hex color))
        (bg (romacs-color-hex (or background (and (fboundp 'face-background) (face-background 'default nil t))))))
    (and fg bg (romacs-color-floor fg bg (or contrast 4.5)))))

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

(defun romacs-theme--shared-aa-p (a b)
  "Whether one colour reads at WCAG AA's 4.5:1 on both A and B: black
or white does, if any colour does."
  (or (>= (min (romacs-color-contrast "#000000" a) (romacs-color-contrast "#000000" b)) 4.5)
      (>= (min (romacs-color-contrast "#ffffff" a) (romacs-color-contrast "#ffffff" b)) 4.5)))

(defun romacs-theme--glass-selection (get)
  "A selected row on glass: the glass tinted toward the accent, by at
most 22%, as far as one colour still reads on both the glass and the
row, so a match keeps its colour on either; the glass itself where no
tint leaves one.  GET reads a slot."
  (let* ((glass (funcall get 'glass)) (accent (funcall get 'accent))
         (amounts '(0.22 0.18 0.14 0.1 0.06 0.03)) (found nil))
    (while (and amounts (not found))
      (let ((tint (romacs-color-mix glass accent (car amounts))))
        (when (romacs-theme--shared-aa-p glass tint)
          (setq found tint)))
      (setq amounts (cdr amounts)))
    (or found glass)))

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
    ;; Glass: the background lifted the same Oklab lightness on every
    ;; theme, so a layer stands off a mid-grey one (zenburn) as well.
    (glass lift-away bg 0.055)
    ;; A selected row on glass, tinted toward the accent from the glass
    ;; itself: a theme's own `selection' is for the editor's background
    ;; and may be darker than the glass.
    (glass-selection call romacs-theme--glass-selection)
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

(defconst romacs-theme-face-table
  '(;; Core.
    (default :foreground (text fg) :background bg)
    (cursor :background cursor)
    (region :background bg-region :extend t)
    (secondary-selection :background bg-highlight :extend t)
    (highlight :background bg-highlight)
    (hl-line :background bg-highlight :extend t)
    ;; Windows are panes apart (romacs-chrome.el): fringes, dividers and
    ;; borders are the window background, so no strip shows beside a
    ;; window, as the reference session's invisible dividers do; an
    ;; indicator with a face of its own (flymake's, diff-hl's) shows.
    (fringe :foreground bg :background bg)
    (window-divider :foreground bg :background bg)
    (window-divider-first-pixel :foreground bg :background bg)
    (window-divider-last-pixel :foreground bg :background bg)
    (vertical-border :foreground bg :background bg)
    (internal-border :background bg)
    (child-frame-border :background border)
    (border :background border)
    (minibuffer-prompt :foreground (text prompt))
    (mode-line :foreground (text fg bg-mode-line) :background bg-mode-line)
    (mode-line-active :inherit mode-line)
    (mode-line-inactive :foreground (dim fg-mode-line-inactive bg-mode-line-inactive)
                        :background bg-mode-line-inactive)
    (mode-line-buffer-id :foreground (text fg-bright bg-mode-line) :weight bold)
    (mode-line-emphasis :foreground (text accent bg-mode-line))
    (mode-line-highlight :background hover)
    (header-line :inherit mode-line :foreground (text fg bg-alt) :background bg-alt)
    (tab-bar :inherit variable-pitch :foreground (text fg bg-dark) :background bg-dark)
    (tab-line :inherit variable-pitch :foreground (text fg bg-dark) :background bg-dark)
    (line-number :inherit default :foreground (dim fg-muted))
    ;; The current line's number in the bright cyan, bold; no band.
    (line-number-current-line :inherit (bold default) :foreground (text cyan))
    (show-paren-match :background bg-paren)
    (show-paren-mismatch :foreground bg :background error)
    (isearch :foreground (text fg-bright bg-search-current) :background bg-search-current)
    (isearch-fail :foreground (text error))
    (lazy-highlight :foreground (text fg-bright bg-search) :background bg-search)
    (match :background bg-match)
    (error :foreground (text error))
    (warning :foreground (text warning))
    (success :foreground (text success))
    (link :foreground (text link) :underline t)
    (link-visited :foreground (text magenta) :underline t)
    (button :inherit link)
    (shadow :foreground (dim fg-muted))
    (trailing-whitespace :background red)
    (escape-glyph :foreground (text orange))
    (homoglyph :foreground (text orange))
    (nobreak-space :foreground (text orange) :underline t)
    (tooltip :inherit variable-pitch :foreground (text fg glass) :background glass)
    ;; Keys stand out by weight and colour, on no box or band of their own.
    (help-key-binding :inherit (bold fixed-pitch) :foreground (text keybind bg bg-alt glass))
    (completions-first-difference :foreground (text fg-bright))
    (completions-annotations :foreground (dim fg-muted))
    (completions-highlight :background selection)
    ;; Font-lock: no bold or italic constructs.
    (font-lock-keyword-face :foreground (text keyword))
    (font-lock-function-name-face :foreground (text function))
    (font-lock-function-call-face :foreground (text function))
    (font-lock-variable-name-face :foreground (text variable))
    (font-lock-variable-use-face :foreground (text variable))
    (font-lock-type-face :foreground (text type))
    (font-lock-constant-face :foreground (text constant))
    (font-lock-string-face :foreground (text string))
    (font-lock-builtin-face :foreground (text builtin))
    (font-lock-doc-face :foreground (text doc))
    (font-lock-preprocessor-face :foreground (text preprocessor))
    (font-lock-negation-char-face :foreground (text negation))
    (font-lock-warning-face :foreground (text fl-warning))
    (font-lock-regexp-grouping-backslash :foreground (text regexp))
    (font-lock-regexp-grouping-construct :foreground (text regexp))
    (font-lock-number-face :foreground (text constant))
    (font-lock-comment-face :foreground (dim comment))
    (font-lock-comment-delimiter-face :inherit font-lock-comment-face)
    ;; Romacs chrome.
    (romacs-pane-focus :foreground glow)
    (romacs-pane-border :foreground border)
    (romacs-layer-glow :foreground glow)
    (romacs-layer-glass :background glass)
    (romacs-layer-toast :foreground (text fg glass) :background glass)
    (romacs-layer-selection :background glass-selection)
    (romacs-landing-mark :foreground glow)
    (romacs-landing-word :foreground fg-bright)
    ;; The selected candidate's own text floored on its row, so it reads
    ;; as well as its neighbours (a match or annotation keeps its face).
    (romacs-complete-current :background glass-selection :extend t :foreground (text fg glass-selection))
    ;; Matches show on the panel and on its selected row.
    (romacs-complete-match :foreground (text accent glass glass-selection))
    (romacs-complete-prompt :foreground (text fg-dim* glass))
    (completions-common-part :foreground (text accent bg selection))
    (romacs-comment :foreground (dim comment))
    (romacs-code :foreground (text string))
    (romacs-result :foreground (text constant))
    (orderless-match-face-0 :foreground (text accent))
    (orderless-match-face-1 :foreground (text accent-alt))
    (orderless-match-face-2 :foreground (text magenta))
    (orderless-match-face-3 :foreground (text cyan))
    ;; The mode line's own faces, on its band.
    ;; Their own specs' inheritance stays: a theme sets colours.
    (romacs-mode-line-dim :inherit shadow :foreground (text fg-dim* bg-mode-line))
    (romacs-mode-line-bright :inherit mode-line-buffer-id :foreground (text fg-bright bg-mode-line))
    (romacs-mode-line-state :inherit (romacs-pane-focus bold) :foreground (text accent bg-mode-line))
    (romacs-mode-line-error :inherit error :foreground (text error bg-mode-line))
    (romacs-mode-line-warning :inherit warning :foreground (text warning bg-mode-line))
    (romacs-mode-line-modified :inherit warning :foreground (text warning bg-mode-line))
    (romacs-mode-line-ok :inherit success :foreground (text success bg-mode-line))
    ;; Headings: Rum's weights (its Modus `headings': 1 ultrabold, 2
    ;; semibold, the rest Modus's bold), on outline and Org headings alike;
    ;; their sizes are the typography's, not a theme's.
    (outline-1 :foreground (text red) :weight ultrabold)
    (org-level-1 :foreground (text red) :weight ultrabold)
    (outline-2 :foreground (text yellow) :weight semibold)
    (org-level-2 :foreground (text yellow) :weight semibold)
    (outline-3 :foreground (text blue) :weight bold)
    (org-level-3 :foreground (text blue) :weight bold)
    (outline-4 :foreground (text magenta) :weight bold)
    (org-level-4 :foreground (text magenta) :weight bold)
    (outline-5 :foreground (text green) :weight bold)
    (org-level-5 :foreground (text green) :weight bold)
    (outline-6 :foreground (text orange) :weight bold)
    (org-level-6 :foreground (text orange) :weight bold)
    (outline-7 :foreground (text cyan) :weight bold)
    (org-level-7 :foreground (text cyan) :weight bold)
    (outline-8 :foreground (text fg-dim*) :weight bold)
    (org-level-8 :foreground (text fg-dim*) :weight bold)
    ;; The terminal.
    (ansi-color-black :foreground ansi-0 :background ansi-0)
    (ansi-color-bright-black :foreground ansi-8 :background ansi-8)
    (ansi-color-red :foreground ansi-1 :background ansi-1)
    (ansi-color-bright-red :foreground ansi-9 :background ansi-9)
    (ansi-color-green :foreground ansi-2 :background ansi-2)
    (ansi-color-bright-green :foreground ansi-10 :background ansi-10)
    (ansi-color-yellow :foreground ansi-3 :background ansi-3)
    (ansi-color-bright-yellow :foreground ansi-11 :background ansi-11)
    (ansi-color-blue :foreground ansi-4 :background ansi-4)
    (ansi-color-bright-blue :foreground ansi-12 :background ansi-12)
    (ansi-color-magenta :foreground ansi-5 :background ansi-5)
    (ansi-color-bright-magenta :foreground ansi-13 :background ansi-13)
    (ansi-color-cyan :foreground ansi-6 :background ansi-6)
    (ansi-color-bright-cyan :foreground ansi-14 :background ansi-14)
    (ansi-color-white :foreground ansi-7 :background ansi-7)
    (ansi-color-bright-white :foreground ansi-15 :background ansi-15)
    ;; Packages in Romacs's Lisp and daily use.
    (diff-added :background diff-add :extend t)
    (diff-removed :background diff-remove :extend t)
    (diff-changed :background diff-change :extend t)
    (diff-refine-added :background diff-add-refine)
    (diff-refine-removed :background diff-remove-refine)
    (diff-refine-changed :background diff-change-refine)
    (diff-header :background bg-alt :extend t)
    (diff-file-header :foreground (text fg-bright bg-alt) :background bg-alt :extend t)
    (diff-hunk-header :foreground (text accent-alt bg-alt) :background bg-alt :extend t)
    (dired-directory :foreground (text accent-alt))
    (dired-symlink :foreground (text cyan))
    (dired-marked :foreground (text yellow))
    (dired-flagged :foreground (text red))
    (dired-header :foreground (text accent))
    (org-block :background bg-block :extend t)
    (org-block-begin-line :background bg-block-delimiter :extend t)
    (org-block-end-line :inherit org-block-begin-line)
    (org-todo :foreground (text bright-green))
    (org-done :foreground (dim fg-muted))
    (org-date :foreground (text cyan) :underline t)
    (org-link :inherit link)
    (org-table :foreground (text fg-light))
    (org-code :foreground (text string))
    (org-verbatim :foreground (text constant))
    (compilation-error :inherit error)
    (compilation-warning :inherit warning)
    (compilation-info :inherit success)
    (flymake-error :underline (wave error))
    (flymake-warning :underline (wave warning))
    (flymake-note :underline (wave info))
    (which-key-key-face :foreground (text accent))
    (which-key-command-description-face :foreground (text fg))
    (eshell-prompt :foreground (text accent))
    (comint-highlight-prompt :foreground (text accent)))
  "The faces a resolved palette gives, as rows (FACE ATTRIBUTE VALUE ...)
that `romacs-theme-faces' reads.  The VALUE of a colour attribute
\(`romacs-theme--colour-attributes') reads the palette: a slot's name is
its colour; (text SLOT ON...) is SLOT's colour floored to 4.5:1 against
each ON slot's (default `bg'), for text; (dim SLOT [ON]) floored to 3:1
against ON's (default `bg'), for de-emphasised text; (wave SLOT) a wavy
underline in SLOT's colour; t is t.  Every other VALUE is itself.  A row
sets colours and styles, never a size; where GNU's own spec gives a face
its font by inheritance (`fixed-pitch', `variable-pitch', the mode
line's), the row keeps it, since a theme's spec replaces the face's own.")

(defun romacs-theme--row-colour (value palette)
  "VALUE, a colour attribute's in `romacs-theme-face-table', on resolved PALETTE."
  (if (symbolp value)
      (if (eq value t) t (cdr (assq value palette)))
    (let* ((kind (car value)) (colour (cdr (assq (nth 1 value) palette)))
           (ons (nthcdr 2 value))
           (on (or (cdr (assq (car ons) palette)) (cdr (assq 'bg palette)))))
      (cond ((eq kind 'text)
             (romacs-color-floor-on colour (if (cdr ons) (mapcar (lambda (each) (cdr (assq each palette))) ons) (list on))
                                    4.5))
            ((eq kind 'dim) (romacs-color-floor colour on 3.0))
            ((eq kind 'wave) (list :style 'wave :color colour))
            (t (error "Unknown colour %S in `romacs-theme-face-table'" value))))))

(defun romacs-theme--row-face (row palette)
  "ROW of `romacs-theme-face-table' as a face row on resolved PALETTE."
  (let ((attributes (cdr row)) (out nil))
    (while attributes
      (let ((key (car attributes)) (value (nth 1 attributes)))
        (push key out)
        (push (if (memq key romacs-theme--colour-attributes) (romacs-theme--row-colour value palette) value) out))
      (setq attributes (cdr (cdr attributes))))
    (apply #'romacs-theme--face (car row) (nreverse out))))

(defun romacs-theme-faces (palette)
  "The faces of resolved PALETTE (`romacs-theme-resolve'), as
`custom-theme-set-faces' takes them: ((FACE ((t ATTRIBUTES))) ...), a
row for each of `romacs-theme-face-table'."
  (mapcar (lambda (row) (romacs-theme--row-face row palette)) romacs-theme-face-table))

;;;; Themes

;; Begin: generated by tools/import-omarchy-palettes.
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
;; End: generated by tools/import-omarchy-palettes.

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
romacs-theme-modus.el gives them, as Modus has them: a theme with its own
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

(defvar romacs-theme--table nil
  "Modus's table as a function of a vector of its names' values and a mode,
built once: a world holds it, so a session's first theme does not pay for
it.  (`romacs-theme--table-names' lists the names.)")

(defvar romacs-theme--table-names nil
  "The names `romacs-theme--table''s vector holds, in order.")

(defun romacs-theme--table ()
  "`romacs-theme--table', built on first use."
  (or romacs-theme--table
      (let ((names (mapcar #'car (append romacs-theme-modus-vivendi-palette romacs-theme--modus-constants)))
            (slot -1))
        (setq romacs-theme--table-names names
              romacs-theme--table
              (eval `(lambda (values mode)
                       (let* ((c t)
                              ,@(mapcar (lambda (name) (list name `(aref values ,(setq slot (1+ slot))))) names))
                         (cons (list ,@romacs-theme-modus-faces)
                               (list ,@romacs-theme-modus-custom-variables
                                     (list 'frame-background-mode (list 'quote mode))))))
                    t)))))

(defun romacs-theme--table-settings (bindings mode)
  "The faces and variables Modus's table gives BINDINGS, (NAME . VALUE),
for a theme of MODE, as (FACES . VARIABLES): `modus-themes-theme''s
evaluation.  VARIABLES are (VARIABLE VALUE), evaluated."
  ;; Modus writes its specs for a colour display of 256 colours or more;
  ;; Romacs draws only on one, and builds its world on a frame without a
  ;; window system, so the specs apply to every display.
  (let ((table (romacs-theme--table)) (index (make-hash-table :test 'eq)))
    (dolist (binding bindings) (puthash (car binding) (cdr binding) index))
    (let ((settings (funcall table
                             (vconcat (mapcar (lambda (name) (gethash name index 'unspecified))
                                              romacs-theme--table-names))
                             mode)))
      (cons (car settings)
            (mapcar (lambda (row) (list (car row) (eval (romacs-theme--second row) t))) (cdr settings))))))

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

(defconst romacs-theme-typography-attributes '(:family :foundry :height :width :font :fontset)
  "The face attributes a theme's own rows may not set: text size and
family are the user's, and every theme shows the typography Modus's table
gives (`romacs-theme-headings' and its other options).")

(defconst romacs-theme--pitch-faces '(fixed-pitch fixed-pitch-serif variable-pitch)
  "The faces whose inheritance changes a face's family.")

(defun romacs-theme--without-pitch (inherit)
  "INHERIT, an `:inherit' value, without `romacs-theme--pitch-faces'."
  (if (listp inherit)
      (let ((kept nil))
        (dolist (face inherit) (unless (memq face romacs-theme--pitch-faces) (push face kept)))
        (if (cdr kept) (nreverse kept) (car kept)))
    (and (not (memq inherit romacs-theme--pitch-faces)) inherit)))

(defun romacs-theme--override (row mode env)
  "ROW of `def-romacs-theme''s FACES as (FACE . ATTRIBUTES) for a theme of
MODE, or nil where it has no branch for MODE.  A font attribute
\(`romacs-theme-typography-attributes', or inheriting a pitch face) is
dropped: a theme sets colours and styles."
  (let ((attributes (if (memq (nth 1 row) '(&dark &light))
                        (plist-get (cdr row) (if (eq mode 'light) '&light '&dark))
                      (cdr row)))
        (out nil))
    (while attributes
      (let* ((key (car attributes))
             (value (cond ((memq key romacs-theme--colour-attributes)
                           (romacs-theme--face-value (nth 1 attributes) env))
                          ((eq key :inherit) (romacs-theme--without-pitch (nth 1 attributes)))
                          (t (nth 1 attributes)))))
        (unless (or (memq key romacs-theme-typography-attributes)
                    (and (eq key :inherit) (null value) (nth 1 attributes)))
          (setq out (append out (list key value))))
        (setq attributes (cdr (cdr attributes)))))
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
roles, on PALETTE (lisp/ui/romacs-theme-roles.el)."
  (require 'romacs-theme-roles)
  (romacs-theme-roles-rows palette (plist-get properties :faces) (plist-get properties :variables)))

(defun romacs-theme--plain-rows (properties resolved)
  "`romacs-theme-rows''s value for a theme of PROPERTIES on a Modus
palette, or in a launch without Custom: with :modus, Modus's table with
Romacs's own faces over it and :floor's floors, else RESOLVED's faces;
:faces on top of either."
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
Custom is loaded it is GNU's theme, enabled and disabled as any other,
and its faces are Modus's table, so every theme maps and sizes faces
alike: with :modus (`romacs-theme--modus-palette') the table is over that
palette, Romacs's own faces over it from :palette, then :floor's floors
and :faces on top; without, the palette makes every role of the table
itself (lisp/ui/romacs-theme-roles.el), floors apply, and :faces and
:variables, as `def-romacs-theme' takes them, go on top.  Elsewhere (a
launch without GNU's Lisp) its faces, `romacs-theme-faces', are declared
as the faces' own specs.  :faces set no font
\(`romacs-theme-typography-attributes').
:style is kept as the theme's `romacs-style' property.  Defining it
changes no face that shows."
  (let* ((given (plist-get properties :palette))
         (mode (or (plist-get properties :mode) (romacs-theme--second (assq 'mode given))
                   (romacs-theme--infer-mode given)))
         (properties (plist-put (copy-sequence properties) :mode mode))
         (palette (if (assq 'mode given) given (cons (list 'mode mode) given)))
         (custom (fboundp 'custom-theme-set-faces))
         (rows (if (and custom (not (plist-get properties :modus)))
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
name a slot or be a `romacs-theme-' form.  A font attribute
\(`romacs-theme-typography-attributes', or inheriting a pitch face) is
ignored: text size and family are the user's.  VARIABLES rows are (VARIABLE
FORM), the slots bound.

The result is an ordinary theme, a plain `deftheme': `load-theme',
`enable-theme' and `disable-theme' treat it as any other."
  (declare (indent 1) (doc-string 2))
  (let ((keys nil))
    (while (keywordp (car args))
      (setq keys (append keys (list (car args) (list 'quote (romacs-theme--second args))))
            args (cdr (cdr args))))
    `(romacs-theme-define ',name ,doc :mode ,(plist-get keys :mode)
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

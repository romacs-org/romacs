;;; romacs-effects.el --- Customization for Romacs visual effects  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Every renderer effect is configured through `defcustom', the way
;; `blink-cursor-interval' and friends are: one option per property, with a
;; type, a docstring and a group.  There is no public setter function.  Setting
;; an option with `setq' or through \\[customize] is the whole interface.
;;
;; The properties themselves are defined in Rust and published by
;; `romacs-effect-schema', which reports each property's kind, its bounds and,
;; for a symbol-valued property, the names it accepts.  Option
;; `romacs-EFFECT-PROPERTY' holds EFFECT's PROPERTY, and
;; `romacs-effects--connect' watches each one, as `romacs-effects--define-gallery'
;; watches each decorative effect's plist option, so any way of setting it
;; reaches the renderer (`setq', `setopt', `let', \[customize]).  It also
;; warns at load time about a property with no option, so a property renamed
;; in Rust is a warning here rather than a value that is silently ignored.

;;; Code:

(defgroup romacs nil
  "The Romacs renderer."
  :group 'frames
  :prefix "romacs-")

(defgroup romacs-window-animation nil
  "How a window layout change is animated.

A `split-window' or `delete-window' does not appear all at once: the panes
travel to their new rectangles, the picture that was on screen holds the ground
it has not yet given up, and a window that is appearing fades in.  Four slots
shape that, named as niri names them — one for a window appearing, one for a
window leaving, one for a pane whose size changes, one for a pane that only
moves.

Geometry uses ONE curve for the whole frame, whichever slot supplies it.  Emacs
windows abut, so two panes animating on separate clocks would disagree about
where their shared edge is, and a one-pixel gap on a moving seam is far more
visible than the motion itself."
  :group 'romacs
  :prefix "romacs-window-")

(defgroup romacs-cursor nil
  "Cursor motion and size transitions.
Blinking is GNU's `blink-cursor-mode'."
  :group 'romacs
  :prefix "romacs-cursor-")

(defgroup romacs-transitions nil
  "Animated transitions when a window's contents change."
  :group 'romacs
  :prefix "romacs-")

(defmacro romacs-effects--defslot (slot summary occasion &rest defaults)
  "Define the ten options of window-animation SLOT.

SUMMARY names what the slot animates and OCCASION when it happens; both are
woven into each option's docstring.  DEFAULTS is a plist of the properties
whose default differs from the shared one.

A macro rather than forty written-out options because the four slots really are
parallel — they take the same ten properties, meaning the same things — and
forty hand-copied docstrings would drift apart at the first edit.

DEFAULTS also accepts three keys that shape the options rather than the slot:
`:group' overrides the customization group (default `romacs-window-animation');
`:master' names the master off-switch option the `enabled' docstring points at
(default `romacs-window-animations-off'); `:enabled-doc' replaces the
paragraph that paragraph family keeps about the offscreen composition ring,
which only applies to slots whose animation reads the previous frame;
`:slide' non-nil adds the slot's extra `slide-pixels' option, which the
;; schema publishes for slots that displace; `:scale' non-nil adds the
;; slot's `scale-from' option the same way. Both are optional because the
;; window-animation slots publish neither.
schema publishes for slots that displace while they fade."
  (let* ((name (symbol-name slot))
         (sym (lambda (property) (intern (format "romacs-%s-%s" name property))))
         ;; A quoted value in the caller's plist arrives here as
         ;; `(quote SYM)'; unwrap it so the defgroup symbol itself is
         ;; spliced back quoted.
         (unquote (lambda (value)
                    (if (and (consp value) (eq (car value) 'quote))
                        (cadr value)
                      value)))
         (group (or (funcall unquote (plist-get defaults :group))
                    'romacs-window-animation))
         (master (or (funcall unquote (plist-get defaults :master))
                     'romacs-window-animations-off))
         (get (lambda (property fallback)
                (if (plist-member defaults property)
                    (plist-get defaults property)
                  fallback))))
    `(progn
       (defcustom ,(funcall sym "enabled") ,(funcall get :enabled t)
          ,(format "Whether %s is animated.
When nil, %s happens immediately.

%s" summary occasion
        (or (plist-get defaults :enabled-doc)
            (format "Turning every window-animation slot off also stops the compositor from keeping
the previous frame in a texture, which it must do to animate from it — see
`%s' for the cheaper way to say that." master)))
          :type 'boolean
          :group ',group)

       (defcustom ,(funcall sym "kind") ,(funcall get :kind ''spring)
          ,(format "Which family of curve animates %s.

`easing' is a fixed-duration curve: it takes `romacs-%s-duration' seconds and
follows `romacs-%s-easing'.  `spring' is a second-order spring with no
duration at all — it is described by its stiffness and damping ratio, and
settles when it arrives." summary name name)
          :type '(choice (const :tag "Fixed-duration curve" easing)
                         (const :tag "Second-order spring" spring))
          :group ',group)

       (defcustom ,(funcall sym "duration") ,(funcall get :duration 0.15)
          ,(format "Seconds %s takes, when `romacs-%s-kind' is `easing'.
Zero disables this slot, exactly as setting `romacs-%s-enabled' to nil does.
Ignored for a spring, which has no duration." summary name name)
          :type 'number
          :group ',group)

       (defcustom ,(funcall sym "easing") ,(funcall get :easing ''ease-out-quad)
          ,(format "Curve shape for %s, when `romacs-%s-kind' is `easing'.

`cubic-bezier' takes its control points from the four `romacs-%s-bezier-*'
options, which lets a curve be transcribed directly from a niri configuration.
Note that `spring' here is a fixed curve *shaped* like a spring, and is not the
same thing as setting `romacs-%s-kind' to `spring'." summary name name name)
          :type '(choice (const linear)
                         (const ease-out-quad)
                         (const ease-out-cubic)
                         (const ease-out-expo)
                         (const ease-in-out-cubic)
                         (const :tag "Spring-shaped curve" spring)
                         (const :tag "Custom cubic Bezier" cubic-bezier))
          :group ',group)

       ,@(let ((axes '(("bezier-x1" :bezier-x1 0.0 "first control point's time")
                       ("bezier-y1" :bezier-y1 0.0 "first control point's value")
                       ("bezier-x2" :bezier-x2 1.0 "second control point's time")
                       ("bezier-y2" :bezier-y2 1.0 "second control point's value"))))
           (mapcar
            (lambda (axis)
              (let ((property (nth 0 axis)) (keyword (nth 1 axis))
                    (default (nth 2 axis)) (what (nth 3 axis)))
                `(defcustom ,(funcall sym property) ,default
                   ,(format "The %s of the Bezier curve for %s.
Used only when `romacs-%s-easing' is `cubic-bezier'.  These are the same four
numbers a niri configuration writes as `cubic-bezier(x1, y1, x2, y2)'.

Time is clamped to 0.0-1.0 so the curve stays solvable; value is deliberately
not, because overshooting past 1.0 is the point of a curve like
`cubic-bezier(0.34, 1.56, 0.64, 1.0)'." what summary name)
                   :type 'number
                   :group ',group)))
            axes))

       (defcustom ,(funcall sym "damping-ratio") ,(funcall get :damping-ratio 1.0)
          ,(format "How %s settles, when `romacs-%s-kind' is `spring'.

1.0 is critically damped: the fastest approach that never overshoots.  Below
1.0 the motion bounces past its destination before returning, and above 1.0 it
creeps in more slowly than it needs to.  Clamped to 0.1-10.0." summary name)
          :type 'number
          :group ',group)

       (defcustom ,(funcall sym "stiffness") ,(funcall get :stiffness 800)
          ,(format "How hard the spring pulls %s toward its destination.

Higher is faster.  With the default damping ratio, 800 settles in about a third
of a second.  This is the same number a niri configuration gives as
`stiffness', and it means the same thing." summary)
          :type 'integer
          :group ',group)

       ,@(append
          (when (plist-get defaults :slide)
            `((defcustom ,(funcall sym "slide-pixels")
                  ,(funcall get :slide 0.0)
                ,(format "How far %s displaces vertically while it animates.

An arriving frame starts this many logical pixels below its placement and
rises onto it; a departing frame falls this far while it fades.  Zero
reduces %s to a pure fade.  The displacement shares the slot's own curve,
so a spring-shaped slot overshoots through its placement and settles back."
                 summary name)
                :type 'number
                :group ',group)))
          (when (plist-get defaults :scale)
            `((defcustom ,(funcall sym "scale-from")
                  ,(funcall get :scale 1.0)
                ,(format "The scale %s starts from, as a fraction of its settled size.

1.0 scales nothing.  Below 1.0, an arriving frame grows from this fraction of
its size and a departing frame shrinks toward it, anchored at the frame's own
top-left so the picture grows outward from the point that anchored it.  The
scale shares the slot's own curve, unclamped like the slide: a spring's
overshoot past the settled size is the point." summary)
                :type 'number
                :group ',group)))))))

(romacs-effects--defslot window-open
  "a window appearing"
  "a new window is simply there on the next frame"
  :kind 'easing :duration 0.15 :easing 'ease-out-expo)

(romacs-effects--defslot window-close
  "a window going away"
  "a deleted window vanishes on the next frame"
  ;; Off by default.  A departing pane's ground is being taken by the neighbour
  ;; growing across it, and the old picture is drawn over the new one, so fading
  ;; it shows the deleted window and its replacement half-visible at once.
  ;; Uncovering it as the neighbour arrives reads better, and is what happens
  ;; when this is nil.
  :enabled nil :kind 'easing :duration 0.15 :easing 'ease-out-quad)

(romacs-effects--defslot window-resize
  "a window whose size changes"
  "a resized window snaps to its new size")

(romacs-effects--defslot window-movement
  "a window that moves without changing size"
  "a moved window snaps to its new position")


;;;; Child frames

(defgroup romacs-child-frame nil
  "How a child frame's lifecycle is animated.

A child frame — a posframe, a Corfu or company popup — floats over its parent
frame instead of tiling with other windows, so its four slots are about the
frame's own lifecycle rather than about a layout: one for a popup appearing,
one for one being dismissed, one for one whose anchor moved, one for one whose
size changes.  They share the shape of the `romacs-window-animation' slots —
one curve, spring or easing — but no geometry is shared with other panes, so
each child frame runs its own clock freely.

`open' and `close' ship on.  A popup fading in costs nothing beyond drawing
the popup across its own animation, and a dismissed popup's ground was always
the parent frame showing through — a deleted *window*'s fade shows the window
that replaces it half-transparent underneath, but a popup has no successor.

`movement' and `resize' ship off, deliberately.  Both animate toward where
Emacs says the popup *will* be while it is not there yet, and a tooltip whose
tail points at one specific character reads a few pixels behind as a
positioning bug.  The exact default is stock behaviour; turn them on if your
popups are the kind that follow the cursor."
  :group 'romacs
  :prefix "romacs-child-frame-")

(romacs-effects--defslot child-frame-open
  "a child frame appearing"
  "a popup is simply there on the next frame"
  :group 'romacs-child-frame
  :master 'romacs-child-frame-animations-off
  :enabled-doc "Nothing else needs to be composed offscreen for it."
  :kind 'easing :duration 0.14 :easing 'ease-out-cubic :slide 8.0 :scale 1.0)

(romacs-effects--defslot child-frame-close
  "a child frame going away"
  "a dismissed popup vanishes on the next frame"
  :group 'romacs-child-frame
  :master 'romacs-child-frame-animations-off
  :enabled-doc "Nothing else needs to be composed offscreen for it."
  :kind 'easing :duration 0.09 :easing 'ease-out-cubic :slide 0.0 :scale 1.0)

(romacs-effects--defslot child-frame-movement
  "a child frame whose anchor moved"
  "a re-anchored popup jumps to its new position"
  :group 'romacs-child-frame
  :master 'romacs-child-frame-animations-off
  :enabled-doc "Nothing else needs to be composed offscreen for it."
  :enabled nil :slide 0.0 :scale 1.0)

(romacs-effects--defslot child-frame-resize
  "a child frame whose size changes"
  "a resized popup snaps to its new size"
  :group 'romacs-child-frame
  :master 'romacs-child-frame-animations-off
  :enabled-doc "Nothing else needs to be composed offscreen for it.
Turning this on today does nothing: the slot is wired for a future
content-crossfade, and until then a resize is instant, exactly as GNU Emacs
does it."
  :kind 'easing :duration 0.08 :easing 'ease-out-cubic :slide 0.0 :scale 1.0)


;;;; Global controls

(defcustom romacs-window-animations-off nil
  "Whether to disable every window-animation slot at once.

This is the master switch, and it is cheaper than turning the four slots off
individually.  Animating a layout change requires the frame as it was *before*
the change, so while any slot is on, every frame is composed through an
offscreen ring — two full-frame textures and one extra blit per frame, whether
or not anything is moving.  Setting this reclaims that; clearing an individual
slot's `enabled' does not, because the others still need the previous picture."
  :type 'boolean
  :group 'romacs-window-animation)

(defcustom romacs-window-animations-slowdown 1.0
  "Multiplier on the length of every window animation.

1.0 is normal speed; 20.0 makes a third-of-a-second motion take about seven
seconds.  Clamped to 0.05-20.0.

This exists mainly so an animation can be watched.  A screenshot cannot sample
a 337ms motion, and two of the four slots are springs with no duration to
lengthen by hand, so before this the only way to see one was to rebuild with a
probe."
  :type 'number
  :group 'romacs-window-animation)

(defcustom romacs-child-frame-animations-off nil
  "Whether to disable every child-frame-animation slot at once.

This is the master switch, and it is cheaper than turning the four slots off
individually: while any slot is enabled, a popup's install and dismissal
carry an animation state and a standing scheduler demand, and every frame of
a fade redraws the popup.  Setting this reclaims that; clearing an
individual slot's `enabled' does not, because the others still need it."
  :type 'boolean
  :group 'romacs-child-frame)

(defcustom romacs-child-frame-animations-slowdown 1.0
  "Multiplier on the length of every child-frame animation.

1.0 is normal speed.  Clamped to 0.05-20.0, for the same reason
`romacs-window-animations-slowdown' is: springs have no duration to
lengthen by hand."
  :type 'number
  :group 'romacs-child-frame)

;;;; Cursor

(defcustom romacs-cursor-motion-enabled t
  "Whether the cursor slides between positions instead of jumping.
When nil the cursor is drawn at its new position on the next frame."
  :type 'boolean
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-speed 1.0
  "How fast the cursor travels, as a multiplier.
Higher is faster.  Interacts with `romacs-cursor-motion-style': a style with
its own duration uses `romacs-cursor-motion-duration' instead."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-style 'neovide
  "The curve the cursor follows between positions.

`neovide' moves each corner on its own spring.  A short move, within two
cells and one row, carries the cursor whole in
`romacs-cursor-motion-short-duration'.  A longer jump, or a move to another
window, stretches the cursor along its travel: the leading corners arrive
first, by `romacs-cursor-motion-trail-size', and the jump takes
`romacs-cursor-motion-duration', longer as it is farther.
`exponential' eases toward the target without a fixed arrival time.
`critically-damped-spring' arrives as fast as possible without overshooting.
The remaining choices are fixed-duration curves and use
`romacs-cursor-motion-duration'."
  :type '(choice (const neovide)
                 (const exponential)
                 (const critically-damped-spring)
                 (const linear)
                 (const ease-out-quad)
                 (const ease-out-cubic)
                 (const ease-out-expo)
                 (const ease-in-out-cubic))
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-duration 0.1
  "Seconds the cursor takes to reach its new position.
For `neovide', this is the time a long jump's trailing corners take, up to
1.6 times it for the farthest jumps; short moves take
`romacs-cursor-motion-short-duration'.  For `critically-damped-spring', it sets spring timing.
Used directly by fixed-duration styles; ignored by `exponential'."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-short-duration 0.04
  "Seconds a short cursor move takes in the `neovide' motion style.
A short move stays within two cells and one row of the same window, such as
typing or moving by a line; the cursor slides there whole, without a trail."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-trail-size 0.7
  "How much of a trail the moving cursor leaves, from 0.0 to 1.0.
Zero draws no trail."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-motion-distance-length-adjust t
  "Whether longer cursor jumps take longer in the `neovide' motion style.
Scale `romacs-cursor-motion-duration' by the logarithm of the distance in
cells, up to 1.6 times it."
  :type 'boolean
  :group 'romacs-cursor)

(defcustom romacs-cursor-look-style 'soft
  "How the cursor is drawn.

`soft' lights the selected window's cursor with a glow while the frame
has focus, shows an unselected or unfocused cursor as a faint ghost
(`romacs-cursor-look-ghost-opacity'), and morphs between kinds, a bar
becoming a block, in `romacs-cursor-look-morph'.  `plain' draws GNU's
shapes, unlit."
  :type '(choice (const :tag "Soft: lit, with an eased morph" soft)
                 (const :tag "Plain: GNU's shapes" plain))
  :group 'romacs-cursor)

(defcustom romacs-cursor-look-glow-reach 6.0
  "How far, in pixels, the glow around a lit bar or hbar cursor reaches."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-look-glow-opacity 0.35
  "How strong the glow around a lit cursor is, from 0.0 (none) to 1.0."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-look-ghost-opacity 0.35
  "How visible an unselected window's or an unfocused frame's cursor is,
from 0.0 (not at all) to 1.0 (as the selected one)."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-cursor-look-morph 0.12
  "Seconds a change of cursor kind and a focus change take to show."
  :type 'number
  :group 'romacs-cursor)

(defcustom romacs-motion-reduced nil
  "Whether to turn every animation off: the cursor's glide and morph,
buffer and scroll transitions, window and layer animations, and animated
effects.  Set from the desktop's own setting at startup when it asks for
less motion."
  :type 'boolean
  :group 'romacs)

(defcustom romacs-cursor-size-transition-enabled nil
  "Whether the cursor animates when it changes shape or size.
This is the change from a box to a bar when entering `overwrite-mode', or
between characters of different widths."
  :type 'boolean
  :group 'romacs-cursor)

(defcustom romacs-cursor-size-transition-duration 0.15
  "Seconds a cursor size or shape change takes."
  :type 'number
  :group 'romacs-cursor)

;;;; Content transitions

(defcustom romacs-buffer-transition-enabled nil
  "Whether switching the buffer shown in a window is animated.
When nil the new buffer is drawn on the next frame."
  :type 'boolean
  :group 'romacs-transitions)

(defcustom romacs-buffer-transition-duration 0.2
  "Seconds a buffer switch takes."
  :type 'number
  :group 'romacs-transitions)

(defcustom romacs-buffer-transition-effect 'slide
  "How the outgoing buffer gives way to the incoming one.
`crossfade' blends them; `slide' moves one across the other; the remainder are
decorative."
  :type '(choice (const crossfade) (const slide) (const scale-zoom)
                 (const fade-edges) (const cascade) (const parallax)
                 (const tilt) (const page-curl) (const card-flip)
                 (const cylinder-roll) (const wobbly) (const wave)
                 (const per-line-spring) (const liquid) (const motion-blur)
                 (const chromatic-aberration) (const ghost-trails)
                 (const color-temperature) (const crt-scanlines)
                 (const depth-of-field) (const typewriter-reveal))
  :group 'romacs-transitions)

(defcustom romacs-buffer-transition-easing 'ease-out-quad
  "Curve shape for a buffer switch."
  :type '(choice (const linear) (const ease-out-quad) (const ease-out-cubic)
                 (const ease-out-expo) (const ease-in-out-cubic)
                 (const spring) (const cubic-bezier))
  :group 'romacs-transitions)

(defcustom romacs-buffer-transition-axis 'auto
  "Which way a directional buffer transition moves.
`auto' lets the effect choose, usually horizontally."
  :type '(choice (const auto) (const horizontal) (const vertical))
  :group 'romacs-transitions)

(defcustom romacs-buffer-transition-direction 'forward
  "Which sense a directional buffer transition runs in.
Used when nothing about the switch implies a direction of its own."
  :type '(choice (const forward) (const backward))
  :group 'romacs-transitions)

(defcustom romacs-scroll-transition-enabled nil
  "Whether scrolling a window is animated.
When nil the viewport jumps to its new position on the next frame."
  :type 'boolean
  :group 'romacs-transitions)

(defcustom romacs-scroll-transition-duration 0.15
  "Seconds a scroll takes."
  :type 'number
  :group 'romacs-transitions)

(defcustom romacs-scroll-transition-effect 'slide
  "How the viewport moves when scrolling."
  :type '(choice (const slide) (const crossfade) (const scale-zoom)
                 (const fade-edges) (const cascade) (const parallax)
                 (const tilt) (const page-curl) (const card-flip)
                 (const cylinder-roll) (const wobbly) (const wave)
                 (const per-line-spring) (const liquid) (const motion-blur)
                 (const chromatic-aberration) (const ghost-trails)
                 (const color-temperature) (const crt-scanlines)
                 (const depth-of-field) (const typewriter-reveal))
  :group 'romacs-transitions)

(defcustom romacs-scroll-transition-easing 'ease-out-quad
  "Curve shape for a scroll."
  :type '(choice (const linear) (const ease-out-quad) (const ease-out-cubic)
                 (const ease-out-expo) (const ease-in-out-cubic)
                 (const spring) (const cubic-bezier))
  :group 'romacs-transitions)


;;;; The gallery

;; The ~150 decorative effects are generated rather than written out. One
;; option per effect, holding its properties as a plist, with a `:type' built
;; from the Rust schema so each property still gets a real widget.
;;
;; Per-effect rather than per-property, unlike the behavioural options above,
;; and that difference is deliberate. Nobody completes on
;; `romacs-effect-argyle-pattern-diamond-size'; what you do with a gallery is
;; browse it, turn one on, and nudge a number. Generating 800 more symbols
;; would bury the 61 above in `M-x customize-apropos', and each would carry a
;; docstring nobody wrote.

(defun romacs-effects--property-type (entry)
  "Build a customization type for one `romacs-effect-schema' ENTRY."
  (pcase (cadr entry)
    ('boolean 'boolean)
    ('integer 'integer)
    ('seconds '(number :tag "Seconds"))
    ('unit-interval '(number :tag "0.0 to 1.0"))
    ('percentage '(number :tag "0 to 100"))
    ('color '(choice (string :tag "#RRGGBB")
                     (repeat :tag "Channels" number)))
    ('color-list '(repeat (repeat :tag "Channels" number)))
    ('symbol `(choice ,@(mapcar (lambda (name) (list 'const name))
                                (cddr entry))))
    (_ 'sexp)))

(defun romacs-effects--plist-type (schema)
  "Build a plist customization type from SCHEMA."
  `(plist :options
          ,(mapcar (lambda (entry)
                     (list (car entry) (romacs-effects--property-type entry)))
                   schema)))

(defun romacs-effects--define-gallery ()
  "Define one option per decorative effect, from the Rust schema."
  (dolist (effect (romacs-effect-names 'shader))
    (let* ((schema (romacs-effect-schema effect))
           (symbol (intern (format "romacs-effect-%s" effect))))
      (custom-declare-variable
       symbol
       `',(romacs-effect-get effect)
       (format "Properties of the `%s' renderer effect.

A plist of the properties this effect accepts; see the widget for the type of
each.  A property the plist leaves out has the renderer's default.  Decorative
effects are configured one effect at a time rather than one property at a
time — see the commentary in romacs-effects.el."
               effect)
       :type (romacs-effects--plist-type schema)
       :group 'romacs-effects-gallery)
      ;; Watched only once defined: the default is the renderer's own.
      (put symbol 'romacs-effect (list effect))
      (add-variable-watcher symbol #'romacs-effects--watch))))

(defgroup romacs-effects-gallery nil
  "Decorative renderer effects.

Roughly a hundred and fifty shader effects — cursor comets, auroras, patterned
backgrounds.  Each is one option holding that effect's properties."
  :group 'romacs
  :prefix "romacs-effect-")

;;;; The desktop's motion

(defun romacs-motion--desktop-reduced-p ()
  "Whether the desktop asks for less motion: GNOME's and GTK's
`enable-animations' turned off."
  (and (executable-find "gsettings")
       (equal (car (ignore-errors
                     (process-lines "gsettings" "get" "org.gnome.desktop.interface"
                                    "enable-animations")))
              "false")))

(defun romacs-motion-follow-desktop ()
  "Reduce motion when the desktop asks for it and the user has not chosen.
The user's choice is `romacs-motion-reduced' saved, customized, or set
non-nil."
  (unless (or romacs-motion-reduced
              (get 'romacs-motion-reduced 'saved-value)
              (get 'romacs-motion-reduced 'customized-value))
    (when (romacs-motion--desktop-reduced-p)
      ;; Not `setopt': its macro autoloads cus-edit, which loads cus-start
      ;; again where the session's world is being built.  The watcher
      ;; reaches the renderer either way.
      (set-default 'romacs-motion-reduced t))))

;;;; Reaching the renderer

(defconst romacs-effects--behavioural
  '(window-open window-close window-resize window-movement window-animations
    cursor-motion cursor-size-transition cursor-look motion buffer-transition scroll-transition
    child-frame-open child-frame-close child-frame-movement child-frame-resize
    child-frame-animations)
  "The effects whose properties are written out above, one option each.")

(defun romacs-effects--watch (symbol value operation where)
  "Send VALUE, SYMBOL's new global value, to the renderer.
A watcher (`add-variable-watcher') for OPERATION in buffer WHERE.  SYMBOL's
`romacs-effect' property is (EFFECT PROPERTY) for an option holding one
property, or (EFFECT) for a gallery option holding EFFECT's plist.  A value
the renderer refuses is an error, before SYMBOL changes: the variable never
holds a value the renderer does not show."
  (when (and (memq operation '(set let unlet)) (null where))
    (pcase-let ((`(,effect ,property) (get symbol 'romacs-effect)))
      (condition-case failure
          (if property
              (romacs--effect-set effect property value)
            (apply #'romacs--effect-replace effect value))
        (error (user-error "`%s': %s" symbol (error-message-string failure)))))))

(defun romacs-effects--connect ()
  "Watch each behavioural option, and warn about a property without one.
The options are written by hand, so a property renamed or removed in Rust
would leave an option that silently does nothing; this reports it instead."
  (dolist (effect romacs-effects--behavioural)
    (dolist (entry (romacs-effect-schema effect))
      (let ((symbol (intern (format "romacs-%s-%s" effect
                                    (substring (symbol-name (car entry)) 1)))))
        (if (not (boundp symbol))
            (display-warning
             'romacs
             (format "%s accepts %s but there is no `%s' to set it"
                     effect (car entry) symbol)
             :warning)
          (put symbol 'romacs-effect (list effect (car entry)))
          (add-variable-watcher symbol #'romacs-effects--watch))))))

(romacs-effects--define-gallery)
(romacs-effects--connect)

(provide 'romacs-effects)

;;; romacs-effects.el ends here

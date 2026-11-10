//! How a child frame's lifecycle animates: five slots in the same vocabulary
//! the window animations use.
//!
//! A child frame (posframe, Corfu, a Romacs layer) is a floating overlay
//! composited over its parent, so its animation questions are the window
//! ones minus the tiling constraints. Romacs's layer animation (design note
//! ui-layer §j; Zed gpui's `Animation` with an easing over a duration is
//! the concept reference) uses four of the slots:
//!
//! * `open` — a 140ms ease-out-cubic fade with an 8px slide from the
//!   frame's anchored edge ([`LayerEdge`]).
//! * `close` — a 90ms fade.
//! * `resize` — an 80ms tween of the frame's rect, as a candidate list
//!   grows or shrinks: the new content is placed where it settles and
//!   clipped to the moving rect, never stretched.
//! * `selection` — a 60ms glide of a layer's selection band between rows.
//!
//! `movement` ships off: a popup whose anchor is exact (a tooltip under
//! one character) reads a few-pixels-behind target as a positioning bug.
//! Whether any of them applies to a frame is its [`Layer`], which
//! Lisp's `romacs-layer-animation` sets.
//!
//! # Why a copy of the ten window-animation slots rather than a shared type
//!
//! [`crate::window_animation::WindowAnimation`] is deliberately flat: the
//! effect registry reflects config structs through serde and can carry only
//! scalar property values. Two slots here need one property the window ones
//! do not have — `slide-pixels`, how far the frame displaces while it fades
//! — and nesting a `WindowAnimation` inside this struct would serialize to
//! an object and break `romacs-effect-get` for the slot. So the ten scalars
//! are restated here and the conversion to a [`MotionSpec`] mirrors
//! [`crate::window_animation::WindowAnimation::motion`] instead of sharing
//! code. The test beside the definition pins the two behaviours together;
//! that is cheaper than a registry that cannot express the slot.

use crate::motion_spec::{
    AngularFrequency, DampingRatio, MotionDuration, MotionSpec, SpringSpec, TweenSpec, UnitBezier,
};
use crate::scroll_animation::TransitionEasing;
use crate::window_animation::{
    MAX_DAMPING_RATIO, MAX_SLOWDOWN, MIN_DAMPING_RATIO, MIN_SLOWDOWN, MotionKind,
};
use std::time::Duration;

/// One child-frame lifecycle slot.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChildFrameAnimation {
    pub enabled: bool,
    pub kind: MotionKind,
    /// Read when `kind` is `easing`. Zero disables the slot.
    pub duration: Duration,
    /// Read when `kind` is `easing`.
    pub easing: TransitionEasing,
    /// Bezier control points, read when `easing` is `cubic-bezier`.
    pub bezier_x1: f32,
    pub bezier_y1: f32,
    pub bezier_x2: f32,
    pub bezier_y2: f32,
    /// Read when `kind` is `spring`. Clamped to niri's own `0.1..=10.0`.
    pub damping_ratio: f32,
    /// Read when `kind` is `spring`. Clamped to at least 1, as niri requires.
    pub stiffness: u32,
    /// How far the frame displaces along the vertical axis while it animates,
    /// in logical pixels.
    ///
    /// An opening frame starts this far *below* its placement and rises to
    /// rest; a closing frame falls this far while it fades. Zero (the `close`
    /// default) reduces the slot to a pure fade. Displacement follows the
    /// same curve as the fade, so a spring-shaped slot overshoots through its
    /// placement and settles back — the property a pop-in reads as.
    pub slide_pixels: f32,
    /// The scale the frame starts from, as a fraction of its settled size.
    ///
    /// `1.0` (every slot's default) scales nothing: the transform is an
    /// identity in the draw pipeline and costs nothing. Below 1.0, an
    /// arriving frame grows from this fraction of its size — anchored at
    /// its own top-left, so it grows outward from the point that anchored
    /// it — while a departing frame shrinks toward it. The scale shares the
    /// slot's curve, unclamped like the slide: a spring's overshoot past
    /// 1.0 is the point.
    pub scale_from: f32,
}

impl ChildFrameAnimation {
    const fn easing(millis: u64, easing: TransitionEasing, slide_pixels: f32) -> Self {
        Self {
            enabled: true,
            kind: MotionKind::Easing,
            duration: Duration::from_millis(millis),
            easing,
            bezier_x1: 0.0,
            bezier_y1: 0.0,
            bezier_x2: 1.0,
            bezier_y2: 1.0,
            damping_ratio: 1.0,
            stiffness: 800,
            slide_pixels,
            scale_from: 1.0,
        }
    }

    const fn spring(damping_ratio: f32, stiffness: u32) -> Self {
        Self {
            enabled: true,
            kind: MotionKind::Spring,
            duration: Duration::from_millis(150),
            easing: TransitionEasing::EaseOutQuad,
            bezier_x1: 0.0,
            bezier_y1: 0.0,
            bezier_x2: 1.0,
            bezier_y2: 1.0,
            damping_ratio,
            stiffness,
            slide_pixels: 0.0,
            scale_from: 1.0,
        }
    }

    /// The spec the renderer samples, or `Instant` when there is nothing to
    /// animate.
    ///
    /// Clamped rather than rejected, for the same reason
    /// [`crate::window_animation::WindowAnimation::motion`] is: a partial
    /// effect-profile push must not silently revert the user's whole child
    /// frame configuration because one number was out of range.
    #[must_use]
    pub fn motion(&self, globals: ChildFrameAnimationsConfig) -> MotionSpec {
        if !self.enabled || globals.off {
            return MotionSpec::Instant;
        }
        let slowdown = if globals.slowdown.is_finite() {
            globals.slowdown.clamp(MIN_SLOWDOWN, MAX_SLOWDOWN)
        } else {
            1.0
        };
        match self.kind {
            MotionKind::Easing => MotionDuration::new(self.duration.mul_f32(slowdown)).map_or(
                MotionSpec::Instant,
                |duration| {
                    MotionSpec::Tween(TweenSpec {
                        duration,
                        easing: self.easing,
                        bezier: matches!(self.easing, TransitionEasing::CubicBezier).then(|| {
                            UnitBezier::new(
                                self.bezier_x1,
                                self.bezier_y1,
                                self.bezier_x2,
                                self.bezier_y2,
                            )
                        }),
                    })
                },
            ),
            MotionKind::Spring => {
                let omega = self.omega() / slowdown;
                let zeta = if self.damping_ratio.is_finite() {
                    self.damping_ratio
                        .clamp(MIN_DAMPING_RATIO, MAX_DAMPING_RATIO)
                } else {
                    1.0
                };
                match (AngularFrequency::new(omega), DampingRatio::new(zeta)) {
                    (Ok(omega), Ok(damping)) => MotionSpec::Spring(SpringSpec { omega, damping }),
                    _ => MotionSpec::Instant,
                }
            }
        }
    }

    /// This slot's undamped angular frequency, in radians per second.
    ///
    /// Same parameterisation as
    /// [`crate::window_animation::WindowAnimation::omega`]; see that method
    /// for the niri conversion.
    #[must_use]
    pub fn omega(&self) -> f32 {
        f64::from(self.stiffness.max(1)).sqrt() as f32
    }
}

/// Global controls over every child-frame slot, matching
/// [`crate::window_animation::WindowAnimationsConfig`].
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChildFrameAnimationsConfig {
    /// Master off switch. Every slot resolves to `MotionSpec::Instant`.
    pub off: bool,
    /// Multiplies every slot's wall-clock length. `1.0` is normal speed.
    pub slowdown: f32,
}

impl Default for ChildFrameAnimationsConfig {
    fn default() -> Self {
        Self {
            off: false,
            slowdown: 1.0,
        }
    }
}

/// A layer appearing: 140ms ease-out cubic with an 8px slide from its
/// anchored edge.
#[must_use]
pub(crate) fn default_child_frame_open() -> ChildFrameAnimation {
    ChildFrameAnimation::easing(140, TransitionEasing::EaseOutCubic, 8.0)
}

/// A layer going away: a 90ms fade.
#[must_use]
pub(crate) fn default_child_frame_close() -> ChildFrameAnimation {
    ChildFrameAnimation::easing(90, TransitionEasing::EaseOutCubic, 0.0)
}

/// Drift toward a new anchor: a critically damped spring, **off**.
#[must_use]
pub(crate) fn default_child_frame_movement() -> ChildFrameAnimation {
    ChildFrameAnimation {
        enabled: false,
        ..ChildFrameAnimation::spring(1.0, 800)
    }
}

/// A layer whose size changes, as a candidate list grows or shrinks: its
/// rect tweens over 80ms from the old to the new, the new content placed
/// where it settles and clipped to the rect, so no picture is stretched.
#[must_use]
pub(crate) fn default_child_frame_resize() -> ChildFrameAnimation {
    ChildFrameAnimation::easing(80, TransitionEasing::EaseOutCubic, 0.0)
}

/// A layer's selection highlight gliding between rows: 60ms.
#[must_use]
pub(crate) fn default_child_frame_selection() -> ChildFrameAnimation {
    ChildFrameAnimation::easing(60, TransitionEasing::EaseOutCubic, 0.0)
}

/// One child frame shown as a Romacs layer: how it animates (whether, from
/// which edge, and where its selection is; the slots above say how long
/// and on what curve) and how it looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layer {
    pub animation: LayerAnimation,
    pub edge: LayerEdge,
    /// The band its selection highlight paints, in the frame's own
    /// coordinates, when it shows one.
    pub selection: Option<SelectionBand>,
    /// Its corners' radius in logical pixels; zero is square.
    pub radius: f32,
    /// The colour it glows in, outside its edge (graphics.md's "light,
    /// not chrome": a raised layer glows, it is not outlined).
    pub glow: Option<crate::Color>,
    /// How far its glow reaches, in logical pixels, and how opaque it is.
    pub glow_light: [f32; 2],
    /// The soft shadow it casts onto what lies beneath, when it casts one:
    /// its reach and drop in logical pixels, and its opacity.
    pub shadow: Option<[f32; 3]>,
    /// A 1px hairline round its edge in this colour, glowing or not (an
    /// error toast's, which must read apart from an ordinary one).
    pub outline: Option<crate::Color>,
}

/// `romacs-layer-animation': whether a layer animates. `Full` leaves every
/// slot to the visual configuration (the default for frames no layer
/// produced); `Reduced` keeps only the open and close fades, without
/// their slide; `Off` animates nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerAnimation {
    Off,
    Reduced,
    #[default]
    Full,
}

/// The edge a layer is anchored at: it slides in from there, and a height
/// change keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerEdge {
    #[default]
    Bottom,
    Top,
}

/// A layer's selection highlight: the rounded row band it paints in COLOR
/// under the row's text, and BASE, the colour the row shows without it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionBand {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub color: crate::Color,
    pub base: crate::Color,
}

#[cfg(test)]
#[path = "child_frame_animation/tests/child_frame_animation_test.rs"]
mod tests;

crate::effect_schema!(ChildFrameAnimation {
    enabled: bool,
    kind: MotionKind,
    duration: Duration,
    easing: TransitionEasing,
    bezier_x1: f32,
    bezier_y1: f32,
    bezier_x2: f32,
    bezier_y2: f32,
    damping_ratio: f32,
    stiffness: u32,
    slide_pixels: f32,
    scale_from: f32,
});
crate::effect_schema!(ChildFrameAnimationsConfig {
    off: bool,
    slowdown: f32
});

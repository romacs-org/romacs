//! Property motion: sampling a [`MotionSpec`] on a scene's document clock.
//!
//! A [`Track`] binds one property of one node to a curve that started at a
//! document time. Sampling is a pure function of the track and the clock, so
//! two frames at the same document time are identical and a frame never drawn
//! costs nothing (the rationale of `frame_compositor/motion.rs`, whose curves
//! these are; S3 moves the layer slots onto this sampler so there is one).
//!
//! Repetition wraps the same curve rather than adding a clock: `Repeat`
//! replays a finite tween, and a drive advances an unbounded property at a
//! rate. Both read the scene's [`MediaClock`], which pauses while no
//! placement is visible and never catches up.

use std::time::Duration;

use romacs_display_protocol::motion_spec::MotionSpec;

use crate::node::{Color, Dim};
use crate::prop::{Prop, Value};

/// Document time of one scene: advances only while a placement is visible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaClock {
    now: Duration,
    last_wall: Option<Duration>,
}

impl MediaClock {
    #[must_use]
    pub const fn now(self) -> Duration {
        self.now
    }

    /// The clock after a presented frame at host time `wall`. Time passes only
    /// between two visible frames, so a hidden stretch is skipped, not replayed.
    #[must_use]
    pub fn tick(self, wall: Duration, visible: bool) -> Self {
        let elapsed = match (visible, self.last_wall) {
            (true, Some(last)) => wall.saturating_sub(last),
            _ => Duration::ZERO,
        };
        Self {
            now: self.now + elapsed,
            last_wall: visible.then_some(wall),
        }
    }
}

/// `romacs-animation`: `t`, `reduced` or `nil`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Policy {
    #[default]
    On,
    Reduced,
    Off,
}

/// How often a finite tween replays.
///
/// `count` counts legs toward the target. Alternating, each but the last is
/// followed by a leg back, so every finite repeat ends on the target: the
/// value Lisp set is the value the node keeps.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Repeat {
    #[default]
    Once,
    Times {
        count: u32,
        alternate: bool,
    },
    Forever {
        alternate: bool,
    },
}

/// The `:motion` of a `set` op.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animate {
    Motion {
        spec: MotionSpec,
        repeat: Repeat,
    },
    /// Advance by `rate` units per second, forever.
    Drive {
        rate: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum Curve {
    Motion { spec: MotionSpec, repeat: Repeat },
    Drive { rate: f32, base: f32, angle: bool },
}

/// One running property motion.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    from: Value,
    to: Value,
    start: Duration,
    curve: Curve,
}

/// How long the reduced-motion fade takes.
const REDUCED_FADE: Duration = Duration::from_millis(90);

impl Track {
    /// A track carrying `prop` from `from` to `to`, or `None` when the policy
    /// or the spec make it a snap. A drive starts at `to` when it is a number.
    #[must_use]
    pub fn start(
        prop: Prop,
        from: Value,
        to: Value,
        animate: Animate,
        now: Duration,
        policy: Policy,
    ) -> Option<Self> {
        let curve = match (animate, policy) {
            (_, Policy::Off) | (Animate::Drive { .. }, Policy::Reduced) => return None,
            (Animate::Drive { rate }, Policy::On) => match to {
                Value::Num(base) => Curve::Drive {
                    rate,
                    base,
                    angle: prop.is_angle(),
                },
                _ => return None,
            },
            (Animate::Motion { spec, .. }, _) if spec.is_instant() => return None,
            (Animate::Motion { spec, repeat }, Policy::On) => Curve::Motion { spec, repeat },
            (Animate::Motion { repeat, .. }, Policy::Reduced) => {
                let fades = matches!(prop, Prop::Opacity) || matches!(to, Value::Color(_));
                if !fades || repeat != Repeat::Once {
                    return None;
                }
                Curve::Motion {
                    spec: reduced_fade(),
                    repeat: Repeat::Once,
                }
            }
        };
        let drive = matches!(curve, Curve::Drive { .. });
        (drive || Value::interpolates(prop, &from, &to)).then_some(Self {
            from,
            to,
            start: now,
            curve,
        })
    }

    /// The value at document time `now`.
    #[must_use]
    pub fn sample(&self, now: Duration) -> Value {
        let elapsed = now.saturating_sub(self.start).as_secs_f32();
        match self.curve {
            Curve::Drive { rate, base, angle } => {
                let value = base + rate * elapsed;
                Value::Num(if angle {
                    value.rem_euclid(360.0)
                } else {
                    value
                })
            }
            Curve::Motion { spec, repeat } => {
                let (progress, backward) = cycle(spec, repeat, elapsed);
                let p = if backward { 1.0 - progress } else { progress };
                lerp(&self.from, &self.to, p)
            }
        }
    }

    /// Whether the track has reached its final value and can be dropped.
    #[must_use]
    pub fn settled(&self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.start).as_secs_f32();
        match self.curve {
            Curve::Drive { .. } => false,
            Curve::Motion { spec, repeat } => match repeat {
                Repeat::Once => finished(spec, elapsed),
                Repeat::Times { count, alternate } => {
                    tween_secs(spec).is_none_or(|d| elapsed >= d * legs(count, alternate) as f32)
                }
                Repeat::Forever { .. } => false,
            },
        }
    }

    #[must_use]
    pub const fn is_drive(&self) -> bool {
        matches!(self.curve, Curve::Drive { .. })
    }
}

/// Progress through the current cycle and whether it runs backward.
fn cycle(spec: MotionSpec, repeat: Repeat, elapsed: f32) -> (f32, bool) {
    let (cycles, alternate) = match repeat {
        Repeat::Once => return (progress(spec, elapsed), false),
        Repeat::Times { count, alternate } => (Some(legs(count, alternate)), alternate),
        Repeat::Forever { alternate } => (None, alternate),
    };
    let Some(duration) = tween_secs(spec) else {
        return (progress(spec, elapsed), false);
    };
    let total = cycles.map(|n| n as f32 * duration);
    let at = total.map_or(elapsed, |t| elapsed.min(t));
    let index = (at / duration).floor();
    // The end of the last cycle is that cycle's end, not the next one's start.
    let (index, within) = match total {
        Some(t) if at >= t => (index - 1.0, duration),
        _ => (index, at - index * duration),
    };
    let backward = alternate && (index as u64) % 2 == 1;
    (progress(spec, within), backward)
}

/// Legs of a finite repeat: `count` toward the target, and between them the
/// legs back when alternating.
fn legs(count: u32, alternate: bool) -> u32 {
    let count = count.max(1);
    if alternate {
        count.saturating_mul(2) - 1
    } else {
        count
    }
}

fn tween_secs(spec: MotionSpec) -> Option<f32> {
    match spec {
        MotionSpec::Tween(t) => Some(t.duration.as_secs_f32()),
        _ => None,
    }
}

fn reduced_fade() -> MotionSpec {
    use romacs_display_protocol::motion_spec::{MotionDuration, TweenSpec};
    use romacs_display_protocol::scroll_animation::TransitionEasing;
    MotionDuration::new(REDUCED_FADE).map_or(MotionSpec::Instant, |duration| {
        MotionSpec::Tween(TweenSpec {
            duration,
            easing: TransitionEasing::Linear,
            bezier: None,
        })
    })
}

/// Interpolate two values; `t` may overshoot `[0, 1]` (springs).
#[must_use]
pub fn lerp(from: &Value, to: &Value, t: f32) -> Value {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    match (from, to) {
        (Value::Num(a), Value::Num(b)) => Value::Num(mix(*a, *b)),
        (Value::Dim(Dim::Px(a)), Value::Dim(Dim::Px(b))) => {
            Value::Dim(Dim::Px(mix(*a, *b).max(0.0)))
        }
        (Value::Color(a), Value::Color(b)) => {
            let c = |x: f32, y: f32| mix(x, y).clamp(0.0, 1.0);
            Value::Color(Color::rgba(
                c(a.r, b.r),
                c(a.g, b.g),
                c(a.b, b.b),
                c(a.a, b.a),
            ))
        }
        _ => to.clone(),
    }
}

/// How close to rest a spring must be, in value and rate, to be finished.
const SPRING_REST_EPSILON: f32 = 0.001;
/// Where a decay is called finished.
const DECELERATION_REST_EPSILON: f32 = 0.001;

/// Progress `seconds` into a motion that started at rest; may overshoot 1.
#[must_use]
pub fn progress(spec: MotionSpec, seconds: f32) -> f32 {
    match spec {
        MotionSpec::Tween(tween) => {
            let u = (seconds / tween.duration.as_secs_f32()).clamp(0.0, 1.0);
            tween
                .bezier
                .map_or_else(|| tween.easing.apply(u), |bezier| bezier.apply(u))
        }
        MotionSpec::Spring(spring) => {
            spring_progress(spring.omega.get(), spring.damping.get(), seconds)
        }
        MotionSpec::Deceleration(d) => 1.0 - (-d.friction.get() * seconds).exp(),
        // Instant, and any variant added later: show the destination.
        _ => 1.0,
    }
}

fn finished(spec: MotionSpec, seconds: f32) -> bool {
    match spec {
        MotionSpec::Tween(tween) => seconds >= tween.duration.as_secs_f32(),
        MotionSpec::Spring(spring) => {
            let at_target = (progress(spec, seconds) - 1.0).abs() < SPRING_REST_EPSILON;
            // Only an under-damped spring passes its target while still moving.
            at_target
                && (spring.damping.get() >= 1.0 || rate(spec, seconds).abs() < SPRING_REST_EPSILON)
        }
        MotionSpec::Deceleration(d) => {
            (-d.friction.get() * seconds).exp() < DECELERATION_REST_EPSILON
        }
        _ => true,
    }
}

fn rate(spec: MotionSpec, seconds: f32) -> f32 {
    const H: f32 = 1e-4;
    let before = (seconds - H).max(0.0);
    (progress(spec, seconds + H) - progress(spec, before)) / (H + seconds - before)
}

/// A second-order spring's normalized displacement at `t`, from rest.
fn spring_progress(omega: f32, zeta: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let error = if zeta < 1.0 {
        let damped = omega * (1.0 - zeta * zeta).sqrt();
        let envelope = (-zeta * omega * t).exp();
        envelope * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
    } else if (zeta - 1.0).abs() < f32::EPSILON {
        (1.0 + omega * t) * (-omega * t).exp()
    } else {
        let root = omega * (zeta * zeta - 1.0).sqrt();
        let fast = -zeta * omega + root;
        let slow = -zeta * omega - root;
        let a = slow / (slow - fast);
        a * (fast * t).exp() + (1.0 - a) * (slow * t).exp()
    };
    1.0 - error
}

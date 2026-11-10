//! Transition policy: which transition a buffer switch or scroll gets, from
//! the effective visual config.

use romacs_display_protocol::{
    AxisMotionTransitionEffect, BufferTransitionConfig, ContentTransitionIntent,
    DirectionlessTransitionEffect, HorizontalTransitionEffect, Rect, ResolvedTransitionEffect,
    ScrollTransitionConfig, TransitionAxis, TransitionDirection, TransitionEasing, TransitionEdge,
    TransitionEffect, VerticalPostProcessTransitionEffect, VerticalTransitionEffect, VisualConfig,
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionPlan {
    pub duration: Duration,
    pub easing: TransitionEasing,
    pub bounds: Rect,
    pub effect: ResolvedTransitionEffect,
}

fn resolve_effect(
    effect: TransitionEffect,
    axis: TransitionAxis,
    direction: TransitionDirection,
    axis_distance: f32,
    vertical_distance: f32,
) -> ResolvedTransitionEffect {
    match effect {
        TransitionEffect::Crossfade => {
            ResolvedTransitionEffect::Directionless(DirectionlessTransitionEffect::Crossfade)
        }
        TransitionEffect::ScaleZoom => {
            ResolvedTransitionEffect::Directionless(DirectionlessTransitionEffect::ScaleZoom)
        }
        TransitionEffect::Slide => ResolvedTransitionEffect::AxisMotion {
            effect: AxisMotionTransitionEffect::Slide,
            axis,
            direction,
            distance: axis_distance,
        },
        TransitionEffect::Parallax => ResolvedTransitionEffect::AxisMotion {
            effect: AxisMotionTransitionEffect::Parallax,
            axis,
            direction,
            distance: axis_distance,
        },
        TransitionEffect::CardFlip => ResolvedTransitionEffect::CardFlip { axis },
        TransitionEffect::PageCurl => ResolvedTransitionEffect::PageCurl {
            edge: TransitionEdge::from_axis_direction(axis, direction),
        },
        TransitionEffect::TypewriterReveal => ResolvedTransitionEffect::Horizontal {
            effect: HorizontalTransitionEffect::TypewriterReveal,
            direction,
        },
        TransitionEffect::FadeEdges => vertical(
            VerticalTransitionEffect::FadeEdges,
            direction,
            vertical_distance,
        ),
        TransitionEffect::Cascade => vertical(
            VerticalTransitionEffect::Cascade,
            direction,
            vertical_distance,
        ),
        TransitionEffect::Tilt => {
            vertical(VerticalTransitionEffect::Tilt, direction, vertical_distance)
        }
        TransitionEffect::CylinderRoll => vertical(
            VerticalTransitionEffect::CylinderRoll,
            direction,
            vertical_distance,
        ),
        TransitionEffect::Wobbly => vertical(
            VerticalTransitionEffect::Wobbly,
            direction,
            vertical_distance,
        ),
        TransitionEffect::Wave => {
            vertical(VerticalTransitionEffect::Wave, direction, vertical_distance)
        }
        TransitionEffect::PerLineSpring => vertical(
            VerticalTransitionEffect::PerLineSpring,
            direction,
            vertical_distance,
        ),
        TransitionEffect::Liquid => vertical(
            VerticalTransitionEffect::Liquid,
            direction,
            vertical_distance,
        ),
        TransitionEffect::MotionBlur => vertical_post_process(
            VerticalPostProcessTransitionEffect::MotionBlur,
            direction,
            vertical_distance,
        ),
        TransitionEffect::ChromaticAberration => vertical_post_process(
            VerticalPostProcessTransitionEffect::ChromaticAberration,
            direction,
            vertical_distance,
        ),
        TransitionEffect::GhostTrails => vertical_post_process(
            VerticalPostProcessTransitionEffect::GhostTrails,
            direction,
            vertical_distance,
        ),
        TransitionEffect::ColorTemperature => vertical_post_process(
            VerticalPostProcessTransitionEffect::ColorTemperature,
            direction,
            vertical_distance,
        ),
        TransitionEffect::CRTScanlines => vertical_post_process(
            VerticalPostProcessTransitionEffect::CRTScanlines,
            direction,
            vertical_distance,
        ),
        TransitionEffect::DepthOfField => vertical_post_process(
            VerticalPostProcessTransitionEffect::DepthOfField,
            direction,
            vertical_distance,
        ),
    }
}

fn vertical_post_process(
    effect: VerticalPostProcessTransitionEffect,
    direction: TransitionDirection,
    distance: f32,
) -> ResolvedTransitionEffect {
    ResolvedTransitionEffect::VerticalPostProcess {
        effect,
        direction,
        distance,
    }
}

fn vertical(
    effect: VerticalTransitionEffect,
    direction: TransitionDirection,
    distance: f32,
) -> ResolvedTransitionEffect {
    ResolvedTransitionEffect::Vertical {
        effect,
        direction,
        distance,
    }
}

/// Animation policy for per-window transitions.
///
/// Smallest viewport, in device pixels, a scroll transition is worth
/// running in.
///
/// Below this a slide is more distracting than informative, and the
/// retained strip is mostly chrome. Device pixels make the judgement mean
/// the same thing on a HiDPI display as on a 1x one.
pub(in crate::render_thread) const MINIMUM_SCROLL_REGION_HEIGHT_PX: f32 = 50.0;

/// The buffer and scroll transition config the render thread plans from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionPolicy {
    pub buffer: BufferTransitionConfig,
    pub scroll: ScrollTransitionConfig,
}

impl TransitionPolicy {
    /// True when at least one transition path needs offscreen snapshots.
    pub fn needs_offscreen(&self) -> bool {
        self.buffer.enabled || self.scroll.enabled
    }

    pub fn buffer_plan(
        &self,
        bounds: Rect,
        intent: ContentTransitionIntent,
    ) -> Option<TransitionPlan> {
        if !self.buffer.enabled {
            return None;
        }
        let axis = self.buffer.axis.resolve(TransitionAxis::Horizontal);
        let direction = intent.resolve(self.buffer.direction);
        Some(TransitionPlan {
            duration: self.buffer.duration,
            easing: self.buffer.easing,
            bounds,
            effect: resolve_effect(
                self.buffer.effect,
                axis,
                direction,
                axis.extent(bounds),
                bounds.height,
            ),
        })
    }

    pub fn scroll_plan(
        &self,
        bounds: Rect,
        direction: TransitionDirection,
        distance: f32,
    ) -> Option<TransitionPlan> {
        if !self.scroll.enabled {
            return None;
        }
        let distance = distance.max(0.0).min(bounds.height);
        Some(TransitionPlan {
            duration: self.scroll.duration,
            easing: self.scroll.easing,
            bounds,
            effect: resolve_effect(
                self.scroll.effect,
                TransitionAxis::Vertical,
                direction,
                distance,
                distance,
            ),
        })
    }
}

impl Default for TransitionPolicy {
    fn default() -> Self {
        Self::from(&VisualConfig::default())
    }
}

impl From<&VisualConfig> for TransitionPolicy {
    fn from(config: &VisualConfig) -> Self {
        Self {
            buffer: config.buffer_transition,
            scroll: config.scroll_transition,
        }
    }
}

#[cfg(test)]
mod tests;

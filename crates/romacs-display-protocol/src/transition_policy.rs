//! Buffer and viewport transition vocabulary shared by the Lisp-facing
//! config, the render thread's transition policy, and the renderer.

use crate::Rect;
use strum::{EnumString, IntoStaticStr};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    EnumString,
    IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
#[derive(strum::VariantNames)]
pub enum TransitionAxisPreference {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    EnumString,
    IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum TransitionAxis {
    Horizontal,
    Vertical,
}

impl TransitionAxis {
    pub fn extent(self, bounds: Rect) -> f32 {
        match self {
            Self::Horizontal => bounds.width,
            Self::Vertical => bounds.height,
        }
    }
}

impl TransitionAxisPreference {
    pub fn resolve(self, automatic: TransitionAxis) -> TransitionAxis {
        match self {
            Self::Auto => automatic,
            Self::Horizontal => TransitionAxis::Horizontal,
            Self::Vertical => TransitionAxis::Vertical,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    EnumString,
    IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
#[derive(strum::VariantNames)]
pub enum TransitionDirection {
    #[default]
    Forward,
    Backward,
}

impl TransitionDirection {
    pub const fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Backward => -1.0,
        }
    }
}

/// Semantic reason for replacing stable content geometry.
///
/// Navigation carries the user's direction across the evaluator/layout/render
/// boundary.  A replacement without navigation deliberately delegates to the
/// configured fallback direction instead of inventing intent from identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ContentTransitionIntent {
    #[default]
    Replace,
    Navigate(TransitionDirection),
}

impl ContentTransitionIntent {
    pub const fn resolve(self, fallback: TransitionDirection) -> TransitionDirection {
        match self {
            Self::Replace => fallback,
            Self::Navigate(direction) => direction,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionlessTransitionEffect {
    Crossfade,
    ScaleZoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisMotionTransitionEffect {
    Slide,
    Parallax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalTransitionEffect {
    FadeEdges,
    Cascade,
    Tilt,
    CylinderRoll,
    Wobbly,
    Wave,
    PerLineSpring,
    Liquid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalPostProcessTransitionEffect {
    MotionBlur,
    ChromaticAberration,
    GhostTrails,
    ColorTemperature,
    CRTScanlines,
    DepthOfField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalTransitionEffect {
    TypewriterReveal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum TransitionEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl TransitionEdge {
    pub const fn from_axis_direction(axis: TransitionAxis, direction: TransitionDirection) -> Self {
        match (axis, direction) {
            (TransitionAxis::Horizontal, TransitionDirection::Forward) => Self::Right,
            (TransitionAxis::Horizontal, TransitionDirection::Backward) => Self::Left,
            (TransitionAxis::Vertical, TransitionDirection::Forward) => Self::Bottom,
            (TransitionAxis::Vertical, TransitionDirection::Backward) => Self::Top,
        }
    }
}

/// Renderer-ready transition effect. Directionless effects cannot carry a
/// direction, while intrinsic text-flow effects cannot accidentally be asked
/// to run along an unsupported axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedTransitionEffect {
    Directionless(DirectionlessTransitionEffect),
    AxisMotion {
        effect: AxisMotionTransitionEffect,
        axis: TransitionAxis,
        direction: TransitionDirection,
        distance: f32,
    },
    CardFlip {
        axis: TransitionAxis,
    },
    PageCurl {
        edge: TransitionEdge,
    },
    Vertical {
        effect: VerticalTransitionEffect,
        direction: TransitionDirection,
        distance: f32,
    },
    VerticalPostProcess {
        effect: VerticalPostProcessTransitionEffect,
        direction: TransitionDirection,
        distance: f32,
    },
    Horizontal {
        effect: HorizontalTransitionEffect,
        direction: TransitionDirection,
    },
}
